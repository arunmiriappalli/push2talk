use crate::transcribe;
use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
pub struct HardwareInfo {
    pub cpu_cores: usize,
    pub total_ram_gb: Option<f64>,
    pub gpu_name: Option<String>,
    pub gpu_backend: Option<&'static str>,
    pub recommended_model: &'static str,
}

pub fn detect() -> HardwareInfo {
    let cpu_cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let total_ram_gb = total_ram_gb();
    let gpu_backend = transcribe::gpu_backend_compiled();
    let gpu_name = if gpu_backend.is_some() {
        detect_gpu_name()
    } else {
        None
    };
    let recommended_model = recommend_model(cpu_cores, total_ram_gb, gpu_backend.is_some());

    HardwareInfo {
        cpu_cores,
        total_ram_gb,
        gpu_name,
        gpu_backend,
        recommended_model,
    }
}

fn recommend_model(cpu_cores: usize, total_ram_gb: Option<f64>, has_gpu: bool) -> &'static str {
    let ram = total_ram_gb.unwrap_or(4.0);
    if has_gpu && ram >= 8.0 {
        "large-v3-q5_0"
    } else if ram >= 8.0 && cpu_cores >= 8 {
        "small.en"
    } else if ram >= 4.0 {
        "base.en"
    } else {
        "tiny.en"
    }
}

#[cfg(target_os = "linux")]
fn total_ram_gb() -> Option<f64> {
    let contents = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = contents.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0 / 1024.0)
}

#[cfg(target_os = "macos")]
fn total_ram_gb() -> Option<f64> {
    let output = Command::new("sysctl")
        .arg("-n")
        .arg("hw.memsize")
        .output()
        .ok()?;
    let bytes: f64 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .ok()?;
    Some(bytes / 1024.0 / 1024.0 / 1024.0)
}

#[cfg(target_os = "linux")]
fn detect_gpu_name() -> Option<String> {
    let output = Command::new("lspci").output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    text.lines()
        .find(|l| {
            let lower = l.to_lowercase();
            lower.contains("vga") || lower.contains("3d controller")
        })
        .map(|l| {
            let after_colon = l.split_once(": ").map(|x| x.1).unwrap_or(l).trim();
            // lspci lines look like "NVIDIA Corporation GB202 [GeForce RTX
            // 5090] (rev a1)" — the bracketed marketing name is what's
            // actually recognizable; fall back to the full string if there
            // isn't one.
            match (after_colon.find('['), after_colon.find(']')) {
                (Some(start), Some(end)) if end > start => after_colon[start + 1..end].to_string(),
                _ => after_colon.to_string(),
            }
        })
}

#[cfg(target_os = "macos")]
fn detect_gpu_name() -> Option<String> {
    let output = Command::new("sysctl")
        .arg("-n")
        .arg("machdep.cpu.brand_string")
        .output()
        .ok()?;
    let chip = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if chip.is_empty() {
        None
    } else {
        Some(format!("{chip} (integrated GPU, Metal)"))
    }
}
