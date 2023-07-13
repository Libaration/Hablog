use crate::connection::{Connection, ConnectionState};
use crate::logger::ConsoleLogger;
use std::net::IpAddr;
use std::net::ToSocketAddrs;
use std::time::Duration;
use tokio::{fs::read_to_string, time::sleep};

pub async fn remove_proxy_if_exists(host: &str) {
    let hosts_file_path = "/etc/hosts";
    let mut contents = read_to_string(hosts_file_path).await.unwrap();

    if contents.lines().any(|line| line.contains(host)) {
        contents = contents
            .lines()
            .filter(|line| !line.contains(host))
            .collect::<Vec<_>>()
            .join("\n");

        tokio::fs::write(hosts_file_path, contents)
            .await
            .expect("Failed to write to hosts file");
        ConsoleLogger::normal(format!("Proxy entry removed for {}.", host));
    } else {
        ConsoleLogger::normal(format!("No proxy entry found for {}.", host));
    }

    sleep(Duration::from_secs(1)).await;
}

pub async fn add_proxy_entry(host: &str) {
    let hosts_file_path = "/etc/hosts";
    let mut contents = read_to_string(hosts_file_path).await.unwrap();

    let entry = format!("127.0.0.1 {}", host);
    contents.push('\n');
    contents.push_str(&entry);

    tokio::fs::write(hosts_file_path, contents)
        .await
        .expect("Failed to write to hosts file");
    ConsoleLogger::normal(format!("Proxy entry added for {}.", host));

    sleep(Duration::from_secs(1)).await;
}

pub async fn resolve_host(connection: &mut Connection) -> Result<IpAddr, String> {
    remove_proxy_if_exists(&connection.game_host).await;
    let host = format!("{}:{}", &connection.game_host, &connection.port);
    ConsoleLogger::normal(format!("Resolving host {}...", host));
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
            ConsoleLogger::normal(format!("Resolved host to {}", ip));
            add_proxy_entry(&connection.game_host).await;
            Ok({
                ConsoleLogger::success(format!(
                    "Tunneling traffic from {} to {}",
                    connection.game_host, connection.client_host
                ));
                ip
            })
        }
        Err(e) => {
            connection.connection_state = ConnectionState::Disconnected;
            Err(format!("Failed to resolve host: {}", e))
        }
    }
}
