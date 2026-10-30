use chrono::Local;
use hablog::capture::CaptureWriter;
use hablog::logger::ConsoleLogger;
use hablog::model::Direction;
use hablog::packet_handler::packet_handler::{PacketHandler, PacketNames};
use hablog::proxy::{ProxyConfig, ProxyEvent, ProxyHandle};
use std::path::PathBuf;
use tokio::signal::unix::{signal, Signal, SignalKind};

const DEFAULT_LISTEN: &str = "127.0.0.1:38101";
const DEFAULT_UPSTREAM: &str = "game-us.habbo.com:38101";
const DEFAULT_CAPTURE_DIR: &str = "logs";

struct Options {
    listen: String,
    upstream: String,
    capture_dir: Option<PathBuf>,
    log_client: bool,
    log_server: bool,
}

#[tokio::main]
async fn main() {
    let options = parse_args();

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
    let (proxy, mut events) = match ProxyHandle::start(ProxyConfig {
        listen: options.listen.clone(),
        upstream: options.upstream.clone(),
    })
    .await
    {
        Ok(started) => started,
        Err(e) => {
            ConsoleLogger::error(e);
            std::process::exit(1);
        }
    };

    let mut interrupt = signal(SignalKind::interrupt()).expect("Failed to set up signal handler");
    let mut terminate = signal(SignalKind::terminate()).expect("Failed to set up signal handler");
    let mut log = TrafficLog::new(options, names);
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
            _ = shutdown_signal(&mut interrupt, &mut terminate) => {
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
    ConsoleLogger::normal("Stopped.");
}

async fn shutdown_signal(interrupt: &mut Signal, terminate: &mut Signal) {
    tokio::select! {
        _ = interrupt.recv() => {}
        _ = terminate.recv() => {}
    }
}

/// The console log and capture file for the current client session.
struct TrafficLog {
    options: Options,
    names: PacketNames,
    sessions: u64,
    capture: Option<CaptureWriter>,
    client: PacketHandler,
    server: PacketHandler,
}

impl TrafficLog {
    fn new(options: Options, names: PacketNames) -> Self {
        TrafficLog {
            options,
            names,
            sessions: 0,
            capture: None,
            client: PacketHandler::new(Direction::ClientToServer),
            server: PacketHandler::new(Direction::ServerToClient),
        }
    }

    fn listening(&self, address: &str) {
        ConsoleLogger::info(format!(
            "Listening on {address}, forwarding to {}",
            self.options.upstream
        ));
        ConsoleLogger::info(format!(
            "Waiting for the client. Route it to {address} (see README, \"Routing the client\")."
        ));
    }

    fn connected(&mut self, peer: &str, upstream: &str) {
        self.sessions += 1;
        self.client.reset();
        self.server.reset();
        ConsoleLogger::success(format!(
            "Caught client connection from {peer}, connected to game server at {upstream}"
        ));
        if let Some(dir) = &self.options.capture_dir {
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
        let (handler, show) = match direction {
            Direction::ClientToServer => (&mut self.client, self.options.log_client),
            Direction::ServerToClient => (&mut self.server, self.options.log_server),
        };
        let was_desynced = handler.is_desynced();
        let packets = handler.push(bytes, &self.names);
        if !show {
            return;
        }
        for packet in packets {
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
}

fn parse_args() -> Options {
    let mut options = Options {
        listen: DEFAULT_LISTEN.into(),
        upstream: DEFAULT_UPSTREAM.into(),
        capture_dir: Some(PathBuf::from(DEFAULT_CAPTURE_DIR)),
        log_client: true,
        log_server: true,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| {
            args.next().unwrap_or_else(|| {
                eprintln!("{flag} requires a value. Use --help for usage.");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--listen" => options.listen = value("--listen"),
            "--upstream" => options.upstream = value("--upstream"),
            "--capture-dir" => options.capture_dir = Some(PathBuf::from(value("--capture-dir"))),
            "--no-capture" => options.capture_dir = None,
            "--no-client-log" => options.log_client = false,
            "--no-server-log" => options.log_server = false,
            "--help" | "-h" => {
                println!(
                    "Hablog: a logging TCP relay for the Habbo client\n\n\
                     Usage: hablog [OPTIONS]\n\n\
                     --listen ADDR       Loopback address the client connects to (default {DEFAULT_LISTEN})\n\
                     --upstream ADDR     Real game server host:port (default {DEFAULT_UPSTREAM})\n\
                     --capture-dir DIR   Where per-session JSONL captures go (default {DEFAULT_CAPTURE_DIR})\n\
                     --no-capture        Don't write capture files\n\
                     --no-client-log     Don't print client to server packets (still captured)\n\
                     --no-server-log     Don't print server to client packets (still captured)\n\
                     --help              Show this help"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {other}. Use --help for usage.");
                std::process::exit(2);
            }
        }
    }
    options
}
