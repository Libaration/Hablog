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
            port: 1337,
            connection_state: ConnectionState::Disconnected,
        }
    }

    pub fn listen(&mut self) {
        self.connection_state = ConnectionState::WaitingToConnect;
        let listener = TcpListener::bind((self.host.as_str(), self.port)).expect("Failed to bind");

        println!("Listening on {}:{}", self.host, self.port);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    println!("New connection: {}", stream.peer_addr().unwrap());
                    self.connection_state = ConnectionState::Connected;
                    thread::spawn(move || {
                        handle_client(stream);
                    });
                }
                Err(e) => {
                    println!("Error: {}", e);
                }
            }
        }
        fn handle_client(mut stream: TcpStream) {
            let mut buffer = [0; 1024];
            loop {
                match stream.read(&mut buffer) {
                    Ok(n) => {
                        if n == 0 {
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
