mod cli;

use chrono::Local;
use cli::{
    Command, CryptoMode, CryptoOptions, DetailsOptions, HabboOptions, Launch, LogOptions,
    OriginsOptions, SESSION_KEY_ENV,
};
use hablog::capture::CaptureWriter;
use hablog::logger::{escaped, ConsoleLogger};
use hablog::model::Direction;
use hablog::origins::crypto::{self, OriginsSession};
use hablog::origins::discovery::{self, DiscoverySnapshot, ObservedEndpoint};
use hablog::origins::observer::{ObserverConfig, ObserverEvent, ObserverHandle, ObserverRuntime};
use hablog::origins::routing::RoutePlan;
use hablog::packet_handler::packet_handler::{PacketHandler, PacketNames};
use hablog::proxy::{ProxyConfig, ProxyEvent, ProxyHandle};
use std::path::Path;
use tokio::sync::mpsc;
use zeroize::Zeroizing;

type Observer = (ObserverHandle, mpsc::Receiver<ObserverEvent>);

#[tokio::main]
async fn main() {
    match cli::parse() {
        Command::Habbo(options) => run_habbo(options).await,
        Command::Origins(options) => run_origins(options).await,
        Command::OriginsDetails(options) => origins_details(options),
        Command::OriginsCrypto(options) => {
            if let Err(e) = origins_crypto(options) {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    }
}

async fn run_habbo(options: HabboOptions) {
    let names = match PacketNames::fetch().await {
        Ok(names) => {
            ConsoleLogger::normal(format!("Loaded {} packet names.", names.count()));
            names
        }
        Err(e) => {
            ConsoleLogger::warning(format!(
                "Packet names unavailable ({e}); packets will show headers only."
            ));
            PacketNames::default()
        }
    };

    ConsoleLogger::normal("Preparing connection...");
    let (proxy, events) = start_relay(&options.listen, &options.upstream).await;
    let log = TrafficLog::new(options.log, options.upstream, Some(names));
    relay_loop(proxy, events, None, log).await;
}

async fn run_origins(options: OriginsOptions) {
    let snapshot = discovery::discover_with(options.roots.appdata, options.roots.local_appdata);
    for line in details(&snapshot) {
        ConsoleLogger::normal(line);
    }
    for warning in &snapshot.warnings {
        ConsoleLogger::warning(warning);
    }
    let mut plan = match RoutePlan::from_discovery(&snapshot, options.server.as_deref()) {
        Ok(plan) => plan,
        Err(e) => {
            ConsoleLogger::error(e);
            std::process::exit(1);
        }
    };
    if let Some(upstream) = options.upstream {
        plan.upstream = upstream;
    }
    ConsoleLogger::info(format!(
        "Routing {} ({}) to {}",
        plan.source_exe.display(),
        plan.server_code.as_deref().unwrap_or("unknown region"),
        plan.upstream
    ));
    if !snapshot.running_clients.is_empty() {
        ConsoleLogger::warning(
            "An Origins client is already running. It is connected directly, so its traffic doesn't go through Hablog.",
        );
    }

    let (proxy, events) = start_relay(&plan.listen, &plan.upstream).await;
    let routed = match plan.write_patched_projector() {
        Ok(path) => path,
        Err(e) => {
            ConsoleLogger::error(e);
            std::process::exit(1);
        }
    };
    ConsoleLogger::success(format!(
        "Wrote {}, a copy of the client that connects to {}",
        routed.display(),
        plan.listen
    ));
    let observer = match options.launch {
        Launch::Manual => {
            ConsoleLogger::info(format!(
                "Start {} to play through Hablog.",
                routed.display()
            ));
            None
        }
        Launch::Direct => {
            launch_direct(&routed);
            None
        }
        Launch::Observe => {
            match start_observer(options.observer_dir.as_deref(), &plan, &routed).await {
                Ok(observer) => Some(observer),
                Err(e) => {
                    ConsoleLogger::error(e);
                    ConsoleLogger::info(format!(
                        "The relay is still running. Start {} yourself, or run without --observe.",
                        routed.display()
                    ));
                    None
                }
            }
        }
    };
    let log = TrafficLog::new(options.log, plan.upstream, None);
    relay_loop(proxy, events, observer, log).await;
}

async fn start_relay(listen: &str, upstream: &str) -> (ProxyHandle, mpsc::Receiver<ProxyEvent>) {
    match ProxyHandle::start(ProxyConfig {
        listen: listen.into(),
        upstream: upstream.into(),
    })
    .await
    {
        Ok(started) => started,
        Err(e) => {
            ConsoleLogger::error(e);
            std::process::exit(1);
        }
    }
}

fn launch_direct(routed: &Path) {
    if !cfg!(windows) {
        ConsoleLogger::info(format!(
            "The Origins client runs on Windows; start {} there.",
            routed.display()
        ));
        return;
    }
    let mut command = std::process::Command::new(routed);
    if let Some(parent) = routed.parent() {
        command.current_dir(parent);
    }
    // The client keeps running after Hablog stops.
    match command.spawn() {
        Ok(child) => {
            ConsoleLogger::success(format!("Launched the routed client (pid {}).", child.id()))
        }
        Err(e) => ConsoleLogger::error(format!("Cannot launch {}: {e}", routed.display())),
    }
}

async fn start_observer(
    dir: Option<&Path>,
    plan: &RoutePlan,
    routed: &Path,
) -> Result<Observer, String> {
    let runtime = ObserverRuntime::locate(dir)?;
    ConsoleLogger::info("Launching the routed client through the observer...");
    ObserverHandle::start(ObserverConfig {
        node_exe: runtime.node_exe,
        host_script: runtime.host_script,
        client_exe: routed.to_path_buf(),
        module_path: plan.module_path.clone(),
        launch: true,
        argv: Vec::new(),
    })
    .await
}

async fn relay_loop(
    proxy: ProxyHandle,
    mut events: mpsc::Receiver<ProxyEvent>,
    mut observer: Option<Observer>,
    mut log: TrafficLog,
) {
    let mut signals = ShutdownSignals::new();
    let mut stopping = false;
    loop {
        tokio::select! {
            event = events.recv() => match event {
                Some(ProxyEvent::Listening { address }) => log.listening(&address),
                Some(ProxyEvent::Connected { peer, upstream }) => log.connected(&peer, &upstream),
                Some(ProxyEvent::Data { direction, bytes }) => log.data(direction, &bytes),
                Some(ProxyEvent::Disconnected) => log.disconnected(),
                Some(ProxyEvent::Error(error)) => ConsoleLogger::error(error),
                Some(ProxyEvent::Stopped) | None => break,
            },
            event = next_observer_event(&mut observer) => {
                let exited = matches!(event, ObserverEvent::Exited);
                log.observer(event);
                if exited {
                    if let Some((mut handle, _)) = observer.take() {
                        handle.stop().await;
                    }
                }
            }
            _ = signals.recv() => {
                if stopping {
                    ConsoleLogger::warning("Stopping immediately.");
                    break;
                }
                stopping = true;
                ConsoleLogger::normal("Closing connection...");
                // Keep draining events so the final traffic and the
                // disconnect are still logged.
                proxy.shutdown();
            }
        }
    }
    if let Some((mut handle, _)) = observer.take() {
        // Detaching leaves the game running.
        handle.stop().await;
    }
    ConsoleLogger::normal("Stopped.");
}

async fn next_observer_event(observer: &mut Option<Observer>) -> ObserverEvent {
    match observer {
        Some((_, events)) => events.recv().await.unwrap_or(ObserverEvent::Exited),
        None => std::future::pending().await,
    }
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

/// The console log and capture file for the current client session.
struct TrafficLog {
    log: LogOptions,
    upstream: String,
    /// Habbo traffic is split into packets; Origins traffic is shown as TCP chunks.
    packets: Option<Packets>,
    sessions: u64,
    capture: Option<CaptureWriter>,
}

struct Packets {
    names: PacketNames,
    client: PacketHandler,
    server: PacketHandler,
}

impl TrafficLog {
    fn new(log: LogOptions, upstream: String, names: Option<PacketNames>) -> Self {
        TrafficLog {
            log,
            upstream,
            packets: names.map(|names| Packets {
                names,
                client: PacketHandler::new(Direction::ClientToServer),
                server: PacketHandler::new(Direction::ServerToClient),
            }),
            sessions: 0,
            capture: None,
        }
    }

    fn shows(&self, direction: Direction) -> bool {
        match direction {
            Direction::ClientToServer => self.log.log_client,
            Direction::ServerToClient => self.log.log_server,
        }
    }

    fn listening(&self, address: &str) {
        ConsoleLogger::info(format!(
            "Listening on {address}, forwarding to {}",
            self.upstream
        ));
        if self.packets.is_some() {
            ConsoleLogger::info(format!(
                "Waiting for the client. Route it to {address} (see README, \"Routing the client\")."
            ));
        } else {
            ConsoleLogger::info("Waiting for the routed Origins client.");
        }
    }

    fn connected(&mut self, peer: &str, upstream: &str) {
        self.sessions += 1;
        if let Some(packets) = self.packets.as_mut() {
            packets.client.reset();
            packets.server.reset();
        }
        ConsoleLogger::success(format!(
            "Caught client connection from {peer}, connected to game server at {upstream}"
        ));
        if let Some(dir) = &self.log.capture_dir {
            let path = dir.join(format!(
                "hablog-{}-{}.jsonl",
                Local::now().format("%Y%m%d-%H%M%S"),
                self.sessions
            ));
            match CaptureWriter::create(&path, peer, upstream) {
                Ok(capture) => {
                    ConsoleLogger::info(format!("Capturing traffic to {}", path.display()));
                    self.capture = Some(capture);
                }
                Err(e) => ConsoleLogger::error(e),
            }
        }
    }

    fn data(&mut self, direction: Direction, bytes: &[u8]) {
        if let Some(capture) = self.capture.as_mut() {
            if let Err(e) = capture.record(direction, bytes) {
                ConsoleLogger::error(format!("{e}. Capture stopped for this session."));
                self.capture = None;
            }
        }
        let show = self.shows(direction);
        let Some(packets) = self.packets.as_mut() else {
            if show {
                ConsoleLogger::log_chunk(direction, bytes);
            }
            return;
        };
        let handler = match direction {
            Direction::ClientToServer => &mut packets.client,
            Direction::ServerToClient => &mut packets.server,
        };
        let was_desynced = handler.is_desynced();
        let decoded = handler.push(bytes, &packets.names);
        if !show {
            return;
        }
        for packet in decoded {
            PacketHandler::process_packet(packet);
        }
        if !was_desynced && handler.is_desynced() {
            ConsoleLogger::warning(format!(
                "{} traffic no longer parses as Habbo packets (it is likely encrypted). \
                 It is still forwarded and captured.",
                direction.label()
            ));
        }
    }

    fn disconnected(&mut self) {
        match self.capture.take() {
            Some(capture) => ConsoleLogger::normal(format!(
                "Client disconnected. Captured {} TCP chunks to {}",
                capture.records(),
                capture.path().display()
            )),
            None => ConsoleLogger::normal("Client disconnected."),
        }
    }

    fn observer(&mut self, event: ObserverEvent) {
        match event {
            ObserverEvent::Status(state) => ConsoleLogger::info(format!("Observer: {state}")),
            ObserverEvent::Crypto(op) => {
                if !self.shows(op.direction) {
                    return;
                }
                // Metadata-only operations carry no bytes.
                match op.plaintext_hex().map(hex::decode) {
                    Some(Ok(plaintext)) => ConsoleLogger::log_decrypted(
                        op.direction,
                        &op.kind,
                        &op.sequence,
                        &plaintext,
                    ),
                    Some(Err(_)) => ConsoleLogger::warning(format!(
                        "Observer sent unreadable bytes for {} #{}",
                        op.operation, op.sequence
                    )),
                    None => {}
                }
            }
            ObserverEvent::SocketConnect {
                api,
                host,
                port,
                result,
            } => ConsoleLogger::normal(format!(
                "Observer: client {api}({host}:{port}) returned {result}"
            )),
            ObserverEvent::Dropped(count) => ConsoleLogger::warning(format!(
                "Observer dropped {count} operations because its buffer was full."
            )),
            ObserverEvent::Error(error) => ConsoleLogger::error(format!("Observer: {error}")),
            ObserverEvent::Exited => {
                ConsoleLogger::warning("Observer detached. The relay keeps running.")
            }
        }
    }
}

fn origins_details(options: DetailsOptions) {
    let snapshot = discovery::discover_with(options.roots.appdata, options.roots.local_appdata);
    if options.json {
        match serde_json::to_string_pretty(&snapshot) {
            Ok(json) => println!("{json}"),
            Err(e) => {
                eprintln!("Cannot serialize discovery: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
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
                    .unwrap_or("unknown region")
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
            client.server_code.as_deref().unwrap_or("unknown region")
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

fn origins_crypto(options: CryptoOptions) -> Result<(), String> {
    let key = Zeroizing::new(match &options.key_file {
        Some(path) => std::fs::read_to_string(path)
            .map_err(|e| format!("Cannot read {}: {e}", path.display()))?,
        None => std::env::var(SESSION_KEY_ENV)
            .map_err(|_| format!("Pass --key-file or set {SESSION_KEY_ENV}."))?,
    });
    let session = OriginsSession::from_shared_key_hex(&key)?;
    match options.mode {
        CryptoMode::Decrypt => {
            let ciphertext = if options.hex_input {
                hex::decode(options.input.trim()).map_err(|_| "The input is not valid hex.")?
            } else {
                crypto::decode_wire(&options.input)?
            };
            let plaintext = session.transform(
                options.direction,
                options.stream,
                options.sequence,
                &ciphertext,
            )?;
            println!("hex:  {}", hex::encode(&plaintext));
            println!("text: {}", escaped(&plaintext, usize::MAX));
        }
        CryptoMode::Encrypt => {
            let plaintext =
                hex::decode(options.input.trim()).map_err(|_| "The input is not valid hex.")?;
            let ciphertext = session.transform(
                options.direction,
                options.stream,
                options.sequence,
                &plaintext,
            )?;
            println!("wire: {}", crypto::encode_wire(&ciphertext));
            println!("hex:  {}", hex::encode(&ciphertext));
        }
    }
    Ok(())
}
