use crate::logger::ConsoleLogger;
use crate::packet_handler::packet::Packet;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

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
        self.process(&mut buf.to_vec());
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

    fn process(&mut self, buffer: &mut Vec<u8>) -> Vec<Packet> {
        let direction_ref = &self.direction;
        if buffer.len() < 6 || direction_ref == &"Out" {
            return Vec::new();
        }

        let mut constructor_packet = Packet::new(Some(buffer.to_vec()), None, None, direction_ref);

        let mut packets_collection: Vec<Packet> = Vec::new();

        while constructor_packet.total_bytes() >= 4
            && constructor_packet.total_bytes() - 4 >= constructor_packet.read_length() as usize
        {
            let new_packet = Packet::new(
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
            packets_collection.push(new_packet);
            *buffer = Self::copy_with_padding(
                &buffer,
                constructor_packet.read_length() as usize + 4,
                buffer.len(),
            );
            constructor_packet = Packet::new(Some(buffer.to_vec()), None, None, direction_ref);
        }
        match packets_collection.len() {
            0 => {}
            1 => {
                println!(
                    "Packet: {}",
                    String::from_utf8_lossy(
                        &packets_collection[0].packet_in_bytes.as_ref().unwrap()
                    )
                );
            }
            _ => {
                println!("Multiple packets");
            }
        }
        packets_collection
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
}
