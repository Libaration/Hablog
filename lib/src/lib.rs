use std::io::{self, BufWriter, Write};
pub mod packet;
use packet::Packet;
use std::sync::Mutex;
lazy_static::lazy_static! {
    static ref OUTPUT_MUTEX: Mutex<()> = Mutex::new(());
}
#[no_mangle]
pub fn parse(data: &mut [u8], origin: &mut String) {
    if origin == "SERVER" {
        return;
    }
    let mut packet = Packet::new(Some(data.to_vec()), None, None, Some("Outgoing"));
    // let header = packet.get_header();
    // if header == 3655 && String::from_utf8_lossy(&data).contains("!debug on") {
    //     unsafe { packet::DEBUG = true };
    // } else if header == 3655 && String::from_utf8_lossy(&data).contains("!debug off") {
    //     unsafe { packet::DEBUG = false };
    // }
    let output = format!("{} : {}\n", origin, packet.to_string());

    let _guard = OUTPUT_MUTEX.lock().unwrap();
    let mut handle = BufWriter::new(io::stdout().lock());
    // handle.flush().unwrap();
    if let Err(_) = handle.write_all(output.as_bytes()) {
        println!("error");
    }
}
