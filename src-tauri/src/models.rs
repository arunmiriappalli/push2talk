use futures_util::StreamExt;
use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

#[derive(Debug, Clone, Serialize)]
pub struct ModelInfo {
    pub name: &'static str,
    pub file: &'static str,
    pub size_label: &'static str,
    pub description: &'static str,
}

pub const CATALOG: &[ModelInfo] = &[
    ModelInfo {
        name: "tiny.en",
        file: "ggml-tiny.en.bin",
        size_label: "~75 MB",
        description: "Fastest, English-only, lowest accuracy",
    },
    ModelInfo {
        name: "base.en",
        file: "ggml-base.en.bin",
        size_label: "~142 MB",
        description: "Fast, English-only, decent accuracy",
    },
    ModelInfo {
        name: "small.en",
        file: "ggml-small.en.bin",
        size_label: "~466 MB",
        description: "Balanced speed/accuracy, English-only",
    },
    ModelInfo {
        name: "medium.en",
        file: "ggml-medium.en.bin",
        size_label: "~1.5 GB",
        description: "Slower, high accuracy, English-only",
    },
    ModelInfo {
        name: "large-v3-q5_0",
        file: "ggml-large-v3-q5_0.bin",
        size_label: "~1.1 GB",
        description: "Quantized large model: best accuracy for the size, multilingual",
    },
    ModelInfo {
        name: "large-v3",
        file: "ggml-large-v3.bin",
        size_label: "~3.1 GB",
        description: "Highest accuracy, multilingual, slowest",
    },
];

const HF_BASE_URL: &str = "https://huggingface.co/ggerganov/whisper.cpp/resolve/main";

/// Resolves a model's file path against `dir`, an explicit models directory
/// (usually `config::resolve_models_dir(&cfg.model_dir)` for whatever the
/// caller currently has in hand) rather than always re-reading it from disk —
/// callers may be acting on a `model_dir` the user just picked but hasn't
/// saved yet.
pub fn model_path(dir: &std::path::Path, file: &str) -> PathBuf {
    dir.join(file)
}

pub fn is_downloaded(dir: &std::path::Path, file: &str) -> bool {
    model_path(dir, file).is_file()
}

#[derive(Debug, Clone, Serialize)]
struct DownloadProgress {
    file: String,
    downloaded: u64,
    total: u64,
}

/// Downloads a ggml model from Hugging Face into `dir`, emitting
/// `model-download-progress` events as it goes so the setup wizard can show a
/// progress bar. No-ops if the file already exists.
pub async fn download(
    app: &AppHandle,
    dir: &std::path::Path,
    file: &str,
) -> Result<PathBuf, String> {
    let dest = model_path(dir, file);
    if dest.is_file() {
        return Ok(dest);
    }

    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| e.to_string())?;

    let url = format!("{HF_BASE_URL}/{file}");
    let response = reqwest::get(&url).await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }
    let total = response.content_length().unwrap_or(0);

    let tmp_path = dest.with_extension("part");
    let mut out = tokio::fs::File::create(&tmp_path)
        .await
        .map_err(|e| e.to_string())?;

    let mut stream = response.bytes_stream();
    let mut downloaded: u64 = 0;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        out.write_all(&chunk).await.map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        let _ = app.emit(
            "model-download-progress",
            DownloadProgress {
                file: file.to_string(),
                downloaded,
                total,
            },
        );
    }
    out.flush().await.map_err(|e| e.to_string())?;
    drop(out);

    tokio::fs::rename(&tmp_path, &dest)
        .await
        .map_err(|e| e.to_string())?;

    Ok(dest)
}
