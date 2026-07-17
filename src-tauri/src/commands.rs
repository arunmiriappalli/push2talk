use crate::config::{AppConfig, HotkeyDescriptor, ModelConfig};
use crate::engine::EngineHandle;
use crate::{audio, config, hotkey, models, typist};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt;

#[tauri::command]
pub fn list_audio_devices() -> Vec<String> {
    audio::list_input_devices()
}

#[tauri::command]
pub async fn capture_hotkey() -> Result<HotkeyDescriptor, String> {
    hotkey::capture_next_key().await
}

#[tauri::command]
pub fn list_models() -> Vec<models::ModelInfo> {
    models::CATALOG.to_vec()
}

#[tauri::command]
pub fn detect_hardware() -> crate::hardware::HardwareInfo {
    crate::hardware::detect()
}

#[tauri::command]
pub fn is_model_downloaded(file: String, model_dir: Option<String>) -> bool {
    let dir = config::resolve_models_dir(&model_dir);
    models::is_downloaded(&dir, &file)
}

#[tauri::command]
pub async fn download_model(
    app: AppHandle,
    file: String,
    model_dir: Option<String>,
) -> Result<String, String> {
    let dir = config::resolve_models_dir(&model_dir);
    let path = models::download(&app, &dir, &file).await?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
pub fn typist_health_check() -> Result<(), String> {
    typist::health_check()
}

#[tauri::command]
pub fn get_config() -> AppConfig {
    config::load()
}

#[tauri::command]
pub fn save_config(engine: State<EngineHandle>, mut cfg: AppConfig) -> Result<(), String> {
    if let Some(model_name) = cfg.model.as_ref().map(|m| m.name.clone()) {
        if let Some(info) = models::CATALOG.iter().find(|m| m.name == model_name) {
            let dir = config::resolve_models_dir(&cfg.model_dir);
            cfg.model = Some(ModelConfig {
                name: info.name.to_string(),
                path: dir.join(info.file).to_string_lossy().to_string(),
            });
        }
    }
    config::save(&cfg).map_err(|e| e.to_string())?;
    engine.reload(cfg);
    Ok(())
}

/// Opens a native folder picker for choosing where Whisper models are cached.
/// Returns `None` if the user cancels.
#[tauri::command]
pub async fn pick_model_dir() -> Option<String> {
    let handle = rfd::AsyncFileDialog::new().pick_folder().await?;
    Some(handle.path().to_string_lossy().to_string())
}

#[tauri::command]
pub fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|e| e.to_string())
    } else {
        manager.disable().map_err(|e| e.to_string())
    }
}
