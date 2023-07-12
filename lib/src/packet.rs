use byteorder::{BigEndian, ReadBytesExt};
use std::io::Cursor;

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct Packet {
    pub packet_in_bytes: Option<Vec<u8>>,
    pub bytes: Vec<u8>,
    pub position: usize,
    pub name: Option<String>,
    pub header: Option<u16>,
    pub direction: Option<&'static str>, // Incoming or Outgoing. Never changes thus I think it's better to be static?
}

impl Packet {
    pub fn new(
        packet: Option<Vec<u8>>,
        name: Option<String>,
        header: Option<u16>,
        direction: Option<&'static str>,
    ) -> Self {
        let bytes = packet.clone();
        Packet {
            packet_in_bytes: Some(packet).unwrap_or_default(),
            bytes: bytes.unwrap_or_default(),
            position: 0,
            name: Some(name).unwrap(),
            header,
            direction: Some(direction).unwrap(),
        }
    }

    fn read_short(&mut self, index: Option<usize>) -> u16 {
        if let Some(index) = index {
            self.position = index;
        }
        let value = &self.bytes[self.position..];
        self.position += 2;
        let mut cursor = Cursor::new(value);
        cursor.read_u16::<BigEndian>().unwrap()
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

    pub fn read_bytes(&mut self, length: usize) -> Vec<u8> {
        let value = &self.bytes[self.position..self.position + length];
        self.position += length;
        value.to_vec()
    }

    pub fn read_byte(&mut self) -> u8 {
        let value = self.bytes[self.position];
        self.position += 1;
        value
    }

    pub fn read_length(&mut self) -> u16 {
        self.read_short(Some(0))
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
