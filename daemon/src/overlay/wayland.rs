use std::collections::HashSet;
use std::num::NonZeroU32;

use smithay_client_toolkit::{
    globals::GlobalData,
    output::OutputState,
    shell::wlr_layer::{LayerShellHandler, LayerSurface, LayerSurfaceConfigure},
};
use tracing::{info, warn};
use smithay_client_toolkit::reexports::protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_client::{
    globals::GlobalList,
    protocol::{wl_output, wl_surface::WlSurface},
    Connection, Dispatch, QueueHandle,
};

use super::{OverlayState, OVERLAY_HEIGHT};

#[derive(Debug, Default, Clone)]
pub struct ToplevelInner {
    pub activated: bool,
    pub outputs: HashSet<wl_output::WlOutput>,
}

pub struct ForeignToplevelState {
    #[allow(dead_code)]
    pub manager: Option<ZwlrForeignToplevelManagerV1>,
    pub handles: Vec<(ZwlrForeignToplevelHandleV1, ToplevelInner)>,
    active_output: Option<wl_output::WlOutput>,
}

impl ForeignToplevelState {
    pub fn new(globals: &GlobalList, qh: &QueueHandle<OverlayState>) -> Self {
        let manager: Option<ZwlrForeignToplevelManagerV1> =
            globals.bind(qh, 1..=3, GlobalData).ok();
        if manager.is_some() {
            info!("overlay: zwlr_foreign_toplevel_manager bound");
        } else {
            warn!("overlay: failed to bind zwlr_foreign_toplevel_manager");
        }
        Self {
            manager,
            handles: Vec::new(),
            active_output: None,
        }
    }

    pub fn update_active(&mut self) {
        self.active_output = self.handles.iter()
            .find(|(_, inner)| inner.activated)
            .and_then(|(_, inner)| inner.outputs.iter().next().cloned());
    }

    pub fn get_active_output(&self) -> Option<wl_output::WlOutput> {
        self.active_output.clone()
    }
}

impl smithay_client_toolkit::compositor::CompositorHandler for OverlayState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
        self.need_redraw = true;
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl smithay_client_toolkit::shm::ShmHandler for OverlayState {
    fn shm_state(&mut self) -> &mut smithay_client_toolkit::shm::Shm {
        &mut self.shm
    }
}

impl smithay_client_toolkit::output::OutputHandler for OverlayState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for OverlayState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        info!("overlay: layer surface closed");
        self.visible = false;
        self.configured = false;
        self.need_redraw = false;
        self.exit = true;
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = NonZeroU32::new(configure.new_size.0).map_or(480, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(OVERLAY_HEIGHT, NonZeroU32::get);
        self.configured = true;
        self.need_redraw = true;
        if self.visible {
            self.draw(qh);
        }
    }
}

impl Dispatch<ZwlrForeignToplevelManagerV1, GlobalData, OverlayState> for OverlayState {
    fn event(
        state: &mut OverlayState,
        _proxy: &ZwlrForeignToplevelManagerV1,
        event: zwlr_foreign_toplevel_manager_v1::Event,
        _: &GlobalData,
        _conn: &Connection,
        _qh: &QueueHandle<OverlayState>,
    ) {
        match event {
            zwlr_foreign_toplevel_manager_v1::Event::Toplevel { .. } => {}
            zwlr_foreign_toplevel_manager_v1::Event::Finished => {
                state.ft_state.manager = None;
            }
            _ => {}
        }
    }

    wayland_client::event_created_child!(
        OverlayState,
        ZwlrForeignToplevelManagerV1,
        [
            zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE =>
                (ZwlrForeignToplevelHandleV1, ())
        ]
    );
}

impl Dispatch<ZwlrForeignToplevelHandleV1, (), OverlayState> for OverlayState {
    fn event(
        state: &mut OverlayState,
        handle: &ZwlrForeignToplevelHandleV1,
        event: zwlr_foreign_toplevel_handle_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<OverlayState>,
    ) {
        match event {
            zwlr_foreign_toplevel_handle_v1::Event::Title { .. } => {}
            zwlr_foreign_toplevel_handle_v1::Event::AppId { .. } => {}
            zwlr_foreign_toplevel_handle_v1::Event::OutputEnter { output } => {
                if let Some((_, inner)) = state.ft_state.handles.iter_mut().find(|(h, _)| h == handle) {
                    inner.outputs.insert(output);
                } else {
                    let mut inner = ToplevelInner::default();
                    inner.outputs.insert(output);
                    state.ft_state.handles.push((handle.clone(), inner));
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::OutputLeave { output } => {
                if let Some((_, inner)) = state.ft_state.handles.iter_mut().find(|(h, _)| h == handle) {
                    inner.outputs.retain(|o| o != &output);
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::State { state: state_arr } => {
                let activated = state_arr.chunks_exact(4).any(|chunk| {
                    u32::from_ne_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) == 2
                });
                if let Some((_, inner)) = state.ft_state.handles.iter_mut().find(|(h, _)| h == handle) {
                    inner.activated = activated;
                } else {
                    let mut inner = ToplevelInner::default();
                    inner.activated = activated;
                    state.ft_state.handles.push((handle.clone(), inner));
                }
            }
            zwlr_foreign_toplevel_handle_v1::Event::Done => {
                let activated = state.ft_state.handles.iter()
                    .any(|(_, inner)| inner.activated);
                let output_count = state.ft_state.handles.iter()
                    .filter(|(_, inner)| inner.activated)
                    .flat_map(|(_, inner)| inner.outputs.iter())
                    .count();
                info!("overlay: toplevel done, activated={}, output_count={}", activated, output_count);
                state.ft_state.update_active();
            }
            zwlr_foreign_toplevel_handle_v1::Event::Closed => {
                state.ft_state.handles.retain(|(h, _)| h != handle);
                handle.destroy();
                state.ft_state.update_active();
            }
            zwlr_foreign_toplevel_handle_v1::Event::Parent { .. } => {}
            _ => {}
        }
    }
}

smithay_client_toolkit::delegate_compositor!(OverlayState);
smithay_client_toolkit::delegate_output!(OverlayState);
smithay_client_toolkit::delegate_shm!(OverlayState);
smithay_client_toolkit::delegate_layer!(OverlayState);
smithay_client_toolkit::delegate_registry!(OverlayState);
