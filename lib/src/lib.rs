use byteorder::{BigEndian, LittleEndian, ReadBytesExt};
use std::collections::HashMap;
use std::io::Cursor;
use std::io::{self, BufWriter, Write};
mod packet;
use packet::Packet;

// type HeaderHandler = fn(&mut [u8]);

// fn walk_header(data: &mut [u8]) {
//     println!("walk {} ", hex::encode(&data));
// }

// fn wave_header(data: &mut [u8]) {
//     println!("wave {} ", hex::encode(&data));
// }

#[no_mangle]
pub fn parse(data: &mut [u8], origin: &mut String) {
    // let headers: HashMap<u32, HeaderHandler> = HashMap::from([
    //     (0x00000039, walk_header as _),
    //     (0x0000000a, wave_header as _),
    // ]);
    if origin == "SERVER" {
        return;
    }
    let mut packet = Packet::new(data.to_vec());
    let header = packet.get_header();
    if header == 3655 && String::from_utf8_lossy(&data).contains("!debug on") {
        unsafe { packet::DEBUG = true };
    } else if header == 3655 && String::from_utf8_lossy(&data).contains("!debug off") {
        unsafe { packet::DEBUG = false };
    }

    let encoded_data = hex::encode(data);
    let hex_bytes: Vec<String> = encoded_data
        .chars()
        .collect::<Vec<char>>()
        .chunks(2)
        .map(|chunk| chunk.iter().collect())
        .collect();
    let hex_output = hex_bytes.join(" ");
    let output = format!(
        "{} : {}\n",
        origin,
        if unsafe { packet::DEBUG } == true {
            hex_output
        } else {
            encoded_data
        }
    );

    let stdout = io::stdout();

    let mut handle = BufWriter::new(stdout.lock());
    if let Err(_) = handle.write_all(output.as_bytes()) {
        println!("error");
    }
}
