mod asr;
mod audio;
mod config;
mod ipc;
mod overlay;
mod resample;

use clap::Parser;
use ipc::{DaemonCommand, IpcResponse};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

const MAX_RECORD_SECS: u32 = 30;
const FEEDBACK_INTERVAL_MS: u64 = 33;

#[derive(Parser, Debug)]
#[command(name = "glm-asrd", about = "GLM ASR daemon for voice typing")]
struct Args {
    #[arg(short, long, help = "Path to config file")]
    config: Option<String>,

    #[arg(short, long, default_value = "info", help = "Log level")]
    log_level: String,

    #[arg(long, help = "Mock ASR mode: return fake results instead of calling API")]
    mock_asr: bool,
}

struct DaemonState {
    audio: Arc<Mutex<Option<audio::AudioCapture>>>,
    asr_client: RwLock<asr::AsrClient>,
    config: RwLock<config::Config>,
    overlay: Arc<std::sync::Mutex<Option<overlay::OverlayHandle>>>,
    recording_start: Arc<Mutex<Option<std::time::Instant>>>,
    auto_stop_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<()>>>>,
    mock_asr: bool,
    use_overlay: Arc<std::sync::Mutex<bool>>,
    overlay_renderer: Arc<std::sync::Mutex<String>>,
}

impl DaemonState {
    fn should_show_overlay(&self) -> bool {
        self.use_overlay.lock().map_or(true, |g| *g)
            && self.overlay.lock().map_or(false, |g| g.is_some())
    }

    fn send_overlay(&self, cmd: overlay::OverlayCommand) {
        if self.should_show_overlay() {
            if let Some(ref ov) = *self.overlay.lock().unwrap() {
                ov.send(cmd);
            }
        }
    }

    fn swap_overlay_renderer(&self, new_type: overlay::OverlayRendererType) -> bool {
        let guard = self.overlay.lock().unwrap();
        if let Some(ref old) = *guard {
            old.send(overlay::OverlayCommand::Quit);
        }
        drop(guard);

        std::thread::sleep(std::time::Duration::from_millis(50));

        let new_handle = overlay::try_spawn_overlay(new_type);
        let mut guard = self.overlay.lock().unwrap();
        match new_handle {
            Some(h) => {
                *guard = Some(h);
                info!("overlay: renderer swapped to {:?}", new_type);
                true
            }
            None => {
                warn!("overlay: failed to spawn new renderer, old overlay is gone");
                *guard = None;
                false
            }
        }
    }
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_new(format!("glm_asrd={}", args.log_level))
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("glm_asrd=info")),
        )
        .init();

    info!("glm-asrd starting...");

    let cfg = match &args.config {
        Some(path) => config::Config::load_from(&std::path::PathBuf::from(path)),
        None => config::Config::load(),
    };

    if let Err(e) = cfg.validate() {
        warn!("Config warning: {}", e);
    }

    let socket_path = cfg.socket_path();
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let asr_client = asr::AsrClient::new(
        &cfg.api_key,
        &cfg.model,
        &cfg.api_url,
        &cfg.hotwords,
    );

    let renderer_type = if cfg.overlay_renderer.starts_with("Vello") {
        overlay::OverlayRendererType::Vello
    } else {
        overlay::OverlayRendererType::Software
    };
    info!("overlay: selected renderer = {:?}", renderer_type);

    let overlay_handle = overlay::try_spawn_overlay(renderer_type);
    if overlay_handle.is_some() {
        info!("overlay: initialized successfully");
    } else {
        warn!("overlay: failed to initialize (no wlr-layer-shell support?), running without overlay");
    }

    let mock_asr = args.mock_asr || std::env::var("GLM_ASR_MOCK").is_ok();
    if mock_asr {
        info!("mock ASR mode enabled - returning fake results");
    }

    let state = Arc::new(DaemonState {
        audio: Arc::new(Mutex::new(None)),
        asr_client: RwLock::new(asr_client.clone()),
        config: RwLock::new(cfg.clone()),
        overlay: Arc::new(std::sync::Mutex::new(overlay_handle)),
        recording_start: Arc::new(Mutex::new(None)),
        auto_stop_tx: Arc::new(Mutex::new(None)),
        mock_asr,
        use_overlay: Arc::new(std::sync::Mutex::new(true)),
        overlay_renderer: Arc::new(std::sync::Mutex::new(cfg.overlay_renderer.clone())),
    });

    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel::<DaemonCommand>(32);

    let ipc_server = match ipc::IpcServer::bind(socket_path, cmd_tx) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to bind IPC socket: {}", e);
            std::process::exit(1);
        }
    };

    tokio::spawn(async move {
        ipc_server.run().await;
    });

    info!("glm-asrd ready, waiting for commands...");

    while let Some(cmd) = cmd_rx.recv().await {
        match cmd {
            DaemonCommand::SetConfig { params, reply } => {
                let renderer_changed = {
                    let old_renderer = state.overlay_renderer.lock().unwrap();
                    *old_renderer != params.overlay_renderer
                };

                {
                    let mut cfg = state.config.write().await;
                    cfg.api_key = params.api_key.clone();
                    cfg.model = params.model.clone();
                    cfg.api_url = params.api_url.clone();
                    cfg.sample_rate = params.sample_rate;
                    cfg.overlay_renderer = params.overlay_renderer.clone();
                    if let Err(e) = cfg.save() {
                        warn!("Failed to save config: {}", e);
                    }
                }
                let hotwords = state.config.read().await.hotwords.clone();
                let new_client = asr::AsrClient::new(
                    &params.api_key,
                    &params.model,
                    &params.api_url,
                    &hotwords,
                );
                *state.asr_client.write().await = new_client;
                if let Ok(mut uo) = state.use_overlay.lock() {
                    *uo = params.use_overlay;
                }
                if let Ok(mut or) = state.overlay_renderer.lock() {
                    *or = params.overlay_renderer.clone();
                }

                if renderer_changed {
                    let new_type = if params.overlay_renderer.starts_with("Vello") {
                        overlay::OverlayRendererType::Vello
                    } else {
                        overlay::OverlayRendererType::Software
                    };
                    let state_clone = state.clone();
                    tokio::task::spawn_blocking(move || {
                        state_clone.swap_overlay_renderer(new_type);
                    });
                }

                info!("Config updated from plugin (use_overlay={}, overlay_renderer={})", params.use_overlay, params.overlay_renderer);
                let _ = reply.try_send(IpcResponse::status(false));
            }

            DaemonCommand::StartRecord { reply } => {
                let audio_guard = state.audio.lock().await;
                if audio_guard.is_some() {
                    let _ = reply.try_send(IpcResponse::error("Already recording"));
                    continue;
                }
                drop(audio_guard);

                let mut capture = match audio::AudioCapture::new() {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = reply.try_send(IpcResponse::error(&format!("Audio init failed: {}", e)));
                        continue;
                    }
                };

                if let Err(e) = capture.start() {
                    let _ = reply.try_send(IpcResponse::error(&format!("Audio start failed: {}", e)));
                    continue;
                }

                *state.audio.lock().await = Some(capture);
                *state.recording_start.lock().await = Some(std::time::Instant::now());

                state.send_overlay(overlay::OverlayCommand::Show);

                let audio = state.audio.clone();
                let overlay = state.overlay.clone();
                let recording_start = state.recording_start.clone();
                let use_overlay = state.use_overlay.clone();
                let (auto_tx, mut auto_rx) = tokio::sync::mpsc::channel::<()>(1);
                *state.auto_stop_tx.lock().await = Some(auto_tx.clone());

                tokio::spawn(async move {
                    let should_ov = use_overlay.lock().map_or(true, |g| *g);
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_millis(FEEDBACK_INTERVAL_MS)) => {
                                let audio_guard = audio.lock().await;
                                if let Some(ref capture) = *audio_guard {
                                    let vol = capture.current_volume();
                                    let waveform = capture.current_waveform(32);
                                    if should_ov {
                                        if let Some(ref ov) = *overlay.lock().unwrap() {
                                            ov.send(overlay::OverlayCommand::SetVolume(vol));
                                            ov.send(overlay::OverlayCommand::SetWaveform(waveform));
                                        }
                                    }
                                    let start_guard = recording_start.lock().await;
                                    if let Some(start) = *start_guard {
                                        let elapsed_s = start.elapsed().as_secs_f32();
                                        let remaining = (MAX_RECORD_SECS as f32 - elapsed_s).max(0.0);
                                        if remaining <= 0.0 {
                                            drop(start_guard);
                                            drop(audio_guard);
                                            let _ = auto_tx.send(()).await;
                                            return;
                                        }
                                    }
                                } else {
                                    return;
                                }
                            }
                            _ = auto_rx.recv() => {
                                return;
                            }
                        }
                    }
                });

                let _ = reply.try_send(IpcResponse::status(true));
            }

            DaemonCommand::StopRecord { reply } => {
                if let Some(tx) = state.auto_stop_tx.lock().await.take() {
                    let _ = tx.send(()).await;
                }

                let mut audio_guard = state.audio.lock().await;
                let mut capture = match audio_guard.take() {
                    Some(c) => c,
                    None => {
                        let _ = reply.try_send(IpcResponse::error("Not recording"));
                        continue;
                    }
                };

                let samples = match capture.stop() {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = reply.try_send(IpcResponse::error(&format!("Audio stop failed: {}", e)));
                        continue;
                    }
                };
                drop(audio_guard);

                if samples.is_empty() {
                    let _ = reply.try_send(IpcResponse::result(""));
                    continue;
                }

                let src_rate = capture.source_sample_rate();
                let src_channels = capture.source_channels();
                let target_rate = state.config.read().await.sample_rate;

                info!(
                    "Processing {} samples ({}Hz, {}ch) → {}Hz mono S16LE",
                    samples.len(), src_rate, src_channels, target_rate
                );

                let pcm = resample::resample_to_mono_s16le(&samples, src_rate, src_channels, target_rate);

                let wav_data = match resample::samples_to_wav(&pcm, target_rate) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = reply.try_send(IpcResponse::error(&format!("WAV encode failed: {}", e)));
                        continue;
                    }
                };

                let _ = reply.try_send(IpcResponse::status(false));

                if state.mock_asr {
                    let overlay = state.overlay.clone();
                    let should_ov = state.should_show_overlay();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let mock_text = "这是一条模拟识别结果，用于调试overlay界面显示效果。";
                        if should_ov {
                            if let Some(ref ov) = *overlay.lock().unwrap() {
                                ov.send(overlay::OverlayCommand::SetText(mock_text.to_string()));
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        if should_ov {
                            if let Some(ref ov) = *overlay.lock().unwrap() {
                                ov.send(overlay::OverlayCommand::Hide);
                            }
                        }
                        let _ = reply.try_send(IpcResponse::result(mock_text));
                    });
                } else {
                    let client = state.asr_client.read().await.clone();
                    let overlay = state.overlay.clone();
                    let should_ov = state.should_show_overlay();
                    tokio::spawn(async move {
                        match client.recognize(wav_data).await {
                            Ok(text) => {
                                if should_ov {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        if text.is_empty() {
                                            ov.send(overlay::OverlayCommand::SetError("未识别到语音内容".to_string()));
                                        } else {
                                            ov.send(overlay::OverlayCommand::SetText(text.clone()));
                                        }
                                    }
                                }
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                if should_ov {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        ov.send(overlay::OverlayCommand::Hide);
                                    }
                                }
                                let _ = reply.try_send(IpcResponse::result(&text));
                            }
                            Err(e) => {
                                if should_ov {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        ov.send(overlay::OverlayCommand::SetError(format!("识别失败: {}", e)));
                                    }
                                }
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                if should_ov {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        ov.send(overlay::OverlayCommand::Hide);
                                    }
                                }
                                let _ = reply.try_send(IpcResponse::error(&format!("ASR failed: {}", e)));
                            }
                        }
                    });
                }
            }

            DaemonCommand::Ping { reply } => {
                let _ = reply.try_send(IpcResponse::status(false));
            }
        }
    }
}


