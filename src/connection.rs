use hot_lib::*;
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
        println!("Connecting to server...");
        let mut connector = TcpStream::connect(("54.165.147.223", 38101))
            .await
            .expect("could not connect to server");
        // match connector {
        //     Ok(mut stream) => {
        //         println!("Connected to server");
        //         self.connection_state = ConnectionState::Connected;
        //     }
        //     Err(e) => {
        //         eprintln!("Failed to connect to server: {}", e);
        //         self.connection_state = ConnectionState::Disconnected;
        //         return;
        //     }
        // }

        let (mut server_reader, mut server_writer) = connector.into_split();

        let (mut client_reader, mut client_writer) = socket.into_split();

        let mut buf = vec![0; 4096];
        let mut server_buf = vec![0; 4096];

        let s_spawn = tokio::spawn(async move {
            loop {
                match server_reader.read(&mut server_buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        // println!("CLIENT -> SERVER, {} bytes sent", n);
                        //println!("{}", String::from_utf8_lossy(&server_buf[..n]));
                        // let hex = hex::encode(&server_buf[..n]);
                        // println!("{}", hex);
                        parse(&mut server_buf[..n], &mut String::from("CLIENT"));
                        client_writer
                            .write_all(&server_buf[..n])
                            .await
                            .expect("Failed to write to client");
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
                match client_reader.read(&mut buf).await {
                    Ok(0) => break,
                    Ok(n) => {
                        // println!("SERVER -> CLIENT, {} bytes sent", n);
                        parse(&mut buf[..n], &mut String::from("SERVER"));
                        server_writer
                            .write_all(&buf[..n])
                            .await
                            .expect("Failed to write to server");
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

// CLIENT : 0000002e02c6000000010000000000000002000000050003302e300000000600000006000d2f666c61746374726c20342f2f
// SERVER : ddb55f45ca171a9305203f0da2f7
// CLIENT : 0000003902c6000000010000000000000002000000050003302e30000000020000000200182f666c61746374726c20342f6d7620332c352c302e302f2f
// CLIENT : 0000002e02c6000000010000000000000003000000050003302e300000000200000002000d2f666c61746374726c20342f2f
// SERVER : 0af898b7c71b0246e32123769946
// CLIENT : 0000003902c6000000010000000000000003000000050003302e30000000060000000600182f666c61746374726c20342f6d7620322c352c302e302f2f
// SERVER : 30d010119e428f584f491dd82e33
// CLIENT : 0000002e02c6000000010000000000000002000000050003302e300000000600000006000d2f666c61746374726c20342f2f
// SERVER : 2c8cb5e261ca601e7673
// CLIENT : 0000000604bd00000005
// SERVER : 400db74912ad88ead9a38bff12bcebdbdf72
// CLIENT : 0000003902c6000000010000000000000002000000050003302e30000000020000000200182f666c61746374726c20342f6d7620332c352c302e302f2f
// CLIENT : 0000002e02c6000000010000000000000003000000050003302e300000000200000002000d2f666c61746374726c20342f2f
// SERVER : 30563b21306d326cbe9d0d9a6013
// CLIENT : 0000003902c6000000010000000000000003000000050003302e30000000060000000600182f666c61746374726c20342f6d7620322c352c302e302f2f
// CLIENT : 0000002e02c6000000010000000000000002000000050003302e300000000600000006000d2f666c61746374726c20342f2f
// SERVER : 8f55559945b59eb9a2bd53e9faa5
// CLIENT : 0000003902c6000000010000000000000002000000050003302e30000000020000000200182f666c61746374726c20342f6d7620332c352c302e302f2f
// CLIENT : 0000002e02c6000000010000000000000003000000050003302e300000000200000002000d2f666c61746374726c20342f2f
// SERVER : 2077fc36507b47108d363b263898
// CLIENT : 0000003902c6000000010000000000000003000000050003302e30000000060000000600182f666c61746374726c20342f6d7620322c352c302e302f2f
// CLIENT : 0000002e02c6000000010000000000000002000000050003302e300000000600000006000d2f666c61746374726c20342f2f
// SERVER : 442a5af1b5513963ba15fbe2cf8d
// CLIENT : 0000003902c6000000010000000000000002000000050003302e30000000020000000200182f666c61746374726c20342f6d7620332c352c302e302f2f
// CLIENT : 0000002e02c6000000010000000000000003000000050003302e300000000200000002000d2f666c61746374726c20342f2f
// SERVER : 294f8960b8cb
// CLIENT : 0000000a06cd0000000000000000
