# push2talk

Hold a key, speak, release it — your words get typed wherever your cursor is,
transcribed locally with [Whisper](https://github.com/ggml-org/whisper.cpp).
Cross-platform (Linux + macOS), local-only (no audio ever leaves your
machine), with a GUI setup wizard for configuring the hotkey, microphone, and
model.

This started as a hardcoded personal bash script and was rebuilt as a proper
[Tauri v2](https://v2.tauri.app) app so it's installable and reconfigurable
without editing shell variables.

## Installing

Grab the latest build for your OS from
[Releases](../../releases):

- **Linux**: `.deb` (Debian/Ubuntu) or `.AppImage` (portable, any distro)
- **macOS**: `.dmg` — pick `aarch64` for Apple Silicon or `x86_64` for Intel

macOS builds are ad-hoc signed (no Apple Developer account behind this
project — that costs $99/year, so this stays free) rather than signed with a
real Developer ID and notarized. Gatekeeper will still flag the first
launch. Try right-click the app → *Open* first; if macOS instead says the
app "is damaged and can't be opened" (a misleading message some macOS
versions show for unsigned apps, particularly on Apple Silicon — it's not
actually a corrupted download), strip the quarantine flag instead:
```bash
xattr -cr /Applications/push2talk.app
```

On first launch, a setup wizard walks through: picking a microphone, setting
your push-to-talk key, choosing and downloading a Whisper model, checking the
typing backend, and optionally enabling launch-on-login. After that it lives
in the tray/menu bar — reopen the window any time from there to change
settings.

### Linux prerequisites

Installing the `.deb`/`.rpm` via your package manager (`apt install
./push2talk_*.deb`, etc.) already handles what a package *can* safely
automate: it pulls in `libvulkan1`/`vulkan-loader` (required at launch) and
[`ydotool`](https://github.com/ReimuNotMoe/ydotool) (recommended — apt/dnf
installs it by default unless you pass `--no-install-recommends`) as
dependencies.

Two things are left as manual one-time steps, both because they touch
kernel-level input device permissions — package managers deliberately don't
grant these silently during install, without you explicitly running the
command yourself. This isn't push2talk being lazy: `ydotool`'s own Debian
package documents the exact same two steps, for the exact same reason.

- **Add your account to the `input` group** — this one group covers both
  push2talk reading raw keyboard events *and* `ydotool`'s daemon writing
  synthetic ones via `/dev/uinput`:
  ```bash
  sudo usermod -aG input $USER   # log out and back in afterward
  ```
- **Start the `ydotool` background service once** — it ships a systemd
  *user* service, which a package's install script can't reliably enable on
  your behalf (it needs your active login session, not available yet during
  `apt install`):
  ```bash
  systemctl --user enable --now ydotool
  ```

The setup wizard's "Typing backend" step checks both and tells you exactly
what's missing. If you installed the `.AppImage` instead of `.deb`/`.rpm`,
you'll need to install `ydotool` yourself too — AppImages don't go through a
package manager, so there's no dependency mechanism to lean on there.

### macOS prerequisites

- Grant **Accessibility** permission to push2talk (System Settings → Privacy
  & Security → Accessibility) so it can globally listen for your hotkey and
  simulate keystrokes.
- Grant **Microphone** permission when prompted.

## Building from source

Requires [Rust](https://rustup.rs) and [Node.js](https://nodejs.org).

Linux build dependencies (Ubuntu/Debian):

```bash
sudo apt install build-essential curl wget file cmake pkg-config \
  libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev \
  libxdo-dev libssl-dev libasound2-dev clang libclang-dev libvulkan-dev glslc
```

`clang`/`libclang-dev` are required by `bindgen` (used by whisper-rs-sys) —
without them the build silently falls back to a stale bundled bindings file
instead of failing, which is easy to not notice (see
[Known limitations](#known-limitations) for the exact failure mode this
causes if skipped). `libvulkan-dev` + `glslc` (package name is literally
`glslc` on Ubuntu) are for whisper.cpp's Vulkan GPU backend.

macOS build dependencies: Xcode Command Line Tools (`xcode-select
--install`) and `cmake` (`brew install cmake`). GPU acceleration there uses
Metal, which needs no extra install — it's part of the OS.

Then:

```bash
npm install
npm run tauri dev     # run in dev mode
npm run tauri build   # produce an installable bundle for this OS
```

`whisper-rs` compiles `whisper.cpp` from source as part of the build (that's
what `cmake` is for) — no separate whisper.cpp clone or model files are
needed at build time; Whisper models are downloaded on demand by the setup
wizard.

To regenerate the app icon from its source (`src-tauri/icons/app-icon-source.svg`)
after editing it:

```bash
npx tauri icon src-tauri/icons/app-icon-source.svg --output src-tauri/icons
rm -rf src-tauri/icons/android src-tauri/icons/ios  # this project is desktop-only
```

## How it works

- **Hotkey capture** — Linux reads raw input events directly from
  `/dev/input/eventN` (via the `evdev` crate), the same approach as the
  original script's `evtest` usage; this works under both X11 and Wayland
  since it bypasses the display server entirely. macOS uses a system-wide key
  event tap (`rdev`).
- **Recording** — `cpal` captures from the selected microphone; audio is
  downmixed to mono and resampled to 16kHz for Whisper.
- **Transcription** — `whisper-rs` (Rust bindings to whisper.cpp) runs
  locally against the model you picked in the wizard, GPU-accelerated via
  Vulkan (Linux) or Metal (macOS) — falls back to CPU automatically if no
  compatible GPU/driver is found at runtime. The setup wizard detects your
  CPU/RAM/GPU and recommends a model tier accordingly.
- **Typing** — Linux shells out to `ydotool type` (uinput-based, works under
  Wayland). macOS uses `enigo` to simulate keystrokes via CGEvent.
- **Config** — stored as JSON at `~/.config/push2talk/config.json` (Linux) or
  `~/Library/Application Support/push2talk/config.json` (macOS); models cache
  alongside it in the platform's data directory.

## Releasing

Pushing a `v*` tag runs [`.github/workflows/release.yml`](.github/workflows/release.yml),
which builds Linux and macOS bundles via `tauri-action` and attaches them to
a draft GitHub release.

Note: the Linux binary dynamically links `libvulkan.so.1` (for GPU
acceleration) but Tauri's bundler doesn't auto-detect that as a package
dependency the way it does for webkit2gtk/GTK — without it, a machine
lacking the Vulkan loader would install the `.deb`/`.rpm` fine via the
package manager but fail to *launch* the app. `libvulkan1` (deb) /
`vulkan-loader` (rpm) are declared explicitly in `tauri.conf.json`'s
`bundle.linux.deb.depends`/`bundle.linux.rpm.depends` to cover this.

## Known limitations

- **If `whisper-rs`'s `vulkan` feature fails to compile** with an error like
  `no ggml_backend_vk_get_device_count in the root`, it's very likely a stale
  `whisper-rs-sys` build cache from an earlier attempt where `clang`/
  `libclang-dev` weren't installed yet — without them, `bindgen` silently
  falls back to an incomplete bundled bindings file instead of failing
  loudly, and Cargo can keep reusing that stale output across later builds
  even after the real prerequisites are installed. Fix: `cargo clean -p
  whisper-rs-sys -p whisper-rs` and rebuild. (This isn't a hypothetical —
  it's exactly what happened building this project; a manual `bindgen` run
  with identical flags produced the correct bindings immediately, which is
  what pointed at stale cache rather than an upstream bug.)
- macOS's hotkey listener (`rdev`) and Metal GPU acceleration are still
  unverified on real hardware — Linux was developed and tested directly;
  the release build installs on Apple Silicon (M3/M4) but functional testing
  (hotkey capture, transcription, typing) hasn't been confirmed there yet.
- macOS releases are ad-hoc signed (no cost, no Apple Developer account) but
  not signed with a real Developer ID or notarized — Gatekeeper still warns
  on first launch, sometimes with a "damaged" message rather than the milder
  "unidentified developer" prompt depending on macOS version. See
  [Installing](#installing) for the workaround.
- Reconfiguring the hotkey while the app is running starts a new listener
  and retires the old one on its next event rather than tearing it down
  immediately — harmless, but means a stray background thread lingers per
  reconfiguration until the app restarts.
- **If you're adapting the tray-app pattern here for your own Tauri/Linux
  project**: don't send the main window to the tray with `window.hide()` and
  bring it back with `window.show()`. On this project that left the
  window's titlebar buttons (minimize/maximize/close) permanently
  unresponsive after the first hide/show cycle — the window content itself
  kept rendering and accepting clicks fine, just not its own chrome.
  `window.minimize()` / `window.unminimize()` is a far more standard,
  better-tested GTK/Wayland window lifecycle and doesn't have this problem;
  see `lib.rs`'s window setup code.
