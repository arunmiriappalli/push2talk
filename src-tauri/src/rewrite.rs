use crate::models;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tauri::AppHandle;

/// Pinned to a specific llama.cpp nightly build ("b" + ggml-org/llama.cpp's
/// incrementing build-number tag, not a semver release) for reproducible
/// downloads, the same way Whisper models are pinned by exact filename.
/// Nightly tags do eventually get pruned from GitHub releases, so this may
/// need bumping to a current build number if the URL starts 404ing.
const LLAMA_BUILD: &str = "b11865";
#[cfg(target_os = "linux")]
const SERVER_ASSET: &str = "llama-b11865-bin-ubuntu-vulkan-x64.tar.gz";
#[cfg(target_os = "macos")]
const SERVER_ASSET: &str = "llama-b11865-bin-macos-arm64.tar.gz";

/// Qwen's own (ungated, directly downloadable) GGUF repo — avoids the
/// license click-through Meta's Llama repos require, which would break an
/// unattended/automated download the way ggerganov/whisper.cpp's repo
/// (already used for Whisper models) doesn't.
pub const MODEL_FILE: &str = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
const MODEL_URL: &str =
    "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf";

const SYSTEM_PROMPT: &str = "You rewrite dictated speech-to-text transcriptions to be clear, \
concise, and grammatically correct, while preserving the speaker's original meaning and tone. \
Output ONLY the rewritten text — no preamble, no explanation, no quotation marks.";

/// Cap on generated tokens: keeps a "crisp" rewrite from running away, and
/// bounds how long a hotkey release can block on this (same tradeoff as
/// Whisper's model-size choice already makes for transcription speed).
const MAX_NEW_TOKENS: u32 = 256;

const PORT: u16 = 58931;

/// Where the extracted llama-server release lives: `<dir>/llama-server/`,
/// containing the binary plus its sibling `.so` files (its RPATH is
/// `$ORIGIN`-relative, so it must stay next to them — no LD_LIBRARY_PATH
/// juggling needed as long as we don't move the binary out on its own).
fn server_dir(dir: &Path) -> PathBuf {
    dir.join("llama-server")
}

fn server_binary_path(dir: &Path) -> PathBuf {
    server_dir(dir).join("llama-server")
}

pub fn model_path(dir: &Path) -> PathBuf {
    models::model_path(dir, MODEL_FILE)
}

/// The GGUF to actually load: `override_path` (an existing local model the
/// user picked, e.g. from LM Studio's cache) if set, otherwise the bundled
/// default downloaded into `dir`.
pub fn effective_model_path(dir: &Path, override_path: &Option<String>) -> PathBuf {
    match override_path {
        Some(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => model_path(dir),
    }
}

pub fn is_downloaded(dir: &Path, override_path: &Option<String>) -> bool {
    server_binary_path(dir).is_file() && effective_model_path(dir, override_path).is_file()
}

/// Downloads the llama-server release archive (then extracts it), and — only
/// when the user hasn't picked their own local model via
/// `rewrite_model_path` — the default GGUF too. Emits the same
/// `model-download-progress` events the Whisper model download uses so the
/// settings UI's existing progress bar covers this too.
pub async fn download(
    app: &AppHandle,
    dir: &Path,
    override_path: &Option<String>,
) -> Result<PathBuf, String> {
    if override_path.is_none() {
        models::download_url(app, dir, MODEL_FILE, MODEL_URL).await?;
    }

    let out_dir = server_dir(dir);
    if !server_binary_path(dir).is_file() {
        let archive_name = "llama-server.tar.gz";
        let url = format!(
            "https://github.com/ggml-org/llama.cpp/releases/download/{LLAMA_BUILD}/{SERVER_ASSET}"
        );
        let archive_path = models::download_url(app, dir, archive_name, &url).await?;

        tokio::fs::create_dir_all(&out_dir)
            .await
            .map_err(|e| e.to_string())?;
        // Shelling out to `tar` rather than adding an archive crate — this
        // app already shells out to other platform tools (ydotool, which)
        // instead of pulling in a Rust equivalent for each.
        let status = Command::new("tar")
            .args(["--strip-components=1", "-xzf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&out_dir)
            .status()
            .map_err(|e| format!("Failed to run tar: {e}"))?;
        if !status.success() {
            return Err(format!("tar extraction exited with {status}"));
        }
        let _ = std::fs::remove_file(&archive_path);
    }

    Ok(effective_model_path(dir, override_path))
}

/// Manages a local `llama-server` child process and talks to it over its
/// OpenAI-compatible HTTP API. Kept separate from in-process inference
/// (e.g. via `llama-cpp-2`) deliberately: linking llama.cpp's `ggml`
/// alongside whisper.cpp's own (separately vendored) copy of the same
/// library into one binary fails at link time with duplicate symbols
/// (`ggml_*`, `gguf_*`, `quantize_row_*`, ...) — confirmed by actually
/// trying it, not a hypothetical. Running it as its own process sidesteps
/// that entirely, the same way `ydotool` being a separate process means its
/// dependencies never have to coexist with this app's in one binary.
pub struct Rewriter {
    child: Child,
    client: reqwest::blocking::Client,
    base_url: String,
}

impl Rewriter {
    pub fn load(bin_dir: &Path, model_path: &Path) -> Result<Self, String> {
        let bin = server_binary_path(bin_dir);
        let mut command = Command::new(&bin);
        command
            .args([
                "--model",
                model_path.to_str().ok_or("model path is not valid UTF-8")?,
                "--port",
                &PORT.to_string(),
                "--host",
                "127.0.0.1",
                "-ngl",
                "999",
                "--no-webui",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        die_with_parent(&mut command);
        let mut child = command
            .spawn()
            .map_err(|e| format!("Failed to start llama-server: {e}"))?;

        let base_url = format!("http://127.0.0.1:{PORT}");
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;

        // Model loading takes a few seconds; poll /health rather than
        // guessing a fixed sleep.
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(resp) = client.get(format!("{base_url}/health")).send() {
                if resp.status().is_success() {
                    break;
                }
            }
            if let Ok(Some(status)) = child.try_wait() {
                return Err(format!("llama-server exited early with {status}"));
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                return Err("llama-server did not become healthy within 60s".to_string());
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        Ok(Self {
            child,
            client,
            base_url,
        })
    }

    /// Returns a rewritten version of `text`, or `None` if the model's
    /// output doesn't meaningfully differ from the input (so callers don't
    /// type out a redundant third line).
    pub fn rewrite(&self, text: &str) -> Result<Option<String>, String> {
        let body = ChatRequest {
            messages: vec![
                ChatMessage { role: "system", content: SYSTEM_PROMPT.to_string() },
                ChatMessage { role: "user", content: text.to_string() },
            ],
            temperature: 0.0,
            max_tokens: MAX_NEW_TOKENS,
        };

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .json(&body)
            .send()
            .map_err(|e| format!("Request to llama-server failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().unwrap_or_default();
            return Err(format!("llama-server returned {status}: {body}"));
        }

        let parsed: ChatResponse = resp
            .json()
            .map_err(|e| format!("Failed to parse llama-server response: {e}"))?;
        let Some(choice) = parsed.choices.into_iter().next() else {
            return Ok(None);
        };

        let trimmed = choice.message.content.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(text.trim()) {
            Ok(None)
        } else {
            Ok(Some(trimmed.to_string()))
        }
    }
}

impl Drop for Rewriter {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// On a normal quit, this app's own exit path calls `_exit()` directly
/// (see lib.rs) to sidestep a whisper.cpp/Metal static-destructor crash —
/// which means `Drop` (and this `Rewriter`'s kill-on-drop above) never
/// runs. `PR_SET_PDEATHSIG`, set from *inside the child* right before it
/// execs (via `pre_exec`, which runs post-fork/pre-exec in the child), asks
/// the kernel to deliver `SIGKILL` to that child the moment this process
/// exits by any means (`_exit()`, a crash, a plain `kill`) — so the server
/// never outlives us regardless of how we go down. No equivalent exists on
/// macOS; there we rely on `Drop`, which only covers non-`_exit()` shutdown
/// paths (a known gap, not unlike other `_exit()`-related tradeoffs already
/// documented in this codebase).
#[cfg(target_os = "linux")]
fn die_with_parent(command: &mut Command) {
    use std::os::unix::process::CommandExt;
    unsafe {
        command.pre_exec(|| {
            // Best-effort: if this fails the child just behaves as it would
            // without this call, no worse than before it existed.
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
            Ok(())
        });
    }
}

#[cfg(target_os = "macos")]
fn die_with_parent(_command: &mut Command) {}

#[derive(Serialize)]
struct ChatMessage {
    role: &'static str,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatResponseMessage,
}

#[derive(Deserialize)]
struct ChatResponseMessage {
    content: String,
}
