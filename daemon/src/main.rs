mod asr;
mod audio;
mod config;
mod ipc;
mod llm;
mod overlay;
mod prompts;
mod resample;

use clap::Parser;
use ipc::{CandidateItem, DaemonCommand, IpcResponse};
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
    llm_client: RwLock<llm::LlmClient>,
    config: RwLock<config::Config>,
    overlay: Arc<std::sync::Mutex<Option<overlay::OverlayHandle>>>,
    recording_start: Arc<Mutex<Option<std::time::Instant>>>,
    auto_stop_tx: Arc<Mutex<Option<tokio::sync::mpsc::Sender<()>>>>,
    mock_asr: bool,
    mock_llm: bool,
    llm_call_count: Arc<std::sync::Mutex<usize>>,
    overlay_renderer: Arc<std::sync::Mutex<String>>,
}

impl DaemonState {
    fn send_overlay(&self, cmd: overlay::OverlayCommand, use_overlay: bool) {
        if !use_overlay {
            return;
        }
        let mut guard = self.overlay.lock().unwrap();
        let alive = guard.as_ref().is_some_and(|ov| ov.is_alive());
        if !alive {
            let renderer_type = {
                let r = self.overlay_renderer.lock().unwrap();
                if r.starts_with("Vello") {
                    overlay::OverlayRendererType::Vello
                } else {
                    overlay::OverlayRendererType::Software
                }
            };
            info!("overlay: process dead, re-spawning (type={renderer_type:?})");
            *guard = overlay::try_spawn_overlay(renderer_type);
        }
        if let Some(ref ov) = *guard {
            ov.send(cmd);
        }
    }

    fn swap_overlay_renderer(&self, new_type: overlay::OverlayRendererType) -> bool {
        let guard = self.overlay.lock().unwrap();
        if let Some(ref old) = *guard {
            old.send(overlay::OverlayCommand::Quit);
        }
        drop(guard);

        std::thread::sleep(std::time::Duration::from_millis(100));

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
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install ring crypto provider");

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

    {
        let dir = prompts::ensure_prompts_dir();
        if let Ok(main) = prompts::load_main_prompt() {
            info!("Loaded main prompt ({}chars)", main.len());
        }
        if let Ok(advices) = prompts::load_correction_advices() {
            info!("Loaded {} correction advice(s)", advices.len());
        }
    }

    prompts::cleanup_orphan_overlays();

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

    let llm_client = llm::LlmClient::new(
        cfg.llm_api_key_resolved(),
        &cfg.llm_model,
        &cfg.llm_api_url,
        cfg.llm_timeout_secs,
    );

    let renderer_type = if cfg.overlay_renderer.starts_with("Vello") {
        overlay::OverlayRendererType::Vello
    } else {
        overlay::OverlayRendererType::Software
    };
    info!("overlay: selected renderer = {:?}", renderer_type);

    let overlay_handle = None;

    let mock_asr = args.mock_asr || std::env::var("GLM_ASR_MOCK").is_ok();
    if mock_asr {
        info!("mock ASR mode enabled - returning fake results");
    }
    let mock_llm = std::env::var("GLM_LLM_MOCK").is_ok();
    if mock_llm {
        info!("mock LLM mode enabled - cycling through test scenarios");
    }

    let state = Arc::new(DaemonState {
        audio: Arc::new(Mutex::new(None)),
        asr_client: RwLock::new(asr_client.clone()),
        llm_client: RwLock::new(llm_client),
        config: RwLock::new(cfg.clone()),
        overlay: Arc::new(std::sync::Mutex::new(overlay_handle)),
        recording_start: Arc::new(Mutex::new(None)),
        auto_stop_tx: Arc::new(Mutex::new(None)),
        mock_asr,
        mock_llm,
        llm_call_count: Arc::new(std::sync::Mutex::new(0)),
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
                    cfg.enable_llm = params.enable_llm;
                    cfg.llm_api_key = params.llm_api_key.clone();
                    cfg.llm_api_url = params.llm_api_url.clone();
                    cfg.llm_model = params.llm_model.clone();
                    cfg.llm_timeout_secs = params.llm_timeout_secs;
                    cfg.llm_thinking_mode = params.llm_thinking_mode;
                    cfg.llm_max_tokens = params.llm_max_tokens;
                    if params.reset_llm_prompts {
                        cfg.reset_prompts();
                    }
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

                let cfg = state.config.read().await;
                let new_llm = llm::LlmClient::new(
                    cfg.llm_api_key_resolved(),
                    &cfg.llm_model,
                    &cfg.llm_api_url,
                    cfg.llm_timeout_secs,
                );
                *state.llm_client.write().await = new_llm;

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

                info!("Config updated from plugin (enable_llm={}, llm_model={}, reset_prompts={})", 
                    params.enable_llm, params.llm_model, params.reset_llm_prompts);
                let _ = reply.try_send(IpcResponse::status(false));
            }

            DaemonCommand::StartRecord { reply, use_overlay } => {
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

                state.send_overlay(overlay::OverlayCommand::Show, use_overlay);

                let audio = state.audio.clone();
                let overlay = state.overlay.clone();
                let recording_start = state.recording_start.clone();
                let (auto_tx, mut auto_rx) = tokio::sync::mpsc::channel::<()>(1);
                *state.auto_stop_tx.lock().await = Some(auto_tx.clone());

                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tokio::time::sleep(std::time::Duration::from_millis(FEEDBACK_INTERVAL_MS)) => {
                                let audio_guard = audio.lock().await;
                                if let Some(ref capture) = *audio_guard {
                                    let vol = capture.current_volume();
                                    let waveform = capture.current_waveform(32);
                                    if use_overlay {
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

            DaemonCommand::StopRecord { reply, use_overlay } => {
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
                    let llm_client = state.llm_client.read().await.clone();
                    let (enable_llm, thinking_mode, max_tokens) = {
                        let cfg = state.config.read().await;
                        (cfg.enable_llm, cfg.llm_thinking_mode, cfg.llm_max_tokens)
                    };
                    let system_prompt = match prompts::merge_system_prompt() {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Failed to load prompts: {}", e);
                            String::new()
                        }
                    };
                    let mock_llm = state.mock_llm;
                    let llm_call_count = state.llm_call_count.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                        let mock_text = "这是二零二五年五月八号的模拟识别结果用于调试。";
                        if use_overlay {
                            if let Some(ref ov) = *overlay.lock().unwrap() {
                                ov.send(overlay::OverlayCommand::SetText(mock_text.to_string()));
                            }
                        }

                        let mut candidates = Vec::new();

                        if enable_llm && !system_prompt.is_empty() {
                            if use_overlay {
                                if let Some(ref ov) = *overlay.lock().unwrap() {
                                    ov.send(overlay::OverlayCommand::SetText("正在生成候选...".to_string()));
                                }
                            }
                            let llm_result = if mock_llm {
                                let idx = {
                                    let mut count = llm_call_count.lock().unwrap();
                                    let i = *count;
                                    *count += 1;
                                    i
                                };
                                let scenario = llm::MockLlmScenario::cycle(idx);
                                info!("mock LLM scenario #{}: {}", idx, scenario.label());
                                scenario.mock_generate(mock_text, 15).await
                            } else {
                                llm_client.generate_candidates(mock_text, &system_prompt, thinking_mode, max_tokens).await
                            };
                            match llm_result {
                                Ok(llm_cands) => {
                                    info!("LLM generated {} candidates", llm_cands.len());
                                    candidates.extend(llm_cands);
                                }
                                Err(e) => {
                                    warn!("LLM error: {}", e);
                                    candidates.push(CandidateItem {
                                        text: String::new(),
                                        source: Some(format!("error_{}", e)),
                                        confidence: None,
                                    });
                                }
                            }
                        }

                        candidates.push(CandidateItem {
                            text: mock_text.to_string(),
                            source: Some("asr".to_string()),
                            confidence: None,
                        });

                        let _ = reply.try_send(IpcResponse::result_with_candidates(candidates));
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                        if use_overlay {
                            if let Some(ref ov) = *overlay.lock().unwrap() {
                                ov.send(overlay::OverlayCommand::Hide);
                            }
                        }
                    });
                } else {
                    let client = state.asr_client.read().await.clone();
                    let overlay = state.overlay.clone();
                    let llm_client = state.llm_client.read().await.clone();
                    let (enable_llm, thinking_mode, max_tokens) = {
                        let cfg = state.config.read().await;
                        (cfg.enable_llm, cfg.llm_thinking_mode, cfg.llm_max_tokens)
                    };
                    let system_prompt = match prompts::merge_system_prompt() {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("Failed to load prompts: {}", e);
                            String::new()
                        }
                    };
                    let mock_llm = state.mock_llm;
                    let llm_call_count = state.llm_call_count.clone();
                    tokio::spawn(async move {
                        match client.recognize(wav_data).await {
                            Ok(text) => {
                                if use_overlay {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        if text.is_empty() {
                                            ov.send(overlay::OverlayCommand::SetError("未识别到语音内容".to_string()));
                                        } else {
                                            ov.send(overlay::OverlayCommand::SetText(text.clone()));
                                        }
                                    }
                                }

                                let mut candidates = Vec::new();

                                if enable_llm && !text.is_empty() && !system_prompt.is_empty() {
                                    if use_overlay {
                                        if let Some(ref ov) = *overlay.lock().unwrap() {
                                            ov.send(overlay::OverlayCommand::SetText("正在生成候选...".to_string()));
                                        }
                                    }
                                    let llm_result = if mock_llm {
                                        let idx = {
                                            let mut count = llm_call_count.lock().unwrap();
                                            let i = *count;
                                            *count += 1;
                                            i
                                        };
                                        let scenario = llm::MockLlmScenario::cycle(idx);
                                        info!("mock LLM scenario #{}: {}", idx, scenario.label());
                                        scenario.mock_generate(&text, 15).await
                                    } else {
                                        llm_client.generate_candidates(&text, &system_prompt, thinking_mode, max_tokens).await
                                    };
                                    match llm_result {
                                        Ok(llm_cands) => {
                                            info!("LLM generated {} candidates", llm_cands.len());
                                            candidates.extend(llm_cands);
                                        }
                                        Err(e) => {
                                            warn!("LLM error: {}", e);
                                            candidates.push(CandidateItem {
                                                text: String::new(),
                                                source: Some(format!("error_{}", e)),
                                                confidence: None,
                                            });
                                        }
                                    }
                                }

                                candidates.push(CandidateItem {
                                    text: text.clone(),
                                    source: Some("asr".to_string()),
                                    confidence: None,
                                });

                                let _ = reply.try_send(IpcResponse::result_with_candidates(candidates));
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                if use_overlay {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        ov.send(overlay::OverlayCommand::Hide);
                                    }
                                }
                            }
                            Err(e) => {
                                if use_overlay {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        ov.send(overlay::OverlayCommand::SetError(format!("识别失败: {}", e)));
                                    }
                                }
                                let _ = reply.try_send(IpcResponse::error(&format!("ASR failed: {}", e)));
                                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                                if use_overlay {
                                    if let Some(ref ov) = *overlay.lock().unwrap() {
                                        ov.send(overlay::OverlayCommand::Hide);
                                    }
                                }
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
