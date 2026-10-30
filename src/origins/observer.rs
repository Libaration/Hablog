//! Drive the local Frida observer companion in `automation/`.
//!
//! The companion (`automation/host.mjs`) launches the routed Origins client
//! suspended, hooks it, then resumes it, and streams the native crypto boundary
//! (the point where BobbaXtra encrypts/decrypts, i.e. the decrypted session
//! data) as one JSON object per line. This module spawns it, sends the start
//! control and forwards parsed events over a channel. It never writes game
//! files or transmits anything over a network.

use crate::model::Direction;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const EVENT_CAPACITY: usize = 1024;
const STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct CryptoOp {
    pub direction: Direction,
    pub kind: String,
    pub operation: String,
    pub sequence: String,
    pub plaintext_side: String,
    pub input_hex: Option<String>,
    pub output_hex: Option<String>,
}

impl CryptoOp {
    /// The decrypted side of the operation: the input of an encryption, the
    /// output of a decryption.
    pub fn plaintext_hex(&self) -> Option<&str> {
        if self.plaintext_side == "output" {
            self.output_hex.as_deref()
        } else {
            self.input_hex.as_deref()
        }
    }
}

#[derive(Clone, Debug)]
pub enum ObserverEvent {
    Status(String),
    Crypto(CryptoOp),
    SocketConnect {
        api: String,
        host: String,
        port: u16,
        result: i64,
    },
    Dropped(u64),
    Error(String),
    Exited,
}

#[derive(Clone, Debug)]
pub struct ObserverConfig {
    pub node_exe: PathBuf,
    pub host_script: PathBuf,
    pub client_exe: PathBuf,
    pub module_path: PathBuf,
    /// When true, Frida launches the client suspended, hooks it, then resumes —
    /// so a single process is both routed and observed.
    pub launch: bool,
    pub argv: Vec<String>,
}

/// Where the companion lives and which Node runs it.
#[derive(Clone, Debug)]
pub struct ObserverRuntime {
    pub node_exe: PathBuf,
    pub host_script: PathBuf,
}

impl ObserverRuntime {
    /// Uses `dir` if given, otherwise `observer/` next to the executable or
    /// `automation/` in the working directory. A `node.exe` in that folder
    /// is preferred over `node` on PATH.
    pub fn locate(dir: Option<&Path>) -> Result<Self, String> {
        let candidates: Vec<PathBuf> = match dir {
            Some(dir) => vec![dir.to_path_buf()],
            None => {
                let mut candidates = Vec::new();
                if let Some(exe_dir) = std::env::current_exe()
                    .ok()
                    .and_then(|exe| exe.parent().map(Path::to_path_buf))
                {
                    candidates.push(exe_dir.join("observer"));
                }
                candidates.push(PathBuf::from("automation"));
                candidates
            }
        };
        let dir = candidates
            .iter()
            .find(|dir| dir.join("host.mjs").is_file())
            .ok_or_else(|| {
                format!(
                    "The observer companion (host.mjs) was not found in {}.",
                    candidates
                        .iter()
                        .map(|dir| dir.display().to_string())
                        .collect::<Vec<_>>()
                        .join(" or ")
                )
            })?;
        if !dir.join("node_modules").join("frida").is_dir() {
            return Err(format!(
                "Frida is not installed for the observer; run `npm ci` in {}.",
                dir.display()
            ));
        }
        let bundled = dir.join("node.exe");
        Ok(Self {
            node_exe: if bundled.is_file() {
                bundled
            } else {
                PathBuf::from("node")
            },
            host_script: dir.join("host.mjs"),
        })
    }
}

pub struct ObserverHandle {
    child: Child,
    stdin: Option<ChildStdin>,
    reader: Option<JoinHandle<()>>,
}

impl ObserverHandle {
    pub async fn start(
        config: ObserverConfig,
    ) -> Result<(Self, mpsc::Receiver<ObserverEvent>), String> {
        let mut command = Command::new(&config.node_exe);
        command
            .arg(&config.host_script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        no_window(&mut command);
        let mut child = command.spawn().map_err(|e| {
            format!(
                "Cannot start the observer companion with {}: {e}",
                config.node_exe.display()
            )
        })?;

        let start = serde_json::json!({
            "command": "start",
            "config": {
                "clientExecutablePath": config.client_exe.to_string_lossy(),
                "modulePath": config.module_path.to_string_lossy(),
                "captureMode": "full",
                "launch": config.launch,
                "argv": config.argv,
            }
        });
        // Keep stdin open so the companion keeps observing; closing it detaches.
        let mut stdin = child
            .stdin
            .take()
            .ok_or("The observer companion has no stdin.")?;
        stdin
            .write_all(format!("{start}\n").as_bytes())
            .await
            .map_err(|e| format!("Cannot send start control: {e}"))?;

        let stdout = child
            .stdout
            .take()
            .ok_or("The observer companion has no stdout.")?;
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let reader = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.trim().is_empty() {
                    continue;
                }
                // Non-JSON output is never forwarded as game data.
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if let Some(event) = parse_event(&value) {
                    if event_tx.send(event).await.is_err() {
                        return;
                    }
                }
            }
            let _ = event_tx.send(ObserverEvent::Exited).await;
        });

        Ok((
            Self {
                child,
                stdin: Some(stdin),
                reader: Some(reader),
            },
            event_rx,
        ))
    }

    /// Asks the companion to detach, which leaves the client running. If it
    /// does not exit promptly, the helper process (not the game) is killed.
    pub async fn stop(&mut self) {
        if let Some(mut stdin) = self.stdin.take() {
            let _ = stdin.write_all(b"{\"command\":\"stop\"}\n").await;
        }
        if tokio::time::timeout(STOP_TIMEOUT, self.child.wait())
            .await
            .is_err()
        {
            let _ = self.child.kill().await;
        }
        if let Some(reader) = self.reader.take() {
            reader.abort();
            let _ = reader.await;
        }
    }
}

#[cfg(windows)]
fn no_window(command: &mut Command) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn no_window(_command: &mut Command) {}

/// Translate one companion JSON line into an event. Unknown shapes are ignored.
pub fn parse_event(value: &serde_json::Value) -> Option<ObserverEvent> {
    let kind = value.get("type")?.as_str()?;
    match kind {
        "crypto" => {
            let direction = match value.get("direction").and_then(|d| d.as_str())? {
                "client_to_server" => Direction::ClientToServer,
                "server_to_client" => Direction::ServerToClient,
                _ => return None,
            };
            Some(ObserverEvent::Crypto(CryptoOp {
                direction,
                kind: str_field(value, "kind"),
                operation: str_field(value, "operation"),
                sequence: str_field(value, "sequence"),
                plaintext_side: str_field(value, "plaintext_side"),
                input_hex: value
                    .get("input_hex")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
                output_hex: value
                    .get("output_hex")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned),
            }))
        }
        "socket_connect" => {
            let endpoint = value.get("endpoint")?;
            Some(ObserverEvent::SocketConnect {
                api: str_field(value, "api"),
                host: endpoint
                    .get("host")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                port: endpoint.get("port").and_then(|v| v.as_u64()).unwrap_or(0) as u16,
                result: value.get("result").and_then(|v| v.as_i64()).unwrap_or(-1),
            })
        }
        "dropped" => Some(ObserverEvent::Dropped(
            value.get("count").and_then(|v| v.as_u64()).unwrap_or(0),
        )),
        "status" => {
            let state = str_field(value, "state");
            let extra = value
                .get("detail")
                .and_then(|v| v.as_str())
                .or_else(|| value.get("module_path").and_then(|v| v.as_str()))
                .unwrap_or("");
            Some(ObserverEvent::Status(if extra.is_empty() {
                state
            } else {
                format!("{state} · {extra}")
            }))
        }
        "error" => Some(ObserverEvent::Error(format!(
            "{}: {}",
            str_field(value, "code"),
            str_field(value, "message")
        ))),
        _ => None,
    }
}

fn str_field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_crypto_boundary_event() {
        let value = serde_json::json!({
            "type": "crypto", "operation": "decrypt_payload",
            "direction": "server_to_client", "kind": "payload", "sequence": "7",
            "plaintext_side": "output", "input_hex": "414243", "output_hex": "0041",
        });
        match parse_event(&value) {
            Some(ObserverEvent::Crypto(op)) => {
                assert_eq!(op.direction, Direction::ServerToClient);
                assert_eq!(op.sequence, "7");
                assert_eq!(op.output_hex.as_deref(), Some("0041"));
                assert_eq!(op.plaintext_hex(), Some("0041"));
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn encryption_plaintext_is_the_input_side() {
        let value = serde_json::json!({
            "type": "crypto", "operation": "encrypt_header",
            "direction": "client_to_server", "kind": "header", "sequence": "0",
            "plaintext_side": "input", "input_hex": "0041", "output_hex": "414243",
        });
        match parse_event(&value) {
            Some(ObserverEvent::Crypto(op)) => assert_eq!(op.plaintext_hex(), Some("0041")),
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn parses_status_socket_and_error() {
        assert!(matches!(
            parse_event(&serde_json::json!({"type":"status","state":"observing"})),
            Some(ObserverEvent::Status(s)) if s == "observing"
        ));
        assert!(matches!(
            parse_event(&serde_json::json!({
                "type":"socket_connect","api":"connect",
                "endpoint":{"host":"127.0.0.1","port":40001},"result":0
            })),
            Some(ObserverEvent::SocketConnect { port, .. }) if port == 40001
        ));
        assert!(matches!(
            parse_event(&serde_json::json!({"type":"error","code":"x","message":"y"})),
            Some(ObserverEvent::Error(_))
        ));
    }

    #[test]
    fn ignores_unknown_and_non_object_lines() {
        assert!(parse_event(&serde_json::json!({"type": "noise"})).is_none());
        assert!(parse_event(&serde_json::json!("plain string")).is_none());
    }
}
