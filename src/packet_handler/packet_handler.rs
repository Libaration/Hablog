use core::ops::Deref;
use lib::packet::Packet;
use std::{
    collections::{HashSet, VecDeque},
    sync::Arc,
};
use tokio::{
    io::{AsyncWriteExt, BufWriter},
    sync::Mutex,
};

use crate::logger::ConsoleLogger;
#[derive(Debug)]
pub enum PacketResult<'a> {
    Owned(Packet),
    Static(&'a Packet),
}
#[derive(Debug, Clone)]
pub struct PacketHandler<'a> {
    packet_queue: VecDeque<Packet>,
    packets: HashSet<Packet>,
    out_stream: &'a Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    direction: String,
}

impl PacketHandler<'_> {
    pub fn new<'a>(
        out_stream: &'a Arc<tokio::sync::Mutex<tokio::net::tcp::OwnedWriteHalf>>,
        direction: String,
    ) -> PacketHandler<'a> {
        PacketHandler {
            packets: HashSet::new(),
            packet_queue: VecDeque::new(),
            out_stream,
            direction,
        }
    }

    pub async fn forward(&mut self, buf: &[u8]) {
        let mut out_stream = self.out_stream.lock().await;
        out_stream.write_all(buf).await.unwrap();
        out_stream.flush().await.unwrap();
        ConsoleLogger::info(format!("Packet: {}", String::from_utf8_lossy(buf)));
    }

    pub fn extract_packet(&self, buffer: &mut Vec<u8>) -> PacketResult {
        // Check if the buffer has enough data for a complete packet via the first 4 bytes which should give length
        if buffer.len() < 6 {
            return PacketResult::Static(Self::empty_packet());
        }

        // get the packet length from the first 4 bytes
        let mut packet = Packet::new(Some(buffer.to_vec()), None, None, Some("Outgoing"));
        let packet_length = packet.read_length() as usize;

        // but does the packet length it told us match the actual length of the buffer?
        if buffer.len() < packet_length {
            return PacketResult::Static(Self::empty_packet());
        }

        // get header and body from packet
        let packet_header = &buffer[4..6];
        let _packet_body = buffer[6..packet_length].to_vec();
        let header_value = u16::from_be_bytes([packet_header[0], packet_header[1]]);
        // take the processed packet out of the buffer
        buffer.drain(0..packet_length);

        //  still in debate if my packet attribute should contain the full packet bytes or just the body
        PacketResult::Owned(Packet::new(
            Some(buffer.to_vec()),
            None,
            Some(header_value),
            Some("Outgoing"),
        ))
    }
    fn empty_packet() -> &'static Packet {
        //i'm thinkin if we create a static reference to an empty packet
        //when we need to return on invalid buffers this will be more performant??
        //as it won't have to create a new packet every time and will just return the static reference in memory
        //i could also be totally wrong. this could be worse idk.
        static EMPTY_PACKET: Packet = Packet {
            packet_in_bytes: None,
            position: 0,
            name: None,
            header: None,
            direction: None,
            bytes: Vec::new(),
        };

        &EMPTY_PACKET
    }

    pub fn handle_buffer(&mut self, buf: &[u8]) -> Packet {
        // Process the buffer and create a new packet
        let packet = Packet::new(None, None, None, None);

        // Check for duplicates and add the packet
        if !self.packets.contains(&packet) {
            self.packets.insert(packet.clone());
            self.packet_queue.push_back(packet.clone());
        }

        // Return the created packet
        packet
    }

    pub fn get_next_packet(&mut self) -> Option<Packet> {
        self.packet_queue.pop_front()
    }

    pub fn add_packets(&mut self, packets: Vec<Packet>) {
        for packet in packets {
            if !self.packets.contains(&packet) {
                self.packets.insert(packet.clone());
                self.packet_queue.push_back(packet.clone());
            }
        }
    }
}
