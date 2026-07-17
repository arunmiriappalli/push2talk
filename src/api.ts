import { invoke } from "@tauri-apps/api/core";
import type { AppConfig, HardwareInfo, HotkeyDescriptor, ModelInfo } from "./types";

export const api = {
  listAudioDevices: () => invoke<string[]>("list_audio_devices"),
  captureHotkey: () => invoke<HotkeyDescriptor>("capture_hotkey"),
  listModels: () => invoke<ModelInfo[]>("list_models"),
  detectHardware: () => invoke<HardwareInfo>("detect_hardware"),
  isModelDownloaded: (file: string, modelDir: string | null) =>
    invoke<boolean>("is_model_downloaded", { file, modelDir }),
  downloadModel: (file: string, modelDir: string | null) =>
    invoke<string>("download_model", { file, modelDir }),
  typistHealthCheck: () => invoke<null>("typist_health_check"),
  getConfig: () => invoke<AppConfig>("get_config"),
  saveConfig: (cfg: AppConfig) => invoke<null>("save_config", { cfg }),
  pickModelDir: () => invoke<string | null>("pick_model_dir"),
  getAutostart: () => invoke<boolean>("get_autostart"),
  setAutostart: (enabled: boolean) => invoke<null>("set_autostart", { enabled }),
};
