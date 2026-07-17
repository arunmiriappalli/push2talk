use super::Typist;
use enigo::{Enigo, Keyboard, Settings};

pub struct MacosTypist {
    enigo: Enigo,
}

impl MacosTypist {
    pub fn new() -> Result<Self, String> {
        let enigo = Enigo::new(&Settings::default())
            .map_err(|e| format!("Could not initialize keyboard simulation: {e}"))?;
        Ok(Self { enigo })
    }
}

impl Typist for MacosTypist {
    fn type_text(&mut self, text: &str, _delay_ms: u64) -> Result<(), String> {
        self.enigo
            .text(text)
            .map_err(|e| format!("Failed to type text: {e}"))?;
        Ok(())
    }
}

/// macOS doesn't expose a direct "is Accessibility granted" query without
/// pulling in extra frameworks, so this is a best-effort construction check —
/// real failures usually surface the first time `type_text` is actually used.
pub fn health_check() -> Result<(), String> {
    Enigo::new(&Settings::default()).map(|_| ()).map_err(|e| {
        format!(
            "Could not initialize keyboard simulation ({e}). Grant Accessibility permission to \
             this app in System Settings > Privacy & Security > Accessibility."
        )
    })
}
