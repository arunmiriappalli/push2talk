use crate::config::HotkeyDescriptor;
use evdev::{EventSummary, KeyCode};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::HotkeyEvent;

/// Real keyboards report KEY_ENTER; mice/trackpads don't (their buttons are
/// still EV_KEY events — BTN_LEFT etc. — so filtering on EventType::KEY alone
/// would let a mouse click get captured as the hotkey). Devices that also
/// report relative motion (REL_X/REL_Y) are pointer devices and excluded too.
fn looks_like_keyboard(device: &evdev::Device) -> bool {
    let has_enter = device
        .supported_keys()
        .is_some_and(|keys| keys.contains(KeyCode::KEY_ENTER));
    let has_relative_motion = device.supported_relative_axes().is_some();
    has_enter && !has_relative_motion
}

/// Opens every readable keyboard device and waits for the first key-down
/// event anywhere, returning which device + key code was pressed. Used by the
/// setup wizard's "press the key you want to use" step.
pub async fn capture_next_key() -> Result<HotkeyDescriptor, String> {
    let devices: Vec<(std::path::PathBuf, String)> = evdev::enumerate()
        .filter(|(_, device)| looks_like_keyboard(device))
        .map(|(path, device)| (path, device.name().unwrap_or("Unknown device").to_string()))
        .collect();

    if devices.is_empty() {
        return Err(
            "No readable keyboard devices found. Make sure your user account is in the \
             `input` group (run `sudo usermod -aG input $USER` and log out/in), then retry."
                .to_string(),
        );
    }

    let (tx, rx) = std::sync::mpsc::channel::<HotkeyDescriptor>();

    let mut handles = Vec::new();
    for (path, name) in devices {
        let tx = tx.clone();
        let path_str = path.to_string_lossy().to_string();
        handles.push(std::thread::spawn(move || {
            let mut device = match evdev::Device::open(&path) {
                Ok(d) => d,
                Err(_) => return,
            };
            loop {
                let events = match device.fetch_events() {
                    Ok(events) => events,
                    Err(_) => return,
                };
                for ev in events {
                    if let EventSummary::Key(_, code, value) = ev.destructure() {
                        if value == 1 {
                            let _ = tx.send(HotkeyDescriptor::Linux {
                                device_path: path_str.clone(),
                                device_name: name.clone(),
                                key_code: code.0,
                                key_name: format!("{:?}", code),
                            });
                            return;
                        }
                    }
                }
            }
        }));
    }
    drop(tx);

    let result = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(20)))
        .await
        .map_err(|e| e.to_string())?;

    match result {
        Ok(descriptor) => Ok(descriptor),
        Err(_) => Err("Timed out waiting for a key press.".to_string()),
    }
}

/// Spawns a background thread that watches `descriptor`'s device for that
/// specific key's press/release events and invokes `on_event`. `generation`
/// lets the caller invalidate a stale listener after the hotkey is reconfigured:
/// the thread stops forwarding (and exits on its next event) once
/// `generation.load() != my_generation`.
pub fn listen<F>(
    descriptor: HotkeyDescriptor,
    on_event: F,
    generation: Arc<AtomicU64>,
    my_generation: u64,
) where
    F: Fn(HotkeyEvent) + Send + 'static,
{
    let HotkeyDescriptor::Linux {
        device_path,
        key_code,
        ..
    } = descriptor
    else {
        return;
    };

    std::thread::spawn(move || {
        let mut device = match evdev::Device::open(&device_path) {
            Ok(d) => d,
            Err(e) => {
                log::error!("push2talk: failed to open {}: {}", device_path, e);
                return;
            }
        };

        loop {
            if generation.load(Ordering::SeqCst) != my_generation {
                return;
            }
            let events = match device.fetch_events() {
                Ok(events) => events,
                Err(_) => return,
            };
            for ev in events {
                if generation.load(Ordering::SeqCst) != my_generation {
                    return;
                }
                if let EventSummary::Key(_, code, value) = ev.destructure() {
                    if code.0 != key_code {
                        continue;
                    }
                    let event = match value {
                        1 => Some(HotkeyEvent::Pressed),
                        0 => Some(HotkeyEvent::Released),
                        _ => None,
                    };
                    if let Some(event) = event {
                        on_event(event);
                    }
                }
            }
        }
    });
}
