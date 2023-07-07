use lib::packet::Packet;
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone)]
pub struct PacketHandler {
    handled_buffer: Vec<u8>,
    packet_queue: VecDeque<Packet>,
    packets: HashSet<Packet>,
}

impl PacketHandler {
    pub fn new() -> Self {
        PacketHandler {
            packets: HashSet::new(),
            handled_buffer: Vec::new(),
            packet_queue: VecDeque::new(),
        }
    }

    pub fn get_stream(&mut self, proxy_buf: Vec<u8>) -> &[u8] {
        if self.handled_buffer.is_empty() {
            self.handled_buffer = proxy_buf;
        } else {
            self.handled_buffer.extend(proxy_buf);
        }
        &self.handled_buffer
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
