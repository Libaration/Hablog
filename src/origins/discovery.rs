//! Read-only discovery of the existing Windows Origins installation.
//!
//! Server addresses come from properties embedded in each installed projector,
//! not a built-in server list. This module neither launches nor modifies clients
//! and never reads command lines, launcher logs, browser data or login tokens.

use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

const MAX_PROJECTOR_SIZE: u64 = 32 * 1024 * 1024;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiscoverySnapshot {
    pub launcher_executable: Option<PathBuf>,
    pub launcher_version: Option<String>,
    pub profile_root: Option<PathBuf>,
    pub preferred_version: Option<String>,
    pub installations: Vec<ClientInstallation>,
    pub running_clients: Vec<RunningClient>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClientInstallation {
    pub version: String,
    pub root: PathBuf,
    pub registered: bool,
    pub last_modified: Option<u64>,
    pub current_default_executable: Option<PathBuf>,
    pub current_default_info_endpoint: Option<ObservedEndpoint>,
    /// Inferred by matching Habbo.exe's embedded endpoint to regional clients.
    pub current_default_server_code: Option<String>,
    pub regional_executables: Vec<RegionalExecutable>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionalExecutable {
    /// Derived from the launcher's observed `hh` + executable suffix mapping.
    pub server_code: String,
    pub large: bool,
    pub path: PathBuf,
    pub info_endpoint: Option<ObservedEndpoint>,
    pub mus_endpoint: Option<ObservedEndpoint>,
    pub external_variables_url: Option<EmbeddedProperty>,
    pub site_url: Option<EmbeddedProperty>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedEndpoint {
    pub host: String,
    pub port: u16,
    pub host_property_offset: u64,
    pub port_property_offset: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbeddedProperty {
    pub value: String,
    /// Offset of the property key, not of its value, in the original file.
    pub offset: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunningClient {
    pub pid: u32,
    pub path: PathBuf,
    pub server_code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OriginalLaunchPlan {
    pub executable: PathBuf,
    pub working_directory: PathBuf,
    pub server_code: Option<String>,
    pub endpoint: ObservedEndpoint,
}

impl DiscoverySnapshot {
    /// Describes a normal launch without arguments or an authentication token.
    /// The caller owns the explicit launch action; discovery remains read-only.
    pub fn normal_launch_plan(&self) -> Result<OriginalLaunchPlan, String> {
        let installation = self
            .installations
            .first()
            .ok_or("No installed Origins client was found.")?;
        Ok(OriginalLaunchPlan {
            executable: installation.current_default_executable.clone().ok_or("The selected Origins installation has no Habbo.exe; open it through the normal launcher first.")?,
            working_directory: installation.root.clone(),
            server_code: installation.current_default_server_code.clone(),
            endpoint: installation.current_default_info_endpoint.clone().ok_or("The selected Habbo.exe has no unambiguous embedded game endpoint.")?,
        })
    }

    /// Like `normal_launch_plan`, but for an explicitly chosen region (e.g.
    /// `hhobr`) uses that region's normal-size projector instead of Habbo.exe.
    pub fn launch_plan(&self, server_code: Option<&str>) -> Result<OriginalLaunchPlan, String> {
        let Some(server_code) = server_code else {
            return self.normal_launch_plan();
        };
        let installation = self
            .installations
            .first()
            .ok_or("No installed Origins client was found.")?;
        let regional = installation
            .regional_executables
            .iter()
            .find(|regional| regional.server_code == server_code && !regional.large)
            .ok_or_else(|| {
                format!("The selected Origins installation has no {server_code} client.")
            })?;
        Ok(OriginalLaunchPlan {
            executable: regional.path.clone(),
            working_directory: installation.root.clone(),
            server_code: Some(server_code.to_owned()),
            endpoint: regional.info_endpoint.clone().ok_or_else(|| {
                format!("The {server_code} client has no unambiguous embedded game endpoint.")
            })?,
        })
    }
}

#[derive(Default, Deserialize)]
struct VersionStore {
    #[serde(default)]
    installations: Vec<RegisteredInstallation>,
    #[serde(default, rename = "lastCheck")]
    last_check: LastCheck,
}

#[derive(Deserialize)]
struct RegisteredInstallation {
    client: String,
    version: String,
    path: PathBuf,
    #[serde(default, rename = "lastModified")]
    last_modified: Option<u64>,
}

#[derive(Default, Deserialize)]
struct LastCheck {
    #[serde(default)]
    shockwave: LastShockwave,
}

#[derive(Default, Deserialize)]
struct LastShockwave {
    #[serde(default)]
    version: Option<String>,
}

pub fn discover() -> DiscoverySnapshot {
    discover_with(None, None)
}

/// Discovery with optional replacements for APPDATA and LOCALAPPDATA, for
/// example the matching folders inside a Wine prefix.
pub fn discover_with(roaming: Option<PathBuf>, local: Option<PathBuf>) -> DiscoverySnapshot {
    let roaming = roaming.or_else(|| std::env::var_os("APPDATA").map(PathBuf::from));
    let local = local.or_else(|| std::env::var_os("LOCALAPPDATA").map(PathBuf::from));
    let mut result = discover_from_roots(roaming.as_deref(), local.as_deref());
    match platform::running_clients(&result.installations) {
        Ok(clients) => result.running_clients = clients,
        Err(error) => result.warnings.push(error),
    }
    result
}

/// Explicit roots support isolated fixtures; normal callers should use discover.
pub fn discover_from_roots(roaming: Option<&Path>, local: Option<&Path>) -> DiscoverySnapshot {
    let mut snapshot = DiscoverySnapshot::default();
    if let Some(local) = local {
        let root = local.join("Programs/habbo-electron-launcher");
        let launcher = root.join("Habbo Launcher.exe");
        if launcher.is_file() {
            snapshot.launcher_executable = Some(launcher);
            snapshot.launcher_version =
                read_asar_entry(&root.join("resources/app.asar"), "package.json")
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .and_then(|json| {
                        json.get("version")
                            .and_then(|v| v.as_str())
                            .map(str::to_owned)
                    });
        }
    }
    let Some(roaming) = roaming else {
        snapshot
            .warnings
            .push("APPDATA is unavailable; Origins installation discovery could not run.".into());
        return snapshot;
    };
    let profile = roaming.join("Habbo Launcher");
    if !profile.is_dir() {
        snapshot
            .warnings
            .push("The Habbo Launcher profile was not found.".into());
        return snapshot;
    }
    snapshot.profile_root = Some(profile.clone());
    let versions_path = profile.join("versions.json");
    match fs::read(&versions_path) {
        Ok(bytes) => match serde_json::from_slice::<VersionStore>(&bytes) {
            Ok(store) => {
                snapshot.preferred_version = store.last_check.shockwave.version;
                for installation in store
                    .installations
                    .into_iter()
                    .filter(|i| i.client == "shockwave")
                {
                    if installation.path.is_dir() {
                        add_installation(
                            &mut snapshot,
                            installation.version,
                            installation.path,
                            true,
                            installation.last_modified,
                        );
                    } else {
                        snapshot.warnings.push(format!(
                            "Registered Origins directory is missing: {}",
                            installation.path.display()
                        ));
                    }
                }
            }
            Err(error) => snapshot
                .warnings
                .push(format!("Cannot parse {}: {error}", versions_path.display())),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => snapshot
            .warnings
            .push(format!("Cannot read {}: {error}", versions_path.display())),
    }
    // Narrow fallback: version folders in the launcher's known Shockwave tree.
    if let Ok(entries) = fs::read_dir(profile.join("downloads/shockwave")) {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                add_installation(
                    &mut snapshot,
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                    false,
                    None,
                );
            }
        }
    }
    let preferred = snapshot.preferred_version.as_deref();
    snapshot.installations.sort_by(|a, b| {
        (Some(b.version.as_str()) == preferred)
            .cmp(&(Some(a.version.as_str()) == preferred))
            .then_with(|| b.last_modified.cmp(&a.last_modified))
            .then_with(|| {
                b.version
                    .parse::<u64>()
                    .ok()
                    .cmp(&a.version.parse::<u64>().ok())
            })
            .then_with(|| a.root.cmp(&b.root))
    });
    snapshot
}

fn add_installation(
    snapshot: &mut DiscoverySnapshot,
    version: String,
    root: PathBuf,
    registered: bool,
    last_modified: Option<u64>,
) {
    if snapshot
        .installations
        .iter()
        .any(|i| paths_equal(&i.root, &root))
    {
        return;
    }
    let mut regional_executables = Vec::new();
    if let Ok(entries) = fs::read_dir(&root) {
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_file()) {
                continue;
            }
            let path = entry.path();
            let Some((server_code, large)) = regional_identity(&path) else {
                continue;
            };
            match projector_metadata(&path) {
                Ok(properties) => regional_executables.push(RegionalExecutable {
                    server_code,
                    large,
                    path,
                    info_endpoint: endpoint(&properties, "connection.info"),
                    mus_endpoint: endpoint(&properties, "connection.mus"),
                    external_variables_url: property(&properties, "external.variables.txt")
                        .cloned(),
                    site_url: property(&properties, "site.url").cloned(),
                }),
                Err(error) => snapshot
                    .warnings
                    .push(format!("Cannot inspect {}: {error}", path.display())),
            }
        }
    }
    regional_executables.sort_by(|a, b| {
        a.server_code
            .cmp(&b.server_code)
            .then(a.large.cmp(&b.large))
    });
    let default = root.join("Habbo.exe");
    let current_default_executable = default.is_file().then_some(default);
    let current_default_info_endpoint = current_default_executable
        .as_deref()
        .and_then(|path| projector_metadata(path).ok())
        .and_then(|properties| endpoint(&properties, "connection.info"));
    let current_default_server_code = current_default_info_endpoint.as_ref().and_then(|default| {
        regional_executables
            .iter()
            .find(|regional| {
                regional.info_endpoint.as_ref().is_some_and(|endpoint| {
                    endpoint.host == default.host && endpoint.port == default.port
                })
            })
            .map(|regional| regional.server_code.clone())
    });
    snapshot.installations.push(ClientInstallation {
        version,
        root,
        registered,
        last_modified,
        current_default_executable,
        current_default_info_endpoint,
        current_default_server_code,
        regional_executables,
    });
}

fn regional_identity(path: &Path) -> Option<(String, bool)> {
    let name = path.file_name()?.to_str()?.to_ascii_lowercase();
    let suffix = name.strip_prefix("habbohotel-")?.strip_suffix(".exe")?;
    let (suffix, large) = suffix
        .strip_suffix("-xl")
        .map_or((suffix, false), |suffix| (suffix, true));
    // Region identifiers observed in the shipped launcher. Other executables
    // are not treated as production Origins clients by filename alone.
    if !["ous", "obr", "oes", "od"].contains(&suffix) {
        return None;
    }
    Some((format!("hh{suffix}"), large))
}

fn projector_metadata(path: &Path) -> Result<Vec<(String, EmbeddedProperty)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > MAX_PROJECTOR_SIZE {
        return Err("Projector exceeds the 32 MiB inspection limit.".into());
    }
    let bytes = fs::read(path).map_err(|error| error.to_string())?;
    if !bytes.starts_with(b"MZ") {
        return Err("Projector has no PE/DOS executable signature.".into());
    }
    let mut result = Vec::new();
    for name in [
        "connection.info.host",
        "connection.info.port",
        "connection.mus.host",
        "connection.mus.port",
        "external.variables.txt",
        "site.url",
    ] {
        let key = format!("{name}=");
        let matches: Vec<_> = bytes
            .windows(key.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == key.as_bytes()).then_some(offset))
            .collect();
        // Ambiguous duplicates are left unresolved rather than choosing a host
        // that may belong to embedded documentation or an obsolete property.
        if matches.len() != 1 {
            continue;
        }
        let offset = matches[0];
        let value: Vec<_> = bytes[offset + key.len()..]
            .iter()
            .copied()
            .take(1024)
            .take_while(|byte| byte.is_ascii_graphic() && *byte != b';')
            .collect();
        if let Ok(value) = String::from_utf8(value) {
            if !value.is_empty() {
                result.push((
                    name.into(),
                    EmbeddedProperty {
                        value,
                        offset: offset as u64,
                    },
                ));
            }
        }
    }
    Ok(result)
}

fn property<'a>(
    properties: &'a [(String, EmbeddedProperty)],
    name: &str,
) -> Option<&'a EmbeddedProperty> {
    properties
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn endpoint(properties: &[(String, EmbeddedProperty)], prefix: &str) -> Option<ObservedEndpoint> {
    let host = property(properties, &format!("{prefix}.host"))?;
    let port = property(properties, &format!("{prefix}.port"))?;
    if !host
        .value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b".-:".contains(&byte))
    {
        return None;
    }
    let port_value = port.value.parse::<u16>().ok()?;
    if port_value == 0 {
        return None;
    }
    Some(ObservedEndpoint {
        host: host.value.clone(),
        port: port_value,
        host_property_offset: host.offset,
        port_property_offset: port.offset,
    })
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

fn read_asar_entry(path: &Path, entry_path: &str) -> Result<Vec<u8>, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut prefix = [0u8; 16];
    file.read_exact(&mut prefix).map_err(|e| e.to_string())?;
    let header_size = u32::from_le_bytes(prefix[4..8].try_into().unwrap()) as usize;
    let json_size = u32::from_le_bytes(prefix[12..16].try_into().unwrap()) as usize;
    if !(8..=16 * 1024 * 1024).contains(&header_size) || json_size > header_size - 8 {
        return Err("Invalid ASAR header.".into());
    }
    let mut bytes = vec![0; json_size];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    let header: serde_json::Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let mut entry = &header;
    for component in entry_path.split('/') {
        entry = entry
            .get("files")
            .and_then(|files| files.get(component))
            .ok_or("ASAR entry not found.")?;
    }
    let offset = entry
        .get("offset")
        .and_then(|offset| offset.as_str())
        .and_then(|offset| offset.parse::<u64>().ok())
        .ok_or("Invalid ASAR offset.")?;
    let size = entry
        .get("size")
        .and_then(|size| size.as_u64())
        .filter(|size| *size <= 1024 * 1024)
        .ok_or("Invalid ASAR entry size.")?;
    let absolute_offset = 8u64
        .checked_add(header_size as u64)
        .and_then(|base| base.checked_add(offset))
        .ok_or("ASAR offset overflow.")?;
    if absolute_offset.checked_add(size).is_none_or(|end| {
        file.metadata()
            .map(|metadata| end > metadata.len())
            .unwrap_or(true)
    }) {
        return Err("ASAR entry is outside the archive.".into());
    }
    file.seek(SeekFrom::Start(absolute_offset))
        .map_err(|e| e.to_string())?;
    let mut bytes = vec![0; size as usize];
    file.read_exact(&mut bytes).map_err(|e| e.to_string())?;
    Ok(bytes)
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub fn running_clients(_: &[ClientInstallation]) -> Result<Vec<RunningClient>, String> {
        Ok(Vec::new())
    }
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::ffi::c_void;
    use std::mem::{size_of, zeroed};
    use std::os::windows::ffi::OsStringExt;
    type Handle = *mut c_void;
    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        executable: [u16; 260],
    }
    #[link(name = "kernel32")]
    extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> Handle;
        fn Process32FirstW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: Handle, entry: *mut ProcessEntry) -> i32;
        fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
        fn QueryFullProcessImageNameW(
            process: Handle,
            flags: u32,
            buffer: *mut u16,
            length: *mut u32,
        ) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
    }
    struct OwnedHandle(Handle);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    pub fn running_clients(
        installations: &[ClientInstallation],
    ) -> Result<Vec<RunningClient>, String> {
        // Toolhelp provides PID/name only; limited query access reads the image
        // path. No process memory or command line is requested.
        let snapshot = unsafe { CreateToolhelp32Snapshot(2, 0) };
        if snapshot as isize == -1 {
            return Err(format!(
                "Cannot enumerate client processes: {}",
                std::io::Error::last_os_error()
            ));
        }
        let snapshot = OwnedHandle(snapshot);
        let mut entry: ProcessEntry = unsafe { zeroed() };
        entry.size = size_of::<ProcessEntry>() as u32;
        let mut found = Vec::new();
        let mut available = unsafe { Process32FirstW(snapshot.0, &mut entry) } != 0;
        while available {
            let length = entry
                .executable
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(entry.executable.len());
            let name = String::from_utf16_lossy(&entry.executable[..length]);
            if name.eq_ignore_ascii_case("Habbo.exe")
                || regional_identity(Path::new(&name)).is_some()
            {
                let process = unsafe { OpenProcess(0x1000, 0, entry.pid) };
                if !process.is_null() {
                    let process = OwnedHandle(process);
                    let mut buffer = vec![0u16; 32768];
                    let mut count = buffer.len() as u32;
                    if unsafe {
                        QueryFullProcessImageNameW(process.0, 0, buffer.as_mut_ptr(), &mut count)
                    } != 0
                    {
                        let path =
                            PathBuf::from(std::ffi::OsString::from_wide(&buffer[..count as usize]));
                        if let Some(installation) = installations.iter().find(|installation| {
                            path.parent()
                                .is_some_and(|parent| paths_equal(parent, &installation.root))
                        }) {
                            let server_code = regional_identity(&path)
                                .map(|(server, _)| server)
                                .or_else(|| installation.current_default_server_code.clone());
                            found.push(RunningClient {
                                pid: entry.pid,
                                path,
                                server_code,
                            });
                        }
                    }
                }
            }
            available = unsafe { Process32NextW(snapshot.0, &mut entry) } != 0;
        }
        Ok(found)
    }
}
