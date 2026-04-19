mod asr;
mod audio;
mod config;
mod ipc;
mod resample;

use clap::Parser;
use ipc::{DaemonCommand, IpcResponse};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tracing::{error, info, warn};

#[derive(Parser, Debug)]
#[command(name = "glm-asrd", about = "GLM ASR daemon for voice typing")]
struct Args {
    #[arg(short, long, help = "Path to config file")]
    config: Option<String>,

    #[arg(short, long, default_value = "info", help = "Log level")]
    log_level: String,
}

struct DaemonState {
    audio: Arc<Mutex<Option<audio::AudioCapture>>>,
    asr_client: RwLock<asr::AsrClient>,
    config: RwLock<config::Config>,
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

    let state = Arc::new(DaemonState {
        audio: Arc::new(Mutex::new(None)),
        asr_client: RwLock::new(asr_client),
        config: RwLock::new(cfg.clone()),
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
                {
                    let mut cfg = state.config.write().await;
                    cfg.api_key = params.api_key.clone();
                    cfg.model = params.model.clone();
                    cfg.api_url = params.api_url.clone();
                    cfg.sample_rate = params.sample_rate;
                }
                let hotwords = state.config.read().await.hotwords.clone();
                let new_client = asr::AsrClient::new(
                    &params.api_key,
                    &params.model,
                    &params.api_url,
                    &hotwords,
                );
                *state.asr_client.write().await = new_client;
                info!("Config updated from plugin");
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
                let _ = reply.try_send(IpcResponse::status(true));
            }

            DaemonCommand::StopRecord { reply } => {
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

                let client = state.asr_client.read().await.clone();
                tokio::spawn(async move {
                    match client.recognize(wav_data).await {
                        Ok(text) => {
                            let _ = reply.try_send(IpcResponse::result(&text));
                        }
                        Err(e) => {
                            let _ = reply.try_send(IpcResponse::error(&format!("ASR failed: {}", e)));
                        }
                    }
                });
            }

            DaemonCommand::Ping { reply } => {
                let _ = reply.try_send(IpcResponse::status(false));
            }
        }
    }
}


