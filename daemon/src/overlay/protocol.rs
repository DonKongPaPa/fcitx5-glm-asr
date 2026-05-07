use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OverlayCommand {
    Show,
    Hide,
    SetVolume(f32),
    SetWaveform(Vec<f32>),
    SetText(String),
    SetError(String),
    Quit,
}
