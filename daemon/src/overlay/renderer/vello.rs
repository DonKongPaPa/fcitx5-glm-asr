use std::any::Any;
use std::num::NonZeroUsize;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::{slot::SlotPool, Shm};
use tracing::{info, warn};
use vello::kurbo::{Affine, CircleSegment, RoundedRect, Stroke};
use vello::peniko::{Color, Fill};
use vello::util::{block_on_wgpu, RenderContext};
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloRendererInner, RendererOptions, Scene};
use wayland_client::protocol::wl_shm;
use wayland_client::QueueHandle;

use super::super::{BORDER_WIDTH, CORNER_RADIUS, MAX_RECORD_SECS, OverlayState};
use super::{DrawState, OverlayRenderer};

const COLOR_BG: Color = Color::from_rgba8(0x1A, 0x1A, 0x2E, 0xEE);
const COLOR_PROGRESS: Color = Color::from_rgba8(0x4C, 0xAF, 0x50, 0xFF);
const COLOR_PROGRESS_BG: Color = Color::from_rgba8(0x30, 0x30, 0x50, 0x80);
const COLOR_ERROR_BORDER: Color = Color::from_rgba8(0xEF, 0x53, 0x50, 0xFF);
const COLOR_VOL: (u8, u8, u8, u8) = (0x66, 0xBB, 0x6A, 0xFF);
const COLOR_VOL_BG: (u8, u8, u8, u8) = (0x30, 0x30, 0x50, 0x80);
const COLOR_TEXT: (u8, u8, u8, u8) = (0xFF, 0xFF, 0xFF, 0xFF);
const COLOR_ERROR_TEXT: (u8, u8, u8, u8) = (0xFF, 0xCD, 0xD2, 0xFF);

pub struct VelloRenderer {
    pool: SlotPool,
    width: u32,
    height: u32,
    context: RenderContext,
    device_id: usize,
    vello_renderer: VelloRendererInner,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl VelloRenderer {
    pub fn new(pool: SlotPool, width: u32, height: u32) -> Result<Self, String> {
        let mut context = RenderContext::new();

        let device_id = pollster::block_on(context.device(None))
            .ok_or("No compatible GPU device found")?;

        let device_handle = &context.devices[device_id];
        let device = &device_handle.device;

        let vello_renderer = VelloRendererInner::new(device, RendererOptions {
            surface_format: None,
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
        })
        .map_err(|e| format!("Vello renderer init failed: {e}"))?;

        info!("overlay: Vello renderer initialized (wgpu device ready)");

        Ok(Self {
            pool,
            width,
            height,
            context,
            device_id,
            vello_renderer,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        })
    }

    fn render_scene(&mut self, state: &DrawState) -> Vec<u8> {
        let w = self.width;
        let h = self.height;
        let device_handle = &self.context.devices[self.device_id];
        let device = &device_handle.device;
        let queue = &device_handle.queue;

        let mut scene = Scene::new();

        if state.visible {
            let bw = BORDER_WIDTH as f64;
            let r = CORNER_RADIUS as f64;
            let outer = RoundedRect::new(0.0, 0.0, w as f64, h as f64, r);

            scene.fill(Fill::NonZero, Affine::IDENTITY, COLOR_BG, None, &outer);

            if state.is_recording {
                let perimeter = outer_perimeter(w as f64, h as f64, r);
                let active_len = state.countdown_frac as f64 * perimeter;
                let progress = build_progress_ring(w as f64, h as f64, r, bw, active_len, perimeter);
                for (start_angle, sweep_angle) in progress {
                    let seg = CircleSegment::new(
                        (w as f64 / 2.0, h as f64 / 2.0),
                        (w as f64 / 2.0),
                        ((w as f64 / 2.0) - bw),
                        start_angle,
                        sweep_angle,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, COLOR_PROGRESS, None, &seg);
                }
            } else {
                let border_color = if state.is_error {
                    COLOR_ERROR_BORDER
                } else {
                    COLOR_PROGRESS
                };
                scene.stroke(
                    &Stroke::new(bw),
                    Affine::IDENTITY,
                    border_color,
                    None,
                    &RoundedRect::new(
                        bw / 2.0, bw / 2.0,
                        w as f64 - bw / 2.0, h as f64 - bw / 2.0,
                        (r - bw / 2.0).max(0.0),
                    ),
                );
            }

            if state.is_recording {
                let content_pad = (BORDER_WIDTH + 10.0) as usize;
                let vol_bar_h = 6usize;
                let vol_bar_y = h as usize - content_pad - vol_bar_h;
                let vol_bar_max_w = w as usize - content_pad * 2;
                let vol_w = ((state.volume.min(1.0).max(0.0) * vol_bar_max_w as f32) as usize).min(vol_bar_max_w);

                let vy = vol_bar_y as f64;
                let vbh = vol_bar_h as f64;
                let vx = content_pad as f64;
                let vmw = vol_bar_max_w as f64;

                scene.fill(
                    Fill::NonZero,
                    Affine::translate((vx, vy)),
                    Color::from_rgba8(COLOR_VOL_BG.0, COLOR_VOL_BG.1, COLOR_VOL_BG.2, COLOR_VOL_BG.3),
                    None,
                    &RoundedRect::new(0.0, 0.0, vmw, vbh, 3.0),
                );
                if vol_w > 0 {
                    scene.fill(
                        Fill::NonZero,
                        Affine::translate((vx, vy)),
                        Color::from_rgba8(COLOR_VOL.0, COLOR_VOL.1, COLOR_VOL.2, COLOR_VOL.3),
                        None,
                        &RoundedRect::new(0.0, 0.0, vol_w as f64, vbh, 3.0),
                    );
                }
            }
        }

        let target = device.create_texture(&vello::wgpu::TextureDescriptor {
            label: Some("vello-overlay"),
            size: vello::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: vello::wgpu::TextureDimension::D2,
            format: vello::wgpu::TextureFormat::Rgba8Unorm,
            usage: vello::wgpu::TextureUsages::STORAGE_BINDING | vello::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&vello::wgpu::TextureViewDescriptor::default());

        let params = RenderParams {
            base_color: Color::from_rgba8(0, 0, 0, 0),
            width: w,
            height: h,
            antialiasing_method: AaConfig::Area,
        };

        if let Err(e) = self.vello_renderer.render_to_texture(device, queue, &scene, &view, &params) {
            warn!("overlay: vello render failed: {e}");
            return vec![0u8; (w * h * 4) as usize];
        }

        let padded_bytes_per_row = (w * 4).next_multiple_of(256) as u64;
        let buffer_size = padded_bytes_per_row * h as u64;
        let readback = device.create_buffer(&vello::wgpu::BufferDescriptor {
            label: Some("vello-readback"),
            size: buffer_size,
            usage: vello::wgpu::BufferUsages::MAP_READ | vello::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor {
            label: Some("vello-copy"),
        });
        encoder.copy_texture_to_buffer(
            target.as_image_copy(),
            vello::wgpu::ImageCopyBuffer {
                buffer: &readback,
                layout: vello::wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row as u32),
                    rows_per_image: None,
                },
            },
            vello::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);

        let buf_slice = readback.slice(..);
        let (sender, receiver) =
            futures_intrusive::channel::shared::oneshot_channel();
        buf_slice.map_async(vello::wgpu::MapMode::Read, move |v| {
            let _ = sender.send(v);
        });

        if let Some(recv_result) = block_on_wgpu(device, receiver.receive()) {
            if let Err(e) = recv_result {
                warn!("overlay: readback map failed: {e}");
                return vec![0u8; (w * h * 4) as usize];
            }
        }

        let data = buf_slice.get_mapped_range();
        let row_bytes = (w * 4) as usize;
        let mut pixels = Vec::<u8>::with_capacity(row_bytes * h as usize);
        for row in 0..h as usize {
            let start = row * padded_bytes_per_row as usize;
            pixels.extend_from_slice(&data[start..start + row_bytes]);
        }
        drop(data);
        readback.unmap();

        for chunk in pixels.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }

        pixels
    }

    fn render_text_cpu(
        &mut self,
        canvas: &mut [u8],
        canvas_w: usize,
        canvas_h: usize,
        text: &str,
        x_start: usize,
        y_center: usize,
        text_color: (u8, u8, u8, u8),
    ) {
        let font_size = 15.0;
        let line_height = 20.0;
        let metrics = Metrics::new(font_size, line_height);
        let attrs = Attrs::new()
            .family(Family::Name("Noto Sans CJK SC"))
            .family(Family::SansSerif);

        let mut buf = Buffer::new(&mut self.font_system, metrics);
        buf.set_size(&mut self.font_system, Some(canvas_w as f32 - x_start as f32 - 16.0), Some(canvas_h as f32));
        buf.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
        buf.shape_until_scroll(&mut self.font_system, true);

        let layout_runs: Vec<_> = buf.layout_runs().collect();
        let total_height: f32 = layout_runs.iter().map(|r| r.line_height).sum();
        let mut y_offset = y_center as f32 - total_height / 2.0;

        for run in &layout_runs {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((x_start as f32, y_offset), 1.0);
                if let Some(img) = self.swash_cache.get_image(&mut self.font_system, physical.cache_key) {
                    let gw = img.placement.width as usize;
                    let gh = img.placement.height as usize;
                    if gw == 0 || gh == 0 { continue; }
                    let x_base = physical.x + img.placement.left;
                    let y_base = physical.y - img.placement.top as i32;

                    match img.content {
                        SwashContent::Mask => {
                            for (row_idx, row) in img.data.chunks(gw).enumerate() {
                                let py = y_base + row_idx as i32;
                                if py < 0 || py >= canvas_h as i32 { continue; }
                                for (col_idx, &alpha) in row.iter().enumerate() {
                                    if alpha == 0 { continue; }
                                    let px = x_base + col_idx as i32;
                                    if px < 0 || px >= canvas_w as i32 { continue; }
                                    let dst = (py as usize * canvas_w + px as usize) * 4;
                                    if dst + 3 >= canvas.len() { continue; }
                                    let a = (text_color.3 as u32 * alpha as u32 / 255) as u8;
                                    alpha_blend(canvas, dst, &[text_color.0, text_color.1, text_color.2, a]);
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
                                    let off = col_idx * 4;
                                    let r = row[off]; let g = row[off+1]; let b = row[off+2]; let a = row[off+3];
                                    if a == 0 && r == 0 && g == 0 && b == 0 { continue; }
                                    let dst = (py as usize * canvas_w + px as usize) * 4;
                                    if dst + 3 >= canvas.len() { continue; }
                                    alpha_blend(canvas, dst, &[r, g, b, a.max(text_color.3)]);
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
                                    let off = col_idx * 4;
                                    let c = [row[off], row[off+1], row[off+2], row[off+3]];
                                    if c[3] == 0 { continue; }
                                    let dst = (py as usize * canvas_w + px as usize) * 4;
                                    if dst + 3 >= canvas.len() { continue; }
                                    alpha_blend(canvas, dst, &c);
                                }
                            }
                        }
                    }
                }
            }
            y_offset += run.line_height;
        }
    }

    pub fn measure_text_width(&mut self, text: &str) -> f32 {
        let font_size = 15.0;
        let line_height = 20.0;
        let metrics = Metrics::new(font_size, line_height);
        let attrs = Attrs::new()
            .family(Family::Name("Noto Sans CJK SC"))
            .family(Family::SansSerif);
        let mut buf = Buffer::new(&mut self.font_system, metrics);
        buf.set_size(&mut self.font_system, Some(8000.0), Some(100.0));
        buf.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
        buf.shape_until_scroll(&mut self.font_system, true);
        buf.layout_runs().next().map_or(200.0, |run| {
            if let Some(last) = run.glyphs.last() {
                last.x + last.w as f32
            } else {
                200.0
            }
        })
    }
}

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

fn outer_perimeter(w: f64, h: f64, r: f64) -> f64 {
    let seg_top = w - 2.0 * r;
    let seg_side = h - 2.0 * r;
    let arc = std::f64::consts::PI * r * 0.5;
    2.0 * seg_top + 2.0 * seg_side + 4.0 * arc
}

fn build_progress_ring(
    w: f64, h: f64, r: f64, bw: f64, active_len: f64, perimeter: f64,
) -> Vec<(f64, f64)> {
    let seg_top = w - 2.0 * r;
    let seg_side = h - 2.0 * r;
    let arc = std::f64::consts::PI * r * 0.5;

    let bg_frac = 1.0 - (active_len / perimeter);
    if bg_frac >= 1.0 { return vec![]; }

    let mut start = bg_frac * perimeter;
    let mut remaining = active_len;
    let mut segments = Vec::new();

    let path_lengths = [
        seg_top, arc, seg_side, arc, seg_top, arc, seg_side, arc,
    ];
    let mut offset = 0.0;
    for &len in &path_lengths {
        if start >= offset + len {
            offset += len;
            continue;
        }
        let local_start = (start - offset).max(0.0);
        let available = len - local_start;
        let sweep = remaining.min(available);
        if sweep > 0.01 {
            segments.push((start, sweep));
            remaining -= sweep;
            if remaining <= 0.01 { break; }
            start += sweep;
        }
        offset += len;
    }

    let cx = w / 2.0;
    let cy = h / 2.0;
    let outer_r = w / 2.0;
    let inner_r = outer_r - bw;

    segments.iter().map(|&(s, sweep)| {
        let angle = point_to_angle(s, w, h, r);
        (angle, sweep * outer_r / r * 0.5)
    }).collect()
}

fn point_to_angle(dist: f64, w: f64, h: f64, r: f64) -> f64 {
    let seg_top = w - 2.0 * r;
    let seg_side = h - 2.0 * r;
    let arc = std::f64::consts::PI * r * 0.5;
    let _ = (dist, h, seg_side);
    -std::f64::consts::FRAC_PI_2 + dist / r * 0.5
}

impl OverlayRenderer for VelloRenderer {
    fn draw(
        &mut self,
        state: &DrawState,
        layer: &LayerSurface,
        _qh: &QueueHandle<OverlayState>,
    ) {
        let width = self.width;
        let height = self.height;
        if width == 0 || height == 0 { return; }

        let mut pixels = self.render_scene(state);

        if state.visible {
            let text_color = if state.is_error { COLOR_ERROR_TEXT } else { COLOR_TEXT };
            let content_pad = (BORDER_WIDTH + 10.0) as usize;
            let text_y = height as usize / 2 - 4;
            let status_text = if state.is_recording {
                format!("🎙  录音中 {}s", state.remaining_secs as u32)
            } else {
                state.display_text.to_string()
            };
            self.render_text_cpu(
                &mut pixels, width as usize, height as usize,
                &status_text, content_pad, text_y, text_color,
            );
        }

        let stride = width as i32 * 4;
        let (buffer, mut canvas) = match self.pool.create_buffer(
            width as i32, height as i32, stride, wl_shm::Format::Argb8888,
        ) {
            Ok(b) => b,
            Err(e) => {
                warn!("overlay: vello create_buffer failed: {e}");
                return;
            }
        };

        canvas.copy_from_slice(&pixels);

        layer.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(layer.wl_surface()).expect("buffer attach");
        layer.commit();
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }

    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
