export type HotkeyDescriptor =
  | { platform: "linux"; device_path: string; device_name: string; key_code: number; key_name: string }
  | { platform: "macos"; key_code: number; key_name: string };

export interface ModelConfig {
  name: string;
  path: string;
}

export interface AppConfig {
  hotkey: HotkeyDescriptor | null;
  audio_device: string | null;
  model: ModelConfig | null;
  model_dir: string | null;
  min_press_ms: number;
  typing_delay_ms: number;
  autostart: boolean;
  setup_complete: boolean;
}

export interface ModelInfo {
  name: string;
  file: string;
  size_label: string;
  description: string;
}

export interface DownloadProgress {
  file: string;
  downloaded: number;
  total: number;
}

export interface EngineStatus {
  state: "idle" | "recording" | "transcribing" | "error";
  message?: string;
}

export interface HardwareInfo {
  cpu_cores: number;
  total_ram_gb: number | null;
  gpu_name: string | null;
  gpu_backend: string | null;
  recommended_model: string;
}

export function defaultConfig(): AppConfig {
  return {
    hotkey: null,
    audio_device: null,
    model: null,
    model_dir: null,
    min_press_ms: 300,
    typing_delay_ms: 1,
    autostart: false,
    setup_complete: false,
  };
}

export function hotkeyLabel(hk: HotkeyDescriptor | null): string {
  if (!hk) return "Not set";
  if (hk.platform === "linux") return `${hk.key_name} (${hk.device_name})`;
  return hk.key_name;
}
