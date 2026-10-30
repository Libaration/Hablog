//! A transparent, single-client TCP relay. Captured chunks are TCP reads, not
//! Habbo packets; forwarding never waits on whoever is consuming the events.

use crate::model::Direction;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

const EVENT_CAPACITY: usize = 1024;
// Data never takes the last slots of the event queue, so connection state
// changes still get through while a slow consumer has fallen behind.
const STATE_RESERVE: usize = 32;
const IO_CHUNK: usize = 16 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug)]
pub struct ProxyConfig {
    pub listen: String,
    pub upstream: String,
}

#[derive(Debug)]
pub enum ProxyEvent {
    Listening {
        address: String,
    },
    Connected {
        peer: String,
        upstream: String,
    },
    Data {
        direction: Direction,
        bytes: Vec<u8>,
    },
    Disconnected,
    Error(String),
    Stopped,
}

pub struct ProxyHandle {
    shutdown: watch::Sender<bool>,
    worker: Option<JoinHandle<()>>,
}

impl ProxyHandle {
    /// Binds only loopback addresses. No client or system settings are changed.
    pub async fn start(config: ProxyConfig) -> Result<(Self, mpsc::Receiver<ProxyEvent>), String> {
        let listen = resolve(&config.listen, "listen").await?;
        if listen.iter().any(|address| !address.ip().is_loopback()) {
            return Err("The listen address must be loopback (127.0.0.1 or ::1).".into());
        }
        let listener = TcpListener::bind(&listen[..])
            .await
            .map_err(|e| format!("Cannot listen on {}: {e}", config.listen))?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let upstream = resolve(&config.upstream, "upstream").await?;
        // A hosts-file redirect also applies to this lookup, which would make
        // the relay connect to itself.
        if upstream.contains(&address) {
            return Err(format!(
                "The upstream {} resolves to this relay's own listener ({address}). \
                 Use the game server's real IP address as the upstream.",
                config.upstream
            ));
        }
        let (event_tx, event_rx) = mpsc::channel(EVENT_CAPACITY);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let events = Events {
            tx: event_tx,
            dropped: Arc::new(AtomicU64::new(0)),
        };
        let worker = tokio::spawn(run(listener, upstream, events, shutdown_rx));
        Ok((
            Self {
                shutdown,
                worker: Some(worker),
            },
            event_rx,
        ))
    }

    /// Asks the relay to close the active connection and the listener without
    /// waiting. The event stream ends with ProxyEvent::Stopped once both are closed.
    pub fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    /// Closes the active connection and the listener, then waits for the worker.
    pub async fn stop(&mut self) {
        self.shutdown();
        if let Some(worker) = self.worker.take() {
            let _ = worker.await;
        }
    }
}

impl Drop for ProxyHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

async fn resolve(value: &str, kind: &str) -> Result<Vec<SocketAddr>, String> {
    let addresses: Vec<_> = tokio::net::lookup_host(value)
        .await
        .map_err(|e| format!("Invalid {kind} address {value}: {e}"))?
        .collect();
    if addresses.is_empty() {
        return Err(format!(
            "The {kind} address {value} did not resolve to a socket address."
        ));
    }
    Ok(addresses)
}

#[derive(Clone)]
struct Events {
    tx: mpsc::Sender<ProxyEvent>,
    dropped: Arc<AtomicU64>,
}

impl Events {
    fn data(&self, direction: Direction, bytes: &[u8]) {
        let queued = self.tx.capacity() > STATE_RESERVE
            && self
                .tx
                .try_send(ProxyEvent::Data {
                    direction,
                    bytes: bytes.to_vec(),
                })
                .is_ok();
        if !queued {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    async fn state(&self, event: ProxyEvent) {
        // Fails only when the receiver is gone, and then nobody is listening.
        let _ = self.tx.send(event).await;
    }

    async fn report_drops(&self) {
        let count = self.dropped.swap(0, Ordering::Relaxed);
        if count != 0 {
            self.state(ProxyEvent::Error(format!(
                "The capture log dropped {count} TCP chunks because its queue was full. Forwarding continued."
            )))
            .await;
        }
    }
}

async fn run(
    listener: TcpListener,
    upstream: Vec<SocketAddr>,
    events: Events,
    mut shutdown: watch::Receiver<bool>,
) {
    let address = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or_default();
    events.state(ProxyEvent::Listening { address }).await;
    let mut session: Option<JoinHandle<()>> = None;
    loop {
        let accepted = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            accepted = listener.accept() => accepted,
        };
        let (client, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(e) if is_transient(&e) => continue,
            Err(e) => {
                events
                    .state(ProxyEvent::Error(format!("Cannot accept a client: {e}")))
                    .await;
                break;
            }
        };
        // Extra clients are closed explicitly, rather than left in the listen
        // backlog and unexpectedly attached after the current session ends.
        if session.as_ref().is_some_and(|task| !task.is_finished()) {
            drop(client);
            events
                .state(ProxyEvent::Error(format!(
                    "Closed {peer}: only one client connection can be active at a time."
                )))
                .await;
            continue;
        }
        let server = tokio::select! {
            _ = shutdown.wait_for(|stop| *stop) => break,
            server = connect(&upstream) => server,
        };
        let server = match server {
            Ok(server) => server,
            Err(e) => {
                events.state(ProxyEvent::Error(e)).await;
                continue;
            }
        };
        if let Err(e) = client.set_nodelay(true).and(server.set_nodelay(true)) {
            events
                .state(ProxyEvent::Error(format!(
                    "Cannot configure connection: {e}"
                )))
                .await;
            continue;
        }
        let upstream_address = server
            .peer_addr()
            .map(|a| a.to_string())
            .unwrap_or_default();
        events
            .state(ProxyEvent::Connected {
                peer: peer.to_string(),
                upstream: upstream_address,
            })
            .await;
        session = Some(tokio::spawn(relay(
            client,
            server,
            events.clone(),
            shutdown.clone(),
        )));
    }
    drop(listener);
    if let Some(task) = session {
        let _ = task.await;
    }
    events.report_drops().await;
    events.state(ProxyEvent::Stopped).await;
}

fn is_transient(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionAborted
            | io::ErrorKind::ConnectionReset
            | io::ErrorKind::Interrupted
    )
}

async fn connect(upstream: &[SocketAddr]) -> Result<TcpStream, String> {
    let mut last_error = String::new();
    for address in upstream {
        match tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect(address)).await {
            Ok(Ok(stream)) => return Ok(stream),
            Ok(Err(e)) => last_error = format!("{address}: {e}"),
            Err(_) => last_error = format!("{address}: no answer within {CONNECT_TIMEOUT:?}"),
        }
    }
    Err(format!("Cannot connect to upstream ({last_error})"))
}

async fn relay(
    client: TcpStream,
    server: TcpStream,
    events: Events,
    mut shutdown: watch::Receiver<bool>,
) {
    let (client_read, client_write) = client.into_split();
    let (server_read, server_write) = server.into_split();
    let forwarding = async {
        tokio::try_join!(
            forward(
                client_read,
                server_write,
                Direction::ClientToServer,
                &events
            ),
            forward(
                server_read,
                client_write,
                Direction::ServerToClient,
                &events
            ),
        )
    };
    // Leaving the select drops all four halves, which closes both sockets.
    let result = tokio::select! {
        result = forwarding => result.map(|_| ()),
        _ = shutdown.wait_for(|stop| *stop) => Ok(()),
    };
    if let Err(e) = result {
        events
            .state(ProxyEvent::Error(format!("Connection error: {e}")))
            .await;
    }
    events.report_drops().await;
    events.state(ProxyEvent::Disconnected).await;
}

/// Copies one direction until EOF, then half-closes the destination. The other
/// direction stays open, so a server can still answer a client's final send.
async fn forward(
    mut source: OwnedReadHalf,
    mut destination: OwnedWriteHalf,
    direction: Direction,
    events: &Events,
) -> io::Result<()> {
    let mut buffer = vec![0u8; IO_CHUNK];
    loop {
        let count = source.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        events.data(direction, &buffer[..count]);
        destination.write_all(&buffer[..count]).await?;
    }
    destination.shutdown().await
}
