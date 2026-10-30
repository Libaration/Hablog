use hablog::origins::discovery::discover_from_roots;
use hablog::origins::routing::RoutePlan;
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    roaming: PathBuf,
    local: PathBuf,
    profile: PathBuf,
}

static FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "hablog-discovery-{}-{}-{}",
            std::process::id(),
            FIXTURE_ID.fetch_add(1, Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let roaming = root.join("roaming");
        let local = root.join("local");
        let profile = roaming.join("Habbo Launcher");
        fs::create_dir_all(&profile).unwrap();
        fs::create_dir_all(&local).unwrap();
        Self {
            root,
            roaming,
            local,
            profile,
        }
    }

    fn client(&self, version: &str, name: &str, host: &str) -> PathBuf {
        let root = self.profile.join("downloads/shockwave").join(version);
        fs::create_dir_all(&root).unwrap();
        let path = root.join(name);
        write_projector(&path, host);
        path
    }

    fn versions(&self, preferred: &str, registrations: serde_json::Value) {
        fs::write(
            self.profile.join("versions.json"),
            serde_json::to_vec(&json!({
                "installations": registrations,
                "lastCheck": { "shockwave": {"version": preferred} }
            }))
            .unwrap(),
        )
        .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // The fixture created this unique directory beneath the OS temp root.
        assert!(self.root.starts_with(std::env::temp_dir()));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn write_projector(path: &Path, host: &str) {
    fs::write(path, format!("MZ\0connection.info.host={host};connection.info.port=45678\0connection.mus.host={host};connection.mus.port=45679\0site.url=https://origins.test.invalid;\0external.variables.txt=https://gamedata.test.invalid/external_variables/1\0")).unwrap();
}

#[test]
fn discovers_normal_large_and_current_default_without_configured_addresses() {
    let fixture = Fixture::new();
    let normal = fixture.client("368", "HabboHotel-ous.exe", "test-english.invalid");
    fixture.client("368", "HabboHotel-ous-xl.exe", "test-english.invalid");
    fixture.client("368", "HabboHotel-obr.exe", "test-portuguese.invalid");
    let default = fixture.client("368", "Habbo.exe", "test-portuguese.invalid");
    fixture.versions("368", json!([{
        "client":"shockwave", "version":"368", "path":normal.parent().unwrap(), "lastModified":123
    }]));
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    assert_eq!(result.installations.len(), 1);
    let installation = &result.installations[0];
    assert!(installation.registered);
    assert_eq!(installation.regional_executables.len(), 3);
    assert!(installation
        .regional_executables
        .iter()
        .any(|client| client.server_code == "hhous" && client.large));
    assert_eq!(
        installation.current_default_server_code.as_deref(),
        Some("hhobr")
    );
    let plan = result.normal_launch_plan().unwrap();
    assert_eq!(plan.executable, default);
    assert_eq!(plan.working_directory, normal.parent().unwrap());
    assert_eq!(plan.endpoint.host, "test-portuguese.invalid");
    assert_eq!(plan.endpoint.port, 45678);
    assert_eq!(plan.endpoint.host_property_offset, 3);
}

#[test]
fn invalid_version_store_uses_only_known_download_tree() {
    let fixture = Fixture::new();
    fixture.client("369", "HabboHotel-oes.exe", "test-spanish.invalid");
    fs::write(fixture.profile.join("versions.json"), b"this is not JSON").unwrap();
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert_eq!(result.installations.len(), 1);
    assert!(!result.installations[0].registered);
    assert!(result
        .warnings
        .iter()
        .any(|warning| warning.contains("Cannot parse")));
    assert_eq!(
        result.installations[0].regional_executables[0]
            .info_endpoint
            .as_ref()
            .unwrap()
            .host,
        "test-spanish.invalid"
    );
    assert!(result.normal_launch_plan().is_err());
}

#[test]
fn ambiguous_endpoints_are_not_guessed() {
    let fixture = Fixture::new();
    let default = fixture.client("368", "Habbo.exe", "first.invalid");
    let mut bytes = fs::read(&default).unwrap();
    bytes.extend_from_slice(b"connection.info.host=second.invalid\0");
    fs::write(default, bytes).unwrap();
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert!(result.installations[0]
        .current_default_info_endpoint
        .is_none());
    assert!(result.normal_launch_plan().is_err());
}

#[test]
fn launcher_selected_version_takes_priority_over_newer_folder() {
    let fixture = Fixture::new();
    fixture.client("368", "Habbo.exe", "selected.invalid");
    fixture.client("999", "Habbo.exe", "other.invalid");
    fixture.versions("368", json!([]));
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert_eq!(result.installations[0].version, "368");
    assert_eq!(
        result.normal_launch_plan().unwrap().endpoint.host,
        "selected.invalid"
    );
}

#[test]
fn ambiguous_selected_client_does_not_silently_launch_an_older_server() {
    let fixture = Fixture::new();
    let selected = fixture.client("368", "Habbo.exe", "selected.invalid");
    fs::write(selected, b"MZ\0missing endpoint\0").unwrap();
    fixture.client("367", "Habbo.exe", "older.invalid");
    fixture.versions("368", json!([]));
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert_eq!(result.installations[0].version, "368");
    assert!(result.normal_launch_plan().is_err());
}

#[test]
fn reads_launcher_version_from_asar_without_loading_application_code() {
    let fixture = Fixture::new();
    let launcher = fixture.local.join("Programs/habbo-electron-launcher");
    fs::create_dir_all(launcher.join("resources")).unwrap();
    fs::write(launcher.join("Habbo Launcher.exe"), b"MZ").unwrap();
    let package = br#"{"version":"1.0.80"}"#;
    let header =
        serde_json::to_vec(&json!({"files":{"package.json":{"size":package.len(),"offset":"0"}}}))
            .unwrap();
    let header_size = header.len() as u32 + 8;
    let mut archive = Vec::new();
    for value in [4, header_size, header_size - 4, header.len() as u32] {
        archive.extend(value.to_le_bytes());
    }
    archive.extend(header);
    archive.extend(package);
    fs::write(launcher.join("resources/app.asar"), archive).unwrap();
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert_eq!(result.launcher_version.as_deref(), Some("1.0.80"));
    assert!(result
        .launcher_executable
        .unwrap()
        .ends_with("Habbo Launcher.exe"));
}

#[test]
fn missing_profile_is_an_explicit_discovery_result() {
    let result = discover_from_roots(None, None);
    assert!(result.installations.is_empty());
    assert!(!result.warnings.is_empty());
    assert!(result.normal_launch_plan().is_err());
}

#[test]
fn an_explicit_region_uses_that_regional_client() {
    let fixture = Fixture::new();
    fixture.client("368", "Habbo.exe", "test-english.invalid");
    fixture.client("368", "HabboHotel-ous.exe", "test-english.invalid");
    let spanish = fixture.client("368", "HabboHotel-oes.exe", "test-spanish.invalid");
    fixture.client("368", "HabboHotel-oes-xl.exe", "test-spanish-xl.invalid");
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    let plan = result.launch_plan(Some("hhoes")).unwrap();
    assert_eq!(plan.executable, spanish);
    assert_eq!(plan.server_code.as_deref(), Some("hhoes"));
    assert_eq!(plan.endpoint.host, "test-spanish.invalid");
    assert_eq!(
        result.launch_plan(None).unwrap().endpoint.host,
        "test-english.invalid"
    );
    assert!(result
        .launch_plan(Some("hhobr"))
        .unwrap_err()
        .contains("no hhobr client"));
}

#[test]
fn routes_a_copy_of_the_discovered_client_to_the_relay() {
    let fixture = Fixture::new();
    let installed = fixture.client("368", "Habbo.exe", "game-test.invalid");
    fixture.client("368", "HabboHotel-ous.exe", "game-test.invalid");
    let original = fs::read(&installed).unwrap();
    let result = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    let plan = RoutePlan::from_discovery(&result, None).unwrap();
    assert_eq!(plan.listen, "127.0.0.1:45678");
    assert_eq!(plan.upstream, "game-test.invalid:45678");
    assert_eq!(plan.server_code.as_deref(), Some("hhous"));

    let routed = plan.write_patched_projector().unwrap();
    assert_eq!(routed.parent(), installed.parent());
    assert_eq!(
        fs::read(&installed).unwrap(),
        original,
        "installed client changed"
    );
    let patched = String::from_utf8_lossy(&fs::read(&routed).unwrap()).into_owned();
    assert_eq!(patched.len(), original.len());
    // The host keeps its length: 127.0.0.1 padded with ';' up to 17 bytes.
    let padding = ";".repeat("game-test.invalid".len() - "127.0.0.1".len());
    assert!(patched.contains(&format!(
        "connection.info.host=127.0.0.1{padding};connection.info.port=45678"
    )));
    // Only the game endpoint is redirected; MUS still goes to the real host.
    assert!(patched.contains("connection.mus.host=game-test.invalid;"));
}

#[test]
fn installation_paths_recorded_by_the_launcher_are_translated() {
    // What WSL sees: versions.json holds Windows paths.
    let fixture = Fixture::new();
    let client = fixture.client("368", "Habbo.exe", "test-english.invalid");
    let root = client.parent().unwrap().to_path_buf();
    fixture.versions(
        "368",
        json!([{ "client": "shockwave", "version": "368", "path": "C:\\Habbo\\368" }]),
    );
    let translated = hablog::origins::discovery::discover_from_roots_with(
        Some(&fixture.roaming),
        Some(&fixture.local),
        &|path| {
            if path == Path::new("C:\\Habbo\\368") {
                root.clone()
            } else {
                path.to_path_buf()
            }
        },
    );
    assert!(translated.warnings.is_empty(), "{:?}", translated.warnings);
    assert!(translated.installations[0].registered);
    // Untranslated, the same store points nowhere and only the download
    // folder fallback finds the client.
    let untranslated = discover_from_roots(Some(&fixture.roaming), Some(&fixture.local));
    assert!(!untranslated.installations[0].registered);
    assert!(untranslated
        .warnings
        .iter()
        .any(|warning| warning.contains("missing")));
}
