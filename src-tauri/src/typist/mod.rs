#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

pub trait Typist {
    fn type_text(&mut self, text: &str, delay_ms: u64) -> Result<(), String>;
}

#[cfg(target_os = "linux")]
pub fn new() -> Result<Box<dyn Typist>, String> {
    Ok(Box::new(linux::LinuxTypist::new()))
}

#[cfg(target_os = "macos")]
pub fn new() -> Result<Box<dyn Typist>, String> {
    Ok(Box::new(macos::MacosTypist::new()?))
}

#[cfg(target_os = "linux")]
pub fn health_check() -> Result<(), String> {
    linux::health_check()
}

#[cfg(target_os = "macos")]
pub fn health_check() -> Result<(), String> {
    macos::health_check()
}
