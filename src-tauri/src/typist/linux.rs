use super::Typist;
use std::path::Path;
use std::process::Command;

pub struct LinuxTypist;

impl LinuxTypist {
    pub fn new() -> Self {
        Self
    }
}

impl Typist for LinuxTypist {
    fn type_text(&mut self, text: &str, delay_ms: u64) -> Result<(), String> {
        // Args are passed directly to exec (no shell), so arbitrary transcribed
        // text is safe here even though it's not sanitized.
        let output = Command::new("ydotool")
            .arg("type")
            .arg(format!("--key-delay={delay_ms}"))
            .arg(text)
            .output()
            .map_err(|e| format!("Failed to run ydotool: {e}. Is it installed and in PATH?"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("ydotool exited with {}: {stderr}", output.status));
        }
        Ok(())
    }
}

/// Sanity-checks that `ydotool` is installed and `ydotoold` appears to be
/// reachable, for the setup wizard's typing-backend check step.
pub fn health_check() -> Result<(), String> {
    let found = Command::new("which")
        .arg("ydotool")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !found {
        return Err(
            "ydotool not found in PATH. Install it (e.g. `sudo apt install ydotool`) and \
             ensure the ydotoold daemon is running."
                .to_string(),
        );
    }

    let socket_candidates: Vec<String> = [
        std::env::var("YDOTOOL_SOCKET").ok(),
        std::env::var("XDG_RUNTIME_DIR")
            .ok()
            .map(|d| format!("{d}/.ydotool_socket")),
        Some("/tmp/.ydotool_socket".to_string()),
    ]
    .into_iter()
    .flatten()
    .collect();

    let socket_ok = socket_candidates.iter().any(|p| Path::new(p).exists());
    if !socket_ok {
        return Err(
            "ydotoold socket not found. Start the daemon, e.g. `sudo systemctl enable --now \
             ydotoold` (or run `ydotoold &`), then retry."
                .to_string(),
        );
    }

    Ok(())
}
