// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hablog::logger::readable;
use hablog::model::Direction;
use hablog::origins::discovery;
use hablog::origins::observer::{self, LocateError, ObserverRuntime};
use hablog::session::{self, Prepared, Problem, SessionEvent, SessionHandle};
use hablog::settings::{self, Settings, Source};
use hablog::{tools, wsl};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

// Session events are sent to the window in batches, so a busy session
// updates it a few dozen times a second rather than once per message.
const BATCH_DELAY: Duration = Duration::from_millis(30);
const BATCH_LIMIT: usize = 2048;

#[derive(Default)]
struct App {
    session: Mutex<Option<SessionHandle>>,
}

/// Everything the window shows before a session starts.
#[derive(Serialize)]
struct Overview {
    settings: Settings,
    settings_note: Option<String>,
    client: Option<Client>,
    problem: Option<Problem>,
    details: Vec<String>,
    hotels: Vec<String>,
    observer: ObserverStatus,
    running: bool,
}

#[derive(Serialize)]
struct Client {
    version: String,
    hotel: Option<String>,
    upstream: String,
    listen: String,
    routed: String,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum ObserverStatus {
    Ready,
    NotInstalled { folder: String },
    Unavailable { reason: String },
    Off,
}

#[derive(Serialize)]
struct Built {
    readable: String,
    hex: String,
}

fn load_settings() -> Result<(Settings, Option<String>), String> {
    let (settings, source) = Settings::load_or_create(Path::new(settings::FILE_NAME))?;
    let note = match source {
        Source::File => None,
        Source::Created => Some(format!(
            "Created {} with the default settings.",
            settings::FILE_NAME
        )),
        Source::Defaults(reason) => Some(format!("Using the default settings ({reason}).")),
    };
    Ok((settings, note))
}

fn observer_status(settings: &Settings) -> ObserverStatus {
    if !settings.decrypt {
        return ObserverStatus::Off;
    }
    if !wsl::runs_windows_programs() {
        return ObserverStatus::Unavailable {
            reason: "Decrypted traffic is only available on Windows.".into(),
        };
    }
    match ObserverRuntime::locate(settings.advanced.observer.as_deref()) {
        Ok(_) => ObserverStatus::Ready,
        Err(LocateError::NotInstalled(folder)) => ObserverStatus::NotInstalled {
            folder: folder.display().to_string(),
        },
        Err(e) => ObserverStatus::Unavailable {
            reason: e.to_string(),
        },
    }
}

fn client(prepared: &Prepared) -> Client {
    let plan = &prepared.plan;
    Client {
        version: prepared.version().into(),
        hotel: plan.server_code.clone(),
        upstream: plan.upstream.clone(),
        listen: plan.listen.clone(),
        routed: wsl::display_path(&plan.routed_exe),
    }
}

#[tauri::command]
fn overview(app: State<'_, App>) -> Result<Overview, String> {
    let (settings, settings_note) = load_settings()?;
    let snapshot = discovery::discover_with(
        settings.advanced.appdata.clone(),
        settings.advanced.local_appdata.clone(),
    );
    let mut hotels: Vec<String> = snapshot
        .installations
        .first()
        .map(|installation| {
            installation
                .regional_executables
                .iter()
                .filter(|regional| !regional.large)
                .map(|regional| regional.server_code.clone())
                .collect()
        })
        .unwrap_or_default();
    hotels.dedup();
    let details = tools::details(&snapshot);
    let (client, problem) = match session::prepare(&settings) {
        Ok(prepared) => (Some(client(&prepared)), None),
        Err(problem) => (None, Some(problem)),
    };
    Ok(Overview {
        observer: observer_status(&settings),
        running: app.session.lock().map(|s| s.is_some()).unwrap_or(false),
        settings,
        settings_note,
        client,
        problem,
        details,
        hotels,
    })
}

/// Starts a session with the saved settings and forwards its events to the
/// window as `session` events.
#[tauri::command]
async fn start(handle: AppHandle, app: State<'_, App>) -> Result<(), String> {
    if app.session.lock().map_err(|e| e.to_string())?.is_some() {
        return Err("Hablog is already running.".into());
    }
    let (settings, _) = load_settings()?;
    let explain = |problem: Problem| match problem.hint {
        Some(hint) => format!("{} {}", problem.error, hint),
        None => problem.error,
    };
    let prepared = session::prepare(&settings).map_err(explain)?;
    // The window asks about installing the observer itself, before starting.
    let observer = (settings.launch && settings.decrypt)
        .then(|| ObserverRuntime::locate(settings.advanced.observer.as_deref()).ok())
        .flatten();
    let (session, mut events) = session::start(settings, prepared, observer)
        .await
        .map_err(explain)?;
    *app.session.lock().map_err(|e| e.to_string())? = Some(session);

    tauri::async_runtime::spawn(async move {
        let mut batch = Vec::new();
        while events.recv_many(&mut batch, BATCH_LIMIT).await > 0 {
            tokio::time::sleep(BATCH_DELAY).await;
            while batch.len() < BATCH_LIMIT {
                match events.try_recv() {
                    Ok(event) => batch.push(event),
                    Err(_) => break,
                }
            }
            let _ = handle.emit("session", &batch);
            if batch.iter().any(|event| matches!(event, SessionEvent::Stopped)) {
                break;
            }
            batch.clear();
        }
        if let Ok(mut session) = handle.state::<App>().session.lock() {
            *session = None;
        }
    });
    Ok(())
}

/// Asks the session to stop; asking again stops it at once.
#[tauri::command]
fn stop(app: State<'_, App>) -> Result<(), String> {
    if let Some(session) = app.session.lock().map_err(|e| e.to_string())?.as_ref() {
        session.stop();
    }
    Ok(())
}

#[tauri::command]
async fn install_observer(folder: String) -> Result<(), String> {
    observer::install(Path::new(&folder)).await
}

#[tauri::command]
fn decode(value: String) -> Vec<String> {
    tools::decode(&value)
}

#[tauri::command]
fn build(direction: String, header: String, parts: Vec<(String, String)>) -> Result<Built, String> {
    let direction = match direction.as_str() {
        "out" => Direction::ClientToServer,
        _ => Direction::ServerToClient,
    };
    let parts: Vec<tools::Part> = parts
        .into_iter()
        .map(|(kind, text)| (kind.chars().next().unwrap_or('r'), text))
        .collect();
    let message = tools::build_parts(direction, &header, &parts)?;
    Ok(Built {
        readable: readable(&message, usize::MAX),
        hex: hex_text(&message),
    })
}

#[tauri::command]
fn save_settings(settings: Settings) -> Result<(), String> {
    settings.save(Path::new(settings::FILE_NAME))
}

fn hex_text(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The folder holding hablog.toml, logs/ and automation/, so the window and
/// the CLI share them: the repository while developing, the nearest folder
/// above the executable with automation/ in it (the repository again for
/// target/release), or else the executable's own folder.
fn home() -> PathBuf {
    if cfg!(debug_assertions) {
        return Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    }
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
    else {
        return PathBuf::from(".");
    };
    exe_dir
        .ancestors()
        .find(|dir| dir.join("automation").join("host.mjs").is_file())
        .map(Path::to_path_buf)
        .unwrap_or(exe_dir)
}

fn main() {
    let _ = std::env::set_current_dir(home());
    tauri::Builder::default()
        .manage(App::default())
        .invoke_handler(tauri::generate_handler![
            overview,
            start,
            stop,
            install_observer,
            decode,
            build,
            save_settings
        ])
        .run(tauri::generate_context!())
        .expect("error while running Hablog");
}
