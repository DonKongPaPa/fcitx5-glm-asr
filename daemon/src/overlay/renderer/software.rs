use std::any::Any;

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, SwashContent};
use smithay_client_toolkit::shell::wlr_layer::LayerSurface;
use smithay_client_toolkit::shell::WaylandSurface;
use smithay_client_toolkit::shm::slot::SlotPool;
use tracing::warn;
use wayland_client::protocol::wl_shm;
use wayland_client::QueueHandle;

use super::super::{BORDER_WIDTH, CORNER_RADIUS, OverlayState};
use super::{DrawState, OverlayRenderer};

const COLOR_BG: [u8; 4] = [0x1A, 0x1A, 0x2E, 0xEE];
const COLOR_PROGRESS: [u8; 4] = [0x4C, 0xAF, 0x50, 0xFF];
const COLOR_PROGRESS_BG: [u8; 4] = [0x30, 0x30, 0x50, 0x80];
const COLOR_VOL: [u8; 4] = [0x66, 0xBB, 0x6A, 0xFF];
const COLOR_VOL_BG: [u8; 4] = [0x30, 0x30, 0x50, 0x80];
const COLOR_TEXT: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];
const COLOR_ERROR_BORDER: [u8; 4] = [0xEF, 0x53, 0x50, 0xFF];
const COLOR_ERROR_TEXT: [u8; 4] = [0xFF, 0xCD, 0xD2, 0xFF];
const COLOR_WAVEFORM: [u8; 4] = [0x4C, 0xAF, 0x50, 0x99];

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
    scale: f32,
) {
    let font_size = 15.0 * scale;
    let line_height = 20.0 * scale;
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

pub fn measure_text_width(text: &str, font_system: &mut FontSystem) -> f32 {
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

pub struct SoftwareRenderer {
    pool: SlotPool,
    width: u32,
    height: u32,
    scale_factor: f32,
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl SoftwareRenderer {
    pub fn new(pool: SlotPool, width: u32, height: u32) -> Self {
        Self {
            pool,
            width,
            height,
            scale_factor: 1.0,
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    pub fn measure_text_width(&mut self, text: &str) -> f32 {
        measure_text_width(text, &mut self.font_system)
    }
}

impl OverlayRenderer for SoftwareRenderer {
    fn draw(
        &mut self,
        state: &DrawState,
        layer: &LayerSurface,
        _qh: &QueueHandle<OverlayState>,
    ) {
        let width = self.width;
        let height = self.height;

        if width == 0 || height == 0 {
            return;
        }

        let scale = self.scale_factor;
        let width_usize = width as usize;
        let height_usize = height as usize;
        let stride = width as i32 * 4;

        let (buffer, mut canvas) = match self.pool.create_buffer(
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

        for chunk in canvas.chunks_exact_mut(4) {
            chunk[0] = 0;
            chunk[1] = 0;
            chunk[2] = 0;
            chunk[3] = 0;
        }

        if state.visible {
            let w = width_usize as f32;
            let h = height_usize as f32;
            let bw = BORDER_WIDTH * scale;
            let r_outer = CORNER_RADIUS * scale;
            let r_inner = (r_outer - bw).max(0.0);
            let inner_w = (w - 2.0 * bw).max(1.0);
            let inner_h = (h - 2.0 * bw).max(1.0);

            let countdown_frac = state.countdown_frac;

            let seg_top = w - 2.0 * r_outer;
            let seg_right = h - 2.0 * r_outer;
            let corner_arc = std::f32::consts::PI * r_outer * 0.5;
            let perimeter = 2.0 * seg_top + 2.0 * seg_right + 4.0 * corner_arc;
            let active_len = countdown_frac * perimeter;

            let fade = state.fade_alpha;
            let fade_color = |c: [u8; 4]| -> [u8; 4] {
                [c[0], c[1], c[2], (c[3] as f32 * fade) as u8]
            };
            let color_bg = fade_color(COLOR_BG);
            let color_progress = fade_color(COLOR_PROGRESS);
            let color_progress_bg = fade_color(COLOR_PROGRESS_BG);
            let color_vol = fade_color(COLOR_VOL);
            let color_vol_bg = fade_color(COLOR_VOL_BG);
            let color_error_border = fade_color(COLOR_ERROR_BORDER);
            let color_text = fade_color(COLOR_TEXT);
            let color_error_text = fade_color(COLOR_ERROR_TEXT);

            let margin = 2;
            for y in margin..height_usize - margin {
                for x in margin..width_usize - margin {
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
                        let offset = (y * width_usize + x) * 4;
                        if offset + 3 >= canvas.len() { continue; }

                        if state.is_recording {
                            let d = path_distance(px, py, w, h, r_outer);
                            let soft_edge = 2.0;
                            let t = ((active_len - d) / soft_edge + 0.5).clamp(0.0, 1.0);
                            if t >= 1.0 {
                                blend_pixel(canvas, offset, &color_progress, border_cov);
                            } else if t <= 0.0 {
                                blend_pixel(canvas, offset, &color_progress_bg, border_cov);
                            } else {
                                let r = (color_progress_bg[0] as f32 * (1.0 - t) + color_progress[0] as f32 * t) as u8;
                                let g = (color_progress_bg[1] as f32 * (1.0 - t) + color_progress[1] as f32 * t) as u8;
                                let b = (color_progress_bg[2] as f32 * (1.0 - t) + color_progress[2] as f32 * t) as u8;
                                let a = (color_progress_bg[3] as f32 * (1.0 - t) + color_progress[3] as f32 * t) as u8;
                                blend_pixel(canvas, offset, &[r, g, b, a], border_cov);
                            }
                        } else {
                            let border_color = if state.is_error {
                                color_error_border
                            } else {
                                color_progress
                            };
                            blend_pixel(canvas, offset, &border_color, border_cov);
                        }
                    }

                    if inner_cov > 0.0 {
                        let offset = (y * width_usize + x) * 4;
                        if offset + 3 < canvas.len() {
                            blend_pixel(canvas, offset, &color_bg, inner_cov);
                        }
                    }
                }
            }

            let text_color = if state.is_error { color_error_text } else { color_text };
            let content_pad = (BORDER_WIDTH * scale + 10.0 * scale) as usize;
            let text_y = height_usize / 2 - (4.0 * scale) as usize;
            let status_text = if state.is_recording {
                format!("🎙  录音中 {}s", state.remaining_secs as u32)
            } else {
                state.display_text.to_string()
            };
            render_text(
                &status_text,
                &mut canvas, width_usize, height_usize,
                content_pad, text_y,
                text_color,
                &mut self.font_system, &mut self.swash_cache,
                scale,
            );

            if state.is_recording {
                if !state.waveform.is_empty() {
                    let color_wave = fade_color(COLOR_WAVEFORM);
                    let bw_f = BORDER_WIDTH * scale;
                    let pad = (bw_f * 0.3) as usize;
                    let area_w = (width_usize - 2 * pad) as f32;
                    let center_y = height_usize as f32 * 0.52;
                    let max_amp = (height_usize as f32 - 2.0 * bw_f) * 0.45;
                    let n = state.waveform.len() as f32;
                    let step_x = area_w / n;
                    let cy = center_y as usize;

                    let amps: Vec<f32> = state.waveform.iter().map(|&a| a.min(1.0)).collect();

                    for x in pad..width_usize - pad {
                        let fx = (x - pad) as f32;
                        let pos = fx / step_x;
                        let i = pos as usize;
                        let frac = pos - i as f32;
                        let amp = if i + 1 < amps.len() {
                            amps[i] * (1.0 - frac) + amps[i + 1] * frac
                        } else if i < amps.len() {
                            amps[i]
                        } else {
                            0.0
                        };
                        let half_h = (amp * max_amp) as usize;
                        for dy in 0..half_h {
                            let top_y = cy.saturating_sub(dy + 1);
                            let bot_y = cy + dy;
                            if top_y < height_usize {
                                let offset = (top_y * width_usize + x) * 4;
                                if offset + 3 < canvas.len() {
                                    alpha_blend(canvas, offset, &color_wave);
                                }
                            }
                            if bot_y < height_usize {
                                let offset = (bot_y * width_usize + x) * 4;
                                if offset + 3 < canvas.len() {
                                    alpha_blend(canvas, offset, &color_wave);
                                }
                            }
                        }
                    }
                } else {
                    let vol_bar_h = (6.0 * scale) as usize;
                    let vol_bar_y = height_usize - content_pad - vol_bar_h;
                    let vol_bar_max_w = width_usize - content_pad * 2;
                    let vol_w = ((state.volume.min(1.0).max(0.0) * vol_bar_max_w as f32) as usize).min(vol_bar_max_w);

                    fill_rect_aa(&mut canvas, width_usize, height_usize,
                                 content_pad, vol_bar_y, vol_bar_max_w, vol_bar_h, color_vol_bg);
                    if vol_w > 0 {
                        fill_rect_aa(&mut canvas, width_usize, height_usize,
                                     content_pad, vol_bar_y, vol_w, vol_bar_h, color_vol);
                    }
                }
            }
        }

        layer.wl_surface().damage_buffer(0, 0, width as i32, height as i32);
        buffer.attach_to(layer.wl_surface()).expect("buffer attach");
        layer.commit();
    }

    fn resize(&mut self, width: u32, height: u32, scale_factor: i32) {
        self.width = width;
        self.height = height;
        self.scale_factor = scale_factor as f32;
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}
