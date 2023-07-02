use std::io::{Read, Write};
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
    pub server: Option<Server>,
}

impl Client {
    pub fn new() -> Client {
        Client {
            host: String::from("127.0.0.1"),
            port: 38101,
            connection_state: ConnectionState::Disconnected,
            server: None,
        }
    }

    pub fn listen(&mut self) {
        self.connection_state = ConnectionState::WaitingToConnect;
        let listener = TcpListener::bind((self.host.as_str(), self.port)).expect("Failed to bind");

        println!("Listening on {}:{}", self.host, self.port);
        let server = Server::new(String::from("54.165.147.223"), 38101);
        self.server = Some(server);
        self.server.as_ref().unwrap().connect();
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

#[derive(Debug)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub connection_state: ConnectionState,
}

impl Server {
    pub fn new(host: String, port: u16) -> Server {
        Server {
            host,
            port,
            connection_state: ConnectionState::Disconnected,
        }
    }
    pub fn connect(&self) {
        println!("Connecting to server...");
        let mut stream = TcpStream::connect(
            self.host.as_str().to_owned() + ":" + self.port.to_string().as_str(),
        )
        .expect("Failed to connect");
        println!("Connected to server");
        let mut buffer = [0; 1024];
        loop {
            match stream.read(&mut buffer) {
                Ok(n) => {
                    if n == 0 {
                        println!("Connection closed by remote end");
                        break;
                    }
                    let message = String::from_utf8_lossy(&buffer[..n]);
                    println!("Received message: {}", message);
                    println!("{:?}", &buffer)
                }
                Err(e) => {
                    println!("Error reading from socket: {}", e);
                    break;
                }
            }
        }
    }
}
