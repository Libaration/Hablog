use hot_lib::*;

use crate::packet_handler::PacketHandler;
use std::net::{IpAddr, ToSocketAddrs};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

#[hot_lib_reloader::hot_module(dylib = "lib")]
mod hot_lib {
    hot_functions_from_file!("lib/src/lib.rs");
}

#[derive(Debug, PartialEq)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    WaitingToConnect,
    Ready,
}

#[derive(Debug)]
pub struct Connection {
    pub connection_state: ConnectionState,
    pub from_ip: Option<IpAddr>,
    pub port: u16,
    pub host: String,
    pub packet_handler: PacketHandler,
}

impl<'a> Connection {
    pub async fn resolve_host(&mut self) -> Result<IpAddr, String> {
        let host = format!("{}:{}", self.host, self.port);
        println!("Resolving host {}", host);
        match host.to_socket_addrs() {
            Ok(addrs) => {
                let ip = addrs
                    .filter(|addr| match addr.ip() {
                        IpAddr::V4(_) => true,
                        IpAddr::V6(_) => false,
                    })
                    .next()
                    .map(|addr| addr.ip())
                    .ok_or_else(|| "No suitable address found".to_string())?;
                self.connection_state = ConnectionState::Ready;
                self.from_ip = Some(ip);
                println!("Resolved host to {}", self.from_ip.unwrap());
                Ok(self.from_ip.unwrap())
            }
            Err(e) => {
                self.connection_state = ConnectionState::Disconnected;
                Err(format!("Failed to resolve host: {}", e))
            }
        }
    }

    pub async fn start(&mut self) {
        let listener = TcpListener::bind(("0.0.0.0", self.port)).await;

        match listener {
            Ok(listener) => {
                println!("Listening for incoming connections on port {}", self.port);
                self.connection_state = ConnectionState::WaitingToConnect;
                loop {
                    match listener.accept().await {
                        Ok((socket, _)) => {
                            println!("New connection from {}", socket.peer_addr().unwrap());
                            self.handle_incoming_connection(socket).await;
                        }
                        Err(e) => {
                            eprintln!("Failed to accept connection: {}", e);
                            break; // Break out of the loop on connection error
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Failed to bind listener: {}", e);
                self.connection_state = ConnectionState::Disconnected;
            }
        }
    }

    async fn handle_incoming_connection(&mut self, mut socket: TcpStream) {
        println!("Connecting to {}...", self.from_ip.unwrap());
        let connector = TcpStream::connect((self.from_ip.unwrap(), 38101))
            .await
            .expect("could not connect to server");

        let (mut server_reader, mut server_writer) = connector.into_split();

        let (mut client_reader, mut client_writer) = socket.into_split();

        let mut packet_handler = self.packet_handler.clone();
        let mut buf = Vec::new();
        let mut server_buf = Vec::new();
        let s_spawn = tokio::spawn(async move {
            loop {
                let mut temp_buf = vec![0; 16384];
                match server_reader.read(&mut temp_buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        server_buf.extend_from_slice(&temp_buf[..n]);
                        parse(&mut server_buf, &mut String::from("CLIENT"));
                        client_writer
                            .write_all(&server_buf)
                            .await
                            .expect("Failed to write to client");
                        server_buf.clear();
                        client_writer.flush().await.unwrap();
                    }
                    Err(e) => {
                        println!("Server disconnect");
                        eprintln!("Error reading from server: {}", e);
                        break;
                    }
                }
            }
        });

        let c_spawn = tokio::spawn(async move {
            loop {
                let mut temp_buf = vec![0u8; 16384];
                match client_reader.read(&mut temp_buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&temp_buf[..n]);
                        parse(&mut buf, &mut String::from("SERVER"));
                        server_writer
                            .write_all(&buf)
                            .await
                            .expect("Failed to write to server");
                        buf.clear();
                        server_writer.flush().await.unwrap();
                    }
                    Err(e) => {
                        println!("Client disconnect");
                        eprintln!("Error reading from client: {}", e);
                        break;
                    }
                }
            }
        });

        tokio::try_join!(s_spawn, c_spawn).expect("Failed to join tasks");
    }
}
