use std::any::Any;

use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use tracing::warn;
use wayland_client::QueueHandle;

use super::super::OverlayState;
use super::{DrawState, OverlayRenderer};

pub struct VelloRenderer {
    width: u32,
    height: u32,
}

impl VelloRenderer {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }
}

impl OverlayRenderer for VelloRenderer {
    fn draw(
        &mut self,
        state: &DrawState,
        _layer: &LayerSurface,
        _qh: &QueueHandle<OverlayState>,
    ) {
        warn!("VelloRenderer::draw not yet implemented, visible={}", state.visible);
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn width(&self) -> u32 {
        self.width
    }

    fn height(&self) -> u32 {
        self.height
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
