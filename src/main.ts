import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "./api";
import {
  AppConfig,
  DownloadProgress,
  EngineStatus,
  HardwareInfo,
  ModelInfo,
  defaultConfig,
  hotkeyLabel,
} from "./types";

const WIZARD_STEPS = ["welcome", "mic", "hotkey", "model", "typing", "autostart", "done"] as const;
type WizardStep = (typeof WIZARD_STEPS)[number];

// Must match `rewrite::MODEL_FILE` in src-tauri/src/rewrite.rs — used as the
// `downloading`/progress-event key for the "crisp rewrite" LLM, the same way
// a whisper ModelInfo.file is used for transcription models.
const LLM_MODEL_FILE = "qwen2.5-1.5b-instruct-q4_k_m.gguf";
const LLM_MODEL_SIZE_LABEL = "~1.1 GB";

interface State {
  view: "loading" | "wizard" | "settings";
  step: WizardStep;
  cfg: AppConfig;
  devices: string[];
  models: ModelInfo[];
  hardware: HardwareInfo | null;
  downloading: string | null;
  downloadProgress: number;
  llmDownloaded: boolean | null;
  typingCheck: { ok: boolean; message: string } | null;
  capturingKey: boolean;
  autostartEnabled: boolean;
  autostartLoaded: boolean;
  engineStatus: EngineStatus;
  savedNotice: boolean;
}

const state: State = {
  view: "loading",
  step: "welcome",
  cfg: defaultConfig(),
  devices: [],
  models: [],
  hardware: null,
  downloading: null,
  downloadProgress: 0,
  llmDownloaded: null,
  typingCheck: null,
  capturingKey: false,
  autostartEnabled: false,
  autostartLoaded: false,
  engineStatus: { state: "idle" },
  savedNotice: false,
};

const app = document.querySelector<HTMLDivElement>("#app")!;

function render() {
  if (state.view === "loading") {
    app.innerHTML = `<p class="muted">Loading…</p>`;
    return;
  }
  if (state.view === "wizard") {
    renderWizard();
    return;
  }
  renderSettings();
}

// ---------- shared field renderers ----------

function micField(): string {
  const options = state.devices
    .map(
      (d) =>
        `<option value="${escapeAttr(d)}" ${d === state.cfg.audio_device ? "selected" : ""}>${escapeHtml(d)}</option>`
    )
    .join("");
  return `
    <label class="field">
      <span>Microphone</span>
      <select id="mic-select">
        <option value="">System default</option>
        ${options}
      </select>
    </label>`;
}

function hotkeyField(): string {
  const label = hotkeyLabel(state.cfg.hotkey);
  return `
    <label class="field">
      <span>Push-to-talk key</span>
      <div class="row">
        <span class="pill">${escapeHtml(label)}</span>
        <button id="capture-key-btn" type="button" ${state.capturingKey ? "disabled" : ""}>
          ${state.capturingKey ? "Press a key…" : "Press to set"}
        </button>
      </div>
    </label>`;
}

function hardwareSummary(): string {
  const hw = state.hardware;
  if (!hw) return `<p class="muted">Detecting your hardware…</p>`;
  const ram = hw.total_ram_gb ? `${hw.total_ram_gb.toFixed(1)} GB RAM` : "unknown RAM";
  const gpu = hw.gpu_backend
    ? hw.gpu_name
      ? `${escapeHtml(hw.gpu_name)} (${hw.gpu_backend} accelerated)`
      : `${hw.gpu_backend} accelerated`
    : "CPU only (no GPU backend compiled in)";
  return `<p class="muted">Detected: ${hw.cpu_cores} CPU cores, ${ram}, ${gpu}.</p>`;
}

function modelField(): string {
  const items = state.models
    .map((m) => {
      const selected = state.cfg.model?.name === m.name;
      const isDownloading = state.downloading === m.file;
      const isRecommended = state.hardware?.recommended_model === m.name;
      return `
      <label class="model-option ${selected ? "selected" : ""}">
        <input type="radio" name="model" value="${escapeAttr(m.name)}" ${selected ? "checked" : ""} />
        <div class="model-info">
          <strong>${escapeHtml(m.name)}</strong> <span class="muted">${escapeHtml(m.size_label)}</span>
          ${isRecommended ? `<span class="pill pill-small">Recommended for your hardware</span>` : ""}
          <div class="muted">${escapeHtml(m.description)}</div>
          ${isDownloading ? `<progress max="100" value="${state.downloadProgress}"></progress>` : ""}
        </div>
      </label>`;
    })
    .join("");
  return `<div class="field"><span>Whisper model</span>${hardwareSummary()}<div class="model-list">${items}</div></div>`;
}

function modelDirField(): string {
  const dir = state.cfg.model_dir ?? "Default (app data directory)";
  return `
    <label class="field">
      <span>Model download location</span>
      <div class="row">
        <span class="pill" title="${escapeAttr(dir)}">${escapeHtml(truncateMiddle(dir, 46))}</span>
        <button id="pick-model-dir-btn" type="button">Choose folder…</button>
      </div>
    </label>`;
}

function typingField(): string {
  const result = state.typingCheck;
  const status = !result
    ? `<span class="muted">Not checked yet</span>`
    : result.ok
      ? `<span class="ok">✓ Ready</span>`
      : `<span class="error">${escapeHtml(result.message)}</span>`;
  return `
    <div class="field">
      <span>Typing backend</span>
      <div class="row">${status} <button id="recheck-typing-btn" type="button">Recheck</button></div>
    </div>`;
}

function crispRewriteField(): string {
  const isDownloading = state.downloading === LLM_MODEL_FILE;
  const hasOverride = !!state.cfg.rewrite_model_path;
  const needsDownload = state.cfg.crisp_rewrite && state.llmDownloaded === false;
  const modelPath = hasOverride
    ? state.cfg.rewrite_model_path!
    : `Default (Qwen2.5-1.5B, ${LLM_MODEL_SIZE_LABEL})`;

  return `
    <div class="field">
      <label class="row">
        <input type="checkbox" id="crisp-rewrite-check" ${state.cfg.crisp_rewrite ? "checked" : ""} />
        <span>Also type a concise, rephrased version (local AI, slower)</span>
      </label>
      ${
        state.cfg.crisp_rewrite
          ? `<label class="field">
              <span>Rewrite model</span>
              <div class="row">
                <span class="pill" title="${escapeAttr(modelPath)}">${escapeHtml(truncateMiddle(modelPath, 46))}</span>
                <button id="pick-rewrite-model-btn" type="button">Choose model file…</button>
                ${hasOverride ? `<button id="clear-rewrite-model-btn" type="button">Use default</button>` : ""}
              </div>
              ${
                needsDownload
                  ? `<div class="row">
                      ${
                        isDownloading
                          ? `<progress max="100" value="${state.downloadProgress}"></progress>`
                          : `<button id="download-llm-btn" type="button">Download ${hasOverride ? "rewrite engine (~35 MB)" : `default model (${LLM_MODEL_SIZE_LABEL})`}</button>`
                      }
                    </div>`
                  : ""
              }
            </label>`
          : ""
      }
    </div>`;
}

function autostartField(): string {
  return `
    <label class="field row">
      <input type="checkbox" id="autostart-check" ${state.autostartEnabled ? "checked" : ""} />
      <span>Launch push2talk on login</span>
    </label>`;
}

// ---------- wizard ----------

function renderWizard() {
  const idx = WIZARD_STEPS.indexOf(state.step);
  const progressPct = Math.round((idx / (WIZARD_STEPS.length - 1)) * 100);

  let body = "";
  switch (state.step) {
    case "welcome":
      body = `
        <h1>Welcome to push2talk</h1>
        <p>Hold a key, speak, release it — your words get typed wherever your cursor is,
        transcribed locally with Whisper. Let's set it up.</p>`;
      break;
    case "mic":
      body = `<h1>Choose a microphone</h1>${micField()}`;
      break;
    case "hotkey":
      body = `<h1>Set your push-to-talk key</h1>
        <p class="muted">Click "Press to set", then press the key you want to hold to talk.</p>
        ${hotkeyField()}`;
      break;
    case "model":
      body = `<h1>Pick a Whisper model</h1>
        <p class="muted">Bigger models are more accurate but slower and larger to download.</p>
        ${modelDirField()}
        ${modelField()}`;
      break;
    case "typing":
      body = `<h1>Typing backend check</h1>
        <p class="muted">push2talk needs OS permission to simulate keystrokes.</p>
        ${typingField()}`;
      break;
    case "autostart":
      body = `<h1>Launch on login?</h1>${autostartField()}`;
      break;
    case "done":
      body = `<h1>All set</h1>
        <p>push2talk is now running in the background. Hold <strong>${escapeHtml(
          hotkeyLabel(state.cfg.hotkey)
        )}</strong> to talk. You can reopen this window any time from the tray icon.</p>`;
      break;
  }

  const isFirst = idx === 0;
  const isLast = state.step === "done";
  const nextDisabled = state.capturingKey || (state.step === "hotkey" && !state.cfg.hotkey);

  app.innerHTML = `
    <div class="wizard">
      <div class="progress-bar"><div class="progress-fill" style="width:${progressPct}%"></div></div>
      <div class="wizard-body">${body}</div>
      <div class="wizard-nav">
        <button id="back-btn" type="button" ${isFirst || state.capturingKey ? "disabled" : ""}>Back</button>
        <button id="next-btn" type="button" class="primary" ${nextDisabled ? "disabled" : ""}>
          ${isLast ? "Finish" : "Next"}
        </button>
      </div>
    </div>`;

  attachWizardListeners();
}

function attachWizardListeners() {
  document.querySelector("#back-btn")?.addEventListener("click", () => {
    const idx = WIZARD_STEPS.indexOf(state.step);
    if (idx > 0) {
      state.step = WIZARD_STEPS[idx - 1];
      render();
    }
  });

  document.querySelector("#next-btn")?.addEventListener("click", async () => {
    if (state.step === "done") {
      state.cfg.setup_complete = true;
      await api.saveConfig(state.cfg);
      state.view = "settings";
      state.savedNotice = false;
      render();
      getCurrentWindow().hide();
      return;
    }
    const idx = WIZARD_STEPS.indexOf(state.step);
    state.step = WIZARD_STEPS[idx + 1];
    render();
    await onStepEnter(state.step);
  });

  attachSharedFieldListeners();
}

async function onStepEnter(step: WizardStep) {
  if (step === "mic" && state.devices.length === 0) {
    state.devices = await api.listAudioDevices();
    render();
  }
  if (step === "model" && state.models.length === 0) {
    state.models = await api.listModels();
    render();
  }
  if (step === "model" && !state.hardware) {
    state.hardware = await api.detectHardware().catch(() => null);
    render();
  }
  if (step === "typing") {
    await runTypingCheck();
  }
  if (step === "autostart") {
    state.autostartEnabled = await api.getAutostart().catch(() => false);
    render();
  }
}

// ---------- settings ----------

function renderSettings() {
  const err = state.engineStatus;
  const errorBanner =
    err.state === "error" && err.message
      ? `<div class="error-banner"><strong>Something went wrong:</strong> ${escapeHtml(err.message)}</div>`
      : "";

  app.innerHTML = `
    <div class="settings">
      <div class="settings-header">
        <h1>push2talk settings</h1>
        <span class="status-badge status-${state.engineStatus.state}">${state.engineStatus.state}</span>
      </div>
      ${errorBanner}
      ${micField()}
      ${hotkeyField()}
      ${modelField()}
      ${modelDirField()}
      ${typingField()}
      ${crispRewriteField()}
      ${autostartField()}
      <div class="row settings-actions">
        <button id="save-btn" type="button" class="primary" ${state.capturingKey ? "disabled" : ""}>Save</button>
        ${state.savedNotice ? `<span class="ok">Saved</span>` : ""}
      </div>
    </div>`;

  attachSharedFieldListeners();
  document.querySelector("#save-btn")?.addEventListener("click", async () => {
    await api.saveConfig(state.cfg);
    await api.setAutostart(state.autostartEnabled);
    state.cfg.autostart = state.autostartEnabled;
    state.savedNotice = true;
    render();
  });

  if (state.devices.length === 0) api.listAudioDevices().then((d) => ((state.devices = d), render()));
  if (state.models.length === 0) api.listModels().then((m) => ((state.models = m), render()));
  if (!state.hardware) api.detectHardware().then((h) => ((state.hardware = h), render()));
  if (!state.typingCheck) runTypingCheck();
  if (state.llmDownloaded === null) {
    api
      .isLlmDownloaded(state.cfg.model_dir, state.cfg.rewrite_model_path)
      .then((d) => ((state.llmDownloaded = d), render()));
  }
  if (state.autostartLoaded) return;
  api.getAutostart().then((v) => {
    state.autostartEnabled = v;
    state.autostartLoaded = true;
    render();
  });
}

// ---------- shared listeners (used by both wizard + settings) ----------

function attachSharedFieldListeners() {
  document.querySelector("#mic-select")?.addEventListener("change", (e) => {
    const value = (e.target as HTMLSelectElement).value;
    state.cfg.audio_device = value || null;
  });

  document.querySelector("#capture-key-btn")?.addEventListener("click", async () => {
    state.capturingKey = true;
    render();
    try {
      const descriptor = await api.captureHotkey();
      state.cfg.hotkey = descriptor;
    } catch (e) {
      console.error(e);
      alert(`Couldn't capture a key: ${e}`);
    } finally {
      state.capturingKey = false;
      render();
    }
  });

  document.querySelector("#pick-model-dir-btn")?.addEventListener("click", async () => {
    const dir = await api.pickModelDir();
    if (dir) {
      state.cfg.model_dir = dir;
      render();
    }
  });

  document.querySelectorAll<HTMLInputElement>('input[name="model"]').forEach((el) => {
    el.addEventListener("change", async () => {
      const model = state.models.find((m) => m.name === el.value);
      if (!model) return;
      state.cfg.model = { name: model.name, path: "" };
      render();
      const already = await api.isModelDownloaded(model.file, state.cfg.model_dir);
      if (!already) {
        state.downloading = model.file;
        state.downloadProgress = 0;
        render();
        try {
          await api.downloadModel(model.file, state.cfg.model_dir);
        } catch (e) {
          alert(`Download failed: ${e}`);
        } finally {
          state.downloading = null;
          render();
        }
      }
    });
  });

  document.querySelector("#recheck-typing-btn")?.addEventListener("click", runTypingCheck);

  document.querySelector("#autostart-check")?.addEventListener("change", (e) => {
    state.autostartEnabled = (e.target as HTMLInputElement).checked;
  });

  document.querySelector("#crisp-rewrite-check")?.addEventListener("change", (e) => {
    state.cfg.crisp_rewrite = (e.target as HTMLInputElement).checked;
    render();
  });

  document.querySelector("#pick-rewrite-model-btn")?.addEventListener("click", async () => {
    const path = await api.pickRewriteModel();
    if (path) {
      state.cfg.rewrite_model_path = path;
      state.llmDownloaded = null;
      render();
    }
  });

  document.querySelector("#clear-rewrite-model-btn")?.addEventListener("click", () => {
    state.cfg.rewrite_model_path = null;
    state.llmDownloaded = null;
    render();
  });

  document.querySelector("#download-llm-btn")?.addEventListener("click", async () => {
    state.downloading = LLM_MODEL_FILE;
    state.downloadProgress = 0;
    render();
    try {
      await api.downloadLlm(state.cfg.model_dir, state.cfg.rewrite_model_path);
      state.llmDownloaded = true;
    } catch (e) {
      alert(`Download failed: ${e}`);
    } finally {
      state.downloading = null;
      render();
    }
  });
}

async function runTypingCheck() {
  try {
    await api.typistHealthCheck();
    state.typingCheck = { ok: true, message: "" };
  } catch (e) {
    state.typingCheck = { ok: false, message: String(e) };
  }
  render();
}

// ---------- utils ----------

function escapeHtml(s: string): string {
  const map: Record<string, string> = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
  return s.replace(/[&<>"']/g, (c) => map[c]!);
}
function escapeAttr(s: string): string {
  return escapeHtml(s);
}
function truncateMiddle(s: string, max: number): string {
  if (s.length <= max) return s;
  const half = Math.floor((max - 1) / 2);
  return `${s.slice(0, half)}…${s.slice(s.length - half)}`;
}

// ---------- boot ----------

async function boot() {
  state.cfg = await api.getConfig();
  state.view = state.cfg.setup_complete ? "settings" : "wizard";
  render();
  if (state.view === "wizard") {
    await onStepEnter(state.step);
  }

  await listen<string>("navigate", (event) => {
    if (event.payload === "settings") {
      state.view = "settings";
      state.savedNotice = false;
      render();
    }
  });

  await listen<EngineStatus>("engine-status", (event) => {
    state.engineStatus = event.payload;
    if (state.view === "settings") render();
  });

  await listen<DownloadProgress>("model-download-progress", (event) => {
    if (state.downloading === event.payload.file && event.payload.total > 0) {
      state.downloadProgress = Math.round((event.payload.downloaded / event.payload.total) * 100);
      render();
    }
  });
}

boot();
