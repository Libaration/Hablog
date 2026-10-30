use crate::logger::ConsoleLogger;
use crate::model::Direction;
use crate::packet_handler::packet::Packet;
use std::collections::HashMap;
use std::time::Duration;

const PACKETS_URL: &str = "https://api.sulek.dev/releases/MAC63-202307041149-55201637/messages";
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
// Anything longer is not a Habbo packet: the stream is encrypted or out of step.
const MAX_PACKET_LENGTH: usize = 1024 * 1024;

/// Message names for both directions, keyed by header.
#[derive(Debug, Default)]
pub struct PacketNames {
    incoming: HashMap<u16, String>,
    outgoing: HashMap<u16, String>,
}

impl PacketNames {
    pub async fn fetch() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .get(PACKETS_URL)
            .send()
            .await
            .and_then(|response| response.error_for_status())
            .map_err(|e| e.to_string())?;
        let json = response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| e.to_string())?;
        Self::from_json(&json)
    }

    pub fn from_json(json: &serde_json::Value) -> Result<Self, String> {
        let list = |direction: &str| -> Result<HashMap<u16, String>, String> {
            let messages = json
                .get("messages")
                .and_then(|messages| messages.get(direction))
                .and_then(|messages| messages.as_array())
                .ok_or_else(|| format!("The response has no {direction} message list"))?;
            Ok(messages
                .iter()
                .filter_map(|message| {
                    let header = u16::try_from(message.get("id")?.as_u64()?).ok()?;
                    let name = message.get("name")?.as_str()?;
                    Some((header, name.to_owned()))
                })
                .collect())
        };
        Ok(PacketNames {
            incoming: list("incoming")?,
            outgoing: list("outgoing")?,
        })
    }

    pub fn get(&self, direction: Direction, header: u16) -> Option<&str> {
        let names = match direction {
            Direction::ClientToServer => &self.outgoing,
            Direction::ServerToClient => &self.incoming,
        };
        names.get(&header).map(String::as_str)
    }

    pub fn count(&self) -> usize {
        self.incoming.len() + self.outgoing.len()
    }
}

/// Reassembles one direction's TCP chunks into Habbo packets: a 4-byte
/// big-endian length, then a 2-byte header and the body. TCP can split a
/// packet across reads or put several in one read, so partial data waits for
/// the next chunk.
#[derive(Debug)]
pub struct PacketHandler {
    direction: Direction,
    buffer: Vec<u8>,
    desynced: bool,
}

impl PacketHandler {
    pub fn new(direction: Direction) -> Self {
        PacketHandler {
            direction,
            buffer: Vec::new(),
            desynced: false,
        }
    }

    /// Forgets partial data, for a new connection.
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.desynced = false;
    }

    /// True once a length prefix was impossible. Habbo encrypts client to
    /// server traffic after the handshake, so it stops parsing there.
    pub fn is_desynced(&self) -> bool {
        self.desynced
    }

    /// Appends a TCP chunk and returns every packet it completes.
    pub fn push(&mut self, chunk: &[u8], names: &PacketNames) -> Vec<Packet> {
        let mut packets = Vec::new();
        if self.desynced {
            return packets;
        }
        self.buffer.extend_from_slice(chunk);
        let mut offset = 0;
        while self.buffer.len() - offset >= 4 {
            let length_bytes = [
                self.buffer[offset],
                self.buffer[offset + 1],
                self.buffer[offset + 2],
                self.buffer[offset + 3],
            ];
            let length = u32::from_be_bytes(length_bytes) as usize;
            if !(2..=MAX_PACKET_LENGTH).contains(&length) {
                self.desynced = true;
                self.buffer = Vec::new();
                return packets;
            }
            let end = offset + 4 + length;
            if self.buffer.len() < end {
                break;
            }
            let mut packet = Packet::new(
                Some(self.buffer[offset..end].to_vec()),
                None,
                None,
                self.direction,
            );
            let header = packet.get_header();
            packet.header = Some(header);
            packet.name = names.get(self.direction, header).map(str::to_owned);
            packets.push(packet);
            offset = end;
        }
        self.buffer.drain(..offset);
        packets
    }

    pub fn process_packet(mut packet: Packet) {
        let packet_body = packet.get_body();

        ConsoleLogger::log_packet::<Packet>(packet, &packet_body);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(header: u16, body: &[u8]) -> Vec<u8> {
        let mut bytes = ((body.len() + 2) as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(&header.to_be_bytes());
        bytes.extend_from_slice(body);
        bytes
    }

    fn names() -> PacketNames {
        PacketNames::from_json(&serde_json::json!({
            "messages": {
                "incoming": [{ "id": 7, "name": "ChatEvent" }],
                "outgoing": [{ "id": 7, "name": "ChatComposer" }]
            }
        }))
        .unwrap()
    }

    #[test]
    fn reassembles_a_packet_split_across_chunks() {
        let names = names();
        let mut handler = PacketHandler::new(Direction::ServerToClient);
        let bytes = frame(7, b"hello");
        assert!(handler.push(&bytes[..3], &names).is_empty());
        assert!(handler.push(&bytes[3..8], &names).is_empty());
        let mut packets = handler.push(&bytes[8..], &names);
        assert_eq!(packets.len(), 1);
        let mut packet = packets.remove(0);
        assert_eq!(packet.header, Some(7));
        assert_eq!(packet.name.as_deref(), Some("ChatEvent"));
        assert_eq!(packet.get_body(), b"hello");
    }

    #[test]
    fn splits_several_packets_in_one_chunk_and_keeps_the_remainder() {
        let names = names();
        let mut handler = PacketHandler::new(Direction::ClientToServer);
        let mut bytes = frame(7, b"a");
        bytes.extend(frame(9, b""));
        let third = frame(7, b"later");
        bytes.extend_from_slice(&third[..5]);
        let packets = handler.push(&bytes, &names);
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].name.as_deref(), Some("ChatComposer"));
        assert_eq!(packets[1].header, Some(9));
        assert_eq!(packets[1].name, None);
        let packets = handler.push(&third[5..], &names);
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].header, Some(7));
    }

    #[test]
    fn stops_parsing_after_an_impossible_length_until_reset() {
        let names = names();
        let mut handler = PacketHandler::new(Direction::ClientToServer);
        assert!(handler
            .push(&[0xde, 0xad, 0xbe, 0xef, 1, 2], &names)
            .is_empty());
        assert!(handler.is_desynced());
        assert!(handler.push(&frame(7, b"x"), &names).is_empty());
        handler.reset();
        assert!(!handler.is_desynced());
        assert_eq!(handler.push(&frame(7, b"x"), &names).len(), 1);
    }

    #[test]
    fn reads_names_for_both_directions() {
        let names = names();
        assert_eq!(names.count(), 2);
        assert_eq!(names.get(Direction::ServerToClient, 7), Some("ChatEvent"));
        assert_eq!(
            names.get(Direction::ClientToServer, 7),
            Some("ChatComposer")
        );
        assert_eq!(names.get(Direction::ClientToServer, 8), None);
    }
}
