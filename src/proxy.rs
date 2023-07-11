use crate::{
    connection::{Connection, ConnectionState},
    hosts,
    logger::ConsoleLogger,
};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::Mutex,
};

#[derive(Debug)]
pub struct Proxy<'a> {
    pub connection: &'a mut Connection,
}

impl Proxy<'_> {
    pub fn new(connection: &mut Connection) -> Proxy<'_> {
        Proxy { connection }
    }
    pub async fn start_proxy(&mut self) {
        ConsoleLogger::info(format!(
            "Starting proxy on {}:{}",
            self.connection.client_host, self.connection.port
        ));
        let client_socket = self
            .wait_for_listener_connection()
            .await
            .unwrap()
            .into_split();

        let server_socket = self
            .wait_for_server_connection()
            .await
            .unwrap()
            .into_split();
        let client_arc_read = Arc::new(Mutex::new(client_socket.0));
        let server_arc_read = Arc::new(Mutex::new(server_socket.0));
        let client_arc_write = Arc::new(Mutex::new(client_socket.1));
        let server_arc_write = Arc::new(Mutex::new(server_socket.1));
        let forward_buffers_client_to_server = Self::forward_buffers(
            client_arc_read.clone(),
            server_arc_write.clone(),
            "Out".to_string(),
        );
        let forward_buffers_server_to_client = Self::forward_buffers(
            server_arc_read.clone(),
            client_arc_write.clone(),
            "In".to_string(),
        );
        tokio::join!(
            forward_buffers_client_to_server,
            forward_buffers_server_to_client
        );
    }

    pub async fn wait_for_listener_connection(
        &mut self,
    ) -> Result<tokio::net::TcpStream, tokio::task::JoinError> {
        self.connection
            .set_connection_state(ConnectionState::WaitingToConnect);
        ConsoleLogger::info(format!(
            "Waiting for client to connect on {}:{}",
            self.connection.client_host, self.connection.port
        ));
        let listener = tokio::net::TcpListener::bind(format!(
            "{}:{}",
            self.connection.client_host, self.connection.port
        ))
        .await
        .unwrap();
        let connection = self.connection.clone();
        tokio::spawn(async move {
            let (client_stream, client_address) =
                if let Ok((stream, address)) = listener.accept().await {
                    (stream, address)
                } else {
                    ConsoleLogger::error("Failed to accept client connection");
                    hosts::remove_proxy_if_exists(connection.game_host.as_str()).await;
                    std::process::exit(1);
                };
            ConsoleLogger::success(format!("Caught client connection from {}", client_address));
            client_stream.set_nodelay(true).unwrap();
            return client_stream;
        })
        .await
    }

    pub async fn wait_for_server_connection(
        &mut self,
    ) -> Result<tokio::net::TcpStream, tokio::task::JoinError> {
        let server = tokio::net::TcpStream::connect(format!(
            "{}:{}",
            self.connection.game_resolved_ip.unwrap(),
            self.connection.port
        ));
        tokio::spawn(async move {
            let server_stream = server.await.unwrap();
            ConsoleLogger::success(format!(
                "Connected to game server at {}",
                server_stream.peer_addr().unwrap()
            ));
            server_stream.set_nodelay(true).unwrap();
            return server_stream;
        })
        .await
    }
    pub async fn forward_buffers(
        source: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedReadHalf>>,
        destination: Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
        direction: String,
    ) {
        let mut buffer = [0; 10000];
        let mut source_stream = source.lock().await;
        let mut destination_stream = destination.lock().await;
        loop {
            match source_stream.read(&mut buffer).await {
                Ok(0) => {
                    break;
                }
                Ok(n) => {
                    ConsoleLogger::info(format!(
                        "{}: {}",
                        direction,
                        String::from_utf8_lossy(&buffer[..n])
                    ));
                    destination_stream.write_all(&buffer[..n]).await.unwrap();
                    // destination_stream.flush().await.unwrap();
                }
                Err(e) => {
                    eprintln!("Failed to read from source stream: {}", e);
                    break;
                }
            }
        }
    }
}
