mod connection;
pub mod hosts;
pub mod logger;
pub mod proxy;
pub mod packet_handler {
    pub mod packet;
    pub mod packet_handler;
}
use connection::Connection;
use logger::ConsoleLogger;

use tokio::signal::unix::{signal, SignalKind};

#[tokio::main]

async fn main() {
    check_if_root();
    packet_handler::packet_handler::PacketHandler::fetch_packets().await;
    ConsoleLogger::normal("Preparing connection...");
    let game_host = String::from("game-us.habbo.com");
    let port = 38101;
    let client_host = String::from("127.0.0.1");
    let mut connection = Connection {
        game_resolved_ip: None,
        port,
        game_host,
        connection_state: connection::ConnectionState::Disconnected,
        // packet_handler: &PACKET_HANDLER,
        client_host,
    };

    ConsoleLogger::normal("Initializing PacketHandler...");
    // println!("Waiting for packets...");
    // connection.packet_handler.add_packets(fetch_packets().await);

    tokio::spawn(async move {
        connection.start().await;
    });

    let mut term_signal = signal(SignalKind::terminate()).expect("Failed to set up signal handler");
    term_signal.recv().await;

    println!("Closing connection...");
}

fn check_if_root() {
    if unsafe { libc::getuid() } != 0 {
        println!("You must run this program as root.");
        std::process::exit(1);
    }
}
