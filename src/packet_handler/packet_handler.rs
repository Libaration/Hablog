use crate::logger::ConsoleLogger;
use crate::packet_handler::packet::Packet;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

lazy_static::lazy_static! {
    static ref PACKET_COLLECTION: Mutex<Vec<Packet>> = Mutex::new(vec![]);
}
#[derive(Debug, Clone)]
pub struct PacketHandler<'a> {
    out_stream: &'a Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    direction: &'static str,
}

impl PacketHandler<'_> {
    pub fn new<'a>(
        out_stream: &'a Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
        direction: &'static str,
    ) -> PacketHandler<'a> {
        PacketHandler {
            out_stream,
            direction,
        }
    }

    pub async fn forward(&mut self, buf: &[u8]) {
        let mut out_stream = self.out_stream.lock().await;
        out_stream.write_all(buf).await.unwrap();
        out_stream.flush().await.unwrap();
        // ConsoleLogger::info(format!("Packet: {}", String::from_utf8_lossy(buf)));
        self.process(&mut buf.to_vec()).await;
    }

    pub fn extract_packet(&self, buffer: &mut Vec<u8>) -> Vec<Packet> {
        // Check if the buffer has enough data for a complete packet via the first 4 bytes which should give length
        if buffer.len() < 6 {
            return Vec::new();
        }

        // get the packet length from the first 4 bytes
        let mut packet = Packet::new(Some(buffer.to_vec()), None, None, "Outgoing");
        let packet_length = packet.read_length() as usize;

        // but does the packet length it told us match the actual length of the buffer?
        if buffer.len() < packet_length {
            return Vec::new();
        }

        // get header and body from packet
        let packet_header = &buffer[4..6];
        let _packet_body = buffer[6..packet_length].to_vec();
        let header_value = u16::from_be_bytes([packet_header[0], packet_header[1]]);
        // take the processed packet out of the buffer
        buffer.drain(0..packet_length);

        return Vec::new();
    }

    async fn get_packet_info(mut packet: Packet) -> Packet {
        let packet_header = packet.get_header();
        let packet_info = {
            let collection_lock = PACKET_COLLECTION.lock().await;
            collection_lock
                .iter()
                .find(|packet_info| packet_info.header == Some(packet_header))
                .cloned()
        };
        let packet_info = match packet_info {
            Some(info) => info,
            None => packet.clone(),
        };
        let packet_name = packet_info.name.clone();

        packet.name = packet_name;

        packet
    }

    async fn process(&mut self, buffer: &mut Vec<u8>) {
        let direction_ref = &self.direction;
        if buffer.len() < 6 || direction_ref == &"Out" {
            return ();
        }

        let mut constructor_packet = Packet::new(Some(buffer.to_vec()), None, None, direction_ref);

        let mut packets_collection: Vec<Packet> = Vec::new();

        while constructor_packet.total_bytes() >= 4
            && constructor_packet.total_bytes() - 4 >= constructor_packet.read_length() as usize
        {
            let mut new_packet = Packet::new(
                Some(Self::copy_with_padding(
                    &buffer,
                    0,
                    constructor_packet.read_length() as usize + 4,
                )),
                None,
                None,
                direction_ref,
            );

            //advance the buffer to the next packet
            new_packet = Self::get_packet_info(new_packet).await;
            let packet_clone = new_packet.clone();
            packets_collection.push(new_packet);
            *buffer = Self::copy_with_padding(
                &buffer,
                constructor_packet.read_length() as usize + 4,
                buffer.len(),
            );
            constructor_packet = Packet::new(Some(buffer.to_vec()), None, None, direction_ref);
            tokio::spawn(async move { Self::process_packet(packet_clone) }).await;
        }
    }

    pub fn process_packet(mut packet: Packet) {
        // let packet_name = packet.name.clone().unwrap();
        // let packet_header = packet.get_header();
        let packet_body = packet.get_body();
        // let packet_direction = packet.direction.clone();

        ConsoleLogger::log_packet::<Packet>(packet, &packet_body);
    }

    pub fn copy_with_padding(original: &[u8], from: usize, to: usize) -> Vec<u8> {
        let length = original.len();
        let padding = if to > length { to - length } else { 0 };
        let padded_to = to + padding;

        let mut result = Vec::with_capacity(padded_to - from);
        result.extend_from_slice(&original[from..to]);

        if padding > 0 {
            result.extend(vec![0; padding]);
        }

        result
    }

    pub async fn fetch_packets() {
        let url = "https://api.sulek.dev/releases/MAC63-202307041149-55201637/messages";
        let response = reqwest::get(url)
            .await
            .expect("Failed to fetch packets")
            .json::<serde_json::Value>()
            .await
            .expect("Failed to parse JSON");

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

            let packet = Packet::new(None, Some(name), Some(header), "Out");
            PACKET_COLLECTION.lock().await.push(packet);
        }
    }
}
