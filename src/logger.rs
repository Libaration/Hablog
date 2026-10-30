use crate::model::Direction;
use crate::packet_handler::packet::Packet;
use chrono::Local;
use std::{
    fmt::{Debug, Display},
    io::Write,
};
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

pub struct ConsoleLogger {}

impl ConsoleLogger {
    fn print_timestamp(stdout: &mut StandardStream) {
        stdout.set_color(ColorSpec::new().set_dimmed(true)).unwrap();
        write!(stdout, "{} ", Local::now().format("%H:%M:%S%.3f")).unwrap();
        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
    }

    fn print_log<T: Display>(level: &str, message: T, color: Color) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        Self::print_timestamp(&mut stdout);
        let log_prefix = format!("[hablog]{}", level);

        stdout
            .set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))
            .unwrap();
        write!(&mut stdout, "{}", log_prefix).unwrap();
        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
        writeln!(&mut stdout, ":: {}", message).unwrap();
    }

    pub fn log_packet<T: Debug>(mut packet: Packet, body: &[u8]) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        Self::print_timestamp(&mut stdout);
        let log_prefix = format!("[{}]", packet.direction.short());
        let direction_color = direction_color(packet.direction);
        let header = format!("[{}]", packet.get_header());
        let name = format!("[{}]", packet.name.unwrap_or_default());

        let body = format!("[{}]", filter_special_chars(body));

        stdout
            .set_color(
                ColorSpec::new()
                    .set_bold(true)
                    .set_fg(Some(direction_color)),
            )
            .unwrap();
        write!(&mut stdout, "{}", log_prefix).unwrap();
        stdout.set_color(ColorSpec::new().set_reset(true)).unwrap();

        stdout
            .set_color(ColorSpec::new().set_bold(true).set_fg(Some(Color::Cyan)))
            .unwrap();
        write!(&mut stdout, "{}", header).unwrap();
        stdout.set_color(ColorSpec::new().set_reset(true)).unwrap();

        stdout
            .set_color(ColorSpec::new().set_bold(true).set_fg(Some(Color::Yellow)))
            .unwrap();
        write!(&mut stdout, "{}", name).unwrap();
        stdout.set_color(ColorSpec::new().set_reset(true)).unwrap();

        stdout
            .set_color(ColorSpec::new().set_bold(true).set_fg(Some(Color::Green)))
            .unwrap();
        writeln!(&mut stdout, "{}", body).unwrap();
        stdout.set_color(ColorSpec::new().set_reset(true)).unwrap();
        fn filter_special_chars(data: &[u8]) -> String {
            let body = String::from_utf8_lossy(data).trim().to_string();
            body.chars().filter(|c| !c.is_control()).collect::<String>()
        }
    }

    /// One relayed TCP chunk, for traffic Hablog doesn't split into packets.
    pub fn log_chunk(direction: Direction, bytes: &[u8]) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        Self::print_timestamp(&mut stdout);
        Self::print_segment(
            &mut stdout,
            format!("[{}]", direction.short()),
            direction_color(direction),
        );
        Self::print_segment(
            &mut stdout,
            format!("[wire {} B]", bytes.len()),
            Color::Cyan,
        );
        writeln!(&mut stdout, "[{}]", escaped(bytes, 64)).unwrap();
    }

    /// One decrypted operation reported by the Origins observer.
    pub fn log_decrypted(direction: Direction, kind: &str, sequence: &str, plaintext: &[u8]) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        Self::print_timestamp(&mut stdout);
        Self::print_segment(
            &mut stdout,
            format!("[{}]", direction.short()),
            direction_color(direction),
        );
        Self::print_segment(&mut stdout, format!("[{kind} #{sequence}]"), Color::Cyan);
        Self::print_segment(
            &mut stdout,
            format!("[{}]", escaped(plaintext, 4096)),
            Color::Green,
        );
        writeln!(&mut stdout).unwrap();
    }

    fn print_segment(stdout: &mut StandardStream, text: String, color: Color) {
        stdout
            .set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))
            .unwrap();
        write!(stdout, "{}", text).unwrap();
        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
    }

    pub fn info<T: Display>(message: T) {
        Self::print_log("", message, Color::Blue);
    }

    pub fn success<T: Display>(message: T) {
        Self::print_log("", message, Color::Green);
    }

    pub fn warning<T: Display>(message: T) {
        Self::print_log("", message, Color::Yellow);
    }

    pub fn error<T: Display>(error: T) {
        let log_message = format!("[hablog]:: Error: {}", error);
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        Self::print_timestamp(&mut stdout);

        stdout
            .set_color(
                ColorSpec::new()
                    .set_bold(true)
                    .set_fg(Some(Color::Red))
                    .set_intense(true)
                    .set_italic(true),
            )
            .unwrap();
        writeln!(&mut stdout, "{}", log_message).unwrap();
        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
    }
    pub fn print_dashes() {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        let colors = [
            Color::Red,
            Color::Yellow,
            Color::Green,
            Color::Cyan,
            Color::Blue,
            Color::Magenta,
        ];

        for color in &colors {
            stdout
                .set_color(ColorSpec::new().set_fg(Some(*color)))
                .unwrap();
            writeln!(
                &mut stdout,
                "--------------------------------------------------------------------"
            )
            .unwrap();
        }

        stdout.set_color(&ColorSpec::new().set_reset(true)).unwrap();
    }
    pub fn print_rainbow_text<T: Display>(message: T) {
        let mut stdout = StandardStream::stdout(ColorChoice::Always);
        let colors = [
            Color::Red,
            Color::Yellow,
            Color::Green,
            Color::Cyan,
            Color::Blue,
            Color::Magenta,
        ];
        let rainbow_chars = format!("[hablog]:: {}", message)
            .chars()
            .enumerate()
            .collect::<Vec<_>>();

        for (index, c) in rainbow_chars {
            let color = colors[index % colors.len()];
            stdout
                .set_color(
                    ColorSpec::new()
                        .set_bold(true)
                        .set_underline(true)
                        .set_fg(Some(color)),
                )
                .unwrap();
            write!(&mut stdout, "{}", c).unwrap();
        }

        writeln!(&mut stdout).unwrap();

        let binary_data = r#"
         01100001 01101010 01101001 01101110 00100000 01101001 01110011 00100000
         01100001 00100000 01100010 01101001 01101110 01100001 01110010 01111001
         00100000 01100100 01100001 01110100 01100001 00101100 00100000 01110011
         01110101 01110010 01100101 00101100 00100000 01101110 01101111 00111111
        "#;

        let binary_data_lines = binary_data.trim().lines();

        for (index, line) in binary_data_lines.enumerate() {
            let color = colors[index % colors.len()];
            stdout
                .set_color(ColorSpec::new().set_bold(true).set_fg(Some(color)))
                .unwrap();
            writeln!(
                &mut stdout,
                "{}",
                line.replace(" ", "").replace("00100000", " ")
            )
            .unwrap();
        }
    }

    pub fn custom<T: Display>(level: &str, message: T, color: Color) {
        Self::print_log(level, message, color);
    }

    pub fn normal<T: Display>(message: T) {
        Self::print_log("", message, Color::White);
    }
}

fn direction_color(direction: Direction) -> Color {
    match direction {
        Direction::ServerToClient => Color::Red,
        Direction::ClientToServer => Color::Magenta,
    }
}

/// Printable ASCII as is, everything else escaped, cut off after `limit` bytes.
pub fn escaped(bytes: &[u8], limit: usize) -> String {
    let mut result = String::new();
    for &byte in bytes.iter().take(limit) {
        match byte {
            b'\n' => result.push_str("\\n"),
            b'\r' => result.push_str("\\r"),
            b'\t' => result.push_str("\\t"),
            b'\\' => result.push_str("\\\\"),
            32..=126 => result.push(byte as char),
            _ => result.push_str(&format!("\\x{byte:02x}")),
        }
    }
    if bytes.len() > limit {
        result.push_str(&format!(" … {} more bytes", bytes.len() - limit));
    }
    result
}
