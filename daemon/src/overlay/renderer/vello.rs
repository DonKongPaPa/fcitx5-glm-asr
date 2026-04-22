use std::any::Any;
use std::num::NonZeroUsize;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent, SwashImage};
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use tracing::{info, warn};
use vello::kurbo::{Affine, BezPath, Point, RoundedRect, Stroke};
use vello::peniko::{Color, Fill};
use vello::util::{block_on_wgpu, RenderContext};
use vello::{AaConfig, AaSupport, RenderParams, Renderer as VelloInner, RendererOptions, Scene};
use wayland_client::protocol::wl_shm;
use wayland_client::QueueHandle;

use super::super::{BORDER_WIDTH, CORNER_RADIUS, OverlayState};
use super::{DrawState, OverlayRenderer};

const COLOR_BG: (u8, u8, u8, u8) = (0x1A, 0x1A, 0x2E, 0xEE);
const COLOR_PROGRESS: (u8, u8, u8, u8) = (0x4C, 0xAF, 0x50, 0xFF);
const COLOR_PROGRESS_BG: (u8, u8, u8, u8) = (0x30, 0x30, 0x50, 0x80);
const COLOR_ERROR_BORDER: (u8, u8, u8, u8) = (0xEF, 0x53, 0x50, 0xFF);
const COLOR_WAVEFORM: (u8, u8, u8, u8) = (0x4C, 0xAF, 0x50, 0x99);
const COLOR_TEXT: (u8, u8, u8, u8) = (0xFF, 0xFF, 0xFF, 0xFF);
const COLOR_ERROR_TEXT: (u8, u8, u8, u8) = (0xFF, 0xCD, 0xD2, 0xFF);

fn fade_color(c: (u8, u8, u8, u8), fade: f32) -> Color {
    Color::from_rgba8(c.0, c.1, c.2, (c.3 as f32 * fade) as u8)
}

struct GpuResources {
    target: vello::wgpu::Texture,
    target_view: vello::wgpu::TextureView,
    readback: vello::wgpu::Buffer,
    padded_bytes_per_row: u64,
}

pub struct VelloRenderer {
    pool: SlotPool,
    width: u32,
    height: u32,
    scale_factor: f32,
    context: RenderContext,
    device_id: usize,
    vello_renderer: VelloInner,
    gpu: GpuResources,
    pixel_buf: Vec<u8>,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl VelloRenderer {
    pub fn new(pool: SlotPool, width: u32, height: u32) -> Result<Self, String> {
        let mut context = RenderContext::new();
        let device_id = pollster::block_on(context.device(None))
            .ok_or("No compatible GPU device found")?;

        let device = &context.devices[device_id].device;
        let vello_renderer = VelloInner::new(device, RendererOptions {
            surface_format: None,
            use_cpu: false,
            antialiasing_support: AaSupport::area_only(),
            num_init_threads: NonZeroUsize::new(1),
        }).map_err(|e| format!("Vello renderer init failed: {e}"))?;

        let gpu = Self::create_gpu_resources(device, width, height);
        let pixel_buf = vec![0u8; (width * height * 4) as usize];

        info!("overlay: Vello renderer initialized (wgpu device ready)");
        Ok(Self {
            pool, width, height, scale_factor: 1.0,
            context, device_id, vello_renderer, gpu, pixel_buf,
            font_system: FontSystem::new(), swash_cache: SwashCache::new(),
        })
    }

    fn create_gpu_resources(device: &vello::wgpu::Device, w: u32, h: u32) -> GpuResources {
        let target = device.create_texture(&vello::wgpu::TextureDescriptor {
            label: Some("vello-overlay"),
            size: vello::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1, sample_count: 1,
            dimension: vello::wgpu::TextureDimension::D2,
            format: vello::wgpu::TextureFormat::Rgba8Unorm,
            usage: vello::wgpu::TextureUsages::STORAGE_BINDING | vello::wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target_view = target.create_view(&vello::wgpu::TextureViewDescriptor::default());
        let padded_bytes_per_row = (w as u64 * 4).next_multiple_of(256);
        let buffer_size = padded_bytes_per_row * h as u64;
        let readback = device.create_buffer(&vello::wgpu::BufferDescriptor {
            label: Some("vello-readback"),
            size: buffer_size,
            usage: vello::wgpu::BufferUsages::MAP_READ | vello::wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        GpuResources { target, target_view, readback, padded_bytes_per_row }
    }

    fn rebuild_gpu_resources(&mut self) {
        let device = &self.context.devices[self.device_id].device;
        self.gpu = Self::create_gpu_resources(device, self.width, self.height);
        self.pixel_buf = vec![0u8; (self.width * self.height * 4) as usize];
    }

    fn render_gpu(&mut self, state: &DrawState) {
        let w = self.width;
        let h = self.height;
        if w == 0 || h == 0 { return; }

        let device = &self.context.devices[self.device_id].device;
        let queue = &self.context.devices[self.device_id].queue;

        let scale = self.scale_factor as f64;
        let fade = state.fade_alpha;
        let mut scene = Scene::new();

        if state.visible {
            let bw = BORDER_WIDTH as f64 * scale;
            let r = CORNER_RADIUS as f64 * scale;
            let outer = RoundedRect::new(0.0, 0.0, w as f64, h as f64, r);
            scene.fill(Fill::NonZero, Affine::IDENTITY, fade_color(COLOR_BG, fade), None, &outer);

            if state.is_recording && !state.waveform.is_empty() {
                let pad = bw * 0.3;
                let area_w = w as f64 - 2.0 * pad;
                let center_y = h as f64 * 0.52;
                let max_amp = (h as f64 - 2.0 * bw) * 0.45;

                let points: Vec<Point> = state.waveform.iter().enumerate().map(|(i, &amp)| {
                    let x = pad + (i as f64 + 0.5) / state.waveform.len() as f64 * area_w;
                    let y = center_y - amp.min(1.0) as f64 * max_amp;
                    Point::new(x, y)
                }).collect();

                let mut path = BezPath::new();
                path.move_to(Point::new(pad, center_y));
                if points.len() >= 2 {
                    path.line_to(points[0]);
                    for i in 0..points.len() - 1 {
                        let p0 = if i > 0 { points[i - 1] } else { points[0] };
                        let p1 = points[i];
                        let p2 = points[i + 1];
                        let p3 = if i + 2 < points.len() { points[i + 2] } else { p2 };
                        let cp1 = Point::new(
                            p1.x + (p2.x - p0.x) / 6.0,
                            p1.y + (p2.y - p0.y) / 6.0,
                        );
                        let cp2 = Point::new(
                            p2.x - (p3.x - p1.x) / 6.0,
                            p2.y - (p3.y - p1.y) / 6.0,
                        );
                        path.curve_to(cp1, cp2, p2);
                    }
                }
                let last_x = pad + area_w;
                path.line_to(Point::new(last_x, center_y));
                for i in (0..points.len()).rev() {
                    let pt = points[i];
                    path.line_to(Point::new(pt.x, center_y + (center_y - pt.y)));
                }
                path.close_path();
                scene.fill(Fill::NonZero, Affine::IDENTITY, fade_color(COLOR_WAVEFORM, fade), None, &path);
            }

            if state.is_recording {
                let inner = RoundedRect::new(bw / 2.0, bw / 2.0, w as f64 - bw / 2.0, h as f64 - bw / 2.0, (r - bw / 2.0).max(0.0));
                scene.stroke(&Stroke::new(bw), Affine::IDENTITY, fade_color(COLOR_PROGRESS_BG, fade), None, &inner);

                let perimeter = rounded_rect_perimeter(w as f64, h as f64, (r - bw / 2.0).max(0.0));
                let active_len = state.countdown_frac as f64 * perimeter;
                if active_len > 0.5 {
                    let path = trace_border_path(w as f64, h as f64, (r - bw / 2.0).max(0.0), bw / 2.0, active_len);
                    scene.stroke(&Stroke::new(bw), Affine::IDENTITY, fade_color(COLOR_PROGRESS, fade), None, &path);
                }
            } else {
                let border_color = if state.is_error { COLOR_ERROR_BORDER } else { COLOR_PROGRESS };
                scene.stroke(&Stroke::new(bw), Affine::IDENTITY, fade_color(border_color, fade), None,
                    &RoundedRect::new(bw / 2.0, bw / 2.0, w as f64 - bw / 2.0, h as f64 - bw / 2.0, (r - bw / 2.0).max(0.0)));
            }
        }

        let params = RenderParams {
            base_color: Color::from_rgba8(0, 0, 0, 0),
            width: w, height: h,
            antialiasing_method: AaConfig::Area,
        };

        if let Err(e) = self.vello_renderer.render_to_texture(device, queue, &scene, &self.gpu.target_view, &params) {
            warn!("overlay: vello render failed: {e}");
            return;
        }

        let mut encoder = device.create_command_encoder(&vello::wgpu::CommandEncoderDescriptor { label: Some("vello-copy") });
        encoder.copy_texture_to_buffer(
            self.gpu.target.as_image_copy(),
            vello::wgpu::ImageCopyBuffer {
                buffer: &self.gpu.readback,
                layout: vello::wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(self.gpu.padded_bytes_per_row as u32),
                    rows_per_image: None,
                },
            },
            vello::wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        queue.submit([encoder.finish()]);

        let buf_slice = self.gpu.readback.slice(..);
        let (tx, rx) = futures_intrusive::channel::shared::oneshot_channel();
        buf_slice.map_async(vello::wgpu::MapMode::Read, move |v| { let _ = tx.send(v); });

        if let Some(res) = block_on_wgpu(device, rx.receive()) {
            if res.is_err() { return; }
        }

        let data = buf_slice.get_mapped_range();
        let row_bytes = (w * 4) as usize;
        let padded = self.gpu.padded_bytes_per_row as usize;
        for row in 0..h as usize {
            let src = row * padded;
            let dst = row * row_bytes;
            self.pixel_buf[dst..dst + row_bytes].copy_from_slice(&data[src..src + row_bytes]);
        }
        drop(data);
        self.gpu.readback.unmap();

        for chunk in self.pixel_buf.chunks_exact_mut(4) {
            chunk.swap(0, 2);
        }
    }

    fn render_text_cpu(&mut self, text: &str, x_start: usize, y_center: usize, text_color: (u8, u8, u8, u8)) {
        let cw = self.width as usize;
        let ch = self.height as usize;
        let scale = self.scale_factor;
        let font_size = 15.0 * scale;
        let line_height = 20.0 * scale;
        let metrics = Metrics::new(font_size, line_height);
        let attrs = Attrs::new().family(Family::Name("Noto Sans CJK SC")).family(Family::SansSerif);

        let mut buf = Buffer::new(&mut self.font_system, metrics);
        buf.set_size(&mut self.font_system, Some(cw as f32 - x_start as f32 - 16.0 * scale), Some(ch as f32));
        buf.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
        buf.shape_until_scroll(&mut self.font_system, true);

        let layout_runs: Vec<_> = buf.layout_runs().collect();
        let total_h: f32 = layout_runs.iter().map(|r| r.line_height).sum();
        let mut y_offset = y_center as f32 - total_h / 2.0;

        for run in &layout_runs {
            for glyph in run.glyphs.iter() {
                let physical = glyph.physical((x_start as f32, y_offset), 1.0);
                if let Some(img) = self.swash_cache.get_image(&mut self.font_system, physical.cache_key) {
                    let gw = img.placement.width as usize;
                    let gh = img.placement.height as usize;
                    if gw == 0 || gh == 0 { continue; }
                    let x_base = physical.x + img.placement.left;
                    let y_base = physical.y - img.placement.top as i32;
                    render_glyph(&mut self.pixel_buf, cw, ch, &img, x_base, y_base, text_color);
                }
            }
            y_offset += run.line_height;
        }
    }

    pub fn measure_text_width(&mut self, text: &str) -> f32 {
        let metrics = Metrics::new(15.0, 20.0);
        let attrs = Attrs::new().family(Family::Name("Noto Sans CJK SC")).family(Family::SansSerif);
        let mut buf = Buffer::new(&mut self.font_system, metrics);
        buf.set_size(&mut self.font_system, Some(8000.0), Some(100.0));
        buf.set_text(&mut self.font_system, text, &attrs, Shaping::Advanced);
        buf.shape_until_scroll(&mut self.font_system, true);
        buf.layout_runs().next().map_or(200.0, |run| {
            run.glyphs.last().map_or(200.0, |g| g.x + g.w as f32)
        })
    }
}

fn render_glyph(canvas: &mut [u8], cw: usize, ch: usize, img: &SwashImage, x_base: i32, y_base: i32, tc: (u8, u8, u8, u8)) {
    let gw = img.placement.width as usize;
    let gh = img.placement.height as usize;
    match img.content {
        SwashContent::Mask => {
            for (ri, row) in img.data.chunks(gw).enumerate() {
                let py = y_base + ri as i32;
                if py < 0 || py >= ch as i32 { continue; }
                for (ci, &a) in row.iter().enumerate() {
                    if a == 0 { continue; }
                    let px = x_base + ci as i32;
                    if px < 0 || px >= cw as i32 { continue; }
                    let d = (py as usize * cw + px as usize) * 4;
                    if d + 3 >= canvas.len() { continue; }
                    let fa = (tc.3 as u32 * a as u32 / 255) as u8;
                    alpha_blend(canvas, d, &[tc.0, tc.1, tc.2, fa]);
                }
            }
        }
        SwashContent::SubpixelMask => {
            let stride = gw * 4;
            for (ri, row) in img.data.chunks(stride).enumerate() {
                if row.len() < stride { continue; }
                let py = y_base + ri as i32;
                if py < 0 || py >= ch as i32 { continue; }
                for ci in 0..gw {
                    let px = x_base + ci as i32;
                    if px < 0 || px >= cw as i32 { continue; }
                    let o = ci * 4;
                    let d = (py as usize * cw + px as usize) * 4;
                    if d + 3 >= canvas.len() { continue; }
                    let r = row[o]; let g = row[o+1]; let b = row[o+2]; let a = row[o+3];
                    if a == 0 && r == 0 && g == 0 && b == 0 { continue; }
                    alpha_blend(canvas, d, &[r, g, b, a.max(tc.3)]);
                }
            }
        }
        SwashContent::Color => {
            let stride = gw * 4;
            for (ri, row) in img.data.chunks(stride).enumerate() {
                if row.len() < stride { continue; }
                let py = y_base + ri as i32;
                if py < 0 || py >= ch as i32 { continue; }
                for ci in 0..gw {
                    let px = x_base + ci as i32;
                    if px < 0 || px >= cw as i32 { continue; }
                    let o = ci * 4;
                    let c = [row[o], row[o+1], row[o+2], row[o+3]];
                    if c[3] == 0 { continue; }
                    let d = (py as usize * cw + px as usize) * 4;
                    if d + 3 >= canvas.len() { continue; }
                    alpha_blend(canvas, d, &c);
                }
            }
        }
    }
}

fn alpha_blend(canvas: &mut [u8], offset: usize, src: &[u8; 4]) {
    if src[3] == 0 { return; }
    if src[3] == 0xFF || canvas[offset + 3] == 0 {
        canvas[offset..offset + 4].copy_from_slice(src);
        return;
    }
    let sa = src[3] as u32;
    let da = canvas[offset + 3] as u32;
    let inv = 255 - sa;
    let oa = sa + da * inv / 255;
    if oa == 0 { return; }
    canvas[offset] = ((src[0] as u32 * sa * 255 + canvas[offset] as u32 * da * inv) / (oa * 255)) as u8;
    canvas[offset + 1] = ((src[1] as u32 * sa * 255 + canvas[offset + 1] as u32 * da * inv) / (oa * 255)) as u8;
    canvas[offset + 2] = ((src[2] as u32 * sa * 255 + canvas[offset + 2] as u32 * da * inv) / (oa * 255)) as u8;
    canvas[offset + 3] = oa as u8;
}

fn rounded_rect_perimeter(w: f64, h: f64, r: f64) -> f64 {
    2.0 * (w - 2.0 * r) + 2.0 * (h - 2.0 * r) + 4.0 * std::f64::consts::PI * r * 0.5
}

fn arc_bezier(cx: f64, cy: f64, r: f64, start: f64, sweep: f64) -> (Point, Point, Point) {
    let alpha = sweep * 0.5;
    let k = (4.0 / 3.0) * (1.0 - alpha.cos()) / alpha.sin();
    let a0 = start;
    let a1 = start + sweep;
    let c0 = a0.cos(); let s0 = a0.sin();
    let c1 = a1.cos(); let s1 = a1.sin();
    (
        Point::new(cx + r * (c0 - k * s0), cy + r * (s0 + k * c0)),
        Point::new(cx + r * (c1 + k * s1), cy + r * (s1 - k * c1)),
        Point::new(cx + r * c1, cy + r * s1),
    )
}

fn trace_border_path(w: f64, h: f64, r: f64, offset: f64, active_len: f64) -> BezPath {
    let x0 = offset;
    let y0 = offset;
    let x1 = w - offset;
    let y1 = h - offset;

    let top_len = (x1 - r) - (x0 + r);
    let side_len = (y1 - r) - (y0 + r);
    let arc_len = std::f64::consts::PI * 0.5 * r;

    struct Seg {
        len: f64,
        end_x: f64,
        end_y: f64,
        is_arc: bool,
        cx: f64,
        cy: f64,
        start_angle: f64,
    }

    let segs = [
        Seg { len: top_len,  end_x: x1-r, end_y: y0,   is_arc: false, cx: 0.0, cy: 0.0, start_angle: 0.0 },
        Seg { len: arc_len,  end_x: x1,   end_y: y0+r, is_arc: true,  cx: x1-r, cy: y0+r, start_angle: -std::f64::consts::FRAC_PI_2 },
        Seg { len: side_len, end_x: x1,   end_y: y1-r, is_arc: false, cx: 0.0, cy: 0.0, start_angle: 0.0 },
        Seg { len: arc_len,  end_x: x1-r, end_y: y1,   is_arc: true,  cx: x1-r, cy: y1-r, start_angle: 0.0 },
        Seg { len: top_len,  end_x: x0+r, end_y: y1,   is_arc: false, cx: 0.0, cy: 0.0, start_angle: 0.0 },
        Seg { len: arc_len,  end_x: x0,   end_y: y1-r, is_arc: true,  cx: x0+r, cy: y1-r, start_angle: std::f64::consts::FRAC_PI_2 },
        Seg { len: side_len, end_x: x0,   end_y: y0+r, is_arc: false, cx: 0.0, cy: 0.0, start_angle: 0.0 },
        Seg { len: arc_len,  end_x: x0+r, end_y: y0,   is_arc: true,  cx: x0+r, cy: y0+r, start_angle: std::f64::consts::PI },
    ];

    let mut path = BezPath::new();
    let mut cur_x = x0 + r;
    let mut cur_y = y0;
    path.move_to(Point::new(cur_x, cur_y));

    let mut remaining = active_len;

    for seg in &segs {
        if remaining <= 0.01 { break; }

        if remaining >= seg.len - 0.01 {
            if seg.is_arc {
                let (p1, p2, p3) = arc_bezier(seg.cx, seg.cy, r, seg.start_angle, std::f64::consts::FRAC_PI_2);
                path.curve_to(p1, p2, p3);
            } else {
                path.line_to(Point::new(seg.end_x, seg.end_y));
            }
            cur_x = seg.end_x;
            cur_y = seg.end_y;
            remaining -= seg.len;
        } else {
            let frac = remaining / seg.len;
            if seg.is_arc {
                let sweep = frac * std::f64::consts::FRAC_PI_2;
                let (p1, p2, p3) = arc_bezier(seg.cx, seg.cy, r, seg.start_angle, sweep);
                path.curve_to(p1, p2, p3);
            } else {
                let nx = cur_x + (seg.end_x - cur_x) * frac;
                let ny = cur_y + (seg.end_y - cur_y) * frac;
                path.line_to(Point::new(nx, ny));
            }
            remaining = 0.0;
        }
    }

    path
}

impl OverlayRenderer for VelloRenderer {
    fn draw(&mut self, state: &DrawState, layer: &LayerSurface, _qh: &QueueHandle<OverlayState>) {
        let w = self.width;
        let h = self.height;
        if w == 0 || h == 0 { return; }

        let expected = (w as usize) * (h as usize) * 4;
        if self.pixel_buf.len() != expected {
            self.pixel_buf = vec![0u8; expected];
        }

        self.render_gpu(state);

        if state.visible {
            let text_color = if state.is_error { COLOR_ERROR_TEXT } else { COLOR_TEXT };
            let fade = state.fade_alpha;
            let faded_tc = (text_color.0, text_color.1, text_color.2, (text_color.3 as f32 * fade) as u8);
            let scale = self.scale_factor;
            let content_pad = (BORDER_WIDTH * scale + 10.0 * scale) as usize;
            let text_y = (h as f32 / 2.0 - 4.0 * scale) as usize;
            let status_text = if state.is_recording {
                format!("🎙  录音中 {}s", state.remaining_secs as u32)
            } else {
                state.display_text.to_string()
            };
            self.render_text_cpu(&status_text, content_pad, text_y, faded_tc);
        }

        let stride = w as i32 * 4;
        let (buffer, mut canvas) = match self.pool.create_buffer(w as i32, h as i32, stride, wl_shm::Format::Argb8888) {
            Ok(b) => b,
            Err(e) => { warn!("overlay: vello create_buffer failed: {e}"); return; }
        };
        let copy_len = canvas.len().min(self.pixel_buf.len());
        canvas[..copy_len].copy_from_slice(&self.pixel_buf[..copy_len]);

        layer.wl_surface().damage_buffer(0, 0, w as i32, h as i32);
        buffer.attach_to(layer.wl_surface()).expect("buffer attach");
        layer.commit();
    }

    fn resize(&mut self, width: u32, height: u32, scale_factor: i32) {
        self.width = width;
        self.height = height;
        self.scale_factor = scale_factor as f32;
        self.rebuild_gpu_resources();
    }

    fn width(&self) -> u32 { self.width }
    fn height(&self) -> u32 { self.height }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}
