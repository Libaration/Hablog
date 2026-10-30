//! Console output. Each line is built in one buffer, so lines never interleave.
//! Traffic lines wait for `ConsoleLogger::flush`, which writes a whole burst at
//! once; status lines are written straight away. Write errors are ignored, so a
//! closed console can't take the relay down.
use crate::model::Direction;
use chrono::Local;
use std::fmt::Display;
use std::io::{IsTerminal, Write};
use std::sync::{Mutex, OnceLock, PoisonError};
use termcolor::{Buffer, BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

// Pending traffic is written early past this size, even without a flush.
const MAX_PENDING: usize = 64 * 1024;

pub struct ConsoleLogger {}

struct Console {
    writer: BufferWriter,
    pending: Mutex<Buffer>,
}

fn console() -> &'static Console {
    static CONSOLE: OnceLock<Console> = OnceLock::new();
    CONSOLE.get_or_init(|| {
        // Colors only for a person watching; redirected output stays plain text.
        let choice = if std::io::stdout().is_terminal() {
            ColorChoice::Auto
        } else {
            ColorChoice::Never
        };
        let writer = BufferWriter::stdout(choice);
        let pending = Mutex::new(writer.buffer());
        Console { writer, pending }
    })
}

impl ConsoleLogger {
    pub fn normal<T: Display>(message: T) {
        Self::status(message, Color::White);
    }

    pub fn info<T: Display>(message: T) {
        Self::status(message, Color::Blue);
    }

    pub fn success<T: Display>(message: T) {
        Self::status(message, Color::Green);
    }

    pub fn warning<T: Display>(message: T) {
        Self::status(message, Color::Yellow);
    }

    pub fn error<T: Display>(error: T) {
        let mut spec = ColorSpec::new();
        spec.set_bold(true)
            .set_fg(Some(Color::Red))
            .set_intense(true)
            .set_italic(true);
        line(true, |buffer| {
            segment(buffer, format!("[hablog]:: Error: {error}"), &spec)
        });
    }

    /// One relayed TCP chunk of (encrypted) game traffic.
    pub fn log_chunk(direction: Direction, bytes: &[u8]) {
        line(false, |buffer| {
            segment(
                buffer,
                format!("[{}]", direction.short()),
                &bold(direction_color(direction)),
            );
            segment(
                buffer,
                format!("[wire {} B]", bytes.len()),
                &bold(Color::Cyan),
            );
            let _ = write!(buffer, "[{}]", escaped(bytes, 64));
        });
    }

    /// One decrypted operation reported by the observer.
    pub fn log_decrypted(direction: Direction, kind: &str, sequence: &str, plaintext: &[u8]) {
        line(false, |buffer| {
            segment(
                buffer,
                format!("[{}]", direction.short()),
                &bold(direction_color(direction)),
            );
            segment(buffer, format!("[{kind} #{sequence}]"), &bold(Color::Cyan));
            segment(
                buffer,
                format!("[{}]", escaped(plaintext, 4096)),
                &bold(Color::Green),
            );
        });
    }

    /// Writes any traffic lines waiting since the last flush.
    pub fn flush() {
        let console = console();
        let mut pending = console
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        print_pending(console, &mut pending);
    }

    fn status<T: Display>(message: T, color: Color) {
        line(true, |buffer| {
            segment(buffer, "[hablog]", &bold(color));
            let _ = write!(buffer, ":: {message}");
        });
    }
}

fn line(print: bool, write: impl FnOnce(&mut Buffer)) {
    let console = console();
    let mut pending = console
        .pending
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let mut timestamp = ColorSpec::new();
    timestamp.set_dimmed(true);
    segment(
        &mut pending,
        Local::now().format("%H:%M:%S%.3f "),
        &timestamp,
    );
    write(&mut pending);
    let _ = writeln!(pending);
    if print || pending.len() >= MAX_PENDING {
        print_pending(console, &mut pending);
    }
}

fn print_pending(console: &Console, pending: &mut Buffer) {
    if !pending.is_empty() {
        let _ = console.writer.print(pending);
        pending.clear();
    }
}

fn segment(buffer: &mut Buffer, text: impl Display, spec: &ColorSpec) {
    let _ = buffer.set_color(spec);
    let _ = write!(buffer, "{text}");
    let _ = buffer.reset();
}

fn bold(color: Color) -> ColorSpec {
    let mut spec = ColorSpec::new();
    spec.set_bold(true).set_fg(Some(color));
    spec
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_binary_and_cuts_long_input() {
        assert_eq!(escaped(b"@B\x01hi\n\\", 64), "@B\\x01hi\\n\\\\");
        assert_eq!(escaped(&[b'a'; 10], 4), "aaaa … 6 more bytes");
    }
}
