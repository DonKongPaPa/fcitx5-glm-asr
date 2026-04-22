use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tracing::{debug, error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcMessage {
    pub cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IpcResponse {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recording: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl IpcResponse {
    pub fn status(recording: bool) -> Self {
        Self {
            msg_type: "status".to_string(),
            recording: Some(recording),
            text: None,
            message: None,
        }
    }

    pub fn result(text: &str) -> Self {
        Self {
            msg_type: "result".to_string(),
            recording: None,
            text: Some(text.to_string()),
            message: None,
        }
    }

    pub fn error(msg: &str) -> Self {
        Self {
            msg_type: "error".to_string(),
            recording: None,
            text: None,
            message: Some(msg.to_string()),
        }
    }
}

pub struct SetConfigParams {
    pub api_key: String,
    pub model: String,
    pub api_url: String,
    pub sample_rate: u32,
    pub use_overlay: bool,
}

pub enum DaemonCommand {
    StartRecord { reply: mpsc::Sender<IpcResponse> },
    StopRecord { reply: mpsc::Sender<IpcResponse> },
    Ping { reply: mpsc::Sender<IpcResponse> },
    SetConfig { params: SetConfigParams, reply: mpsc::Sender<IpcResponse> },
}

pub struct IpcServer {
    listener: UnixListener,
    cmd_tx: mpsc::Sender<DaemonCommand>,
}

impl IpcServer {
    pub fn bind(socket_path: PathBuf, cmd_tx: mpsc::Sender<DaemonCommand>) -> Result<Self, Box<dyn std::error::Error>> {
        if socket_path.exists() {
            std::fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)?;
        info!("IPC server listening on {:?}", socket_path);
        Ok(Self { listener, cmd_tx })
    }

    pub async fn run(self) {
        loop {
            match self.listener.accept().await {
                Ok((stream, _addr)) => {
                    let tx = self.cmd_tx.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_client(stream, tx).await {
                            debug!("Client handler error: {}", e);
                        }
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }
}

async fn handle_client(
    stream: UnixStream,
    cmd_tx: mpsc::Sender<DaemonCommand>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = IpcResponse::error(&format!("Invalid JSON: {}", e));
                let resp_str = serde_json::to_string(&resp)?;
                writer.write_all(format!("{}\n", resp_str).as_bytes()).await?;
                continue;
            }
        };

        let cmd = msg.get("cmd").and_then(|v| v.as_str()).unwrap_or("");

        let (reply_tx, mut reply_rx) = mpsc::channel::<IpcResponse>(4);

        match cmd {
            "start_record" => {
                cmd_tx.send(DaemonCommand::StartRecord { reply: reply_tx }).await?;
            }
            "stop_record" => {
                cmd_tx.send(DaemonCommand::StopRecord { reply: reply_tx }).await?;
            }
            "ping" => {
                cmd_tx.send(DaemonCommand::Ping { reply: reply_tx }).await?;
            }
            "set_config" => {
                let params = SetConfigParams {
                    api_key: msg.get("api_key").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    model: msg.get("model").and_then(|v| v.as_str()).unwrap_or("glm-asr-2512").to_string(),
                    api_url: msg.get("api_url").and_then(|v| v.as_str())
                        .unwrap_or("https://open.bigmodel.cn/api/paas/v4/audio/transcriptions")
                        .to_string(),
                    sample_rate: msg.get("sample_rate").and_then(|v| v.as_u64()).unwrap_or(16000) as u32,
                    use_overlay: msg.get("use_overlay").and_then(|v| v.as_bool()).unwrap_or(true),
                };
                cmd_tx.send(DaemonCommand::SetConfig { params, reply: reply_tx }).await?;
            }
            _ => {
                let resp = IpcResponse::error(&format!("Unknown command: {}", cmd));
                let resp_str = serde_json::to_string(&resp)?;
                writer.write_all(format!("{}\n", resp_str).as_bytes()).await?;
                continue;
            }
        }

        while let Some(resp) = reply_rx.recv().await {
            let resp_str = serde_json::to_string(&resp)?;
            writer.write_all(format!("{}\n", resp_str).as_bytes()).await?;
        }
    }

    Ok(())
}
