use std::any::Any;

use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use wayland_client::QueueHandle;

use super::OverlayState;

pub mod software;

#[cfg(feature = "vello-renderer")]
pub mod vello;

pub struct DrawState<'a> {
    pub visible: bool,
    pub is_recording: bool,
    pub countdown_frac: f32,
    pub remaining_secs: f32,
    pub volume: f32,
    pub display_text: &'a str,
    pub is_error: bool,
    pub fade_alpha: f32,
    pub waveform: &'a [f32],
}

pub trait OverlayRenderer: Send {
    fn draw(
        &mut self,
        state: &DrawState,
        layer: &LayerSurface,
        qh: &QueueHandle<OverlayState>,
    );

    fn resize(&mut self, width: u32, height: u32, scale_factor: i32);

    fn as_any_mut(&mut self) -> &mut dyn Any;
}
