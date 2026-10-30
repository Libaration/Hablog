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
// Message bodies and raw chunks are cut off after this many bytes.
const BODY_LIMIT: usize = 4096;
const RAW_LIMIT: usize = 64;

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

    /// One game message, as an aligned row: direction, the two-character
    /// header, its numeric ID, then the body.
    pub fn log_message(direction: Direction, header: &[u8], id: Option<u16>, body: &[u8]) {
        line(false, |buffer| {
            direction_column(buffer, direction);
            segment(
                buffer,
                format!("{:<2}", readable(header, usize::MAX)),
                &bold(Color::Cyan),
            );
            segment(
                buffer,
                format!(" {:>4}", id.map(|id| id.to_string()).unwrap_or_default()),
                &plain(Color::Cyan),
            );
            if !body.is_empty() {
                let _ = write!(buffer, "  ");
                write_readable(buffer, body, BODY_LIMIT);
            }
        });
    }

    /// Raw traffic, such as an encrypted TCP chunk, kept visually in the background.
    pub fn log_raw(direction: Direction, label: &str, bytes: &[u8]) {
        let mut dim = ColorSpec::new();
        dim.set_dimmed(true);
        line(false, |buffer| {
            direction_column(buffer, direction);
            segment(
                buffer,
                format!(
                    "{label:<7}  {}  ({} B)",
                    readable(bytes, RAW_LIMIT),
                    bytes.len()
                ),
                &dim,
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

fn direction_column(buffer: &mut Buffer, direction: Direction) {
    segment(
        buffer,
        format!("{:>3}  ", direction.short().to_ascii_uppercase()),
        &bold(direction_color(direction)),
    );
}

fn plain(color: Color) -> ColorSpec {
    let mut spec = ColorSpec::new();
    spec.set_fg(Some(color));
    spec
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

/// Printable ASCII as is and any other byte as `[n]`, the way Habbo packet
/// loggers show them (`[2]` ends a string), cut off after `limit` bytes.
pub fn readable(bytes: &[u8], limit: usize) -> String {
    let mut result = String::new();
    for &byte in bytes.iter().take(limit) {
        if (32..127).contains(&byte) {
            result.push(byte as char);
        } else {
            result.push_str(&format!("[{byte}]"));
        }
    }
    if bytes.len() > limit {
        result.push_str(&format!(" … {} more bytes", bytes.len() - limit));
    }
    result
}

/// The reverse of `readable`: `[n]` becomes byte n, the rest is taken as is.
pub fn from_readable(text: &str) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('[') {
        bytes.extend_from_slice(&rest.as_bytes()[..start]);
        let after = &rest[start + 1..];
        match after
            .find(']')
            .and_then(|end| Some((after[..end].parse::<u8>().ok()?, end)))
        {
            Some((byte, end)) => {
                bytes.push(byte);
                rest = &after[end + 1..];
            }
            None => {
                bytes.push(b'[');
                rest = after;
            }
        }
    }
    bytes.extend_from_slice(rest.as_bytes());
    bytes
}

/// `readable`, with the `[n]` bytes dimmed so the text stands out.
fn write_readable(buffer: &mut Buffer, bytes: &[u8], limit: usize) {
    let mut dim = ColorSpec::new();
    dim.set_dimmed(true);
    let shown = &bytes[..bytes.len().min(limit)];
    for run in shown.chunk_by(|a, b| (32..127).contains(a) == (32..127).contains(b)) {
        if (32..127).contains(&run[0]) {
            let _ = buffer.write_all(run);
        } else {
            segment(buffer, readable(run, usize::MAX), &dim);
        }
    }
    if bytes.len() > limit {
        segment(
            buffer,
            format!(" … {} more bytes", bytes.len() - limit),
            &dim,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shows_unprintable_bytes_as_numbers_and_cuts_long_input() {
        assert_eq!(
            readable(b"fuse_login\x02@\x7f\xe9", 64),
            "fuse_login[2]@[127][233]"
        );
        assert_eq!(readable(&[b'a'; 10], 4), "aaaa … 6 more bytes");
    }

    #[test]
    fn reads_back_what_it_shows() {
        let bytes = b"@BPBfuse_login\x02[_D\x01";
        assert_eq!(from_readable(&readable(bytes, usize::MAX)), bytes);
        assert_eq!(from_readable("a[256]b[x][2"), b"a[256]b[x][2");
    }

    #[test]
    fn dims_only_the_unprintable_bytes() {
        let mut buffer = Buffer::no_color();
        write_readable(&mut buffer, b"ab\x02\x01cd", 64);
        assert_eq!(buffer.as_slice(), b"ab[2][1]cd");
    }
}
