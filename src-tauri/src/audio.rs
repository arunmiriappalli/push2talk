use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

const TARGET_SAMPLE_RATE: u32 = 16_000;

/// ALSA exposes many names per physical device (`hw:`, `plughw:`,
/// `sysdefault:`, `front:`, `dmix`, `dsnoop`, `surround*`, `iec958`, ...) —
/// a machine with two real input devices can report 20+ entries. This is
/// Linux/ALSA-specific noise; harmless to apply everywhere since these
/// patterns don't occur in CoreAudio device names, so it's a no-op there.
/// Kept: generic DE-routed names (default/pulse/pipewire) plus one
/// `plughw:` entry per physical device — `plughw` auto-converts sample
/// rate/format like the other aliases, and is what the original bash
/// script targeted directly.
fn is_useful_alsa_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    matches!(lower.as_str(), "default" | "pulse" | "pipewire") || lower.starts_with("plughw:")
}

pub fn list_input_devices() -> Vec<String> {
    let host = cpal::default_host();
    let names: Vec<String> = match host.input_devices() {
        Ok(devices) => devices.map(|d| d.to_string()).collect(),
        Err(_) => return Vec::new(),
    };

    let filtered: Vec<String> = names
        .iter()
        .filter(|n| is_useful_alsa_name(n))
        .cloned()
        .collect();

    if filtered.is_empty() {
        names
    } else {
        filtered
    }
}

fn find_device(name: Option<&str>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        None => host
            .default_input_device()
            .ok_or_else(|| "No default input device found".to_string()),
        Some(name) => host
            .input_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.to_string() == name)
            .ok_or_else(|| format!("Input device '{name}' not found")),
    }
}

/// An in-progress recording. Must be created and dropped on the same thread
/// that owns the engine loop — `cpal::Stream` is not `Send` on most backends.
pub struct Recording {
    stream: cpal::Stream,
    samples: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
}

pub fn start(device_name: Option<&str>) -> Result<Recording, String> {
    let device = find_device(device_name)?;
    let config = device
        .default_input_config()
        .map_err(|e| format!("No default input config: {e}"))?;
    let sample_rate = config.sample_rate();
    let channels = config.channels();
    log::info!(
        "push2talk: starting recording on device {device}, {sample_rate} Hz, {channels} channel(s), format {:?}",
        config.sample_format()
    );
    let samples: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(Vec::new()));

    let samples_cb = samples.clone();
    let err_fn = |err| log::error!("push2talk: audio stream error: {err}");

    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => device
            .build_input_stream(
                config.into(),
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    samples_cb.lock().unwrap().extend_from_slice(data);
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string())?,
        cpal::SampleFormat::I16 => device
            .build_input_stream(
                config.into(),
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    let mut buf = samples_cb.lock().unwrap();
                    buf.extend(data.iter().map(|s| *s as f32 / i16::MAX as f32));
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string())?,
        cpal::SampleFormat::U16 => device
            .build_input_stream(
                config.into(),
                move |data: &[u16], _: &cpal::InputCallbackInfo| {
                    let mut buf = samples_cb.lock().unwrap();
                    buf.extend(data.iter().map(|s| (*s as f32 - 32768.0) / 32768.0));
                },
                err_fn,
                None,
            )
            .map_err(|e| e.to_string())?,
        other => return Err(format!("Unsupported sample format: {other:?}")),
    };

    stream.play().map_err(|e| e.to_string())?;

    Ok(Recording {
        stream,
        samples,
        sample_rate,
        channels,
    })
}

/// Stops the stream and returns mono, 16kHz f32 PCM samples ready for whisper-rs.
pub fn stop(recording: Recording) -> Vec<f32> {
    drop(recording.stream);
    // NOT `Arc::try_unwrap(recording.samples)...unwrap_or_default()`: that
    // requires this to already be the last owner of the Arc, but the
    // callback closure holds its own clone, and dropping the stream doesn't
    // synchronously guarantee that clone is released before this line runs
    // (confirmed on real hardware -- the callback was firing with real,
    // non-empty buffers the whole time, but try_unwrap still lost the race
    // often enough to reliably return an empty Vec via unwrap_or_default,
    // silently discarding a full recording every time). Locking and taking
    // the contents works regardless of how many Arc clones are still live,
    // since by this point the stream is stopped and nothing is going to
    // write to it again.
    let raw = std::mem::take(&mut *recording.samples.lock().unwrap());
    log::info!("push2talk: captured {} raw samples", raw.len());

    let mono = downmix(&raw, recording.channels);
    resample_linear(&mono, recording.sample_rate, TARGET_SAMPLE_RATE)
}

fn downmix(samples: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return samples.to_vec();
    }
    let channels = channels as usize;
    samples
        .chunks(channels)
        .map(|frame| frame.iter().sum::<f32>() / frame.len() as f32)
        .collect()
}

/// Simple linear-interpolation resampler. Not as accurate as a sinc resampler,
/// but sufficient for speech input feeding Whisper, and avoids an extra
/// dependency for an otherwise small MVP surface.
fn resample_linear(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = (samples.len() as f64 / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx = src_pos.floor() as usize;
        let frac = (src_pos - idx as f64) as f32;
        let s0 = samples.get(idx).copied().unwrap_or(0.0);
        let s1 = samples.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 + (s1 - s0) * frac);
    }
    out
}
