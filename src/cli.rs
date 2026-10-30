use hablog::model::Direction;
use hablog::origins::crypto::StreamKind;
use std::path::PathBuf;

pub const DEFAULT_LISTEN: &str = "127.0.0.1:38101";
pub const DEFAULT_UPSTREAM: &str = "game-us.habbo.com:38101";
pub const DEFAULT_CAPTURE_DIR: &str = "logs";
pub const SESSION_KEY_ENV: &str = "HABLOG_SESSION_KEY";

const HELP: &str = "\
Hablog: a logging TCP relay for Habbo and Habbo Origins

Usage:
  hablog [OPTIONS]                   Relay the Habbo client
  hablog origins [OPTIONS]           Find the Origins client, route a copy of it
                                     through the relay and launch it
  hablog origins-details [--json]    Print the Origins client's connection details
  hablog origins-decrypt [CRYPTO] WIRE
                                     Decrypt one Origins operation
  hablog origins-encrypt [CRYPTO] HEX
                                     Encrypt one Origins operation

Habbo options:
  --listen ADDR          Loopback address the client connects to (default 127.0.0.1:38101)
  --upstream ADDR        Real game server host:port (default game-us.habbo.com:38101)

Origins options:
  --server CODE          Region: hhous, hhobr, hhoes or hhod (default: the client
                         the launcher last selected)
  --upstream ADDR        Use this game server instead of the one in the client
  --observe              Launch through the Frida observer and log decrypted traffic
  --no-launch            Only write the routed client; start it yourself
  --observer-dir DIR     Folder with host.mjs (default: observer/ next to hablog,
                         then ./automation)
  --appdata DIR          Use DIR instead of %APPDATA% (also for origins-details)
  --local-appdata DIR    Use DIR instead of %LOCALAPPDATA% (also for origins-details)

Logging options (Habbo and Origins):
  --capture-dir DIR      Where per-session JSONL captures go (default logs)
  --no-capture           Don't write capture files
  --no-client-log        Don't print client to server traffic (still captured)
  --no-server-log        Don't print server to client traffic (still captured)

Crypto options:
  --key-file FILE        File holding your session's shared key, 112 hex characters
                         (or set HABLOG_SESSION_KEY)
  --direction in|out     in: server to client, out: client to server
  --stream header|payload
  --sequence N           The operation's counter in that stream, from 0
  --hex                  Decrypt: WIRE is hex instead of unpadded Base64

  --help                 Show this help";

pub enum Command {
    Habbo(HabboOptions),
    Origins(OriginsOptions),
    OriginsDetails(DetailsOptions),
    OriginsCrypto(CryptoOptions),
}

pub struct LogOptions {
    pub capture_dir: Option<PathBuf>,
    pub log_client: bool,
    pub log_server: bool,
}

pub struct HabboOptions {
    pub listen: String,
    pub upstream: String,
    pub log: LogOptions,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    Direct,
    Observe,
    Manual,
}

#[derive(Default)]
pub struct Roots {
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
}

pub struct OriginsOptions {
    pub server: Option<String>,
    pub upstream: Option<String>,
    pub launch: Launch,
    pub observer_dir: Option<PathBuf>,
    pub roots: Roots,
    pub log: LogOptions,
}

pub struct DetailsOptions {
    pub roots: Roots,
    pub json: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CryptoMode {
    Decrypt,
    Encrypt,
}

pub struct CryptoOptions {
    pub mode: CryptoMode,
    pub key_file: Option<PathBuf>,
    pub direction: Direction,
    pub stream: StreamKind,
    pub sequence: u64,
    pub hex_input: bool,
    pub input: String,
}

type Args = std::vec::IntoIter<String>;

pub fn parse() -> Command {
    let mut args: Args = std::env::args().skip(1).collect::<Vec<_>>().into_iter();
    match args.as_slice().first().map(String::as_str) {
        Some("origins") => {
            args.next();
            Command::Origins(parse_origins(args))
        }
        Some("origins-details") => {
            args.next();
            Command::OriginsDetails(parse_details(args))
        }
        Some("origins-decrypt") => {
            args.next();
            Command::OriginsCrypto(parse_crypto(CryptoMode::Decrypt, args))
        }
        Some("origins-encrypt") => {
            args.next();
            Command::OriginsCrypto(parse_crypto(CryptoMode::Encrypt, args))
        }
        _ => Command::Habbo(parse_habbo(args)),
    }
}

fn parse_habbo(mut args: Args) -> HabboOptions {
    let mut options = HabboOptions {
        listen: DEFAULT_LISTEN.into(),
        upstream: DEFAULT_UPSTREAM.into(),
        log: LogOptions::default(),
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--listen" => options.listen = value(&mut args, &arg),
            "--upstream" => options.upstream = value(&mut args, &arg),
            _ if log_option(&arg, &mut args, &mut options.log) => {}
            _ => other(&arg),
        }
    }
    options
}

fn parse_origins(mut args: Args) -> OriginsOptions {
    let mut options = OriginsOptions {
        server: None,
        upstream: None,
        launch: Launch::Direct,
        observer_dir: None,
        roots: Roots::default(),
        log: LogOptions::default(),
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => options.server = Some(value(&mut args, &arg).to_ascii_lowercase()),
            "--upstream" => options.upstream = Some(value(&mut args, &arg)),
            "--observe" => options.launch = Launch::Observe,
            "--no-launch" => options.launch = Launch::Manual,
            "--observer-dir" => options.observer_dir = Some(PathBuf::from(value(&mut args, &arg))),
            _ if roots_option(&arg, &mut args, &mut options.roots) => {}
            _ if log_option(&arg, &mut args, &mut options.log) => {}
            _ => other(&arg),
        }
    }
    options
}

fn parse_details(mut args: Args) -> DetailsOptions {
    let mut options = DetailsOptions {
        roots: Roots::default(),
        json: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => options.json = true,
            _ if roots_option(&arg, &mut args, &mut options.roots) => {}
            _ => other(&arg),
        }
    }
    options
}

fn parse_crypto(mode: CryptoMode, mut args: Args) -> CryptoOptions {
    let mut key_file = None;
    let mut direction = None;
    let mut stream = None;
    let mut sequence = None;
    let mut hex_input = false;
    let mut input = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--key-file" => key_file = Some(PathBuf::from(value(&mut args, &arg))),
            "--direction" => {
                direction = Some(match value(&mut args, &arg).as_str() {
                    "in" => Direction::ServerToClient,
                    "out" => Direction::ClientToServer,
                    _ => fail("--direction must be in or out."),
                })
            }
            "--stream" => {
                stream = Some(match value(&mut args, &arg).as_str() {
                    "header" => StreamKind::Header,
                    "payload" => StreamKind::Payload,
                    _ => fail("--stream must be header or payload."),
                })
            }
            "--sequence" => {
                sequence = Some(
                    value(&mut args, &arg)
                        .parse()
                        .unwrap_or_else(|_| fail("--sequence must be a whole number.")),
                )
            }
            "--hex" if mode == CryptoMode::Decrypt => hex_input = true,
            _ if !arg.starts_with("--") && input.is_none() => input = Some(arg),
            _ => other(&arg),
        }
    }
    CryptoOptions {
        mode,
        key_file,
        direction: direction.unwrap_or_else(|| fail("--direction is required.")),
        stream: stream.unwrap_or_else(|| fail("--stream is required.")),
        sequence: sequence.unwrap_or_else(|| fail("--sequence is required.")),
        hex_input,
        input: input.unwrap_or_else(|| fail("The bytes to transform are required.")),
    }
}

impl Default for LogOptions {
    fn default() -> Self {
        LogOptions {
            capture_dir: Some(PathBuf::from(DEFAULT_CAPTURE_DIR)),
            log_client: true,
            log_server: true,
        }
    }
}

fn log_option(arg: &str, args: &mut Args, log: &mut LogOptions) -> bool {
    match arg {
        "--capture-dir" => log.capture_dir = Some(PathBuf::from(value(args, arg))),
        "--no-capture" => log.capture_dir = None,
        "--no-client-log" => log.log_client = false,
        "--no-server-log" => log.log_server = false,
        _ => return false,
    }
    true
}

fn roots_option(arg: &str, args: &mut Args, roots: &mut Roots) -> bool {
    match arg {
        "--appdata" => roots.appdata = Some(PathBuf::from(value(args, arg))),
        "--local-appdata" => roots.local_appdata = Some(PathBuf::from(value(args, arg))),
        _ => return false,
    }
    true
}

fn value(args: &mut Args, flag: &str) -> String {
    args.next()
        .unwrap_or_else(|| fail(&format!("{flag} requires a value.")))
}

fn other(arg: &str) -> ! {
    if arg == "--help" || arg == "-h" {
        println!("{HELP}");
        std::process::exit(0);
    }
    fail(&format!("Unknown argument: {arg}."))
}

fn fail(message: &str) -> ! {
    eprintln!("{message} Use --help for usage.");
    std::process::exit(2)
}
