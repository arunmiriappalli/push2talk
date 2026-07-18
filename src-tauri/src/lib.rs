mod audio;
mod commands;
mod config;
mod engine;
mod hardware;
mod hotkey;
mod models;
mod transcribe;
mod typist;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{Emitter, Listener, Manager};
use tauri_plugin_autostart::MacosLauncher;

extern "C" {
    fn _exit(code: i32) -> !;
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            commands::list_audio_devices,
            commands::capture_hotkey,
            commands::list_models,
            commands::detect_hardware,
            commands::is_model_downloaded,
            commands::download_model,
            commands::typist_health_check,
            commands::get_config,
            commands::save_config,
            commands::pick_model_dir,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let engine_handle = engine::spawn(handle.clone());
            app.manage(engine_handle);

            let settings_item =
                MenuItem::with_id(app, "settings", "Open Settings", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&settings_item, &quit_item])?;

            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            let tray = TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .menu(&menu)
                .tooltip("push2talk — idle")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "settings" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.emit("navigate", "settings");
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        // whisper.cpp's Metal backend frees a global device via a
                        // C++ static destructor that runs during libc's normal
                        // exit() cleanup (__cxa_finalize_ranges); on macOS that
                        // destructor (ggml_metal_rsets_free) can race an async
                        // Metal "residency set" init and hit a GGML_ASSERT,
                        // aborting the whole process on quit (confirmed via a
                        // real crash report, not a hypothetical). app.exit()
                        // routes through that same exit() path, so it's not
                        // used here -- _exit() terminates immediately without
                        // running any static destructors, which sidesteps the
                        // race entirely. There's nothing left to clean up
                        // gracefully at this point anyway; the OS reclaims
                        // everything on process exit regardless.
                        unsafe {
                            _exit(0);
                        }
                    }
                    _ => {}
                })
                .build(app)?;
            app.manage(tray);

            let status_handle = handle.clone();
            app.listen("engine-status", move |event| {
                let label = match serde_json::from_str::<serde_json::Value>(event.payload()) {
                    Ok(value) => match value.get("state").and_then(|s| s.as_str()) {
                        Some("recording") => "push2talk — recording…",
                        Some("transcribing") => "push2talk — transcribing…",
                        Some("error") => "push2talk — error (open settings)",
                        _ => "push2talk — idle",
                    },
                    Err(_) => "push2talk",
                };
                if let Some(tray) = status_handle.try_state::<tauri::tray::TrayIcon>() {
                    let _ = tray.set_tooltip(Some(label));
                    let _ = tray.set_title(Some(label));
                }
            });

            let cfg = config::load();
            if let Some(window) = app.get_webview_window("main") {
                // Minimize rather than hide() to send the window to the
                // background on startup / close: a hide()-then-later-show()
                // cycle left this window's titlebar buttons (minimize/
                // maximize/close) permanently unresponsive on Linux, even
                // though the window content itself rendered and accepted
                // clicks fine — confirmed by testing a version that skipped
                // hide()/show() entirely. minimize()/unminimize() is a far
                // more standard, better-tested GTK/Wayland window lifecycle
                // and doesn't exhibit the same issue.
                if cfg.setup_complete {
                    let _ = window.minimize();
                } else {
                    let _ = window.set_focus();
                }
                let window_for_close = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_for_close.minimize();
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
