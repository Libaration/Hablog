#[no_mangle]
pub fn parse(data: &mut [u8], origin: &mut String) {
    println!("{} : {}", origin, hex::encode(data));
}
