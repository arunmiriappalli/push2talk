use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "platform", rename_all = "lowercase")]
pub enum HotkeyDescriptor {
    Linux {
        device_path: String,
        device_name: String,
        key_code: u16,
        key_name: String,
    },
    Macos {
        /// Debug representation of the rdev::Key variant (e.g. "F2"), used both
        /// for display and as the comparison key when matching live events —
        /// rdev's Key enum isn't serializable, so this string is the identity.
        key_name: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkey: Option<HotkeyDescriptor>,
    pub audio_device: Option<String>,
    pub model: Option<ModelConfig>,
    /// Overrides the default models cache directory when set (chosen via the
    /// setup wizard / settings' folder picker).
    #[serde(default)]
    pub model_dir: Option<String>,
    #[serde(default = "default_min_press_ms")]
    pub min_press_ms: u64,
    #[serde(default = "default_typing_delay_ms")]
    pub typing_delay_ms: u64,
    #[serde(default)]
    pub autostart: bool,
    #[serde(default)]
    pub setup_complete: bool,
}

fn default_min_press_ms() -> u64 {
    300
}

fn default_typing_delay_ms() -> u64 {
    1
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkey: None,
            audio_device: None,
            model: None,
            model_dir: None,
            min_press_ms: default_min_press_ms(),
            typing_delay_ms: default_typing_delay_ms(),
            autostart: false,
            setup_complete: false,
        }
    }
}

fn project_dirs() -> directories::ProjectDirs {
    directories::ProjectDirs::from("com", "push2talk", "push2talk")
        .expect("could not determine home directory for config storage")
}

pub fn config_path() -> PathBuf {
    let dirs = project_dirs();
    dirs.config_dir().join("config.json")
}

fn default_models_dir() -> PathBuf {
    let dirs = project_dirs();
    dirs.data_dir().join("models")
}

/// Resolves the Whisper model cache directory: `override_dir` if set (the
/// user's chosen location), otherwise the app's default data directory. Takes
/// the override explicitly rather than reading it from disk, since callers
/// may be acting on a `model_dir` that's about to be saved but isn't yet.
pub fn resolve_models_dir(override_dir: &Option<String>) -> PathBuf {
    match override_dir {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => default_models_dir(),
    }
}

pub fn load() -> AppConfig {
    let path = config_path();
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => AppConfig::default(),
    }
}

pub fn save(config: &AppConfig) -> Result<(), std::io::Error> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let contents = serde_json::to_string_pretty(config)?;
    fs::write(path, contents)
}
