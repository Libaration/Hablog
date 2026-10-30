//! One Hablog session: the relay, the routed game and the observer, run
//! together. A front end (the CLI or the GUI) prepares it, starts it and
//! shows its events; the session itself prints nothing. Login passwords are
//! masked here, so no front end ever receives one.
use crate::capture::CaptureWriter;
use crate::model::{hex_bytes, now_ms, Direction};
use crate::origins::discovery::{self, DiscoverySnapshot};
use crate::origins::fuse;
use crate::origins::observer::{ObserverConfig, ObserverEvent, ObserverHandle, ObserverRuntime};
use crate::origins::routing::RoutePlan;
use crate::proxy::{ProxyConfig, ProxyEvent, ProxyHandle};
use crate::settings::Settings;
use crate::wsl;
use chrono::Local;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tokio::process::Child;
use tokio::sync::mpsc;

// Relay events handled per capture write.
const EVENT_BATCH: usize = 256;
const EVENT_CAPACITY: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Normal,
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RawKind {
    /// A TCP chunk exactly as relayed.
    Wire,
    /// The decrypted frame header in front of a message.
    Frame,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    /// A line for the user.
    Status {
        level: Level,
        text: String,
    },
    /// The relay is listening.
    Ready {
        listen: String,
        upstream: String,
    },
    Connected {
        peer: String,
        upstream: String,
        capture: Option<String>,
    },
    Disconnected {
        capture: Option<String>,
        chunks: u64,
    },
    /// Whether the observer is reporting decrypted messages.
    Decrypting {
        on: bool,
    },
    /// One game message, header included, exactly as sent (but with the
    /// login password masked).
    Message {
        time_ms: u64,
        direction: Direction,
        #[serde(serialize_with = "hex_bytes::serialize")]
        bytes: Vec<u8>,
        decrypted: bool,
    },
    /// Raw traffic. `plain` is true for a chunk that held unencrypted
    /// messages, which are also reported as `Message`s.
    Raw {
        time_ms: u64,
        direction: Direction,
        raw: RawKind,
        #[serde(serialize_with = "hex_bytes::serialize")]
        bytes: Vec<u8>,
        plain: bool,
    },
    /// The game is opening a connection.
    Connecting {
        host: String,
        port: u16,
    },
    /// Bytes relayed so far in each direction.
    Traffic {
        bytes_in: u64,
        bytes_out: u64,
    },
    Stopped,
}

/// What a session needs from discovery: the installation and the route.
#[derive(Clone, Debug, Serialize)]
pub struct Prepared {
    pub snapshot: DiscoverySnapshot,
    pub plan: RoutePlan,
}

impl Prepared {
    pub fn version(&self) -> &str {
        self.snapshot
            .installations
            .first()
            .map_or("?", |installation| installation.version.as_str())
    }
}

/// Why a session can't start, and what to do about it.
#[derive(Clone, Debug, Serialize)]
pub struct Problem {
    pub error: String,
    pub hint: Option<String>,
    pub warnings: Vec<String>,
}

/// Finds the client and plans its route, without changing anything.
pub fn prepare(settings: &Settings) -> Result<Prepared, Problem> {
    let snapshot = discovery::discover_with(
        settings.advanced.appdata.clone(),
        settings.advanced.local_appdata.clone(),
    );
    let mut plan = match RoutePlan::from_discovery(&snapshot, settings.server_code()) {
        Ok(plan) => plan,
        Err(error) => {
            let hint = if snapshot.profile_root.is_some() {
                "Open Origins once through the Habbo Launcher so it's installed, then try again."
            } else if wsl::detected() {
                "Hablog couldn't find Windows' AppData folder from WSL. Set appdata and local_appdata under [advanced] in hablog.toml, for example '/mnt/c/Users/you/AppData/Roaming'."
            } else if cfg!(windows) {
                "Install Origins with the Habbo Launcher and open it once, then try again."
            } else {
                "Origins is a Windows game. Set appdata and local_appdata under [advanced] in hablog.toml to a Windows AppData folder, for example inside a Wine prefix."
            };
            return Err(Problem {
                error,
                hint: Some(hint.into()),
                warnings: snapshot.warnings.clone(),
            });
        }
    };
    if let Some(upstream) = &settings.advanced.upstream {
        plan.upstream = upstream.clone();
    }
    Ok(Prepared { snapshot, plan })
}

/// Controls a running session.
pub struct SessionHandle {
    stop: mpsc::UnboundedSender<()>,
}

impl SessionHandle {
    /// Asks the session to stop; it still reports the final traffic, then
    /// `Stopped`. Asking a second time stops it at once. The game stays
    /// open either way.
    pub fn stop(&self) {
        let _ = self.stop.send(());
    }
}

/// Starts the relay, writes the routed client and starts the game: through
/// `observer` when given, so its traffic is decrypted.
pub async fn start(
    settings: Settings,
    prepared: Prepared,
    observer: Option<ObserverRuntime>,
) -> Result<(SessionHandle, mpsc::Receiver<SessionEvent>), Problem> {
    let plan = prepared.plan;
    let problem = |error: String, hint: Option<&str>| Problem {
        error,
        hint: hint.map(str::to_owned),
        warnings: Vec::new(),
    };
    let (proxy, proxy_events) = ProxyHandle::start(ProxyConfig {
        listen: plan.listen.clone(),
        upstream: plan.upstream.clone(),
    })
    .await
    .map_err(|e| problem(e, Some("Is Hablog already running?")))?;
    let routed = plan
        .write_patched_projector()
        .map_err(|e| problem(e, None))?;

    let (tx, rx) = mpsc::channel(EVENT_CAPACITY);
    let events = Events(tx);
    let game = if settings.launch {
        start_game(&events, &plan, &routed, observer).await
    } else {
        events
            .status(
                Level::Info,
                format!(
                    "Start {} to play through Hablog.",
                    wsl::display_path(&routed)
                ),
            )
            .await;
        Game::Manual
    };
    let (stop, requests) = mpsc::unbounded_channel();
    let upstream = plan.upstream.clone();
    tokio::spawn(run(
        settings,
        upstream,
        proxy,
        proxy_events,
        game,
        events,
        requests,
    ));
    Ok((SessionHandle { stop }, rx))
}

#[derive(Clone)]
struct Events(mpsc::Sender<SessionEvent>);

impl Events {
    async fn send(&self, event: SessionEvent) {
        // Fails only when the front end is gone.
        let _ = self.0.send(event).await;
    }

    async fn status(&self, level: Level, text: impl Into<String>) {
        self.send(SessionEvent::Status {
            level,
            text: text.into(),
        })
        .await;
    }
}

/// The game Hablog is relaying for, and how to tell when it closes.
enum Game {
    /// Started by the user; the session runs until it is stopped.
    Manual,
    /// Started by Hablog; the session stops when it closes.
    Direct(Child),
    Observed(Observed),
    /// Closed, or no longer tracked.
    Ended,
}

struct Observed {
    handle: ObserverHandle,
    events: mpsc::Receiver<ObserverEvent>,
    spawned: bool,
    routed: PathBuf,
}

enum GameEvent {
    Closed,
    Observer(ObserverEvent),
}

impl Game {
    async fn next(&mut self) -> GameEvent {
        match self {
            Game::Direct(child) => {
                let _ = child.wait().await;
                GameEvent::Closed
            }
            Game::Observed(observed) => GameEvent::Observer(
                observed
                    .events
                    .recv()
                    .await
                    .unwrap_or(ObserverEvent::Exited),
            ),
            Game::Manual | Game::Ended => std::future::pending().await,
        }
    }

    /// Detaches the observer, if any. The game itself is left running.
    async fn stop(&mut self) {
        if let Game::Observed(observed) = self {
            observed.handle.stop().await;
        }
        *self = Game::Ended;
    }
}

async fn start_game(
    events: &Events,
    plan: &RoutePlan,
    routed: &Path,
    observer: Option<ObserverRuntime>,
) -> Game {
    if let Some(runtime) = observer {
        events
            .status(Level::Info, "Starting the game with decrypted traffic...")
            .await;
        match ObserverHandle::start(ObserverConfig {
            node_exe: runtime.node_exe,
            host_script: runtime.host_script,
            client_exe: routed.to_path_buf(),
            module_path: plan.module_path.clone(),
            launch: true,
            argv: Vec::new(),
        })
        .await
        {
            Ok((handle, observer_events)) => {
                return Game::Observed(Observed {
                    handle,
                    events: observer_events,
                    spawned: false,
                    routed: routed.to_path_buf(),
                })
            }
            Err(e) => {
                events
                    .status(
                        Level::Warning,
                        format!("{e}. Starting the game without decrypted traffic."),
                    )
                    .await
            }
        }
    }
    launch_direct(events, routed).await
}

async fn launch_direct(events: &Events, routed: &Path) -> Game {
    if !wsl::runs_windows_programs() {
        events
            .status(
                Level::Info,
                format!(
                    "Origins runs on Windows: start {} there to play through Hablog.",
                    routed.display()
                ),
            )
            .await;
        return Game::Manual;
    }
    // Under WSL, interop runs the Windows program and this child stands in
    // for it until it exits.
    let mut command = tokio::process::Command::new(routed);
    if let Some(parent) = routed.parent() {
        command.current_dir(parent);
    }
    match command.spawn() {
        Ok(child) => {
            events.status(Level::Success, "Started the game.").await;
            Game::Direct(child)
        }
        Err(e) => {
            events
                .status(
                    Level::Error,
                    format!("Cannot start {}: {e}", wsl::display_path(routed)),
                )
                .await;
            events
                .status(Level::Info, "Start it yourself to play through Hablog.")
                .await;
            Game::Manual
        }
    }
}

async fn run(
    settings: Settings,
    upstream: String,
    proxy: ProxyHandle,
    mut proxy_events: mpsc::Receiver<ProxyEvent>,
    mut game: Game,
    events: Events,
    requests: mpsc::UnboundedReceiver<()>,
) {
    let mut requests = Some(requests);
    let mut recorder = Recorder::new(settings, events.clone());
    let mut stopping = false;
    let mut batch = Vec::with_capacity(EVENT_BATCH);
    loop {
        tokio::select! {
            received = proxy_events.recv_many(&mut batch, EVENT_BATCH) => {
                if received == 0 {
                    break;
                }
                let mut stopped = false;
                for event in batch.drain(..) {
                    match event {
                        ProxyEvent::Listening { address } => {
                            events
                                .send(SessionEvent::Ready { listen: address, upstream: upstream.clone() })
                                .await
                        }
                        ProxyEvent::Connected { peer, upstream } => recorder.connected(&peer, &upstream).await,
                        ProxyEvent::Data { direction, bytes } => recorder.data(direction, &bytes).await,
                        ProxyEvent::Disconnected => recorder.disconnected().await,
                        ProxyEvent::Error(error) => events.status(Level::Error, error).await,
                        ProxyEvent::Stopped => stopped = true,
                    }
                }
                recorder.traffic().await;
                if stopped {
                    break;
                }
            }
            event = game.next() => match event {
                GameEvent::Closed => {
                    game = Game::Ended;
                    begin_stop(&mut stopping, &proxy, &events, "The game closed. Stopping Hablog...").await;
                }
                GameEvent::Observer(ObserverEvent::Spawned) => {
                    if let Game::Observed(observed) = &mut game {
                        observed.spawned = true;
                    }
                }
                GameEvent::Observer(ObserverEvent::Detached { reason }) if reason == "process-terminated" => {
                    game.stop().await;
                    begin_stop(&mut stopping, &proxy, &events, "The game closed. Stopping Hablog...").await;
                }
                GameEvent::Observer(event @ (ObserverEvent::Error(_) | ObserverEvent::Exited)) => {
                    let unstarted = match &game {
                        Game::Observed(observed) if !observed.spawned => Some(observed.routed.clone()),
                        _ => None,
                    };
                    match unstarted {
                        // The observer failed before the game existed, so
                        // start it without decrypted traffic instead.
                        Some(routed) => {
                            let text = match event {
                                ObserverEvent::Error(error) => format!(
                                    "Decrypted traffic isn't available ({error}). Starting the game normally."
                                ),
                                _ => "The observer stopped before starting the game. Starting it normally.".into(),
                            };
                            events.status(Level::Warning, text).await;
                            game.stop().await;
                            game = launch_direct(&events, &routed).await;
                        }
                        None => {
                            let exited = matches!(event, ObserverEvent::Exited);
                            recorder.observer(event).await;
                            if exited {
                                // The game keeps running unobserved; Hablog
                                // can no longer tell when it closes.
                                game.stop().await;
                            }
                        }
                    }
                }
                GameEvent::Observer(event) => recorder.observer(event).await,
            },
            () = next_request(&mut requests) => {
                if stopping {
                    events.status(Level::Warning, "Stopping immediately.").await;
                    break;
                }
                begin_stop(
                    &mut stopping,
                    &proxy,
                    &events,
                    "Stopping Hablog. The game stays open but loses its connection...",
                )
                .await;
            }
        }
        recorder.flush().await;
    }
    recorder.flush().await;
    game.stop().await;
    events.send(SessionEvent::Stopped).await;
}

/// The next stop request. A dropped handle counts as one, once.
async fn next_request(requests: &mut Option<mpsc::UnboundedReceiver<()>>) {
    match requests {
        Some(receiver) => {
            if receiver.recv().await.is_none() {
                *requests = None;
            }
        }
        None => std::future::pending().await,
    }
}

async fn begin_stop(stopping: &mut bool, proxy: &ProxyHandle, events: &Events, message: &str) {
    if !*stopping {
        *stopping = true;
        events.status(Level::Normal, message).await;
        // Keep draining events so the final traffic and the disconnect are
        // still reported.
        proxy.shutdown();
    }
}

/// Turns relay and observer events into session events, and writes the
/// capture file for the current game connection.
struct Recorder {
    settings: Settings,
    events: Events,
    sessions: u64,
    capture: Option<CaptureWriter>,
    bytes: [u64; 2],
    reported: [u64; 2],
}

impl Recorder {
    fn new(settings: Settings, events: Events) -> Self {
        Recorder {
            settings,
            events,
            sessions: 0,
            capture: None,
            bytes: [0; 2],
            reported: [0; 2],
        }
    }

    async fn connected(&mut self, peer: &str, upstream: &str) {
        self.sessions += 1;
        let mut capture_path = None;
        if let Some(dir) = self.settings.captures() {
            let path = dir.join(format!(
                "hablog-{}-{}.jsonl",
                Local::now().format("%Y%m%d-%H%M%S"),
                self.sessions
            ));
            match CaptureWriter::create(&path, peer, upstream) {
                Ok(capture) => {
                    capture_path = Some(path.display().to_string());
                    self.capture = Some(capture);
                }
                Err(e) => self.events.status(Level::Error, e).await,
            }
        }
        self.events
            .send(SessionEvent::Connected {
                peer: peer.into(),
                upstream: upstream.into(),
                capture: capture_path,
            })
            .await;
    }

    async fn data(&mut self, direction: Direction, bytes: &[u8]) {
        if let Some(capture) = self.capture.as_mut() {
            if let Err(e) = capture.record(direction, bytes) {
                self.capture = None;
                self.events
                    .status(
                        Level::Error,
                        format!("{e}. Saving stopped for this session."),
                    )
                    .await;
            }
        }
        self.bytes[direction_index(direction)] += bytes.len() as u64;
        let plain = fuse::plain_messages(direction, bytes);
        let time_ms = now_ms();
        self.events
            .send(SessionEvent::Raw {
                time_ms,
                direction,
                raw: RawKind::Wire,
                bytes: bytes.to_vec(),
                plain: plain.is_some(),
            })
            .await;
        // Before encryption starts, the handshake travels as plain messages.
        for message in plain.unwrap_or_default() {
            self.message(time_ms, direction, message, false).await;
        }
    }

    async fn disconnected(&mut self) {
        let (capture, chunks) = match self.capture.take() {
            Some(mut capture) => {
                if let Err(e) = capture.flush() {
                    self.events.status(Level::Error, e).await;
                }
                (
                    Some(capture.path().display().to_string()),
                    capture.records(),
                )
            }
            None => (None, 0),
        };
        self.events
            .send(SessionEvent::Disconnected { capture, chunks })
            .await;
    }

    async fn observer(&mut self, event: ObserverEvent) {
        let events = self.events.clone();
        match event {
            ObserverEvent::Status { state, detail } => match state.as_str() {
                "observing" => {
                    events.send(SessionEvent::Decrypting { on: true }).await;
                    events.status(Level::Success, "Decrypted traffic is on.").await;
                }
                "client_resumed" => events.status(Level::Success, "Started the game.").await,
                "signature_mismatch_proceeding" => {
                    events
                        .status(
                            Level::Warning,
                            format!("The game's crypto module differs from the verified one ({detail}); decrypting anyway."),
                        )
                        .await
                }
                "module_unloaded" => {
                    events
                        .status(Level::Warning, "The game unloaded its crypto module.")
                        .await
                }
                _ => {}
            },
            ObserverEvent::Crypto(op) => match op.plaintext_hex().map(hex::decode) {
                Some(Ok(plaintext)) if op.kind == "payload" => {
                    let time_ms = now_ms();
                    for message in fuse::split_payload(op.direction, &plaintext) {
                        self.message(time_ms, op.direction, message, true).await;
                    }
                }
                // The frame header only carries the payload's length.
                Some(Ok(plaintext)) => {
                    events
                        .send(SessionEvent::Raw {
                            time_ms: now_ms(),
                            direction: op.direction,
                            raw: RawKind::Frame,
                            bytes: plaintext,
                            plain: true,
                        })
                        .await
                }
                Some(Err(_)) => {
                    events
                        .status(
                            Level::Warning,
                            format!(
                                "The observer sent unreadable bytes for {} #{}",
                                op.operation, op.sequence
                            ),
                        )
                        .await
                }
                // Metadata-only operations carry no bytes.
                None => {}
            },
            ObserverEvent::SocketConnect { host, port, .. } => {
                events.send(SessionEvent::Connecting { host, port }).await
            }
            ObserverEvent::Dropped(count) => {
                events
                    .status(
                        Level::Warning,
                        format!("The observer skipped {count} operations because its buffer was full."),
                    )
                    .await
            }
            ObserverEvent::Error(error) => {
                events.status(Level::Error, format!("Observer: {error}")).await
            }
            ObserverEvent::Detached { .. } | ObserverEvent::Exited => {
                events.send(SessionEvent::Decrypting { on: false }).await;
                events
                    .status(
                        Level::Warning,
                        "The observer stopped, so decrypted traffic is no longer shown.",
                    )
                    .await;
            }
            ObserverEvent::Spawned => {}
        }
    }

    /// One message. Logs get shared, so the login password is masked.
    async fn message(&self, time_ms: u64, direction: Direction, message: &[u8], decrypted: bool) {
        let masked = (direction == Direction::ClientToServer)
            .then(|| fuse::mask_login_password(message))
            .flatten();
        self.events
            .send(SessionEvent::Message {
                time_ms,
                direction,
                bytes: masked.unwrap_or_else(|| message.to_vec()),
                decrypted,
            })
            .await;
    }

    /// Reports the byte counters when they changed.
    async fn traffic(&mut self) {
        if self.bytes != self.reported {
            self.reported = self.bytes;
            self.events
                .send(SessionEvent::Traffic {
                    bytes_in: self.bytes[direction_index(Direction::ServerToClient)],
                    bytes_out: self.bytes[direction_index(Direction::ClientToServer)],
                })
                .await;
        }
    }

    async fn flush(&mut self) {
        if let Some(capture) = self.capture.as_mut() {
            if let Err(e) = capture.flush() {
                self.capture = None;
                self.events
                    .status(
                        Level::Error,
                        format!("{e}. Saving stopped for this session."),
                    )
                    .await;
            }
        }
    }
}

fn direction_index(direction: Direction) -> usize {
    match direction {
        Direction::ServerToClient => 0,
        Direction::ClientToServer => 1,
    }
}
