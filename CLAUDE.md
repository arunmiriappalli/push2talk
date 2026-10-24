# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

push2talk is a cross-platform (Linux + macOS) Tauri v2 desktop app: hold a
hotkey, speak, release it, and the transcribed text (via local Whisper,
through `whisper-rs`/whisper.cpp) is typed wherever the cursor is. Everything
runs locally — no audio ever leaves the machine. Frontend is a small
vanilla TypeScript app (no framework); backend is Rust.

## Commands

```bash
npm install
npm run tauri dev     # run the full app in dev mode (Vite + cargo run)
npm run tauri build   # produce an installable bundle for this OS
npm run dev            # Vite frontend only (no Tauri/Rust backend)
npm run build           # tsc typecheck + vite build (frontend only)
```

There is no test suite and no lint command configured in this repo.

### Linux build prerequisites

```bash
sudo apt install build-essential curl wget file cmake pkg-config \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libxdo-dev libssl-dev libasound2-dev clang libclang-dev libvulkan-dev glslc
```

`clang`/`libclang-dev` are required by `bindgen` (used by `whisper-rs-sys`).
**Without them, the build silently falls back to a stale bundled bindings
file instead of failing** — if `whisper-rs`'s `vulkan` feature later fails
with `no ggml_backend_vk_get_device_count in the root`, it's almost always
this, fixed by installing the deps and then:
```bash
cargo clean -p whisper-rs-sys -p whisper-rs
```
This is a real, previously-hit failure mode, not a hypothetical.

`whisper-rs` compiles whisper.cpp from source as part of the Rust build
(that's what `cmake` is for) — no separate whisper.cpp clone needed. Whisper
models themselves are downloaded on demand by the in-app setup wizard, not
at build time.

### Runtime prerequisites (Linux)

Both touch kernel-level input device permissions, so they're manual
one-time steps rather than something a package install can automate:
- `sudo usermod -aG input $USER` (log out/in after) — needed for both
  push2talk's raw keyboard reading and ydotool's `/dev/uinput` writes.
- `systemctl --user enable --now ydotool` — ydotool is the typing backend.

## Architecture

### Process split

- **Frontend** (`src/`): vanilla TS, no framework. `main.ts` does manual DOM
  string-templating + `innerHTML` re-renders driven by a single module-level
  `state` object (see `render()` dispatching to `renderWizard()` /
  `renderSettings()`). `api.ts` wraps all `@tauri-apps/api` `invoke()` calls.
  There's a first-run setup wizard (mic → hotkey → model → typing backend →
  autostart) and a settings view shown after `setup_complete`.
- **Backend** (`src-tauri/src/`): Tauri commands in `commands.rs` are the
  only bridge the frontend calls through (registered in `lib.rs`'s
  `invoke_handler`). The actual push-to-talk logic runs independently of any
  webview call, in an engine thread spawned once at startup.

### The engine thread (`engine.rs`)

`engine::spawn()` is called once in `lib.rs`'s `setup()` and owns an
`mpsc` channel (`EngineHandle`). It runs a dedicated thread holding
`EngineState` (current config, lazily-(re)built `Transcriber`, `Typist`) and
reacts to two message types: `Hotkey(HotkeyEvent)` (from the OS-level hotkey
listener) and `Reload(AppConfig)` (sent whenever settings are saved, so a new
mic/model/hotkey takes effect without restarting the app). It emits
`engine-status` events (`Idle`/`Recording`/`Transcribing`/`Error`) that both
the settings UI and the tray icon tooltip/title listen for.

### Platform-specific modules (`hotkey/`, `typist/`)

Both have a `mod.rs` that dispatches to `linux.rs` or `macos.rs` via
`cfg(target_os = ...)`:
- **Hotkey capture** — Linux reads raw events directly from
  `/dev/input/eventN` via `evdev`, bypassing the display server (works
  under X11 and Wayland). macOS uses a hand-rolled `CGEventTap` that reads
  only the raw keycode integer. This is deliberate, not a stylistic choice:
  the `rdev` crate used to be used here and crashed on real Apple Silicon
  hardware, because its callback resolves a human-readable key name via
  Carbon/TSM (`TSMGetInputSourceProperty`), which asserts it's on the main
  dispatch queue and aborts when run from `rdev::listen()`'s background
  thread. The hand-rolled tap never touches Carbon/TSM. See the
  `HotkeyDescriptor` enum in `config.rs` for how each platform's descriptor
  is shaped and persisted.
- **Typing** — Linux shells out to `ydotool type` (uinput-based, works
  under Wayland). macOS uses `enigo` via CGEvent.

### Crisp rewrite (`rewrite.rs`)

Optional, off by default (`AppConfig::crisp_rewrite`), toggled in Settings.
Asks a local LLM to rephrase the transcription for clarity/concision; if the
result differs from the original, the typed output becomes
`"{original}\n{rewritten}"` so the user can delete whichever line they don't
want.

**Runs as a separate `llama-server` process, not an in-process library.**
This was tried first via the `llama-cpp-2` crate and abandoned: both it and
`whisper-rs` statically link their own vendored copy of `ggml` under the
same symbol names, which fails at link time with duplicate-symbol errors
(confirmed by actually trying it, not a hypothetical). `rewrite::Rewriter`
instead downloads an official llama.cpp release archive once (per-OS: a
Vulkan build on Linux, the standard Metal-enabled build on macOS — see
`LLAMA_BUILD`/`SERVER_ASSET` in `rewrite.rs`) and spawns `llama-server` as a
child process, talking to it over its OpenAI-compatible HTTP API
(`reqwest::blocking`, since `engine.rs`'s loop is synchronous). On Linux the
child is spawned with `PR_SET_PDEATHSIG` (via `pre_exec`) so the kernel
kills it the moment push2talk exits by any means — including `_exit()`,
which (see below) skips `Drop` entirely. No equivalent exists on macOS,
where cleanup relies on `Drop` and therefore misses the `_exit()` path (a
known, documented gap).

The GGUF model itself is either downloaded (Qwen2.5-1.5B-Instruct, a small
ungated default — see `MODEL_FILE`/`MODEL_URL`) or, via
`AppConfig::rewrite_model_path`, an arbitrary local file the user points at
through Settings' file picker (e.g. a larger model already present from LM
Studio or another tool) — `rewrite::effective_model_path` resolves which.

### Config and persistence

`config.rs` defines `AppConfig`, stored as JSON at
`~/.config/push2talk/config.json` (Linux) /
`~/Library/Application Support/push2talk/config.json` (macOS). Whisper
models cache alongside it in the platform data dir unless overridden by a
user-picked model dir.

### Tray / window lifecycle quirks (don't "fix" these without re-reading the context)

All in `lib.rs`, each backed by a real crash report or observed bug, not
speculation:
- The window is sent to background with `window.minimize()` /
  `window.unminimize()`, never `hide()`/`show()` — a hide/show cycle left
  the window's titlebar buttons permanently unresponsive on Linux
  afterward, even though content kept rendering/accepting clicks.
- Quit always funnels through `RunEvent::ExitRequested` → `_exit(0)`
  (declared via `extern "C"`), never Tauri's `app.exit()`'s normal path or
  a raw `exit()`. whisper.cpp's Metal backend frees a global device in a
  C++ static destructor that can race an async Metal init and abort the
  process; `_exit()` skips static destructors entirely and sidesteps it.
  This is why the app defines a custom macOS app menu (replacing Tauri's
  default, which uses `PredefinedMenuItem::quit()` and bypasses
  `ExitRequested` via a direct native `exit(0)` call) — Cmd+Q and the
  system "Quit push2talk" item need to route through the same handler as
  the tray's Quit item.
- A missing `NSMicrophoneUsageDescription` (macOS `Info.plist`, auto-merged
  by Tauri) doesn't make recording fail loudly — it silently records
  silence, which Whisper then hallucinates into things like "Thank you".

## Releasing

Pushing a `v*` tag runs `.github/workflows/release.yml` (builds Linux +
macOS bundles via `tauri-action`, attaches them to a draft GitHub release).
The Linux `.deb`/`.rpm` explicitly declare `libvulkan1`/`vulkan-loader` as
package dependencies in `tauri.conf.json` — Tauri's bundler doesn't
auto-detect that dynamic link the way it does for webkit2gtk/GTK.
