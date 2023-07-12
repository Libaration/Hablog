use crate::{
    connection::{Connection, ConnectionState},
    hosts,
    logger::ConsoleLogger,
    packet_handler::packet_handler::PacketHandler,
};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, BufReader};
use tokio::sync::Mutex;
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

        let forward_buffers_client_to_server = tokio::spawn(async move {
            Self::forward_buffers(client_socket.0, server_socket.1, "Out".to_string()).await;
        });

        let forward_buffers_server_to_client = tokio::spawn(async move {
            Self::forward_buffers(server_socket.0, client_socket.1, "In".to_string()).await;
        });

        let (res1, res2) = tokio::join!(
            forward_buffers_client_to_server,
            forward_buffers_server_to_client
        );
        res1.unwrap();
        res2.unwrap();
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

        let (client_stream, client_address) = if let Ok((stream, address)) = listener.accept().await
        {
            (stream, address)
        } else {
            ConsoleLogger::error("Failed to accept client connection");
            hosts::remove_proxy_if_exists(connection.game_host.as_str()).await;
            std::process::exit(1);
        };
        ConsoleLogger::success(format!("Caught client connection from {}", client_address));
        client_stream.set_nodelay(true).unwrap();
        return Ok(client_stream);
    }

    pub async fn wait_for_server_connection(
        &mut self,
    ) -> Result<tokio::net::TcpStream, tokio::task::JoinError> {
        let server = tokio::net::TcpStream::connect(format!(
            "{}:{}",
            self.connection.game_resolved_ip.unwrap(),
            self.connection.port
        ));

        let server_stream = server.await.unwrap();
        ConsoleLogger::success(format!(
            "Connected to game server at {}",
            server_stream.peer_addr().unwrap()
        ));
        server_stream.set_nodelay(true).unwrap();
        return Ok(server_stream);
    }

    pub async fn forward_buffers(
        source_stream: tokio::net::tcp::OwnedReadHalf,
        destination_stream: tokio::net::tcp::OwnedWriteHalf,
        direction: String,
    ) {
        let mut buffer = [0u8; 10000];
        let mut source_reader = BufReader::new(source_stream);

        let destination_stream_arc = Arc::new(Mutex::new(destination_stream));

        let mut packet_handler = PacketHandler::new(&destination_stream_arc, direction);
        loop {
            //buffer.fill(0);
            let read_length = match source_reader.read(&mut buffer).await {
                Ok(n) if n != 0 => n,
                Ok(_) | Err(_) => {
                    continue;
                }
            };

            packet_handler.forward(&buffer[0..read_length]).await;
        }
    }
}
