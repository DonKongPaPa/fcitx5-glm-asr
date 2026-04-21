use std::num::NonZeroU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, delegate_shm,
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
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_buffer::WlBuffer, wl_output, wl_shm, wl_surface::WlSurface},
    Connection, QueueHandle,
};

use tracing::{debug, error, info, warn};

#[derive(Debug, Clone)]
pub enum OverlayCommand {
    Show,
    Hide,
    SetText(String),
    SetVolume(f32),
    SetCountdown(u32),
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
    shm: Shm,
    pool: SlotPool,
    width: u32,
    height: u32,
    need_redraw: bool,
    visible: bool,
    exit: bool,
    alive: Arc<AtomicBool>,
    cmd_rx: std::sync::mpsc::Receiver<OverlayCommand>,
    layer: LayerSurface,
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
        self.width = NonZeroU32::new(configure.new_size.0).map_or(400, NonZeroU32::get);
        self.height = NonZeroU32::new(configure.new_size.1).map_or(48, NonZeroU32::get);
        self.need_redraw = true;
        self.draw(qh);
    }
}

delegate_compositor!(OverlayState);
delegate_output!(OverlayState);
delegate_shm!(OverlayState);
delegate_layer!(OverlayState);
delegate_registry!(OverlayState);

impl OverlayState {
    fn draw(&mut self, qh: &QueueHandle<Self>) {
        if !self.visible || self.width == 0 || self.height == 0 {
            return;
        }

        let width = self.width;
        let height = self.height;
        let stride = width as i32 * 4;

        let (buffer, canvas) = match self.pool.create_buffer(
            width as i32,
            height as i32,
            stride,
            wl_shm::Format::Argb8888,
        ) {
            Ok(b) => b,
            Err(e) => {
                warn!("overlay: create_buffer failed: {e}");
                return;
            }
        };

        let bg: u32 = 0xD9_1A_1A_2E;
        for chunk in canvas.chunks_exact_mut(4) {
            let bytes = bg.to_le_bytes();
            chunk[0] = bytes[0];
            chunk[1] = bytes[1];
            chunk[2] = bytes[2];
            chunk[3] = bytes[3];
        }

        self.layer.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
        self.layer.wl_surface().frame(qh, self.layer.wl_surface().clone());

        buffer.attach_to(self.layer.wl_surface()).expect("buffer attach");
        self.layer.commit();

        self.need_redraw = false;
    }

    fn process_commands(&mut self, qh: &QueueHandle<Self>) {
        while let Ok(cmd) = self.cmd_rx.try_recv() {
            match cmd {
                OverlayCommand::Show => {
                    if !self.visible {
                        self.visible = true;
                        self.need_redraw = true;
                        info!("overlay: received Show command, drawing...");
                        self.draw(qh);
                    }
                }
                OverlayCommand::Hide => {
                    if self.visible {
                        self.visible = false;
                        self.layer.wl_surface().attach(None::<&WlBuffer>, 0, 0);
                        self.layer.commit();
                        info!("overlay: received Hide command");
                    }
                }
                OverlayCommand::SetText(_) => {
                    self.need_redraw = true;
                    self.draw(qh);
                }
                OverlayCommand::SetVolume(_) => {
                    self.need_redraw = true;
                    self.draw(qh);
                }
                OverlayCommand::SetCountdown(_) => {
                    self.need_redraw = true;
                    self.draw(qh);
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

    let surface = compositor.create_surface(&qh);

    let layer = layer_shell.create_layer_surface(
        &qh,
        surface,
        Layer::Top,
        Some("glm-asr-overlay"),
        None::<&wl_output::WlOutput>,
    );

    layer.set_anchor(Anchor::BOTTOM);
    layer.set_size(400, 48);
    layer.set_keyboard_interactivity(KeyboardInteractivity::None);
    layer.set_exclusive_zone(0);
    layer.set_margin(8, 0, 8, 0);
    layer.commit();

    let pool = SlotPool::new(400 * 48 * 4, &shm)?;

    let mut state = OverlayState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        shm,
        pool,
        width: 400,
        height: 48,
        need_redraw: false,
        visible: false,
        exit: false,
        alive,
        cmd_rx,
        layer,
    };

    info!("overlay: wayland layer-shell initialized");

    loop {
        event_queue.blocking_dispatch(&mut state)?;
        state.process_commands(&qh);

        if state.exit {
            break;
        }

        if state.need_redraw {
            state.draw(&qh);
        }
    }

    state.alive.store(false, Ordering::Relaxed);
    info!("overlay: thread exiting");
    Ok(())
}
