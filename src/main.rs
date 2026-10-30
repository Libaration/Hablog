use chrono::Local;
use hablog::capture::CaptureWriter;
use hablog::logger::{escaped, ConsoleLogger};
use hablog::model::Direction;
use hablog::origins::crypto::{self, OriginsSession, StreamKind};
use hablog::origins::discovery::{self, DiscoverySnapshot, ObservedEndpoint};
use hablog::origins::fuse;
use hablog::origins::observer::{
    self, LocateError, ObserverConfig, ObserverEvent, ObserverHandle, ObserverRuntime,
};
use hablog::origins::routing::RoutePlan;
use hablog::proxy::{ProxyConfig, ProxyEvent, ProxyHandle};
use hablog::settings::{self, Settings, Source};
use hablog::wsl;
use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use tokio::process::Child;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

const HELP: &str = "\
Hablog relays Habbo Origins through your computer and logs its traffic.

  hablog           Start Origins through Hablog. It finds the installed client,
                   routes it through the relay, starts the game and logs its
                   traffic until the game closes or you press Ctrl+C.
  hablog details   Show the installed client's connection details.
  hablog decrypt   Decrypt a captured message with your session key.

Settings are in hablog.toml, which Hablog creates with an explanation of every
option the first time it runs.";

const SESSION_KEY_FILE: &str = "session.secret";
// Relay events handled per console/capture write.
const EVENT_BATCH: usize = 256;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    if args.next().is_some() {
        usage_error("Hablog takes no options; settings are in hablog.toml.");
    }
    match command.as_deref() {
        None => run().await,
        Some("details") => details_command(),
        Some("decrypt") => decrypt_command(),
        Some("help" | "--help" | "-h") => println!("{HELP}"),
        Some(other) => usage_error(&format!("Unknown command: {other}")),
    }
}

fn usage_error(message: &str) -> ! {
    eprintln!("{message}\n\n{HELP}");
    std::process::exit(2)
}

async fn run() {
    let settings = match Settings::load_or_create(Path::new(settings::FILE_NAME)) {
        Ok((settings, source)) => {
            match source {
                Source::File => ConsoleLogger::normal(format!("Settings: {}", settings::FILE_NAME)),
                Source::Created => ConsoleLogger::info(format!(
                    "Created {} with the default settings. Edit it to change how Hablog runs.",
                    settings::FILE_NAME
                )),
                Source::Defaults(reason) => {
                    ConsoleLogger::warning(format!("Using the default settings ({reason})."))
                }
            }
            settings
        }
        Err(e) => {
            ConsoleLogger::error(e);
            std::process::exit(1);
        }
    };

    let snapshot = discovery::discover_with(
        settings.advanced.appdata.clone(),
        settings.advanced.local_appdata.clone(),
    );
    let mut plan = match RoutePlan::from_discovery(&snapshot, settings.server_code()) {
        Ok(plan) => plan,
        Err(e) => {
            for warning in &snapshot.warnings {
                ConsoleLogger::warning(warning);
            }
            ConsoleLogger::error(e);
            ConsoleLogger::info(if snapshot.profile_root.is_some() {
                "Open Origins once through the Habbo Launcher so it's installed, then run Hablog again."
            } else if wsl::detected() {
                "Hablog couldn't find Windows' AppData folder from WSL. Set appdata and local_appdata under [advanced] in hablog.toml, for example '/mnt/c/Users/you/AppData/Roaming'."
            } else if cfg!(windows) {
                "Install Origins with the Habbo Launcher and open it once, then run Hablog again."
            } else {
                "Origins is a Windows game. Set appdata and local_appdata under [advanced] in hablog.toml to a Windows AppData folder, for example inside a Wine prefix."
            });
            std::process::exit(1);
        }
    };
    if let Some(upstream) = &settings.advanced.upstream {
        plan.upstream = upstream.clone();
    }
    ConsoleLogger::info(format!(
        "Found Origins {} ({}), game server {}",
        snapshot
            .installations
            .first()
            .map_or("?", |installation| installation.version.as_str()),
        plan.server_code.as_deref().unwrap_or("unknown hotel"),
        plan.upstream
    ));
    if !snapshot.running_clients.is_empty() {
        ConsoleLogger::warning(
            "Origins is already open and connected directly. Close it so its traffic goes through Hablog.",
        );
    }

    let (proxy, events) = match ProxyHandle::start(ProxyConfig {
        listen: plan.listen.clone(),
        upstream: plan.upstream.clone(),
    })
    .await
    {
        Ok(started) => started,
        Err(e) => {
            ConsoleLogger::error(e);
            ConsoleLogger::info("Is Hablog already running?");
            std::process::exit(1);
        }
    };
    let routed = match plan.write_patched_projector() {
        Ok(path) => path,
        Err(e) => {
            ConsoleLogger::error(e);
            std::process::exit(1);
        }
    };
    let game = if settings.launch {
        start_game(&settings, &plan, &routed).await
    } else {
        ConsoleLogger::info(format!("Start {} to play through Hablog.", shown(&routed)));
        Game::Manual
    };
    let log = TrafficLog::new(settings, plan.upstream);
    relay_loop(proxy, events, game, log).await;
}

/// The game Hablog is relaying for, and how to tell when it closes.
enum Game {
    /// Started by the user; Hablog runs until Ctrl+C.
    Manual,
    /// Started by Hablog; Hablog stops when it closes.
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

async fn start_game(settings: &Settings, plan: &RoutePlan, routed: &Path) -> Game {
    if settings.decrypt {
        if let Some(runtime) = observer_runtime(settings.advanced.observer.as_deref()).await {
            ConsoleLogger::info("Starting the game with decrypted traffic...");
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
                Ok((handle, events)) => {
                    return Game::Observed(Observed {
                        handle,
                        events,
                        spawned: false,
                        routed: routed.to_path_buf(),
                    })
                }
                Err(e) => ConsoleLogger::warning(format!(
                    "{e}. Starting the game without decrypted traffic."
                )),
            }
        }
    }
    launch_direct(routed)
}

fn launch_direct(routed: &Path) -> Game {
    if !wsl::runs_windows_programs() {
        ConsoleLogger::info(format!(
            "Origins runs on Windows: start {} there to play through Hablog.",
            routed.display()
        ));
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
            ConsoleLogger::success("Started the game.");
            Game::Direct(child)
        }
        Err(e) => {
            ConsoleLogger::error(format!("Cannot start {}: {e}", shown(routed)));
            ConsoleLogger::info("Start it yourself to play through Hablog.");
            Game::Manual
        }
    }
}

/// A path as the user would type it into Windows: under WSL, the Windows form.
fn shown(path: &Path) -> String {
    wsl::detected()
        .then(|| wsl::to_windows(path))
        .flatten()
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// Finds the observer, offering to install its dependencies the first time.
async fn observer_runtime(dir: Option<&Path>) -> Option<ObserverRuntime> {
    match ObserverRuntime::locate(dir) {
        Ok(runtime) => Some(runtime),
        Err(_) if !wsl::runs_windows_programs() => {
            ConsoleLogger::info("Decrypted traffic is only available on Windows.");
            None
        }
        Err(LocateError::NotInstalled(folder)) => {
            let question = format!(
                "Decrypted traffic needs Frida, which isn't installed yet. Install it now with `npm ci` in {}? [Y/n] ",
                folder.display()
            );
            if !std::io::stdin().is_terminal() || !confirm(&question) {
                ConsoleLogger::info(format!(
                    "Starting without decrypted traffic. Run `npm ci` in {} to set it up, or set decrypt = false in {} to stop this question.",
                    folder.display(),
                    settings::FILE_NAME
                ));
                return None;
            }
            match observer::install(&folder).await {
                Ok(()) => match ObserverRuntime::locate(dir) {
                    Ok(runtime) => Some(runtime),
                    Err(e) => {
                        ConsoleLogger::error(e);
                        None
                    }
                },
                Err(e) => {
                    ConsoleLogger::error(e);
                    None
                }
            }
        }
        Err(e) => {
            ConsoleLogger::info(format!("Decrypted traffic is off: {e}"));
            None
        }
    }
}

async fn relay_loop(
    proxy: ProxyHandle,
    mut events: mpsc::Receiver<ProxyEvent>,
    mut game: Game,
    mut log: TrafficLog,
) {
    let mut signals = ShutdownSignals::new();
    let mut stopping = false;
    let mut batch = Vec::with_capacity(EVENT_BATCH);
    let stop = |stopping: &mut bool, message: &str| {
        if !*stopping {
            *stopping = true;
            ConsoleLogger::normal(message);
            // Keep draining events so the final traffic and the disconnect
            // are still logged.
            proxy.shutdown();
        }
    };
    loop {
        tokio::select! {
            received = events.recv_many(&mut batch, EVENT_BATCH) => {
                if received == 0 {
                    break;
                }
                let mut stopped = false;
                for event in batch.drain(..) {
                    match event {
                        ProxyEvent::Listening { address } => log.listening(&address),
                        ProxyEvent::Connected { peer, upstream } => log.connected(&peer, &upstream),
                        ProxyEvent::Data { direction, bytes } => log.data(direction, &bytes),
                        ProxyEvent::Disconnected => log.disconnected(),
                        ProxyEvent::Error(error) => ConsoleLogger::error(error),
                        ProxyEvent::Stopped => stopped = true,
                    }
                }
                if stopped {
                    break;
                }
            }
            event = game.next() => match event {
                GameEvent::Closed => {
                    game = Game::Ended;
                    stop(&mut stopping, "The game closed. Stopping Hablog...");
                }
                GameEvent::Observer(ObserverEvent::Spawned) => {
                    if let Game::Observed(observed) = &mut game {
                        observed.spawned = true;
                    }
                }
                GameEvent::Observer(ObserverEvent::Detached { reason }) if reason == "process-terminated" => {
                    game.stop().await;
                    stop(&mut stopping, "The game closed. Stopping Hablog...");
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
                            ConsoleLogger::warning(match event {
                                ObserverEvent::Error(error) => format!(
                                    "Decrypted traffic isn't available ({error}). Starting the game normally."
                                ),
                                _ => "The observer stopped before starting the game. Starting it normally.".into(),
                            });
                            game.stop().await;
                            game = launch_direct(&routed);
                        }
                        None => {
                            let exited = matches!(event, ObserverEvent::Exited);
                            log.observer(event);
                            if exited {
                                // The game keeps running unobserved; Hablog
                                // can no longer tell when it closes.
                                game.stop().await;
                            }
                        }
                    }
                }
                GameEvent::Observer(event) => log.observer(event),
            },
            _ = signals.recv() => {
                if stopping {
                    ConsoleLogger::warning("Stopping immediately.");
                    break;
                }
                stop(&mut stopping, "Stopping Hablog. The game stays open but loses its connection...");
            }
        }
        log.flush();
    }
    log.flush();
    game.stop().await;
    ConsoleLogger::normal("Stopped.");
}

#[cfg(unix)]
struct ShutdownSignals {
    interrupt: tokio::signal::unix::Signal,
    terminate: tokio::signal::unix::Signal,
}

#[cfg(unix)]
impl ShutdownSignals {
    fn new() -> Self {
        use tokio::signal::unix::{signal, SignalKind};
        ShutdownSignals {
            interrupt: signal(SignalKind::interrupt()).expect("Failed to set up signal handler"),
            terminate: signal(SignalKind::terminate()).expect("Failed to set up signal handler"),
        }
    }

    async fn recv(&mut self) {
        tokio::select! {
            _ = self.interrupt.recv() => {}
            _ = self.terminate.recv() => {}
        }
    }
}

#[cfg(windows)]
struct ShutdownSignals {
    ctrl_c: tokio::signal::windows::CtrlC,
    ctrl_close: tokio::signal::windows::CtrlClose,
}

#[cfg(windows)]
impl ShutdownSignals {
    fn new() -> Self {
        use tokio::signal::windows::{ctrl_c, ctrl_close};
        ShutdownSignals {
            ctrl_c: ctrl_c().expect("Failed to set up Ctrl+C handler"),
            ctrl_close: ctrl_close().expect("Failed to set up console close handler"),
        }
    }

    async fn recv(&mut self) {
        tokio::select! {
            _ = self.ctrl_c.recv() => {}
            _ = self.ctrl_close.recv() => {}
        }
    }
}

/// The console log and capture file for the current game session.
struct TrafficLog {
    settings: Settings,
    upstream: String,
    sessions: u64,
    capture: Option<CaptureWriter>,
}

impl TrafficLog {
    fn new(settings: Settings, upstream: String) -> Self {
        TrafficLog {
            settings,
            upstream,
            sessions: 0,
            capture: None,
        }
    }

    fn listening(&self, address: &str) {
        ConsoleLogger::info(format!(
            "Relay ready on {address}, forwarding to {}",
            self.upstream
        ));
    }

    fn connected(&mut self, peer: &str, upstream: &str) {
        self.sessions += 1;
        ConsoleLogger::success(format!(
            "The game connected ({peer}), relaying to {upstream}"
        ));
        if let Some(dir) = self.settings.captures() {
            let path = dir.join(format!(
                "hablog-{}-{}.jsonl",
                Local::now().format("%Y%m%d-%H%M%S"),
                self.sessions
            ));
            match CaptureWriter::create(&path, peer, upstream) {
                Ok(capture) => {
                    ConsoleLogger::info(format!("Saving traffic to {}", path.display()));
                    self.capture = Some(capture);
                }
                Err(e) => ConsoleLogger::error(e),
            }
        }
    }

    fn data(&mut self, direction: Direction, bytes: &[u8]) {
        if let Some(capture) = self.capture.as_mut() {
            if let Err(e) = capture.record(direction, bytes) {
                ConsoleLogger::error(format!("{e}. Saving stopped for this session."));
                self.capture = None;
            }
        }
        if self.settings.shows(direction) {
            ConsoleLogger::log_chunk(direction, bytes);
        }
    }

    fn disconnected(&mut self) {
        match self.capture.take() {
            Some(mut capture) => {
                if let Err(e) = capture.flush() {
                    ConsoleLogger::error(e);
                }
                ConsoleLogger::normal(format!(
                    "The game disconnected. Saved {} TCP chunks to {}",
                    capture.records(),
                    capture.path().display()
                ))
            }
            None => ConsoleLogger::normal("The game disconnected."),
        }
    }

    fn observer(&mut self, event: ObserverEvent) {
        match event {
            ObserverEvent::Status { state, detail } => match state.as_str() {
                "observing" => ConsoleLogger::success("Decrypted traffic is on."),
                "client_resumed" => ConsoleLogger::success("Started the game."),
                "signature_mismatch_proceeding" => ConsoleLogger::warning(format!(
                    "The game's crypto module differs from the verified one ({detail}); decrypting anyway."
                )),
                "module_unloaded" => {
                    ConsoleLogger::warning("The game unloaded its crypto module.")
                }
                _ => {}
            },
            ObserverEvent::Crypto(op) => {
                if !self.settings.shows(op.direction) {
                    return;
                }
                // Metadata-only operations carry no bytes.
                match op.plaintext_hex().map(hex::decode) {
                    Some(Ok(plaintext)) => {
                        // Logs get shared; the login password stays off the screen.
                        let masked = (op.direction == Direction::ClientToServer
                            && op.kind == "payload")
                            .then(|| fuse::mask_login_password(&plaintext))
                            .flatten();
                        ConsoleLogger::log_decrypted(
                            op.direction,
                            &op.kind,
                            &op.sequence,
                            masked.as_deref().unwrap_or(&plaintext),
                        )
                    }
                    Some(Err(_)) => ConsoleLogger::warning(format!(
                        "The observer sent unreadable bytes for {} #{}",
                        op.operation, op.sequence
                    )),
                    None => {}
                }
            }
            ObserverEvent::SocketConnect { host, port, .. } => {
                ConsoleLogger::normal(format!("The game is connecting to {host}:{port}."))
            }
            ObserverEvent::Dropped(count) => ConsoleLogger::warning(format!(
                "The observer skipped {count} operations because its buffer was full."
            )),
            ObserverEvent::Error(error) => ConsoleLogger::error(format!("Observer: {error}")),
            ObserverEvent::Detached { .. } | ObserverEvent::Exited => ConsoleLogger::warning(
                "The observer stopped, so decrypted traffic is no longer shown.",
            ),
            ObserverEvent::Spawned => {}
        }
    }

    /// Writes the batch's console lines and capture records.
    fn flush(&mut self) {
        if let Some(capture) = self.capture.as_mut() {
            if let Err(e) = capture.flush() {
                ConsoleLogger::error(format!("{e}. Saving stopped for this session."));
                self.capture = None;
            }
        }
        ConsoleLogger::flush();
    }
}

fn details_command() {
    let settings = match Settings::load(Path::new(settings::FILE_NAME)) {
        Ok(settings) => settings,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let snapshot =
        discovery::discover_with(settings.advanced.appdata, settings.advanced.local_appdata);
    for line in details(&snapshot) {
        println!("{line}");
    }
    for warning in &snapshot.warnings {
        println!("Warning: {warning}");
    }
}

/// The discovered launcher, installations, endpoints and running clients.
fn details(snapshot: &DiscoverySnapshot) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(launcher) = &snapshot.launcher_executable {
        lines.push(format!(
            "Launcher: {} (version {})",
            launcher.display(),
            snapshot.launcher_version.as_deref().unwrap_or("unknown")
        ));
    }
    if let Some(profile) = &snapshot.profile_root {
        lines.push(format!("Launcher profile: {}", profile.display()));
    }
    if let Some(version) = &snapshot.preferred_version {
        lines.push(format!("Launcher's selected version: {version}"));
    }
    if snapshot.installations.is_empty() {
        lines.push("No Origins installation was found.".into());
    }
    for installation in &snapshot.installations {
        lines.push(format!(
            "Origins {} at {}{}",
            installation.version,
            installation.root.display(),
            if installation.registered {
                ""
            } else {
                " (not registered with the launcher)"
            }
        ));
        match &installation.current_default_executable {
            Some(_) => lines.push(format!(
                "  Habbo.exe:  game {} ({})",
                endpoint_text(installation.current_default_info_endpoint.as_ref()),
                installation
                    .current_default_server_code
                    .as_deref()
                    .unwrap_or("unknown hotel")
            )),
            None => lines.push("  Habbo.exe:  missing".into()),
        }
        for regional in &installation.regional_executables {
            lines.push(format!(
                "  {:<9} game {}, MUS {}  ({})",
                format!(
                    "{}{}",
                    regional.server_code,
                    if regional.large { " XL" } else { "" }
                ),
                endpoint_text(regional.info_endpoint.as_ref()),
                endpoint_text(regional.mus_endpoint.as_ref()),
                regional
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
            ));
            if let Some(site) = &regional.site_url {
                lines.push(format!("            site {}", site.value));
            }
            if let Some(variables) = &regional.external_variables_url {
                lines.push(format!(
                    "            external variables {}",
                    variables.value
                ));
            }
        }
    }
    for client in &snapshot.running_clients {
        lines.push(format!(
            "Running client: pid {} {} ({})",
            client.pid,
            client.path.display(),
            client.server_code.as_deref().unwrap_or("unknown hotel")
        ));
    }
    lines
}

fn endpoint_text(endpoint: Option<&ObservedEndpoint>) -> String {
    endpoint.map_or_else(
        || "unknown".into(),
        |endpoint| format!("{}:{}", endpoint.host, endpoint.port),
    )
}

/// Asks for each value, remembering the previous answers as defaults.
fn decrypt_command() {
    let key = match std::fs::read_to_string(SESSION_KEY_FILE) {
        Ok(key) => {
            println!("Using the session key in {SESSION_KEY_FILE}.");
            Zeroizing::new(key)
        }
        Err(_) => match ask(
            "Session key (112 hex characters, from the client's Crypto_GetSharedKeyHex)",
            "",
        ) {
            Some(key) => Zeroizing::new(key),
            None => return,
        },
    };
    let session = match OriginsSession::from_shared_key_hex(&key) {
        Ok(session) => session,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!("Press Enter on an empty ciphertext to quit.");
    let mut direction = "in".to_string();
    let mut stream = "payload".to_string();
    let mut sequence = 0u64;
    loop {
        let Some(wire) = ask("Ciphertext (Base64, as sent on the wire)", "") else {
            return;
        };
        if wire.is_empty() {
            return;
        }
        let Some(answer) = ask(
            "Direction: in (server to game) or out (game to server)",
            &direction,
        ) else {
            return;
        };
        direction = answer;
        let Some(answer) = ask("Stream: payload or header", &stream) else {
            return;
        };
        stream = answer;
        let Some(answer) = ask(
            "Sequence (that stream's counter, from 0)",
            &sequence.to_string(),
        ) else {
            return;
        };
        let result = (|| {
            let direction = match direction.as_str() {
                "in" => Direction::ServerToClient,
                "out" => Direction::ClientToServer,
                _ => return Err("Direction must be in or out.".to_string()),
            };
            let kind = match stream.as_str() {
                "payload" => StreamKind::Payload,
                "header" => StreamKind::Header,
                _ => return Err("Stream must be payload or header.".to_string()),
            };
            sequence = answer
                .parse()
                .map_err(|_| "The sequence must be a whole number.".to_string())?;
            let ciphertext = crypto::decode_wire(&wire)?;
            session.transform(direction, kind, sequence, &ciphertext)
        })();
        match result {
            Ok(plaintext) => {
                println!("Plaintext: {}", escaped(&plaintext, usize::MAX));
                println!("      hex: {}", hex::encode(&plaintext));
                sequence = sequence.wrapping_add(1);
            }
            Err(e) => println!("{e}"),
        }
    }
}

/// Prints `question`, returns the trimmed answer or `default` for an empty
/// one, and None once input ends.
fn ask(question: &str, default: &str) -> Option<String> {
    if default.is_empty() {
        print!("{question}: ");
    } else {
        print!("{question} [{default}]: ");
    }
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    match std::io::stdin().lock().read_line(&mut answer) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            let answer = answer.trim();
            Some(if answer.is_empty() { default } else { answer }.to_owned())
        }
    }
}

fn confirm(question: &str) -> bool {
    print!("{question}");
    let _ = std::io::stdout().flush();
    let mut answer = String::new();
    if std::io::stdin().lock().read_line(&mut answer).is_err() {
        return false;
    }
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    )
}
