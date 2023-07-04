//u8 = 1 byte = 8 bits = 2 hex chars = 0x00
//u16 = 2 bytes = 16 bits = 4 hex chars = 0x0000
//u32 = 4 bytes = 32 bits = 8 hex chars = 0x00000000
//u64 = 8 bytes = 64 bits = 16 hex chars = 0x0000000000000000

use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;

pub struct Packet {
    packetInBytes: Vec<u8>,
    cursor: Cursor<Vec<u8>>,
}

impl Packet {
    pub fn new(packet: Vec<u8>) -> Self {
        Packet {
            packetInBytes: packet,
            cursor: Cursor::new(packet),
        }
    }

    fn read_short(&mut self, index: Option<usize>) -> u16 {
        let index = match index {
            Some(index) => index,
            None => self.cursor.position() as usize,
        };
        self.cursor.read_u16::<LittleEndian>().unwrap()
    }

    fn get_header(&mut self) -> u16 {
        self.read_short(Some(4))
    }
}
