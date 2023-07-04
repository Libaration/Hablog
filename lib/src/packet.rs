//u8 = 1 byte = 8 bits = 2 hex chars = 0x00
//u16 = 2 bytes = 16 bits = 4 hex chars = 0x0000
//u32 = 4 bytes = 32 bits = 8 hex chars = 0x00000000
//u64 = 8 bytes = 64 bits = 16 hex chars = 0x0000000000000000

//the first 4 bytes of each packet seem to be length of the packet
//the next 2 bytes are the header ?

use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use std::io::Cursor;
pub(crate) static mut DEBUG: bool = false;
pub struct Packet {
    packet_in_bytes: Vec<u8>,
    cursor: Cursor<Vec<u8>>,
}

impl Packet {
    pub fn new(packet: Vec<u8>) -> Self {
        Packet {
            packet_in_bytes: packet.clone(),
            cursor: Cursor::new(packet),
        }
    }

    fn read_short(&mut self, index: Option<usize>) -> u16 {
        if let Some(index) = index {
            self.cursor.set_position(index as u64);
        }
        if unsafe { DEBUG } {
            println!("-------------------------------------------------------");
            println!("String: {}", String::from_utf8_lossy(&self.packet_in_bytes));
            if let Some(index) = index {
                println!(
                    "Header Hex: {}",
                    hex::encode(&self.packet_in_bytes[index..index + 2])
                );
            }
        }

        let header = self.cursor.read_u16::<BigEndian>().unwrap();
        if unsafe { DEBUG } {
            println!("Header: {}", header);
            println!(
                "Read 2 bytes. (+4) Moving cursor index to +{}",
                self.cursor.position()
            );
        }

        header
    }

    pub fn get_header(&mut self) -> u16 {
        self.read_short(Some(4))
    }
}
