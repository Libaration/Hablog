//! Route the installed Origins client through the local relay.
//!
//! The original Axed forced the Shockwave page to `connection.info.host=
//! 127.0.0.1`. The modern client reads `connection.info.host`/`port` from a
//! length-prefixed string embedded in the projector executable. This module
//! reproduces that redirection without touching the installed client: it copies
//! the projector and overwrites only the host *value* in place (padding with
//! ';' so the byte length — and the Director length prefix — stay valid), so the
//! copy connects to the loopback relay while the port and everything else is
//! unchanged. The real client, launcher and hosts file are never modified.

use std::fs;
use std::path::{Path, PathBuf};

use crate::origins::discovery::DiscoverySnapshot;

const HOST_KEY: &[u8] = b"connection.info.host=";
const LOOPBACK: &[u8] = b"127.0.0.1";
const ROUTED_NAME: &str = "Hablog-Routed.exe";

#[derive(Clone, Debug)]
pub struct RoutePlan {
    /// The installed projector to copy (`Habbo.exe` or a regional client).
    pub source_exe: PathBuf,
    /// Where the patched copy is written (inside the installation directory so
    /// its `Xtras/BobbaXtra.x32` resolves for the observer).
    pub routed_exe: PathBuf,
    /// `installation/Xtras/BobbaXtra.x32`, for the observer's module check.
    pub module_path: PathBuf,
    /// Loopback address the relay listens on (`127.0.0.1:<info port>`).
    pub listen: String,
    /// The real game endpoint the relay forwards to (`host:port`).
    pub upstream: String,
    /// File offset of the host *value* (just past `connection.info.host=`).
    pub host_value_offset: usize,
    /// Original host value length that the replacement must preserve.
    pub host_value_len: usize,
    /// The original host, used to verify the patch site before writing.
    pub original_host: String,
    pub server_code: Option<String>,
}

impl RoutePlan {
    /// Build a plan from a discovery snapshot, for the current default client
    /// or an explicit region. Does not read or write anything.
    pub fn from_discovery(
        snapshot: &DiscoverySnapshot,
        server_code: Option<&str>,
    ) -> Result<Self, String> {
        let plan = snapshot.launch_plan(server_code)?;
        let endpoint = &plan.endpoint;
        let source_exe = plan.executable.clone();
        let root = plan.working_directory.clone();
        let host_value_offset = endpoint.host_property_offset as usize + HOST_KEY.len();
        if LOOPBACK.len() > endpoint.host.len() {
            return Err(format!(
                "The embedded host '{}' is too short to hold a loopback address.",
                endpoint.host
            ));
        }
        Ok(Self {
            source_exe,
            routed_exe: root.join(ROUTED_NAME),
            module_path: root.join("Xtras").join("BobbaXtra.x32"),
            listen: format!("127.0.0.1:{}", endpoint.port),
            upstream: format!("{}:{}", endpoint.host, endpoint.port),
            host_value_offset,
            host_value_len: endpoint.host.len(),
            original_host: endpoint.host.clone(),
            server_code: plan.server_code.clone(),
        })
    }

    /// Copy the projector and redirect its embedded host to loopback. Returns
    /// the path of the patched copy. The installed client is never modified.
    pub fn write_patched_projector(&self) -> Result<PathBuf, String> {
        let mut bytes = fs::read(&self.source_exe)
            .map_err(|e| format!("Cannot read {}: {e}", self.source_exe.display()))?;
        patch_host_in_place(
            &mut bytes,
            self.host_value_offset,
            self.host_value_len,
            self.original_host.as_bytes(),
            LOOPBACK,
        )?;
        // Refuse to overwrite the real client; only our named copy is written.
        if paths_equal(&self.routed_exe, &self.source_exe) {
            return Err("The routed copy must not overwrite the installed client.".into());
        }
        // An unchanged copy is left alone, so one that is still running
        // (and therefore locked on Windows) doesn't block a restart.
        if fs::read(&self.routed_exe).is_ok_and(|existing| existing == bytes) {
            return Ok(self.routed_exe.clone());
        }
        fs::write(&self.routed_exe, &bytes).map_err(|e| {
            format!(
                "Cannot write {} (is it still running?): {e}",
                self.routed_exe.display()
            )
        })?;
        Ok(self.routed_exe.clone())
    }
}

/// Overwrite the host value at `offset..offset+len` with `new_host`, verifying
/// the original bytes first and padding with ';' so the length is preserved.
/// The ';' terminates the host for the client's `;`-delimited parser; the extra
/// separators become empty entries it ignores.
pub fn patch_host_in_place(
    bytes: &mut [u8],
    offset: usize,
    len: usize,
    expected_host: &[u8],
    new_host: &[u8],
) -> Result<(), String> {
    let end = offset
        .checked_add(len)
        .ok_or("Host value offset overflow.")?;
    if end > bytes.len() {
        return Err("Host value lies outside the projector image.".into());
    }
    if &bytes[offset..end] != expected_host {
        return Err(
            "The projector's embedded host does not match discovery; refusing to patch.".into(),
        );
    }
    if new_host.len() > len {
        return Err("The replacement host is longer than the original value.".into());
    }
    let mut value = Vec::with_capacity(len);
    value.extend_from_slice(new_host);
    while value.len() < len {
        value.push(b';');
    }
    bytes[offset..end].copy_from_slice(&value);
    Ok(())
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    let a = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    if cfg!(windows) {
        a.to_string_lossy()
            .eq_ignore_ascii_case(&b.to_string_lossy())
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> (Vec<u8>, usize, usize) {
        // MZ header + the real length-prefixed connection string layout.
        let mut bytes = b"MZ\x90\x00".to_vec();
        bytes.extend_from_slice(b"\x00\x00\x00\x00\x43");
        let prefix = bytes.len();
        let value = b"connection.info.host=game-ous.habbo.com;connection.info.port=40001\x00";
        bytes.extend_from_slice(value);
        let host_key_off = prefix; // "connection.info.host=" starts here
        let host_value_off = host_key_off + HOST_KEY.len();
        let host_len = b"game-ous.habbo.com".len();
        (bytes, host_value_off, host_len)
    }

    fn read_pair(bytes: &[u8]) -> (String, String) {
        let text = String::from_utf8_lossy(bytes);
        let start = text.find("connection.info.host=").unwrap() + HOST_KEY.len();
        let host: String = text[start..]
            .chars()
            .take_while(|c| c.is_ascii_graphic() && *c != ';')
            .collect();
        let pstart = text.find("connection.info.port=").unwrap() + HOST_KEY.len();
        let port: String = text[pstart..]
            .chars()
            .take_while(|c| c.is_ascii_graphic() && *c != ';')
            .collect();
        (host, port)
    }

    #[test]
    fn redirects_host_to_loopback_and_keeps_port() {
        let (mut bytes, off, len) = sample();
        let original_len = bytes.len();
        patch_host_in_place(&mut bytes, off, len, b"game-ous.habbo.com", LOOPBACK).unwrap();
        assert_eq!(bytes.len(), original_len, "byte length must be preserved");
        let (host, port) = read_pair(&bytes);
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, "40001");
    }

    #[test]
    fn refuses_when_original_host_does_not_match() {
        let (mut bytes, off, len) = sample();
        let err =
            patch_host_in_place(&mut bytes, off, len, b"wrong-host.example", LOOPBACK).unwrap_err();
        assert!(err.contains("does not match"));
    }

    #[test]
    fn refuses_offset_past_end() {
        let mut bytes = vec![0u8; 10];
        assert!(patch_host_in_place(&mut bytes, 8, 8, b"xxxxxxxx", LOOPBACK).is_err());
    }

    #[test]
    fn preserves_length_with_shortest_origins_host() {
        // The shortest regional host must still fit a loopback replacement.
        let host = b"game-od.habbo.com";
        let mut bytes = Vec::new();
        bytes.extend_from_slice(HOST_KEY);
        bytes.extend_from_slice(host);
        bytes.push(b';');
        patch_host_in_place(&mut bytes, HOST_KEY.len(), host.len(), host, LOOPBACK).unwrap();
        let host_read: String = String::from_utf8_lossy(&bytes[HOST_KEY.len()..])
            .chars()
            .take_while(|c| c.is_ascii_graphic() && *c != ';')
            .collect();
        assert_eq!(host_read, "127.0.0.1");
    }
}
