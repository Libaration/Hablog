//! Running under WSL. Hablog is a Linux program there, but Origins, its
//! launcher and the observer's Node are Windows programs: WSL's interop runs
//! them, and `wslpath` converts paths between the two sides.
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

pub fn detected() -> bool {
    static WSL: OnceLock<bool> = OnceLock::new();
    *WSL.get_or_init(|| {
        cfg!(target_os = "linux")
            && (std::env::var_os("WSL_DISTRO_NAME").is_some()
                || fs::read_to_string("/proc/sys/kernel/osrelease")
                    .is_ok_and(|release| is_wsl_kernel(&release)))
    })
}

/// Whether Hablog can start Windows programs: on Windows, or through WSL.
pub fn runs_windows_programs() -> bool {
    cfg!(windows) || detected()
}

fn is_wsl_kernel(release: &str) -> bool {
    let release = release.to_ascii_lowercase();
    release.contains("microsoft") || release.contains("wsl")
}

/// A Windows folder from an environment variable such as `APPDATA`, as a WSL
/// path. Without interop it falls back to the only Windows profile whose
/// `under_profile` folder contains `marker`.
pub fn windows_folder(variable: &str, under_profile: &str, marker: &str) -> Option<PathBuf> {
    windows_variable(variable)
        .and_then(|value| from_windows(Path::new(&value)))
        .or_else(|| scan_profiles(under_profile, marker))
}

fn windows_variable(variable: &str) -> Option<String> {
    // cmd.exe complains on stderr when started from a Linux folder, but
    // still prints the value.
    let output = Command::new("cmd.exe")
        .args(["/d", "/c", &format!("echo %{variable}%")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    // An unset variable is echoed back as written.
    (!value.is_empty() && !value.contains('%')).then_some(value)
}

fn scan_profiles(under_profile: &str, marker: &str) -> Option<PathBuf> {
    let mut found = fs::read_dir("/mnt/c/Users")
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(under_profile))
        .filter(|folder| folder.join(marker).exists());
    let first = found.next()?;
    // Several Windows users with Origins: don't guess.
    found.next().is_none().then_some(first)
}

/// `C:\Users\me` to `/mnt/c/Users/me`. None for anything that isn't a
/// Windows path, or when `wslpath` is unavailable.
pub fn from_windows(path: &Path) -> Option<PathBuf> {
    if !is_windows_path(&path.to_string_lossy()) {
        return None;
    }
    wslpath("-u", path)
}

/// `/mnt/c/Users/me` to `C:\Users\me`, for Windows programs. The path must exist.
pub fn to_windows(path: &Path) -> Option<PathBuf> {
    wslpath("-w", &fs::canonicalize(path).ok()?)
}

fn wslpath(flag: &str, path: &Path) -> Option<PathBuf> {
    let output = Command::new("wslpath")
        .arg(flag)
        .arg(path)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim_end_matches(['\r', '\n']);
    (output.status.success() && !value.is_empty()).then(|| PathBuf::from(value))
}

/// A drive path such as `C:\x` or `c:/x`.
fn is_windows_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'\\' || bytes[2] == b'/')
}

/// Whether `path` is on a mounted Windows drive, where Windows programs can
/// also use it as a working folder.
pub fn on_windows_drive(path: &Path) -> bool {
    to_windows(path).is_some_and(|windows| is_windows_path(&windows.to_string_lossy()))
}

/// A path as the user would type it into Windows: under WSL, its Windows form.
pub fn display_path(path: &Path) -> String {
    detected()
        .then(|| to_windows(path))
        .flatten()
        .unwrap_or_else(|| path.to_path_buf())
        .display()
        .to_string()
}

/// Image names and PIDs of the running Windows programs, from `tasklist.exe`.
pub fn windows_processes() -> Result<Vec<(u32, String)>, String> {
    let output = Command::new("tasklist.exe")
        .args(["/fo", "csv", "/nh"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| format!("Cannot list Windows programs: {e}"))?;
    Ok(parse_tasklist(&String::from_utf8_lossy(&output.stdout)))
}

/// Parses `tasklist /fo csv /nh` rows: `"Image.exe","1234","Console","1","9,876 K"`.
pub fn parse_tasklist(text: &str) -> Vec<(u32, String)> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.trim().trim_matches('"').split("\",\"");
            let name = fields.next()?.to_owned();
            let pid = fields.next()?.parse().ok()?;
            Some((pid, name))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_wsl_kernels() {
        assert!(is_wsl_kernel("5.15.167.4-microsoft-standard-WSL2\n"));
        assert!(is_wsl_kernel("4.4.0-19041-Microsoft"));
        assert!(!is_wsl_kernel("6.8.0-45-generic"));
    }

    #[test]
    fn recognizes_windows_drive_paths() {
        assert!(is_windows_path(r"C:\Users\me\AppData\Roaming"));
        assert!(is_windows_path("d:/games"));
        assert!(!is_windows_path("/mnt/c/Users"));
        assert!(!is_windows_path(r"\\wsl.localhost\Ubuntu\home"));
        assert!(!is_windows_path("C:"));
        assert_eq!(from_windows(Path::new("/home/me")), None);
    }

    #[test]
    fn parses_tasklist_rows() {
        let text = "\"System Idle Process\",\"0\",\"Services\",\"0\",\"8 K\"\r\n\
                    \"Habbo.exe\",\"4242\",\"Console\",\"1\",\"123,456 K\"\r\n\
                    \r\n";
        assert_eq!(
            parse_tasklist(text),
            vec![
                (0, "System Idle Process".into()),
                (4242, "Habbo.exe".into())
            ]
        );
    }
}
