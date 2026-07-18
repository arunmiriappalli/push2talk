use crate::config::HotkeyDescriptor;
use core_foundation::runloop::CFRunLoop;
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::HotkeyEvent;

/// macOS's virtual keycodes (kVK_* in Carbon's Events.h) are stable and
/// hardware/layout-independent — this table covers common push-to-talk key
/// choices without ever calling Carbon/TSM APIs, which must run on the main
/// thread and crash otherwise (see the Cargo.toml comment on this target's
/// dependencies for why that matters).
fn key_name_for(code: i64) -> String {
    let name = match code {
        0x7A => "F1",
        0x78 => "F2",
        0x63 => "F3",
        0x76 => "F4",
        0x60 => "F5",
        0x61 => "F6",
        0x62 => "F7",
        0x64 => "F8",
        0x65 => "F9",
        0x6D => "F10",
        0x67 => "F11",
        0x6F => "F12",
        0x69 => "F13",
        0x6B => "F14",
        0x71 => "F15",
        0x31 => "Space",
        0x30 => "Tab",
        0x24 => "Return",
        0x35 => "Escape",
        0x39 => "Caps Lock",
        0x37 => "Left Command",
        0x36 => "Right Command",
        0x38 => "Left Shift",
        0x3C => "Right Shift",
        0x3A => "Left Option",
        0x3D => "Right Option",
        0x3B => "Left Control",
        0x3E => "Right Control",
        0x3F => "Fn",
        _ => "",
    };
    if name.is_empty() {
        format!("Key {code}")
    } else {
        name.to_string()
    }
}

const NO_TAP_ERROR: &str = "Could not install a global key listener — grant Accessibility \
    permission to this app in System Settings > Privacy & Security > Accessibility, then retry.";

/// Starts a temporary global key-tap and waits for the first key-down anywhere,
/// returning its identity. Used by the setup wizard's "press the key you want to
/// use" step. Requires Accessibility permission to be granted to this app
/// (System Settings > Privacy & Security > Accessibility) or the tap fails to
/// install.
///
/// Reads only the raw keycode integer field from each event — never a
/// human-readable name — so this never touches the Carbon/TSM keyboard-layout
/// APIs that crash when called off the main thread (this listener runs on a
/// background thread, like the persistent one in `listen_hotkey`).
pub async fn capture_next_key() -> Result<HotkeyDescriptor, String> {
    let (tx, rx) = std::sync::mpsc::channel::<HotkeyDescriptor>();

    std::thread::spawn(move || {
        let run_loop = CFRunLoop::get_current();
        let result = CGEventTap::with_enabled(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::KeyDown],
            move |_proxy, _event_type, event| {
                let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                let _ = tx.send(HotkeyDescriptor::Macos {
                    key_code: code,
                    key_name: key_name_for(code),
                });
                run_loop.stop();
                CallbackResult::Keep
            },
            CFRunLoop::run_current,
        );
        if result.is_err() {
            log::error!("push2talk: failed to create macOS event tap for key capture");
        }
    });

    let result = tokio::task::spawn_blocking(move || rx.recv_timeout(Duration::from_secs(20)))
        .await
        .map_err(|e| e.to_string())?;

    match result {
        Ok(descriptor) => Ok(descriptor),
        Err(_) => Err(format!("Timed out waiting for a key press. {NO_TAP_ERROR}")),
    }
}

/// Starts a global key-tap watching for `descriptor`'s specific key and invokes
/// `on_event` on press/release. See linux::listen for the `generation` staleness
/// mechanism used to retire a listener after reconfiguration. Like
/// `capture_next_key`, only ever reads the raw keycode field.
pub fn listen_hotkey<F>(
    descriptor: HotkeyDescriptor,
    on_event: F,
    generation: Arc<AtomicU64>,
    my_generation: u64,
) where
    F: Fn(HotkeyEvent) + Send + 'static,
{
    let HotkeyDescriptor::Macos {
        key_code: target_code,
        ..
    } = descriptor
    else {
        return;
    };

    std::thread::spawn(move || {
        let result = CGEventTap::with_enabled(
            CGEventTapLocation::Session,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::ListenOnly,
            vec![CGEventType::KeyDown, CGEventType::KeyUp],
            move |_proxy, event_type, event| {
                if generation.load(Ordering::SeqCst) != my_generation {
                    return CallbackResult::Keep;
                }
                let code = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                if code != target_code {
                    return CallbackResult::Keep;
                }
                let hotkey_event = match event_type {
                    CGEventType::KeyDown => HotkeyEvent::Pressed,
                    CGEventType::KeyUp => HotkeyEvent::Released,
                    _ => return CallbackResult::Keep,
                };
                on_event(hotkey_event);
                CallbackResult::Keep
            },
            CFRunLoop::run_current,
        );
        if result.is_err() {
            log::error!("{NO_TAP_ERROR}");
        }
    });
}
