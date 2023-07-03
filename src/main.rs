use std::{
    fs::File,
    fs::OpenOptions,
    io::{Read, Write},
};

mod connection;
use connection::Connection;

#[tokio::main]
async fn main() {
    check_if_root();
    let host = String::from("game-us.habbo.com");
    //check_hosts_file(&host);
    let port = 38101;
    let mut connection = Connection {
        from_ip: None,
        port,
        host,
        connection_state: connection::ConnectionState::Disconnected,
    };
    connection.resolve_host().expect("Could not resolve host");
    connection.start().await
}

fn check_if_root() {
    if unsafe { libc::getuid() } != 0 {
        println!("You must run this program as root.");
        std::process::exit(1);
    }
}

fn check_hosts_file(host: &str) {
    let mut hosts_file = OpenOptions::new()
        .read(true)
        .append(true)
        .write(true)
        .open("/etc/hosts")
        .expect("Failed to open hosts file");
    let mut contents = String::new();
    hosts_file
        .read_to_string(&mut contents)
        .expect("Failed to read hosts file");
    println!("Found hosts file: ");
    println!("{}", contents);
    println!("Checking for proxy entry... ");
    if contents.lines().any(|line| line.contains(host)) {
        println!("Found proxy entry for {}.", host);
    } else {
        println!("Not found.");
        add_proxy_entry(host, hosts_file);
    }
}

fn remove_proxy_entry(host: &str, mut hosts_file: File, contents: String) {
    println!("Removing proxy entry for {}...", host);
    let new_contents = contents
        .lines()
        .filter(|line| !line.contains(host))
        .collect::<Vec<&str>>()
        .join("\n");
    hosts_file
        .set_len(0)
        .expect("Failed to truncate hosts file");
    writeln!(hosts_file, "{}", new_contents).expect("Failed to write to file");
}

fn add_proxy_entry(host: &str, mut hosts_file: File) {
    println!("Adding proxy entry for {}...", host);
    writeln!(hosts_file, "127.0.0.1 {}", host).expect("Failed to write to file");
}
