use hablog::logger::{readable, ConsoleLogger};
use hablog::model::Direction;
use hablog::origins::crypto::{self, OriginsSession, StreamKind};
use hablog::origins::discovery;
use hablog::origins::observer::{self, LocateError, ObserverRuntime};
use hablog::session::{self, Level, RawKind, SessionEvent};
use hablog::settings::{self, Settings, Show, Source};
use hablog::tools;
use hablog::wsl;
use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;
use zeroize::Zeroizing;

const HELP: &str = "\
Hablog relays Habbo Origins through your computer and logs its traffic.

  hablog           Start Origins through Hablog. It finds the installed client,
                   routes it through the relay, starts the game and logs its
                   traffic until the game closes or you press Ctrl+C.
  hablog details   Show the installed client's connection details.
  hablog decode    Turn VL64 and B64 values from the log into numbers, and
                   numbers into them: `hablog decode PB`, `hablog decode 206`.
  hablog build     Build an exact message from typed values, the reverse of
                   decode: `hablog build @C b:true i:0 s:hello`.
  hablog decrypt   Decrypt a captured message with your session key.

Settings are in hablog.toml, which Hablog creates with an explanation of every
option the first time it runs.";

const SESSION_KEY_FILE: &str = "session.secret";
// Session events printed per console write.
const EVENT_BATCH: usize = 256;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    match command.as_deref() {
        Some("decode") => return decode_command(&rest.join(" ")),
        Some("build") => return build_command(&rest.join(" ")),
        _ => {}
    }
    if !rest.is_empty() {
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

    let prepared = match session::prepare(&settings) {
        Ok(prepared) => prepared,
        Err(problem) => fail(problem),
    };
    ConsoleLogger::info(format!(
        "Found Origins {} ({}), game server {}",
        prepared.version(),
        prepared
            .plan
            .server_code
            .as_deref()
            .unwrap_or("unknown hotel"),
        prepared.plan.upstream
    ));
    if !prepared.snapshot.running_clients.is_empty() {
        ConsoleLogger::warning(
            "Origins is already open and connected directly. Close it so its traffic goes through Hablog.",
        );
    }
    let observer = if settings.launch && settings.decrypt {
        observer_runtime(settings.advanced.observer.as_deref()).await
    } else {
        None
    };

    let (handle, mut events) = match session::start(settings.clone(), prepared, observer).await {
        Ok(started) => started,
        Err(problem) => fail(problem),
    };
    let mut console = Console::new(settings);
    let mut signals = ShutdownSignals::new();
    let mut batch = Vec::with_capacity(EVENT_BATCH);
    loop {
        tokio::select! {
            received = events.recv_many(&mut batch, EVENT_BATCH) => {
                if received == 0 {
                    break;
                }
                for event in batch.drain(..) {
                    console.show(event);
                }
                ConsoleLogger::flush();
            }
            _ = signals.recv() => handle.stop(),
        }
    }
}

fn fail(problem: session::Problem) -> ! {
    for warning in &problem.warnings {
        ConsoleLogger::warning(warning);
    }
    ConsoleLogger::error(problem.error);
    if let Some(hint) = problem.hint {
        ConsoleLogger::info(hint);
    }
    std::process::exit(1);
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

/// Prints session events, following the `show` settings.
struct Console {
    settings: Settings,
    /// The observer is reporting decrypted messages, so the encrypted chunks
    /// they came in are only shown with `show_raw`.
    decrypting: bool,
}

impl Console {
    fn new(settings: Settings) -> Self {
        Console {
            settings,
            decrypting: false,
        }
    }

    fn show(&mut self, event: SessionEvent) {
        match event {
            SessionEvent::Status { level, text } => match level {
                Level::Normal => ConsoleLogger::normal(text),
                Level::Info => ConsoleLogger::info(text),
                Level::Success => ConsoleLogger::success(text),
                Level::Warning => ConsoleLogger::warning(text),
                Level::Error => ConsoleLogger::error(text),
            },
            SessionEvent::Ready { listen, upstream } => {
                ConsoleLogger::info(format!("Relay ready on {listen}, forwarding to {upstream}"));
                if self.settings.show != Show::None {
                    ConsoleLogger::normal(
                        "Each message is one line: direction, header, ID and body, with unprintable bytes as [n].",
                    );
                }
            }
            SessionEvent::Connected {
                peer,
                upstream,
                capture,
            } => {
                ConsoleLogger::success(format!(
                    "The game connected ({peer}), relaying to {upstream}"
                ));
                if let Some(path) = capture {
                    ConsoleLogger::info(format!("Saving traffic to {path}"));
                }
            }
            SessionEvent::Disconnected { capture, chunks } => match capture {
                Some(path) => ConsoleLogger::normal(format!(
                    "The game disconnected. Saved {chunks} TCP chunks to {path}"
                )),
                None => ConsoleLogger::normal("The game disconnected."),
            },
            SessionEvent::Decrypting { on } => self.decrypting = on,
            SessionEvent::Message {
                direction, bytes, ..
            } => {
                if self.settings.shows(direction) {
                    let (header, body) = bytes.split_at(bytes.len().min(2));
                    ConsoleLogger::log_message(
                        direction,
                        header,
                        hablog::origins::fuse::id(&bytes),
                        body,
                    );
                }
            }
            SessionEvent::Raw {
                direction,
                raw,
                bytes,
                plain,
                ..
            } => {
                // Without decrypted traffic, the encrypted chunks are all
                // there is to show.
                let fallback = raw == RawKind::Wire && !plain && !self.decrypting;
                if self.settings.shows(direction) && (self.settings.show_raw || fallback) {
                    let label = match raw {
                        RawKind::Wire => "wire",
                        RawKind::Frame => "frame",
                    };
                    ConsoleLogger::log_raw(direction, label, &bytes);
                }
            }
            SessionEvent::Connecting { host, port } => {
                if self.settings.show_raw {
                    ConsoleLogger::normal(format!("The game is connecting to {host}:{port}."));
                }
            }
            SessionEvent::Traffic { .. } => {}
            SessionEvent::Stopped => ConsoleLogger::normal("Stopped."),
        }
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
    for line in tools::details(&snapshot) {
        println!("{line}");
    }
    for warning in &snapshot.warnings {
        println!("Warning: {warning}");
    }
}

/// Decodes `value`, or each value typed at the prompt when it is empty.
fn decode_command(value: &str) {
    if !value.is_empty() {
        for line in tools::decode(value) {
            println!("{line}");
        }
        return;
    }
    println!("Paste a value from the log, or type a number. Press Enter on an empty line to quit.");
    while let Some(value) = ask("Value", "").filter(|value| !value.is_empty()) {
        for line in tools::decode(&value) {
            println!("  {line}");
        }
    }
}

/// Builds the message described by `spec`, or each one typed at the prompt.
fn build_command(spec: &str) {
    if !spec.is_empty() {
        match tools::build(spec) {
            Ok(message) => print_built(&message, ""),
            Err(e) => {
                eprintln!("{e}\n\n{}", tools::BUILD_HELP);
                std::process::exit(2);
            }
        }
        return;
    }
    println!(
        "{}\n\nPress Enter on an empty line to quit.",
        tools::BUILD_HELP
    );
    while let Some(spec) = ask("Message", "").filter(|spec| !spec.is_empty()) {
        match tools::build(&spec) {
            Ok(message) => print_built(&message, "  "),
            Err(e) => println!("  {e}"),
        }
    }
}

fn print_built(message: &[u8], indent: &str) {
    println!("{indent}{}", readable(message, usize::MAX));
    println!("{indent}hex: {}", hex::encode(message));
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
                println!("Plaintext: {}", readable(&plaintext, usize::MAX));
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
