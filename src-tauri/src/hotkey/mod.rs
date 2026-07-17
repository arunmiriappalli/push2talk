#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub use linux::capture_next_key;
#[cfg(target_os = "linux")]
pub use linux::listen as listen_hotkey;

#[cfg(target_os = "macos")]
pub use macos::capture_next_key;
#[cfg(target_os = "macos")]
pub use macos::listen_hotkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyEvent {
    Pressed,
    Released,
}
