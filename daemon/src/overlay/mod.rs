pub mod renderer;
mod wayland;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use renderer::{DrawState, OverlayRenderer};
use renderer::software::SoftwareRenderer;
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::{ProvidesRegistryState, RegistryState},
    registry_handlers,
    shell::wlr_layer::{LayerShell, LayerSurface},
    shell::WaylandSurface,
    shm::{slot::SlotPool, Shm},
};
use tracing::{error, info, warn};
use wayland_client::{
    protocol::wl_output,
    Connection, QueueHandle,
};

use wayland::ForeignToplevelState;

pub(crate) const OVERLAY_WIDTH: u32 = 480;
pub(crate) const OVERLAY_HEIGHT: u32 = 84;
pub(crate) const OVERLAY_MAX_WIDTH: u32 = 900;
const OVERLAY_BOTTOM_MARGIN: i32 = 84;
pub(crate) const BORDER_WIDTH: f32 = 5.0;
pub(crate) const CORNER_RADIUS: f32 = 14.0;
pub(crate) const MAX_RECORD_SECS: u32 = 30;
const FADE_DURATION: f32 = 0.3;

#[derive(Debug, Clone)]
pub enum OverlayCommand {
    Show,
    Hide,
    SetVolume(f32),
    SetWaveform(Vec<f32>),
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

struct OverlayState {
    registry_state: RegistryState,
    output_state: OutputState,
    compositor: CompositorState,
    layer_shell: LayerShell,
    shm: Shm,
    width: u32,
    height: u32,
    scale_factor: i32,
    need_redraw: bool,
    pending_reset_size: bool,
    visible: bool,
    configured: bool,
    exit: bool,
    alive: Arc<AtomicBool>,
    cmd_rx: std::sync::mpsc::Receiver<OverlayCommand>,
    layer: LayerSurface,
    layer_output: Option<wl_output::WlOutput>,
    ft_state: ForeignToplevelState,
    volume: f32,
    waveform: Vec<f32>,
    countdown_start: Option<std::time::Instant>,
    is_error: bool,
    display_text: String,
    fade_alpha: f32,
    fade_target: f32,
    fade_start: Option<std::time::Instant>,
    renderer: Box<dyn OverlayRenderer>,
}

impl ProvidesRegistryState for OverlayState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

impl OverlayState {
    fn create_layer(&mut self, qh: &QueueHandle<Self>, output: Option<&wl_output::WlOutput>) {
        let surface = self.compositor.create_surface(qh);
        let layer = self.layer_shell.create_layer_surface(
            qh,
            surface,
            smithay_client_toolkit::shell::wlr_layer::Layer::Top,
            Some("glm-asr-overlay"),
            output,
        );

        layer.set_anchor(smithay_client_toolkit::shell::wlr_layer::Anchor::BOTTOM);
        layer.set_size(self.width, OVERLAY_HEIGHT);
        layer.set_keyboard_interactivity(
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None,
        );
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
        let text_w = if let Some(sw) = self.renderer.as_any_mut().downcast_mut::<renderer::software::SoftwareRenderer>() {
            sw.measure_text_width(text)
        } else {
            #[cfg(feature = "vello-renderer")]
            {
                if let Some(vr) = self.renderer.as_any_mut().downcast_mut::<renderer::vello::VelloRenderer>() {
                    vr.measure_text_width(text)
                } else {
                    200.0
                }
            }
            #[cfg(not(feature = "vello-renderer"))]
            { 200.0 }
        };
        let content_pad = (BORDER_WIDTH + 10.0) * 2.0 + 20.0;
        let target_w = (text_w + content_pad).max(OVERLAY_WIDTH as f32).min(OVERLAY_MAX_WIDTH as f32) as u32;
        if target_w != self.width {
            self.width = target_w;
            let phys_w = target_w * self.scale_factor as u32;
            let phys_h = OVERLAY_HEIGHT * self.scale_factor as u32;
            self.renderer.resize(phys_w, phys_h, self.scale_factor);
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
            let phys_w = OVERLAY_WIDTH * self.scale_factor as u32;
            let phys_h = OVERLAY_HEIGHT * self.scale_factor as u32;
            self.renderer.resize(phys_w, phys_h, self.scale_factor);
            self.layer.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);
            self.layer.commit();
        }
    }

    fn draw(&mut self, qh: &QueueHandle<Self>) {
        if !self.configured || self.width == 0 || self.height == 0 {
            return;
        }

        if let Some(start) = self.fade_start {
            let elapsed = start.elapsed().as_secs_f32();
            let t = (elapsed / FADE_DURATION).min(1.0);
            let smooth = t * t * (3.0 - 2.0 * t);
            self.fade_alpha = if self.fade_target > 0.5 { smooth } else { 1.0 - smooth };
            if t >= 1.0 {
                self.fade_start = None;
                self.fade_alpha = self.fade_target;
                if self.fade_target < 0.5 {
                    self.visible = false;
                    self.volume = 0.0;
                    self.countdown_start = None;
                    self.is_error = false;
                    self.display_text.clear();
                    self.waveform.clear();
                    self.pending_reset_size = true;
                }
            }
            self.need_redraw = true;
        }

        let countdown_frac = if let Some(start) = self.countdown_start {
            let elapsed = start.elapsed().as_secs_f32();
            (MAX_RECORD_SECS as f32 - elapsed).max(0.0) / MAX_RECORD_SECS as f32
        } else {
            1.0
        };

        let remaining_secs = if let Some(start) = self.countdown_start {
            let elapsed = start.elapsed().as_secs_f32();
            (MAX_RECORD_SECS as f32 - elapsed).max(0.0)
        } else {
            MAX_RECORD_SECS as f32
        };

        let draw_state = DrawState {
            visible: self.visible,
            is_recording: self.countdown_start.is_some(),
            countdown_frac,
            remaining_secs,
            volume: self.volume,
            display_text: &self.display_text,
            is_error: self.is_error,
            fade_alpha: self.fade_alpha,
            waveform: &self.waveform,
            scale_factor: self.scale_factor as f32,
        };

        self.layer.wl_surface().set_buffer_scale(self.scale_factor);
        self.renderer.draw(&draw_state, &self.layer, qh);
        self.need_redraw = false;

        if self.visible && (self.countdown_start.is_some() || self.fade_start.is_some()) {
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
                    self.waveform.clear();
                    self.countdown_start = Some(std::time::Instant::now());
                    self.fade_alpha = 0.0;
                    self.fade_target = 1.0;
                    self.fade_start = Some(std::time::Instant::now());
                    self.ensure_output(qh);
                    if self.configured && self.width == OVERLAY_WIDTH {
                        self.need_redraw = true;
                    } else {
                        self.reset_size();
                    }
                }
                OverlayCommand::Hide => {
                    self.fade_target = 0.0;
                    self.fade_start = Some(std::time::Instant::now());
                    self.countdown_start = None;
                    if self.fade_alpha <= 0.0 {
                        self.visible = false;
                        self.volume = 0.0;
                        self.is_error = false;
                        self.display_text.clear();
                        self.waveform.clear();
                        self.need_redraw = true;
                        self.draw(qh);
                    } else {
                        self.need_redraw = true;
                    }
                }
                OverlayCommand::SetVolume(vol) => {
                    self.volume = vol;
                    self.need_redraw = true;
                }
                OverlayCommand::SetWaveform(samples) => {
                    self.waveform = samples;
                    self.need_redraw = true;
                }
                OverlayCommand::SetText(text) => {
                    self.display_text = text;
                    self.is_error = false;
                    self.countdown_start = None;
                    self.waveform.clear();
                    let txt = self.display_text.clone();
                    if !self.resize_for_text(&txt) {
                        self.need_redraw = true;
                    }
                }
                OverlayCommand::SetError(text) => {
                    self.display_text = text;
                    self.is_error = true;
                    self.countdown_start = None;
                    self.waveform.clear();
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

#[derive(Debug, Clone, Copy)]
pub enum OverlayRendererType {
    Software,
    Vello,
}

pub fn try_spawn_overlay(renderer_type: OverlayRendererType) -> Option<OverlayHandle> {
    let (tx, rx) = std::sync::mpsc::channel::<OverlayCommand>();
    let alive = Arc::new(AtomicBool::new(true));
    let alive_clone = alive.clone();

    std::thread::Builder::new()
        .name("wayland-overlay".into())
        .spawn(move || {
            if let Err(e) = run_overlay(rx, alive_clone, renderer_type) {
                error!("overlay thread exited with error: {e}");
            }
        })
        .ok()?;

    Some(OverlayHandle { tx, alive })
}

fn run_overlay(
    cmd_rx: std::sync::mpsc::Receiver<OverlayCommand>,
    alive: Arc<AtomicBool>,
    renderer_type: OverlayRendererType,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let conn = Connection::connect_to_env()?;
    let (globals, mut event_queue) = wayland_client::globals::registry_queue_init::<OverlayState>(&conn)?;
    let qh = event_queue.handle();

    let compositor = CompositorState::bind(&globals, &qh)?;
    let shm = Shm::bind(&globals, &qh)?;
    let layer_shell = LayerShell::bind(&globals, &qh)?;
    let ft_state = ForeignToplevelState::new(&globals, &qh);

    let surface = compositor.create_surface(&qh);

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        smithay_client_toolkit::shell::wlr_layer::Layer::Top,
        Some("glm-asr-overlay"),
        None::<&wl_output::WlOutput>,
    );

    layer.set_anchor(smithay_client_toolkit::shell::wlr_layer::Anchor::BOTTOM);
    layer.set_size(OVERLAY_WIDTH, OVERLAY_HEIGHT);
    layer.set_keyboard_interactivity(
        smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None,
    );
    layer.set_exclusive_zone(0);
    layer.set_margin(OVERLAY_BOTTOM_MARGIN, 0, OVERLAY_BOTTOM_MARGIN, 0);
    layer.commit();

    let pool = SlotPool::new(OVERLAY_MAX_WIDTH as usize * 2 * OVERLAY_HEIGHT as usize * 2 * 4, &shm)?;

    let renderer: Box<dyn OverlayRenderer> = match renderer_type {
        OverlayRendererType::Vello => {
            #[cfg(feature = "vello-renderer")]
            {
                match renderer::vello::VelloRenderer::new(pool, OVERLAY_WIDTH, OVERLAY_HEIGHT) {
                    Ok(r) => {
                        info!("overlay: using Vello renderer (experimental)");
                        Box::new(r)
                    }
                    Err(e) => {
                        warn!("overlay: Vello init failed ({e}), falling back to software");
                        let fallback_pool = SlotPool::new(
                            OVERLAY_MAX_WIDTH as usize * 2 * OVERLAY_HEIGHT as usize * 2 * 4, &shm,
                        )?;
                        Box::new(renderer::software::SoftwareRenderer::new(
                            fallback_pool, OVERLAY_WIDTH, OVERLAY_HEIGHT,
                        ))
                    }
                }
            }
            #[cfg(not(feature = "vello-renderer"))]
            {
                warn!("overlay: Vello renderer requested but not compiled, falling back to software");
                Box::new(renderer::software::SoftwareRenderer::new(pool, OVERLAY_WIDTH, OVERLAY_HEIGHT))
            }
        }
        OverlayRendererType::Software => {
            Box::new(renderer::software::SoftwareRenderer::new(pool, OVERLAY_WIDTH, OVERLAY_HEIGHT))
        }
    };

    let mut state = OverlayState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        compositor,
        layer_shell,
        shm,
        width: OVERLAY_WIDTH,
        height: OVERLAY_HEIGHT,
        scale_factor: 1,
        need_redraw: false,
        pending_reset_size: false,
        visible: false,
        configured: false,
        exit: false,
        alive,
        cmd_rx,
        layer,
        layer_output: None,
        ft_state,
        volume: 0.0,
        waveform: Vec::new(),
        countdown_start: None,
        is_error: false,
        display_text: String::new(),
        fade_alpha: 0.0,
        fade_target: 0.0,
        fade_start: None,
        renderer,
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

        if state.fade_start.is_some() {
            state.need_redraw = true;
        }

        if state.need_redraw {
            state.draw(&qh);
        }

        if state.pending_reset_size {
            state.pending_reset_size = false;
            state.reset_size();
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
