use hablog::model::Direction;
use hablog::proxy::{ProxyConfig, ProxyEvent, ProxyHandle};
use std::future::Future;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::Receiver;

const TIMEOUT: Duration = Duration::from_secs(5);

async fn within<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(TIMEOUT, future)
        .await
        .expect("timed out")
}

struct Fixture {
    upstream: TcpListener,
    handle: ProxyHandle,
    events: Receiver<ProxyEvent>,
    address: String,
}

impl Fixture {
    async fn new() -> Self {
        let upstream = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let (handle, mut events) = ProxyHandle::start(ProxyConfig {
            listen: "127.0.0.1:0".into(),
            upstream: upstream.local_addr().unwrap().to_string(),
        })
        .await
        .unwrap();
        let address = match within(events.recv()).await.unwrap() {
            ProxyEvent::Listening { address } => address,
            event => panic!("unexpected startup event: {event:?}"),
        };
        Self {
            upstream,
            handle,
            events,
            address,
        }
    }

    async fn connect(&mut self) -> (TcpStream, TcpStream) {
        let client = within(TcpStream::connect(&self.address)).await.unwrap();
        let (server, _) = within(self.upstream.accept()).await.unwrap();
        wait_for(&mut self.events, |event| {
            matches!(event, ProxyEvent::Connected { .. })
        })
        .await;
        (client, server)
    }
}

async fn wait_for(events: &mut Receiver<ProxyEvent>, predicate: impl Fn(&ProxyEvent) -> bool) {
    loop {
        let event = within(events.recv()).await.expect("event stream ended");
        assert!(
            !matches!(event, ProxyEvent::Error(_)),
            "proxy error: {event:?}"
        );
        if predicate(&event) {
            return;
        }
    }
}

#[tokio::test]
async fn forwards_binary_bytes_in_both_directions_and_captures_exact_bytes() {
    let mut fixture = Fixture::new().await;
    let (mut client, mut server) = fixture.connect().await;
    let request: Vec<_> = (0..262_144).map(|index| index as u8).collect();
    let response: Vec<_> = (0..131_072)
        .map(|index| (255 - index % 256) as u8)
        .collect();

    let mut received = vec![0; request.len()];
    let (written, read) = within(async {
        tokio::join!(client.write_all(&request), server.read_exact(&mut received))
    })
    .await;
    written.unwrap();
    read.unwrap();
    assert_eq!(received, request);

    let mut received = vec![0; response.len()];
    let (written, read) = within(async {
        tokio::join!(
            server.write_all(&response),
            client.read_exact(&mut received)
        )
    })
    .await;
    written.unwrap();
    read.unwrap();
    assert_eq!(received, response);

    let mut captured_request = Vec::new();
    let mut captured_response = Vec::new();
    while captured_request.len() < request.len() || captured_response.len() < response.len() {
        match within(fixture.events.recv()).await.unwrap() {
            ProxyEvent::Data {
                direction: Direction::ClientToServer,
                bytes,
            } => captured_request.extend(bytes),
            ProxyEvent::Data {
                direction: Direction::ServerToClient,
                bytes,
            } => captured_response.extend(bytes),
            event => panic!("unexpected event: {event:?}"),
        }
    }
    assert_eq!(captured_request, request);
    assert_eq!(captured_response, response);
}

#[tokio::test]
async fn half_close_flushes_final_bytes_and_allows_server_reply_then_reconnect() {
    let mut fixture = Fixture::new().await;
    let (mut client, mut server) = fixture.connect().await;
    client.write_all(b"last request\0\xff").await.unwrap();
    client.shutdown().await.unwrap();
    let mut received = Vec::new();
    within(server.read_to_end(&mut received)).await.unwrap();
    assert_eq!(received, b"last request\0\xff");
    server.write_all(b"last reply\x80\0").await.unwrap();
    server.shutdown().await.unwrap();
    received.clear();
    within(client.read_to_end(&mut received)).await.unwrap();
    assert_eq!(received, b"last reply\x80\0");
    wait_for(&mut fixture.events, |event| {
        matches!(event, ProxyEvent::Disconnected)
    })
    .await;

    let (mut next_client, mut next_server) = fixture.connect().await;
    next_client.write_all(b"reconnected").await.unwrap();
    let mut message = [0; 11];
    within(next_server.read_exact(&mut message)).await.unwrap();
    assert_eq!(&message, b"reconnected");
}

#[tokio::test]
async fn a_second_client_is_closed_while_one_is_active() {
    let mut fixture = Fixture::new().await;
    let (mut client, mut server) = fixture.connect().await;
    let mut extra = within(TcpStream::connect(&fixture.address)).await.unwrap();
    match within(fixture.events.recv()).await.unwrap() {
        ProxyEvent::Error(message) => assert!(message.contains("only one client")),
        event => panic!("unexpected event: {event:?}"),
    }
    let mut byte = [0];
    assert_eq!(within(extra.read(&mut byte)).await.unwrap_or(0), 0);

    client.write_all(b"still here").await.unwrap();
    let mut message = [0; 10];
    within(server.read_exact(&mut message)).await.unwrap();
    assert_eq!(&message, b"still here");
}

#[tokio::test]
async fn stop_closes_listener_and_both_connected_sockets() {
    let mut fixture = Fixture::new().await;
    let (mut client, mut server) = fixture.connect().await;
    within(fixture.handle.stop()).await;
    let mut byte = [0];
    assert_eq!(within(client.read(&mut byte)).await.unwrap(), 0);
    assert_eq!(within(server.read(&mut byte)).await.unwrap(), 0);
    assert!(TcpStream::connect(&fixture.address).await.is_err());
    wait_for(&mut fixture.events, |event| {
        matches!(event, ProxyEvent::Stopped)
    })
    .await;
    fixture.handle.stop().await;
}

#[tokio::test]
async fn rejects_non_loopback_listeners() {
    let result = ProxyHandle::start(ProxyConfig {
        listen: "0.0.0.0:0".into(),
        upstream: "127.0.0.1:1".into(),
    })
    .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn rejects_an_upstream_that_resolves_to_its_own_listener() {
    // What a hosts-file redirect of the game host looks like to the relay.
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = free.local_addr().unwrap().to_string();
    drop(free);
    let result = ProxyHandle::start(ProxyConfig {
        listen: address.clone(),
        upstream: address,
    })
    .await;
    match result {
        Err(message) => assert!(message.contains("own listener"), "{message}"),
        Ok(_) => panic!("a relay to itself must be refused"),
    }
}

#[tokio::test]
async fn a_full_capture_queue_never_stalls_forwarding() {
    let mut fixture = Fixture::new().await;
    let (mut client, mut server) = fixture.connect().await;
    // Far more than the event queue's capacity; events are deliberately unread.
    let request = vec![0xA5; 32 * 1024 * 1024];
    let expected_len = request.len();
    let writer = tokio::spawn(async move {
        client.write_all(&request).await.unwrap();
        client.shutdown().await.unwrap();
        client
    });
    let mut received = Vec::new();
    within(server.read_to_end(&mut received)).await.unwrap();
    assert_eq!(received.len(), expected_len);
    assert!(received.iter().all(|&byte| byte == 0xA5));
    let _client = within(writer).await.unwrap();
    within(fixture.handle.stop()).await;

    let mut events = Vec::new();
    while let Ok(event) = fixture.events.try_recv() {
        events.push(event);
    }
    assert!(events.iter().any(|event| matches!(
        event,
        ProxyEvent::Error(message) if message.contains("dropped") && message.contains("Forwarding continued")
    )), "saturated capture should report its omitted chunks");
    assert!(
        events.iter().all(|event| !matches!(
            event,
            ProxyEvent::Error(message) if !message.contains("dropped")
        )),
        "unexpected proxy error: {events:?}"
    );
    assert!(events
        .iter()
        .any(|event| matches!(event, ProxyEvent::Stopped)));
}
