use crate::config::AppConfig;
use crate::hotkey::{self, HotkeyEvent};
use crate::{audio, transcribe, typist};
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

enum EngineMsg {
    Hotkey(HotkeyEvent),
    Reload(AppConfig),
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineStatus {
    Idle,
    Recording,
    Transcribing,
    Error { message: String },
}

#[derive(Clone)]
pub struct EngineHandle {
    tx: mpsc::Sender<EngineMsg>,
}

impl EngineHandle {
    pub fn reload(&self, config: AppConfig) {
        let _ = self.tx.send(EngineMsg::Reload(config));
    }
}

struct EngineState {
    app: AppHandle,
    config: AppConfig,
    transcriber: Option<transcribe::Transcriber>,
    typist: Option<Box<dyn typist::Typist>>,
    recording: Option<audio::Recording>,
    record_start: Option<Instant>,
    generation: Arc<AtomicU64>,
    self_tx: mpsc::Sender<EngineMsg>,
}

pub fn spawn(app: AppHandle) -> EngineHandle {
    let (tx, rx) = mpsc::channel::<EngineMsg>();
    let handle = EngineHandle { tx: tx.clone() };

    std::thread::spawn(move || {
        let mut state = EngineState {
            app,
            config: AppConfig::default(),
            transcriber: None,
            typist: None,
            recording: None,
            record_start: None,
            generation: Arc::new(AtomicU64::new(0)),
            self_tx: tx,
        };

        let initial = crate::config::load();
        state.apply_config(initial);

        for msg in rx {
            match msg {
                EngineMsg::Reload(cfg) => state.apply_config(cfg),
                EngineMsg::Hotkey(HotkeyEvent::Pressed) => state.on_press(),
                EngineMsg::Hotkey(HotkeyEvent::Released) => state.on_release(),
            }
        }
    });

    handle
}

impl EngineState {
    fn emit_status(&self, status: EngineStatus) {
        let _ = self.app.emit("engine-status", status);
    }

    fn apply_config(&mut self, config: AppConfig) {
        let model_changed =
            self.config.model.as_ref().map(|m| &m.path) != config.model.as_ref().map(|m| &m.path);
        let hotkey_changed = self.config.hotkey != config.hotkey;

        self.config = config.clone();

        // Respawn the (cheap, near-instant) hotkey listener before the
        // (slow, can take 10s+ for large models) transcriber reload, so a
        // hotkey press isn't stuck queued behind a model load that's blocking
        // this single-threaded engine loop.
        if hotkey_changed {
            if let Some(descriptor) = config.hotkey.clone() {
                let my_generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
                let tx = self.self_tx.clone();
                let generation = self.generation.clone();
                hotkey::listen_hotkey(
                    descriptor,
                    move |event| {
                        let _ = tx.send(EngineMsg::Hotkey(event));
                    },
                    generation,
                    my_generation,
                );
            }
        }

        if model_changed {
            if let Some(model) = &config.model {
                let path = std::path::Path::new(&model.path);
                match transcribe::Transcriber::load(path, num_cpus()) {
                    Ok(t) => self.transcriber = Some(t),
                    Err(e) => {
                        log::error!("push2talk: failed to load model: {e}");
                        self.emit_status(EngineStatus::Error { message: e });
                    }
                }
            }
        }

        if self.typist.is_none() {
            match typist::new() {
                Ok(t) => self.typist = Some(t),
                Err(e) => {
                    log::error!("push2talk: failed to init typist: {e}");
                    self.emit_status(EngineStatus::Error { message: e });
                }
            }
        }

        self.emit_status(EngineStatus::Idle);
    }

    fn on_press(&mut self) {
        if self.recording.is_some() {
            return;
        }
        if self.transcriber.is_none() || self.typist.is_none() {
            return;
        }
        match audio::start(self.config.audio_device.as_deref()) {
            Ok(recording) => {
                self.recording = Some(recording);
                self.record_start = Some(Instant::now());
                self.emit_status(EngineStatus::Recording);
            }
            Err(e) => {
                log::error!("push2talk: failed to start recording: {e}");
                self.emit_status(EngineStatus::Error { message: e });
            }
        }
    }

    fn on_release(&mut self) {
        let Some(recording) = self.recording.take() else {
            return;
        };
        let elapsed_ms = self
            .record_start
            .take()
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);

        let samples = audio::stop(recording);
        self.emit_status(EngineStatus::Idle);

        if elapsed_ms < self.config.min_press_ms {
            return;
        }

        let Some(transcriber) = &self.transcriber else {
            return;
        };
        self.emit_status(EngineStatus::Transcribing);

        let text = match transcriber.transcribe(&samples) {
            Ok(t) => t,
            Err(e) => {
                log::error!("push2talk: transcription failed: {e}");
                self.emit_status(EngineStatus::Error { message: e });
                return;
            }
        };

        self.emit_status(EngineStatus::Idle);

        if text.is_empty() {
            return;
        }

        if let Some(typist) = &mut self.typist {
            if let Err(e) = typist.type_text(&text, self.config.typing_delay_ms) {
                log::error!("push2talk: typing failed: {e}");
                self.emit_status(EngineStatus::Error { message: e });
            }
        }
    }
}

fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
}
