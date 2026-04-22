use std::collections::HashSet;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
    globals::GlobalData,
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::{
        wlr_layer::{
            Anchor, KeyboardInteractivity, Layer, LayerShell, LayerShellHandler, LayerSurface,
            LayerSurfaceConfigure,
        },
        WaylandSurface,
    },
    shm::{slot::SlotPool, Shm, ShmHandler},
};
use tracing::{error, info, warn};
use smithay_client_toolkit::reexports::protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self, ZwlrForeignToplevelManagerV1},
};
use wayland_client::{
    globals::{registry_queue_init, GlobalList},
    protocol::{wl_output, wl_shm, wl_surface::WlSurface},
    Connection, Dispatch, QueueHandle,
};

const OVERLAY_WIDTH: u32 = 480;
const OVERLAY_HEIGHT: u32 = 84;
const OVERLAY_MAX_WIDTH: u32 = 900;
const OVERLAY_BOTTOM_MARGIN: i32 = 84;
const CORNER_RADIUS: f32 = 14.0;
const BORDER_WIDTH: f32 = 5.0;

const MAX_RECORD_SECS: u32 = 30;

const COLOR_BG: [u8; 4] = [0x1A, 0x1A, 0x2E, 0xEE];
const COLOR_PROGRESS: [u8; 4] = [0x4C, 0xAF, 0x50, 0xFF];
const COLOR_PROGRESS_BG: [u8; 4] = [0x30, 0x30, 0x50, 0x80];
const COLOR_VOL: [u8; 4] = [0x66, 0xBB, 0x6A, 0xFF];
const COLOR_VOL_BG: [u8; 4] = [0x30, 0x30, 0x50, 0x80];
const COLOR_TEXT: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const COLOR_ERROR_BORDER: [u8; 4] = [0xEF, 0x53, 0x50, 0xFF];
const COLOR_ERROR_TEXT: [u8; 4] = [0xFF, 0xCD, 0xD2, 0xFF];

#[derive(Debug, Clone)]
pub enum OverlayCommand {
    Show,
    Hide,
    SetVolume(f32),
    SetText(String),
    SetError(String),
    Quit,
}

pub struct OverlayHandle {
    tx: std::sync::mpsc::Sender<OverlayCommand>,
    alive: Arc<AtomicBool>,
}

impl OverlayHandle {
    pub fn send(&self, cmd: OverlayCommand) {
        if self.alive.load(Ordering::Relaxed) {
            let _ = self.tx.send(cmd);
        }
    }

    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Relaxed)
    }
}

#[derive(Debug, Default, Clone)]
struct ToplevelInner {
    activated: bool,
    outputs: HashSet<wl_output::WlOutput>,
}

struct ForeignToplevelState {
    #[allow(dead_code)]
    manager: Option<ZwlrForeignToplevelManagerV1>,
    handles: Vec<(ZwlrForeignToplevelHandleV1, ToplevelInner)>,
    active_output: Option<wl_output::WlOutput>,
}

impl ForeignToplevelState {
    fn new(globals: &GlobalList, qh: &QueueHandle<OverlayState>) -> Self {
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

    fn update_active(&mut self) {
        self.active_output = self.handles.iter()
            .find(|(_, inner)| inner.activated)
            .and_then(|(_, inner)| inner.outputs.iter().next().cloned());
    }

    fn get_active_output(&self) -> Option<wl_output::WlOutput> {
        self.active_output.clone()
    }
}

struct OverlayState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    pool: SlotPool,
    width: u32,
    height: u32,
    need_redraw: bool,
    visible: bool,
    configured: bool,
    exit: bool,
    alive: Arc<AtomicBool>,
    cmd_rx: std::sync::mpsc::Receiver<OverlayCommand>,
    layer: LayerSurface,
    layer_output: Option<wl_output::WlOutput>,
    ft_state: ForeignToplevelState,
    volume: f32,
    countdown_start: Option<std::time::Instant>,
    is_error: bool,
    display_text: String,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl ProvidesRegistryState for OverlayState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl CompositorHandler for OverlayState {
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

impl ShmHandler for OverlayState {
    fn shm_state(&mut self) -> &mut Shm {
        &mut self.shm
    }
}

impl OutputHandler for OverlayState {
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
        self.width = NonZeroU32::new(configure.new_size.0).map_or(OVERLAY_WIDTH, NonZeroU32::get);
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
        [zwlr_foreign_toplevel_manager_v1::EVT_TOPLEVEL_OPCODE =>
            (ZwlrForeignToplevelHandleV1, ())]
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

delegate_compositor!(OverlayState);
delegate_output!(OverlayState);
delegate_shm!(OverlayState);
delegate_layer!(OverlayState);
delegate_registry!(OverlayState);

fn alpha_blend(canvas: &mut [u8], offset: usize, src: &[u8; 4]) {
    if src[3] == 0 { return; }
    if src[3] == 0xFF || canvas[offset + 3] == 0 {
        canvas[offset] = src[0];
        canvas[offset + 1] = src[1];
        canvas[offset + 2] = src[2];
        canvas[offset + 3] = src[3];
        return;
    }
    let sa = src[3] as u32;
    let da = canvas[offset + 3] as u32;
    let inv_sa = 255 - sa;
    let out_a = sa + da * inv_sa / 255;
    if out_a == 0 { return; }
    canvas[offset] = ((src[0] as u32 * sa * 255 + canvas[offset] as u32 * da * inv_sa) / (out_a * 255)) as u8;
    canvas[offset + 1] = ((src[1] as u32 * sa * 255 + canvas[offset + 1] as u32 * da * inv_sa) / (out_a * 255)) as u8;
    canvas[offset + 2] = ((src[2] as u32 * sa * 255 + canvas[offset + 2] as u32 * da * inv_sa) / (out_a * 255)) as u8;
    canvas[offset + 3] = out_a as u8;
}

fn blend_pixel(canvas: &mut [u8], offset: usize, color: &[u8; 4], coverage: f32) {
    if coverage <= 0.0 { return; }
    let a = ((color[3] as f32 * coverage.min(1.0)) + 0.5) as u8;
    alpha_blend(canvas, offset, &[color[0], color[1], color[2], a]);
}

fn sdf_rounded_rect(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let half_w = w * 0.5;
    let half_h = h * 0.5;
    let qx = (px - half_w).abs() - (half_w - r);
    let qy = (py - half_h).abs() - (half_h - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    inside + outside - r
}

fn sdf_coverage(sdf: f32) -> f32 {
    (0.5 - sdf).clamp(0.0, 1.0)
}

fn path_distance(px: f32, py: f32, w: f32, h: f32, r: f32) -> f32 {
    let seg_top = w - 2.0 * r;
    let seg_right = h - 2.0 * r;
    let corner_arc = std::f32::consts::PI * r * 0.5;

    let in_corner = (px < r && py < r) || (px >= w - r && py < r)
        || (px < r && py >= h - r) || (px >= w - r && py >= h - r);

    if !in_corner {
        if py < r && px >= r && px < w - r {
            return px - r;
        }
        if px >= w - r && py >= r && py < h - r {
            return seg_top + corner_arc + (py - r);
        }
        if py >= h - r && px >= r && px < w - r {
            return seg_top + corner_arc + seg_right + corner_arc + (w - r - px);
        }
        if px < r && py >= r && py < h - r {
            return seg_top + corner_arc + seg_right + corner_arc + seg_top + corner_arc + (h - r - py);
        }
        return 0.0;
    }

    let (cx, cy, base, angle_start) = match (px < r, py < r, px >= w - r, py >= h - r) {
        (_, _, true, true) => {
            (w - r, h - r, seg_top + corner_arc + seg_right, 0.0)
        }
        (_, _, true, _) => {
            (w - r, r, seg_top, std::f32::consts::PI * 1.5)
        }
        (true, _, _, true) => {
            (r, h - r,
             seg_top + corner_arc + seg_right + corner_arc + seg_top,
             std::f32::consts::PI * 0.5)
        }
        (true, true, _, _) => {
            (r, r,
             seg_top + corner_arc + seg_right + corner_arc + seg_top + corner_arc + seg_right,
             std::f32::consts::PI)
        }
        _ => { return 0.0; }
    };

    let dx = px - cx;
    let dy = py - cy;
    let mut angle = dy.atan2(dx);
    if angle < 0.0 { angle += 2.0 * std::f32::consts::PI; }

    let frac = ((angle - angle_start) / (std::f32::consts::PI * 0.5)).clamp(0.0, 1.0);
    base + frac * corner_arc
}

fn draw_aa_rect(canvas: &mut [u8], cw: usize, ch: usize,
                x0: f32, y0: f32, w: f32, h: f32, r: f32,
                color: &[u8; 4]) {
    let x_min = (x0 - 1.0).max(0.0) as usize;
    let y_min = (y0 - 1.0).max(0.0) as usize;
    let x_max = (x0 + w + 1.0).min(cw as f32) as usize;
    let y_max = (y0 + h + 1.0).min(ch as f32) as usize;

    for y in y_min..y_max {
        for x in x_min..x_max {
            let px = x as f32 + 0.5 - x0;
            let py = y as f32 + 0.5 - y0;
            let cov = sdf_coverage(sdf_rounded_rect(px, py, w, h, r));
            if cov > 0.0 {
                let offset = (y * cw + x) * 4;
                if offset + 3 < canvas.len() {
                    blend_pixel(canvas, offset, color, cov);
                }
            }
        }
    }
}

fn fill_rect_aa(canvas: &mut [u8], cw: usize, _ch: usize,
                x0: usize, y0: usize, w: usize, h: usize, color: [u8; 4]) {
    for y in y0..y0 + h {
        if y >= cw { break; }
        for x in x0..x0 + w {
            if x >= cw { break; }
            let offset = (y * cw + x) * 4;
            if offset + 3 < canvas.len() {
                alpha_blend(canvas, offset, &color);
            }
        }
    }
}
fn render_text(
    text: &str,
    canvas: &mut [u8],
    canvas_w: usize,
    canvas_h: usize,
    x_start: usize,
    y_center: usize,
    text_color: [u8; 4],
    font_system: &mut FontSystem,
    swash_cache: &mut SwashCache,
) {
    let font_size = 15.0;
    let line_height = 20.0;
    let metrics = Metrics::new(font_size, line_height);

    let attrs = Attrs::new()
        .family(Family::Name("Noto Sans CJK SC"))
        .family(Family::SansSerif);

    let mut buf = Buffer::new(font_system, metrics);
    buf.set_size(font_system, Some(canvas_w as f32 - x_start as f32 - 16.0), Some(canvas_h as f32));
    buf.set_text(font_system, text, &attrs, Shaping::Advanced);
    buf.shape_until_scroll(font_system, true);

    let layout_runs: Vec<_> = buf.layout_runs().collect();
    let total_height: f32 = layout_runs.iter().map(|r| r.line_height).sum();
    let mut y_offset = y_center as f32 - total_height / 2.0;

    for run in &layout_runs {
        for glyph in run.glyphs.iter() {
            let physical = glyph.physical((x_start as f32, y_offset), 1.0);

            let img_opt = swash_cache.get_image(font_system, physical.cache_key);
            if let Some(img) = img_opt {
                let gw = img.placement.width as usize;
                let gh = img.placement.height as usize;
                if gw == 0 || gh == 0 {
                    continue;
                }
                let x_base = physical.x + img.placement.left;
                let y_base = physical.y - img.placement.top as i32;

                match img.content {
                    SwashContent::Mask => {
                        let stride = gw;
                        for (row_idx, row) in img.data.chunks(stride).enumerate() {
                            let py = y_base + row_idx as i32;
                            if py < 0 || py >= canvas_h as i32 { continue; }
                            for (col_idx, &alpha) in row.iter().enumerate() {
                                if alpha == 0 { continue; }
                                let px = x_base + col_idx as i32;
                                if px < 0 || px >= canvas_w as i32 { continue; }
                                let dst = (py as usize * canvas_w + px as usize) * 4;
                                if dst + 3 >= canvas.len() { continue; }
                                let color = [text_color[0], text_color[1], text_color[2], alpha];
                                alpha_blend(canvas, dst, &color);
                            }
                        }
                    }
                    SwashContent::SubpixelMask => {
                        let stride = gw * 4;
                        for (row_idx, row) in img.data.chunks(stride).enumerate() {
                            if row.len() < stride { continue; }
                            let py = y_base + row_idx as i32;
                            if py < 0 || py >= canvas_h as i32 { continue; }
                            for col_idx in 0..gw {
                                let px = x_base + col_idx as i32;
                                if px < 0 || px >= canvas_w as i32 { continue; }
                                let src_off = col_idx * 4;
                                let r = row[src_off];
                                let g = row[src_off + 1];
                                let b = row[src_off + 2];
                                let a = row[src_off + 3];
                                if a == 0 && r == 0 && g == 0 && b == 0 { continue; }
                                let dst = (py as usize * canvas_w + px as usize) * 4;
                                if dst + 3 >= canvas.len() { continue; }
                                let color = [r, g, b, a.max(text_color[3])];
                                alpha_blend(canvas, dst, &color);
                            }
                        }
                    }
                    SwashContent::Color => {
                        let stride = gw * 4;
                        for (row_idx, row) in img.data.chunks(stride).enumerate() {
                            if row.len() < stride { continue; }
                            let py = y_base + row_idx as i32;
                            if py < 0 || py >= canvas_h as i32 { continue; }
                            for col_idx in 0..gw {
                                let px = x_base + col_idx as i32;
                                if px < 0 || px >= canvas_w as i32 { continue; }
                                let src_off = col_idx * 4;
                                let color = [row[src_off], row[src_off + 1], row[src_off + 2], row[src_off + 3]];
                                if color[3] == 0 { continue; }
                                let dst = (py as usize * canvas_w + px as usize) * 4;
                                if dst + 3 >= canvas.len() { continue; }
                                alpha_blend(canvas, dst, &color);
                            }
                        }
                    }
                }
            }
        }
        y_offset += run.line_height;
    }
}

fn measure_text_width(text: &str, font_system: &mut FontSystem) -> f32 {
    let font_size = 15.0;
    let line_height = 20.0;
    let metrics = Metrics::new(font_size, line_height);
    let attrs = Attrs::new()
        .family(Family::Name("Noto Sans CJK SC"))
        .family(Family::SansSerif);
    let mut buf = Buffer::new(font_system, metrics);
    buf.set_size(font_system, Some(8000.0), Some(100.0));
    buf.set_text(font_system, text, &attrs, Shaping::Advanced);
    buf.shape_until_scroll(font_system, true);
    buf.layout_runs().next().map_or(200.0, |run| {
        if let Some(last) = run.glyphs.last() {
            last.x + last.w as f32
        } else {
            200.0
        }
    })
}

impl OverlayState {
    fn create_layer(&mut self, qh: &QueueHandle<Self>, output: Option<&wl_output::WlOutput>) {
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            Layer::Top,
            Some("glm-asr-overlay"),
            output,
        );

        layer.set_anchor(Anchor::BOTTOM);
        layer.set_size(self.width, OVERLAY_HEIGHT);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        layer.set_exclusive_zone(0);
        layer.set_margin(OVERLAY_BOTTOM_MARGIN, 0, OVERLAY_BOTTOM_MARGIN, 0);
        layer.commit();

        self.layer = layer;
        self.layer_output = output.cloned();
        self.configured = false;
    }

    fn ensure_output(&mut self, qh: &QueueHandle<Self>) {
        let target = self.ft_state.get_active_output();

        let target = match target {
            Some(o) => Some(o),
            None => self.query_kwin_active_output(),
        };

        let layer_output_info = self.layer_output.as_ref().and_then(|o| {
            self.output_state.info(o).and_then(|i| i.name.clone())
        });
        let target_output_info = target.as_ref().and_then(|o| {
            self.output_state.info(o).and_then(|i| i.name.clone())
        });
        info!("overlay: ensure_output layer={:?} target={:?} handles={}",
              layer_output_info, target_output_info, self.ft_state.handles.len());

        let need_recreate = match (&self.layer_output, &target) {
            (Some(cur), Some(tgt)) => cur != tgt,
            (None, None) => false,
            _ => true,
        };

        if need_recreate {
            info!("overlay: recreating layer on output {:?}", target_output_info);
            self.create_layer(qh, target.as_ref());
        }
    }

    fn query_kwin_active_output(&self) -> Option<wl_output::WlOutput> {
        let output = std::process::Command::new("qdbus6")
            .args(["org.kde.KWin", "/KWin", "org.kde.KWin.activeOutputName"])
            .output()
            .ok()
            .or_else(|| {
                std::process::Command::new("qdbus")
                    .args(["org.kde.KWin", "/KWin", "org.kde.KWin.activeOutputName"])
                    .output()
                    .ok()
            })?;
        if !output.status.success() {
            return None;
        }
        let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if name.is_empty() {
            return None;
        }
        for output in self.output_state.outputs() {
            if let Some(info) = self.output_state.info(&output) {
                if info.name.as_deref() == Some(name.as_str()) {
                    info!("overlay: kwin active output = {:?}", info.name);
                    return Some(output);
                }
            }
        }
        warn!("overlay: kwin output '{}' not found in wayland outputs", name);
        None
    }

    fn resize_for_text(&mut self, text: &str) -> bool {
        let text_w = measure_text_width(text, &mut self.font_system);
        let content_pad = (BORDER_WIDTH + 10.0) * 2.0 + 20.0;
        let target_w = (text_w + content_pad).max(OVERLAY_WIDTH as f32).min(OVERLAY_MAX_WIDTH as f32) as u32;
        if target_w != self.width {
            self.width = target_w;
            self.layer.set_size(target_w, OVERLAY_HEIGHT);
            self.layer.commit();
            true
        } else {
            false
        }
    }

    fn reset_size(&mut self) {
        if self.width != OVERLAY_WIDTH {
            self.width = OVERLAY_WIDTH;
            self.layer.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);
            self.layer.commit();
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }

        let width = self.width as usize;
        let height = self.height as usize;
        let stride = self.width as i32 * 4;

        let (buffer, mut canvas) = match self.pool.create_buffer(
            self.width as i32,
            self.height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(b) => b,
            Err(e) => {
                warn!("overlay: create_buffer failed: {e}");
                return;
            }
        };

        for chunk in canvas.chunks_exact_mut(4) {
            chunk[0] = 0;
            chunk[1] = 0;
            chunk[2] = 0;
            chunk[3] = 0;
        }

        if self.visible {
            let is_recording = self.countdown_start.is_some();
            let w = width as f32;
            let h = height as f32;
            let bw = BORDER_WIDTH;
            let r_outer = CORNER_RADIUS;
            let r_inner = (r_outer - bw).max(0.0);
            let inner_w = (w - 2.0 * bw).max(1.0);
            let inner_h = (h - 2.0 * bw).max(1.0);

            let countdown_frac = if let Some(start) = self.countdown_start {
                let elapsed = start.elapsed().as_secs_f32();
                let remaining = (MAX_RECORD_SECS as f32 - elapsed).max(0.0);
                remaining / MAX_RECORD_SECS as f32
            } else {
                1.0
            };

            let remaining_secs = if let Some(start) = self.countdown_start {
                let elapsed = start.elapsed().as_secs_f32();
                (MAX_RECORD_SECS as f32 - elapsed).max(0.0)
            } else {
                MAX_RECORD_SECS as f32
            };

            let seg_top = w - 2.0 * r_outer;
            let seg_right = h - 2.0 * r_outer;
            let corner_arc = std::f32::consts::PI * r_outer * 0.5;
            let perimeter = 2.0 * seg_top + 2.0 * seg_right + 4.0 * corner_arc;
            let active_len = countdown_frac * perimeter;

            let margin = 2;
            for y in margin..height - margin {
                for x in margin..width - margin {
                    let px = x as f32 + 0.5;
                    let py = y as f32 + 0.5;

                    let outer_sdf = sdf_rounded_rect(px, py, w, h, r_outer);
                    let outer_cov = sdf_coverage(outer_sdf);

                    if outer_cov <= 0.0 {
                        continue;
                    }

                    let inner_sdf = sdf_rounded_rect(
                        px - bw, py - bw, inner_w, inner_h, r_inner,
                    );
                    let inner_cov = sdf_coverage(inner_sdf);

                    let border_cov = (outer_cov - inner_cov).max(0.0);

                    if border_cov > 0.0 {
                        let offset = (y * width + x) * 4;
                        if offset + 3 >= canvas.len() { continue; }

                        if is_recording {
                            let d = path_distance(px, py, w, h, r_outer);
                            let soft_edge = 2.0;
                            let t = ((active_len - d) / soft_edge + 0.5).clamp(0.0, 1.0);
                            if t >= 1.0 {
                                blend_pixel(canvas, offset, &COLOR_PROGRESS, border_cov);
                            } else if t <= 0.0 {
                                blend_pixel(canvas, offset, &COLOR_PROGRESS_BG, border_cov);
                            } else {
                                let r = (COLOR_PROGRESS_BG[0] as f32 * (1.0 - t) + COLOR_PROGRESS[0] as f32 * t) as u8;
                                let g = (COLOR_PROGRESS_BG[1] as f32 * (1.0 - t) + COLOR_PROGRESS[1] as f32 * t) as u8;
                                let b = (COLOR_PROGRESS_BG[2] as f32 * (1.0 - t) + COLOR_PROGRESS[2] as f32 * t) as u8;
                                let a = (COLOR_PROGRESS_BG[3] as f32 * (1.0 - t) + COLOR_PROGRESS[3] as f32 * t) as u8;
                                blend_pixel(canvas, offset, &[r, g, b, a], border_cov);
                            }
                        } else {
                            let border_color = if self.is_error {
                                COLOR_ERROR_BORDER
                            } else {
                                COLOR_PROGRESS
                            };
                            blend_pixel(canvas, offset, &border_color, border_cov);
                        }
                    }

                    if inner_cov > 0.0 {
                        let offset = (y * width + x) * 4;
                        if offset + 3 < canvas.len() {
                            blend_pixel(canvas, offset, &COLOR_BG, inner_cov);
                        }
                    }
                }
            }

            let text_color = if self.is_error { COLOR_ERROR_TEXT } else { COLOR_TEXT };
            let content_pad = (BORDER_WIDTH + 10.0) as usize;
            let text_y = height / 2 - 4;
            let status_text = if is_recording {
                format!("🎙  录音中 {}s", remaining_secs as u32)
            } else {
                self.display_text.clone()
            };
            render_text(
                &status_text,
                &mut canvas, width, height,
                content_pad, text_y,
                text_color,
                &mut self.font_system, &mut self.swash_cache,
            );

            if is_recording {
                let vol_bar_h = 6usize;
                let vol_bar_y = height - content_pad - vol_bar_h;
                let vol_bar_max_w = width - content_pad * 2;
                let vol_w = ((self.volume.min(1.0).max(0.0) * vol_bar_max_w as f32) as usize).min(vol_bar_max_w);

                fill_rect_aa(&mut canvas, width, height,
                             content_pad, vol_bar_y, vol_bar_max_w, vol_bar_h, COLOR_VOL_BG);
                if vol_w > 0 {
                    fill_rect_aa(&mut canvas, width, height,
                                 content_pad, vol_bar_y, vol_w, vol_bar_h, COLOR_VOL);
                }
            }
        }

        self.layer.wl_surface().damage_buffer(0, 0, self.width as i32, self.height as i32);
        buffer.attach_to(self.layer.wl_surface()).expect("buffer attach");
        self.layer.commit();

        self.need_redraw = false;

        if self.visible && self.countdown_start.is_some() {
            self.layer.wl_surface().frame(qh, self.layer.wl_surface().clone());
        }
    }

    fn process_commands(&mut self, qh: &QueueHandle<Self>) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                OverlayCommand::Show => {
                    self.visible = true;
                    self.is_error = false;
                    self.display_text.clear();
                    self.countdown_start = Some(std::time::Instant::now());
                    self.ensure_output(qh);
                    if self.configured && self.width == OVERLAY_WIDTH {
                        self.need_redraw = true;
                    } else {
                        self.reset_size();
                    }
                }
                OverlayCommand::Hide => {
                    self.visible = false;
                    self.volume = 0.0;
                    self.countdown_start = None;
                    self.is_error = false;
                    self.display_text.clear();
                    self.need_redraw = true;
                    self.draw(qh);
                }
                OverlayCommand::SetVolume(vol) => {
                    self.volume = vol;
                    self.need_redraw = true;
                }
                OverlayCommand::SetText(text) => {
                    self.display_text = text;
                    self.is_error = false;
                    self.countdown_start = None;
                    let txt = self.display_text.clone();
                    if !self.resize_for_text(&txt) {
                        self.need_redraw = true;
                    }
                }
                OverlayCommand::SetError(text) => {
                    self.display_text = text;
                    self.is_error = true;
                    self.countdown_start = None;
                    let txt = self.display_text.clone();
                    if !self.resize_for_text(&txt) {
                        self.need_redraw = true;
                    }
                }
                OverlayCommand::Quit => {
                    self.exit = true;
                }
            }
        }
    }
}

pub fn try_spawn_overlay() -> Option<OverlayHandle> {
    let (tx, rx) = std::sync::mpsc::channel::<OverlayCommand>();
    let alive = Arc::new(AtomicBool::new(true));
    let alive_clone = alive.clone();

    std::thread::Builder::new()
        .name("wayland-overlay".into())
        .spawn(move || {
            if let Err(e) = run_overlay(rx, alive_clone) {
                error!("overlay thread exited with error: {e}");
            }
        })
        .ok()?;

    Some(OverlayHandle { tx, alive })
}

fn run_overlay(
    cmd_rx: std::sync::mpsc::Receiver<OverlayCommand>,
    alive: Arc<AtomicBool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = registry_queue_init::<OverlayState>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let ft_state = ForeignToplevelState::new(&globals, &qh);

    let surface = compositor.create_surface(&qh);

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some("glm-asr-overlay"),
        None::<&wl_output::WlOutput>,
    );

    layer.set_anchor(Anchor::BOTTOM);
    layer.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(0);
    layer.set_margin(OVERLAY_BOTTOM_MARGIN, 0, OVERLAY_BOTTOM_MARGIN, 0);
    layer.commit();

    let pool = SlotPool::new(OVERLAY_MAX_WIDTH as usize * OVERLAY_HEIGHT as usize * 4, &shm)?;

    let font_system = FontSystem::new();
    let swash_cache = SwashCache::new();

    let mut state = OverlayState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        pool,
        width: OVERLAY_WIDTH,
        height: OVERLAY_HEIGHT,
        need_redraw: false,
        visible: false,
        configured: false,
        exit: false,
        alive,
        cmd_rx,
        layer,
        layer_output: None,
        ft_state,
        volume: 0.0,
        countdown_start: None,
        is_error: false,
        display_text: String::new(),
        font_system,
        swash_cache,
    };

    info!("overlay: wayland layer-shell initialized ({OVERLAY_WIDTH}x{OVERLAY_HEIGHT}, margin bottom {OVERLAY_BOTTOM_MARGIN})");

    use std::os::unix::io::{AsFd, AsRawFd};

    loop {
        if let Some(guard) = event_queue.prepare_read() {
            let _ = guard.read();
        }

        event_queue.dispatch_pending(&mut state)?;

        state.process_commands(&qh);

        if state.exit {
            break;
        }

        if state.visible && state.countdown_start.is_some() && state.display_text.is_empty() {
            if let Some(start) = state.countdown_start {
                if start.elapsed().as_secs_f32() >= MAX_RECORD_SECS as f32 {
                    state.visible = false;
                    state.volume = 0.0;
                    state.countdown_start = None;
                    state.display_text.clear();
                    state.need_redraw = true;
                }
            }
            state.need_redraw = true;
        }

        if state.need_redraw {
            state.draw(&qh);
        }

        let _ = event_queue.flush();

        let fd = conn.as_fd().as_raw_fd();
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let ret = unsafe { libc::poll(&mut pfd, 1, 33) };
        if ret < 0 {
            break;
        }
    }

    state.alive.store(false, Ordering::Relaxed);
    info!("overlay: thread exiting");
    Ok(())
}
