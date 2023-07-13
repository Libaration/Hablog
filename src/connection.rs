use crate::hosts;
use crate::proxy::Proxy;
use std::net::IpAddr;

#[derive(Debug, PartialEq, Clone)]
pub enum ConnectionState {
    Connected,
    Disconnected,
    WaitingToConnect,
    Ready,
}

#[derive(Debug, Clone)]
pub struct Connection {
    pub connection_state: ConnectionState,
    pub port: u16,
    pub game_resolved_ip: Option<IpAddr>,
    pub client_host: String,
    pub game_host: String,
}

impl Connection {
    pub fn set_connection_state(&mut self, state: ConnectionState) {
        self.connection_state = state;
    }
    pub async fn start(&mut self) {
        self.connection_state = ConnectionState::Ready;
        self.prepare_proxy().await;
        let mut proxy = Proxy::new(self);
        proxy.start_proxy().await;
    }

    pub async fn prepare_proxy(&mut self) {
        match hosts::resolve_host(self).await {
            Ok(ip) => {
                self.game_resolved_ip = Some(ip);
            }
            Err(e) => {
                eprintln!("Failed to resolve host: {}", e);
                self.connection_state = ConnectionState::Disconnected;
            }
        }
    }
}
