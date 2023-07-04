use std::io::{self, BufWriter, Write};

#[no_mangle]
pub fn parse(data: &mut [u8], origin: &mut String) {
    let encoded_data = hex::encode(data);
    let output = format!("{} : {}\n", origin, encoded_data);

    let stdout = io::stdout();
    let mut handle = BufWriter::new(stdout.lock());

    if let Err(_) = handle.write_all(output.as_bytes()) {
        println!("error");
    }
}
