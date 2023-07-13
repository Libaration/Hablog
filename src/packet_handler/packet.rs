use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use std::io::Cursor;
use std::io::Read;
#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct Packet {
    pub packet_in_bytes: Option<Vec<u8>>,
    pub bytes: Vec<u8>,
    pub position: usize,
    pub name: Option<String>,
    pub header: Option<u16>,
    pub direction: &'static str, // Incoming or Outgoing. Never changes thus I think it's better to be static?
}

impl Packet {
    pub fn new(
        packet: Option<Vec<u8>>,
        name: Option<String>,
        header: Option<u16>,
        direction: &'static str,
    ) -> Self {
        let bytes = packet.clone();
        Packet {
            packet_in_bytes: Some(packet).unwrap(),
            bytes: bytes.unwrap_or_default(),
            position: 0,
            name: Some(name).unwrap(),
            header,
            direction,
        }
    }

    fn read_u32(&mut self, index: Option<usize>) -> u32 {
        if let Some(index) = index {
            self.position = index;
        }
        let value = &self.bytes[self.position..];
        self.position += 4;
        let mut cursor = Cursor::new(value);
        cursor.read_u32::<BigEndian>().unwrap()
    }

    fn read_short(&mut self, index: Option<usize>) -> u16 {
        if let Some(index) = index {
            self.position = index;
        }
        let value = &self.bytes[self.position..];
        self.position += 2;
        let mut cursor = Cursor::new(value);
        match cursor.read_u16::<BigEndian>() {
            Ok(value) => value,
            Err(_) => 0,
        }
    }

    pub fn read_long(&mut self, index: Option<usize>) -> u32 {
        if let Some(index) = index {
            self.position = index;
        }
        let value = &self.bytes[self.position..];
        self.position += 4;
        let mut cursor = Cursor::new(value);
        cursor.read_u32::<BigEndian>().unwrap()
    }

    pub fn get_header(&mut self) -> u16 {
        self.read_short(Some(4))
    }

    pub fn get_body(&mut self) -> Vec<u8> {
        let bytes = if let Some(bytes) = self.packet_in_bytes.clone() {
            self.read_bytes(bytes.len() - 6, Some(6))
        } else {
            Vec::new()
        };
        bytes
    }

    pub fn read_bytes(&mut self, length: usize, index: Option<usize>) -> Vec<u8> {
        if let Some(index) = index {
            self.position = index;
        }
        let value = &self.bytes[self.position..];
        self.position += length;
        let mut cursor = Cursor::new(value);
        let mut bytes = vec![0; length];
        cursor.read_exact(&mut bytes).unwrap();
        bytes
    }

    pub fn read_byte(&mut self) -> u8 {
        let value = self.bytes[self.position];
        self.position += 1;
        value
    }

    pub fn total_bytes(&self) -> usize {
        self.bytes.len()
    }
    pub fn read_length(&mut self) -> u32 {
        self.read_u32(Some(0))
    }

    pub fn to_string(&self) -> String {
        let mut packet_string = String::new();

        for x in &self.bytes {
            // Check if byte is a control character or not
            if (*x < 32 && *x >= 0) || *x == 93 || *x == 91 || *x == 125 || *x == 123 || *x == 127 {
                packet_string.push('[');
                packet_string.push_str(&x.to_string());
                packet_string.push(']');
            } else {
                packet_string.push(*x as char);
            }
        }

        packet_string
    }
}
