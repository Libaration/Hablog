//! Versioned JSONL captures: a header line, then one line per TCP chunk with its
//! direction, timestamp and exact bytes. Each client session gets its own file.
//! Records are buffered; call `flush` after a batch.
use crate::model::{hex_bytes, now_ms, Direction};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "hablog-capture";
pub const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Header {
    pub format: String,
    pub version: u32,
    pub record_kind: String,
    pub client: String,
    pub upstream: String,
    pub started_ms: u64,
}

/// A `TrafficRecord` that borrows its bytes, so recording copies nothing.
#[derive(Serialize)]
struct RecordRef<'a> {
    index: u64,
    timestamp_ms: u64,
    direction: Direction,
    #[serde(serialize_with = "hex_bytes::serialize")]
    bytes: &'a [u8],
}

pub struct CaptureWriter {
    writer: BufWriter<File>,
    path: PathBuf,
    next_index: u64,
}

impl CaptureWriter {
    /// Creates the capture and its directory. An existing file is never overwritten.
    pub fn create(path: &Path, client: &str, upstream: &str) -> Result<Self, String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create capture directory: {e}"))?;
        }
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("Cannot create capture {}: {e}", path.display()))?;
        let mut capture = Self {
            writer: BufWriter::new(file),
            path: path.to_path_buf(),
            next_index: 1,
        };
        capture.write_line(&Header {
            format: FORMAT.into(),
            version: VERSION,
            record_kind: "tcp_chunk".into(),
            client: client.into(),
            upstream: upstream.into(),
            started_ms: now_ms(),
        })?;
        capture.flush()?;
        Ok(capture)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn records(&self) -> u64 {
        self.next_index - 1
    }

    pub fn record(&mut self, direction: Direction, bytes: &[u8]) -> Result<(), String> {
        self.write_line(&RecordRef {
            index: self.next_index,
            timestamp_ms: now_ms(),
            direction,
            bytes,
        })?;
        self.next_index += 1;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<(), String> {
        self.writer
            .flush()
            .map_err(|e| format!("Cannot write capture {}: {e}", self.path.display()))
    }

    fn write_line<T: Serialize>(&mut self, value: &T) -> Result<(), String> {
        serde_json::to_writer(&mut self.writer, value)
            .map_err(io::Error::from)
            .and_then(|_| self.writer.write_all(b"\n"))
            .map_err(|e| format!("Cannot write capture {}: {e}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TrafficRecord;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    fn path() -> PathBuf {
        std::env::temp_dir().join(format!(
            "hablog-capture-{}-{}/session.jsonl",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn writes_header_and_exact_binary_records() {
        let path = path();
        let mut capture = CaptureWriter::create(&path, "127.0.0.1:50000", "1.2.3.4:38101").unwrap();
        capture
            .record(Direction::ServerToClient, &[0, 1, 2, 127, 128, 255])
            .unwrap();
        capture.record(Direction::ClientToServer, b"hi").unwrap();
        assert_eq!(capture.records(), 2);
        capture.flush().unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let mut lines = text.lines();
        let header: Header = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(header.format, FORMAT);
        assert_eq!(header.version, VERSION);
        assert_eq!(header.client, "127.0.0.1:50000");
        assert_eq!(header.upstream, "1.2.3.4:38101");
        let records: Vec<TrafficRecord> = lines.map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].index, 1);
        assert_eq!(records[0].direction, Direction::ServerToClient);
        assert_eq!(records[0].bytes, vec![0, 1, 2, 127, 128, 255]);
        assert_eq!(records[1].index, 2);
        assert_eq!(records[1].direction, Direction::ClientToServer);
        assert_eq!(records[1].bytes, b"hi");
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn refuses_to_overwrite_an_existing_capture() {
        let path = path();
        drop(CaptureWriter::create(&path, "a", "b").unwrap());
        assert!(CaptureWriter::create(&path, "a", "b").is_err());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
