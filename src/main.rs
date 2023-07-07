use std::{
    fs::File,
    fs::OpenOptions,
    io::{Read, Write},
    time::Duration,
};
mod connection;
mod packet_handler;
use connection::Connection;
use lib::packet::Packet;
use tokio::{
    fs::{read_to_string, write},
    io::AsyncWriteExt,
};
use tokio::{
    signal::unix::{signal, SignalKind},
    time::sleep,
};

#[tokio::main]

async fn main() {
    check_if_root();
    let host = String::from("game-us.habbo.com");
    let port = 38101;
    remove_proxy_entry(&host).await;
    sleep(Duration::from_secs(1)).await;
    let mut connection = Connection {
        from_ip: None,
        port,
        host,
        connection_state: connection::ConnectionState::Disconnected,
        packet_handler: packet_handler::PacketHandler::new(),
    };

    println!("Initializing PacketHandler...");
    // println!("Waiting for packets...");
    // connection.packet_handler.add_packets(fetch_packets().await);
    connection.resolve_host().await.unwrap();
    add_proxy_entry(&connection.host).await;
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

async fn check_hosts_file(host: &str) -> bool {
    let hosts_file_path = "/etc/hosts";
    let contents = read_to_string(hosts_file_path).await.unwrap();
    let proxy_entry_exists = contents.lines().any(|line| line.contains(host));

    if proxy_entry_exists {
        println!("Found proxy entry for {}.", host);
    } else {
        println!("No proxy entry found for {}.", host);
    }
    proxy_entry_exists
}

async fn remove_proxy_entry(host: &str) {
    println!("Removing proxy entry for {}...", host);
    let hosts_file_path = "/etc/hosts";
    let contents = read_to_string(hosts_file_path)
        .await
        .expect("Failed to read hosts file");
    let filtered_lines: Vec<&str> = contents
        .lines()
        .filter(|line| !line.contains(host))
        .collect();
    write(hosts_file_path, filtered_lines.join("\n"))
        .await
        .unwrap();
}

async fn add_proxy_entry(host: &str) {
    let hosts_file_path = "/etc/hosts";
    let mut contents = read_to_string(hosts_file_path).await.unwrap();
    if contents.lines().any(|line| line.contains(host)) {
        println!("Proxy entry for {} already exists.", host);
        return;
    }
    let entry = format!("127.0.0.1 {}", host);
    contents.push('\n');
    contents.push_str(&entry);
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(hosts_file_path)
        .await
        .expect("Failed to open hosts file");
    file.write_all(&contents.as_bytes()).await.unwrap();
    println!("Proxy entry added for {}.", host);
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
            .expect("Failed to get packet name");
        let header = packet
            .get("id")
            .and_then(|header| header.as_u64())
            .expect("Failed to get packet header") as u16;
        let packet = Packet::new(
            None,
            Some(name.to_owned()),
            Some(header),
            Some("Outgoing".to_owned()),
        );
        packets.push(packet);
    }
    packets
}
