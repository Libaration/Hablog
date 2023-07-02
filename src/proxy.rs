use std::io::Read;
use std::net::{TcpListener, TcpStream};
use std::thread;

#[derive(Debug)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    WaitingToConnect,
}

pub struct Client {
    pub host: String,
    pub port: u16,
    pub connection_state: ConnectionState,
}

impl Client {
    pub fn new() -> Client {
        Client {
            host: String::from("127.0.0.1"),
            port: 38101,
            connection_state: ConnectionState::Disconnected,
        }
    }

    pub fn listen(&mut self) {
        self.connection_state = ConnectionState::WaitingToConnect;
        let listener = TcpListener::bind((self.host.as_str(), self.port)).expect("Failed to bind");

        println!("Listening on {}:{}", self.host, self.port);

        match listener.accept() {
            Ok((mut stream, addr)) => {
                println!("New connection: {}", addr);
                self.connection_state = ConnectionState::Connected;
                let mut buffer = [0; 1024];
                loop {
                    match stream.read(&mut buffer) {
                        Ok(n) => {
                            if n == 0 {
                                println!("Connection closed by remote end");
                                self.connection_state = ConnectionState::Disconnected;
                                break;
                            }
                            let message = String::from_utf8_lossy(&buffer[..n]);
                            println!("Received message: {}", message);
                        }
                        Err(e) => {
                            println!("Error reading from socket: {}", e);
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }
        self.connection_state = ConnectionState::Disconnected;
    }
}

pub struct Server {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub connection_state: ConnectionState,
}

impl Server {
    pub fn new() -> Server {
        Server {
            host: None,
            port: None,
            connection_state: ConnectionState::Disconnected,
        }
    }
}
