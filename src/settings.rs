//! `hablog.toml`. Hablog writes this template on first run, so every setting
//! is documented where it is changed. A missing setting keeps its default.
use crate::model::Direction;
use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "hablog.toml";

pub const TEMPLATE: &str = r#"# Hablog settings. Every line is optional; delete one to go back to its default.

# Which Origins hotel to play on. "auto" uses the one the Habbo Launcher last
# opened. Or pick one: "hhous" (English), "hhobr" (Portuguese) or "hhoes"
# (Spanish). `hablog details` shows what is installed.
hotel = "auto"

# Start the game once Hablog is ready. When false, Hablog prints the path of
# the game copy to start yourself.
launch = true

# Show the game's decrypted traffic. This starts the game through the Frida
# observer in the automation folder, which Hablog offers to set up the first
# time. When it isn't available, the game starts normally and only the
# encrypted traffic is shown.
decrypt = true

# Which traffic to print: "both", "in" (server to game), "out" (game to
# server) or "none". The capture files always get both.
show = "both"

# Save each session's raw traffic to its own file in captures_folder.
save_captures = true
captures_folder = "logs"

# Settings you normally don't need.
[advanced]
# Connect to this game server instead of the one built into the client.
#upstream = "game-ous.habbo.com:40001"

# Look for the Habbo Launcher in other AppData folders. Hablog finds them
# itself on Windows and under WSL; this is for setups like a Wine prefix.
# Single quotes keep backslashes as they are.
#appdata = 'C:\Users\you\AppData\Roaming'
#local_appdata = 'C:\Users\you\AppData\Local'

# Folder containing the observer (host.mjs). Found automatically.
#observer = 'automation'
"#;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    pub hotel: String,
    pub launch: bool,
    pub decrypt: bool,
    pub show: Show,
    pub save_captures: bool,
    pub captures_folder: PathBuf,
    pub advanced: Advanced,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Advanced {
    pub upstream: Option<String>,
    pub appdata: Option<PathBuf>,
    pub local_appdata: Option<PathBuf>,
    pub observer: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Show {
    Both,
    In,
    Out,
    None,
}

/// Where the settings came from, so the user can be told once.
#[derive(Debug, PartialEq, Eq)]
pub enum Source {
    File,
    Created,
    Defaults(String),
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotel: "auto".into(),
            launch: true,
            decrypt: true,
            show: Show::Both,
            save_captures: true,
            captures_folder: PathBuf::from("logs"),
            advanced: Advanced::default(),
        }
    }
}

impl Settings {
    /// Reads `path`, or writes the documented template there and uses the
    /// defaults. If it can't be written, the defaults are used anyway.
    pub fn load_or_create(path: &Path) -> Result<(Self, Source), String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text)
                .map(|settings| (settings, Source::File))
                .map_err(|e| format!("{} is not valid: {e}", path.display())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let source = match fs::write(path, TEMPLATE) {
                    Ok(()) => Source::Created,
                    Err(e) => Source::Defaults(format!("cannot create {}: {e}", path.display())),
                };
                Ok((Self::default(), source))
            }
            Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
        }
    }

    /// Reads `path` if it exists, otherwise the defaults. Never writes.
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => {
                Self::parse(&text).map_err(|e| format!("{} is not valid: {e}", path.display()))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("Cannot read {}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        toml::from_str(text).map_err(|e| e.to_string())
    }

    /// The chosen region, or None for the launcher's current client.
    pub fn server_code(&self) -> Option<&str> {
        (!self.hotel.eq_ignore_ascii_case("auto")).then_some(self.hotel.as_str())
    }

    pub fn shows(&self, direction: Direction) -> bool {
        matches!(
            (self.show, direction),
            (Show::Both, _)
                | (Show::In, Direction::ServerToClient)
                | (Show::Out, Direction::ClientToServer)
        )
    }

    pub fn captures(&self) -> Option<&Path> {
        self.save_captures.then_some(self.captures_folder.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn the_template_documents_exactly_the_defaults() {
        assert_eq!(Settings::parse(TEMPLATE).unwrap(), Settings::default());
    }

    #[test]
    fn the_commented_advanced_examples_parse() {
        let uncommented = TEMPLATE
            .replace("\n#upstream", "\nupstream")
            .replace("\n#appdata", "\nappdata")
            .replace("\n#local_appdata", "\nlocal_appdata")
            .replace("\n#observer", "\nobserver");
        let settings = Settings::parse(&uncommented).unwrap();
        assert_eq!(
            settings.advanced.upstream.as_deref(),
            Some("game-ous.habbo.com:40001")
        );
        assert_eq!(
            settings.advanced.appdata,
            Some(PathBuf::from(r"C:\Users\you\AppData\Roaming"))
        );
        assert_eq!(
            settings.advanced.observer,
            Some(PathBuf::from("automation"))
        );
    }

    #[test]
    fn missing_settings_keep_their_defaults() {
        let settings = Settings::parse("hotel = \"hhobr\"\nshow = \"in\"").unwrap();
        assert_eq!(settings.server_code(), Some("hhobr"));
        assert!(settings.launch && settings.decrypt);
        assert!(settings.shows(Direction::ServerToClient));
        assert!(!settings.shows(Direction::ClientToServer));
        assert_eq!(Settings::default().server_code(), None);
        let off = Settings::parse("save_captures = false").unwrap();
        assert_eq!(off.captures(), None);
    }

    #[test]
    fn typos_are_reported_instead_of_ignored() {
        assert!(Settings::parse("lunch = true")
            .unwrap_err()
            .contains("lunch"));
        assert!(Settings::parse("show = \"everything\"").is_err());
        assert!(Settings::parse("[advanced]\nupstraem = \"x\"").is_err());
    }

    #[test]
    fn creates_the_template_once_then_reads_it() {
        let dir = std::env::temp_dir().join(format!(
            "hablog-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(FILE_NAME);
        assert_eq!(Settings::load(&path).unwrap(), Settings::default());
        assert!(!path.exists(), "load must not create the file");
        let (settings, source) = Settings::load_or_create(&path).unwrap();
        assert_eq!((settings, source), (Settings::default(), Source::Created));
        assert_eq!(fs::read_to_string(&path).unwrap(), TEMPLATE);
        fs::write(&path, "launch = false").unwrap();
        let (settings, source) = Settings::load_or_create(&path).unwrap();
        assert_eq!(source, Source::File);
        assert!(!settings.launch);
        fs::write(&path, "launch = maybe").unwrap();
        assert!(Settings::load_or_create(&path)
            .unwrap_err()
            .contains("not valid"));
        fs::remove_dir_all(dir).unwrap();
    }
}
