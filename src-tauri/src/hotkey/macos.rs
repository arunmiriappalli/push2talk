use crate::config::HotkeyDescriptor;
use rdev::{listen, EventType};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::HotkeyEvent;

/// Starts a temporary global key-tap and waits for the first key-down anywhere,
/// returning its identity. Used by the setup wizard's "press the key you want to
/// use" step. Requires Accessibility permission to be granted to this app
/// (System Settings > Privacy & Security > Accessibility) or the listen call
/// will silently receive no events.
pub async fn capture_next_key() -> Result<HotkeyDescriptor, String> {
    let (tx, rx) = std::sync::mpsc::channel::<HotkeyDescriptor>();

    std::thread::spawn(move || {
        let callback = move |event: rdev::Event| {
            if let EventType::KeyPress(key) = event.event_type {
                let _ = tx.send(HotkeyDescriptor::Macos {
                    key_name: format!("{:?}", key),
                });
            }
        };
        // listen() blocks for the process lifetime of this thread; once we get
        // our first match the recv_timeout below returns and the thread is
        // simply abandoned (cheap: it's just an idle event tap from then on
        // unless another capture is requested, which starts its own thread).
        let _ = listen(callback);
    });

    let result = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(20)))
        .await
        .map_err(|e| e.to_string())?;

    match result {
        Ok(descriptor) => Ok(descriptor),
        Err(_) => Err(
            "Timed out waiting for a key press. If this keeps happening, check that \
             Accessibility permission is granted to this app in System Settings > \
             Privacy & Security > Accessibility."
                .to_string(),
        ),
    }
}

/// Starts a global key-tap watching for `descriptor`'s specific key and invokes
/// `on_event` on press/release. See linux::listen for the `generation` staleness
/// mechanism used to retire a listener after reconfiguration.
pub fn listen_hotkey<F>(
    descriptor: HotkeyDescriptor,
    on_event: F,
    generation: Arc<AtomicU64>,
    my_generation: u64,
) where
    F: Fn(HotkeyEvent) + Send + 'static,
{
    let HotkeyDescriptor::Macos { key_name } = descriptor else {
        return;
    };

    std::thread::spawn(move || {
        let callback = move |event: rdev::Event| {
            if generation.load(Ordering::SeqCst) != my_generation {
                return;
            }
            let (key, pressed) = match event.event_type {
                EventType::KeyPress(key) => (key, true),
                EventType::KeyRelease(key) => (key, false),
                _ => return,
            };
            if format!("{:?}", key) != key_name {
                return;
            }
            let hotkey_event = if pressed {
                HotkeyEvent::Pressed
            } else {
                HotkeyEvent::Released
            };
            on_event(hotkey_event);
        };
        if let Err(e) = listen(callback) {
            log::error!("push2talk: rdev listen error: {:?}", e);
        }
    });
}
