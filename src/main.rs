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
use packet_handler::packet::Packet;

use tokio::signal::unix::{signal, SignalKind};
// lazy_static::lazy_static! {
//     static ref PACKET_HANDLER: tokio::sync::Mutex<packet_handler::PacketHandler> = tokio::sync::Mutex::new(packet_handler::PacketHandler::new());

// }

#[tokio::main]

async fn main() {
    check_if_root();
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

async fn fetch_packets() -> Vec<Packet> {
    let url = "https://api.sulek.dev/releases/MAC63-202307041149-55201637/messages";
    let response = reqwest::get(url)
        .await
        .expect("Failed to fetch packets")
        .json::<serde_json::Value>()
        .await
        .expect("Failed to parse JSON");
    let mut packets: Vec<Packet> = Vec::new();
    let response_packets = response
        .get("messages")
        .and_then(|messages| messages.get("incoming"))
        .and_then(|incoming| incoming.as_array())
        .expect("Failed to get incoming packets");
    for packet in response_packets {
        let name = packet
            .get("name")
            .and_then(|name| name.as_str())
            .expect("Failed to get packet name")
            .to_owned();
        let header = packet
            .get("id")
            .and_then(|header| header.as_u64())
            .expect("Failed to get packet header") as u16;

        let packet = Packet::new(None, Some(name), Some(header), "Outgoing");
        packets.push(packet);
    }
    packets
}
