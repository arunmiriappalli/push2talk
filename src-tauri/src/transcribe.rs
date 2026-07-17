use std::path::Path;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

/// Whether this build was compiled with a GPU backend. Keyed off `target_os`
/// rather than `cfg!(feature = ...)` because whisper-rs's GPU cargo features
/// are enabled on the *dependency* per Cargo.toml's per-target sections —
/// `cfg!(feature = ...)` only ever sees this crate's own declared features,
/// so it can't observe that. whisper.cpp still falls back to CPU at runtime
/// if no compatible device/driver is found even when a GPU backend is
/// compiled in.
pub fn gpu_backend_compiled() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("Metal")
    } else if cfg!(target_os = "linux") {
        Some("Vulkan")
    } else {
        None
    }
}

pub struct Transcriber {
    ctx: WhisperContext,
    threads: i32,
}

impl Transcriber {
    pub fn load(model_path: &Path, threads: i32) -> Result<Self, String> {
        let mut params = WhisperContextParameters::default();
        // Defaults to `cfg!(feature = "_gpu")` already, i.e. true when built
        // with the vulkan/metal feature — set explicitly for clarity.
        params.use_gpu(gpu_backend_compiled().is_some());

        let ctx = WhisperContext::new_with_params(
            model_path
                .to_str()
                .ok_or_else(|| "Model path is not valid UTF-8".to_string())?,
            params,
        )
        .map_err(|e| format!("Failed to load Whisper model: {e}"))?;
        Ok(Self { ctx, threads })
    }

    /// Runs transcription on mono 16kHz f32 PCM samples and returns the first
    /// non-empty line of output, mirroring the original script's
    /// `awk 'NF{print; exit}'` cleanup (drops leading blank/whitespace segments).
    pub fn transcribe(&self, samples: &[f32]) -> Result<String, String> {
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| format!("Failed to create Whisper state: {e}"))?;

        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
        params.set_n_threads(self.threads);
        params.set_translate(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_special(false);
        params.set_print_timestamps(false);
        params.set_single_segment(false);
        params.set_suppress_blank(true);

        state
            .full(params, samples)
            .map_err(|e| format!("Whisper inference failed: {e}"))?;

        let n_segments = state.full_n_segments();

        for i in 0..n_segments {
            let Some(segment) = state.get_segment(i) else {
                continue;
            };
            let text = segment.to_str_lossy().unwrap_or_default();
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Ok(trimmed.to_string());
            }
        }
        Ok(String::new())
    }
}
