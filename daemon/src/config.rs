use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_api_key")]
    pub api_key: String,

    #[serde(default = "default_model")]
    pub model: String,

    #[serde(default = "default_api_url")]
    pub api_url: String,

    #[serde(default = "default_sample_rate")]
    pub sample_rate: u32,

    #[serde(default)]
    pub hotwords: Vec<String>,

    #[serde(default)]
    pub hotword_manager_url: Option<String>,

    #[serde(default = "default_socket_path_suffix")]
    pub socket_path: String,

    #[serde(default = "default_overlay_renderer")]
    pub overlay_renderer: String,

    #[serde(default)]
    pub enable_llm: bool,

    #[serde(default = "default_llm_api_url")]
    pub llm_api_url: String,

    #[serde(default = "default_llm_model")]
    pub llm_model: String,

    #[serde(default = "default_llm_api_key")]
    pub llm_api_key: String,

    #[serde(default = "default_llm_timeout_secs")]
    pub llm_timeout_secs: u64,

    #[serde(default)]
    pub llm_thinking_mode: bool,

    #[serde(default = "default_llm_max_tokens")]
    pub llm_max_tokens: u32,
}

fn default_api_key() -> String {
    String::new()
}

fn default_model() -> String {
    "glm-asr-2512".to_string()
}

fn default_api_url() -> String {
    "https://open.bigmodel.cn/api/paas/v4/audio/transcriptions".to_string()
}

fn default_sample_rate() -> u32 {
    16000
}

fn default_socket_path_suffix() -> String {
    "glm-asrd.sock".to_string()
}

fn default_overlay_renderer() -> String {
    "Software".to_string()
}

fn default_llm_api_url() -> String {
    "https://open.bigmodel.cn/api/paas/v4".to_string()
}

fn default_llm_model() -> String {
    "glm-4-flash".to_string()
}

fn default_llm_api_key() -> String {
    String::new()
}

fn default_llm_timeout_secs() -> u64 {
    15
}

fn default_llm_max_tokens() -> u32 {
    2048
}

impl Default for Config {
    fn default() -> Self {
        Self {
            api_key: default_api_key(),
            model: default_model(),
            api_url: default_api_url(),
            sample_rate: default_sample_rate(),
            hotwords: vec![],
            hotword_manager_url: None,
            socket_path: default_socket_path_suffix(),
            overlay_renderer: default_overlay_renderer(),
            enable_llm: false,
            llm_api_url: default_llm_api_url(),
            llm_model: default_llm_model(),
            llm_api_key: default_llm_api_key(),
            llm_timeout_secs: default_llm_timeout_secs(),
            llm_thinking_mode: false,
            llm_max_tokens: default_llm_max_tokens(),
        }
    }
}

impl Config {
    pub fn config_dir() -> PathBuf {
        let dirs =
            directories::ProjectDirs::from("com", "glm-asr", "glm-asrd").unwrap_or_else(|| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
                directories::ProjectDirs::from_path(PathBuf::from(home))
                    .expect("failed to create project dirs")
            });
        dirs.config_dir().to_path_buf()
    }

    pub fn socket_path(&self) -> PathBuf {
        if self.socket_path.starts_with('/') {
            PathBuf::from(&self.socket_path)
        } else {
            let uid = unsafe { libc::getuid() };
            PathBuf::from(format!("/run/user/{}/{}", uid, self.socket_path))
        }
    }

    pub fn load() -> Self {
        let config_path = Self::config_dir().join("config.json");
        Self::load_from(&config_path)
    }

    pub fn load_from(path: &PathBuf) -> Self {
        if !path.exists() {
            let dir = path.parent().unwrap();
            let _ = fs::create_dir_all(dir);
            let default = Self::default();
            match serde_json::to_string_pretty(&default) {
                Ok(json) => {
                    if let Err(e) = fs::write(path, json) {
                        warn!("Failed to write default config: {}", e);
                    } else {
                        info!("Created default config at {:?}", path);
                    }
                }
                Err(e) => warn!("Failed to serialize default config: {}", e),
            }
            return default;
        }

        match fs::read_to_string(path) {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(config) => {
                    info!("Loaded config from {:?}", path);
                    config
                }
                Err(e) => {
                    warn!("Failed to parse config: {}, using defaults", e);
                    Self::default()
                }
            },
            Err(e) => {
                warn!("Failed to read config: {}, using defaults", e);
                Self::default()
            }
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.api_key.is_empty() {
            return Err(format!(
                "API key is empty. Please set it in {:?}",
                Self::config_dir().join("config.json")
            ));
        }
        Ok(())
    }

    pub fn save(&self) -> Result<(), String> {
        let config_path = Self::config_dir().join("config.json");
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&config_path, json).map_err(|e| format!("Failed to write config: {}", e))
    }

    pub fn reset_prompts(&mut self) {
        match crate::prompts::reset_all_prompts() {
            Ok(_) => info!("LLM prompts reset to defaults"),
            Err(e) => warn!("Failed to reset prompts: {}", e),
        }
    }

    pub fn llm_api_key_resolved(&self) -> &str {
        if self.llm_api_key.is_empty() {
            &self.api_key
        } else {
            &self.llm_api_key
        }
    }
}
