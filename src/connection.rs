use std::net::{IpAddr, ToSocketAddrs};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

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
}

impl Connection {
    fn default() -> Self {
        Self {
            connection_state: ConnectionState::Disconnected,
            from_ip: None,
            port: 38101,
            host: String::from("game-us.habbo.com"),
        }
    }

    pub fn resolve_host(&mut self) -> Result<IpAddr, String> {
        let host = format!("{}:{}", self.host, self.port);
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
                Ok(ip)
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
                            println!("Connection state: {:?}", self.connection_state);
                            println!("Connected on port {}", self.port);
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

    async fn handle_incoming_connection(&mut self, socket: TcpStream) {
        // Connect to the real server
        println!("Connecting to the real server");
        match TcpStream::connect(("54.165.147.223", self.port)).await {
            Ok(remote_stream) => {
                println!("Connected to the real server");
                self.connection_state = ConnectionState::Connected;
                self.forward_data(socket, remote_stream).await;
            }
            Err(e) => {
                eprintln!("Failed to connect to the real server: {}", e);
                self.connection_state = ConnectionState::Disconnected;
            }
        }
    }

    async fn forward_data(&mut self, mut client: TcpStream, mut server: TcpStream) {
        let mut client_buffer = vec![0; 32768];
        let mut server_buffer = vec![0; 32768];

        let (mut client_reader, mut client_writer) = client.split();
        let (mut server_reader, mut server_writer) = server.split();

        loop {
            tokio::select! {
                res = client_reader.read(&mut client_buffer) => {
                    match res {
                        Ok(0) => break,
                        Ok(n) => {
                            println!("CLIENT -> SERVER, {} bytes sent", n);
                            if let Err(e) = server_writer.write_all(&client_buffer[..n]).await {
                                eprintln!("Error forwarding data from client to server: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Error reading from client: {}", e);
                            break;
                        }
                    }
                }
                res = server_reader.read(&mut server_buffer) => {
                    match res {
                        Ok(0) => break,
                        Ok(n) => {
                            println!("SERVER -> CLIENT, {} bytes sent", n);
                            if let Err(e) = client_writer.write_all(&server_buffer[..n]).await {
                                eprintln!("Error forwarding data from server to client: {}", e);
                                break;
                            }
                        }
                        Err(e) => {
                            eprintln!("Error reading from server: {}", e);
                            break;
                        }
                    }
                }
            }
        }

        self.connection_state = ConnectionState::Disconnected;
        println!("Connection closed");
    }
}
