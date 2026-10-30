use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    ClientToServer,
    ServerToClient,
}

impl Direction {
    pub fn label(self) -> &'static str {
        match self {
            Self::ClientToServer => "Client to server",
            Self::ServerToClient => "Server to client",
        }
    }

    /// Habbo's own naming: the client sends "outgoing" messages and receives "incoming" ones.
    pub fn short(self) -> &'static str {
        match self {
            Self::ClientToServer => "Out",
            Self::ServerToClient => "In",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrafficRecord {
    pub index: u64,
    pub timestamp_ms: u64,
    pub direction: Direction,
    #[serde(with = "hex_bytes")]
    pub bytes: Vec<u8>,
}

pub(crate) mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(value))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let text = String::deserialize(d)?;
        hex::decode(text).map_err(serde::de::Error::custom)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
