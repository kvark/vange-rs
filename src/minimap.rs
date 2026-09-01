//! Local radar: a window of the height field, centered on the player.
//!
//! The world is a torus. Samples and markers wrap with the shortest
//! offset so a car that just crossed x=0 stays on the map instead of
//! jumping to the far edge.
//!
//! X is flipped to match the left-handed chase camera (`scale.y = -1`):
//! world +X is screen-left, so a left turn on the road is a left turn
//! on the radar.

use crate::level::{Level, Point, Texel};
use glam::Vec2;

/// Pixels on a side.
pub const SIZE: u32 = 128;
/// World texels per map pixel.
pub const SCALE: f32 = 8.0;

/// A blip on the radar, in world XY.
#[derive(Clone, Copy, Debug)]
pub struct Mark {
    pub pos: Vec2,
    pub color: egui::Color32,
    pub large: bool,
}

pub struct Minimap {
    pixels: Vec<u8>,
    tex: Option<egui::TextureHandle>,
}

impl Default for Minimap {
    fn default() -> Self {
        Self::new()
    }
}

impl Minimap {
    pub fn new() -> Self {
        Minimap {
            pixels: vec![0; SIZE as usize * SIZE as usize * 4],
            tex: None,
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        level: &Level,
        center: Vec2,
        heading: Vec2,
        marks: &[Mark],
    ) {
        blit(level, center, SIZE, SCALE, &mut self.pixels);
        let image =
            egui::ColorImage::from_rgba_unmultiplied([SIZE as usize, SIZE as usize], &self.pixels);
        let options = egui::TextureOptions::NEAREST;
        let tex_id = match self.tex.as_mut() {
            Some(handle) => {
                handle.set(image, options);
                handle.id()
            }
            None => {
                let handle = ctx.load_texture("minimap", image, options);
                let id = handle.id();
                self.tex = Some(handle);
                id
            }
        };

        let side = SIZE as f32;
        const BORDER: f32 = 4.0;
        let frame = side + BORDER * 2.0;
        let ink = egui::Color32::from_rgb(12, 8, 4);
        let edge = egui::Color32::from_rgb(200, 140, 48);
        let you = egui::Color32::from_rgb(255, 230, 80);
        egui::Area::new(egui::Id::new("minimap"))
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(12.0, -12.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let (resp, painter) =
                    ui.allocate_painter(egui::vec2(frame, frame), egui::Sense::hover());
                let outer = resp.rect;
                let rect = outer.shrink(BORDER);
                painter.rect_filled(outer, 3.0, ink);
                painter.image(
                    tex_id,
                    rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
                let origin = rect.center();
                for mark in marks {
                    if let Some(p) = mark_pixel(level, center, mark.pos, SIZE, SCALE) {
                        let pos = origin + egui::vec2(p.x, p.y);
                        if rect.contains(pos) {
                            let r = if mark.large { 3.5 } else { 2.0 };
                            painter.circle_filled(pos, r, mark.color);
                        }
                    }
                }
                painter.circle_filled(origin, 3.0, you);
                let h = heading.normalize_or(Vec2::Y);
                // World +Y is screen-up; world +X is screen-left (left-handed view).
                let tip = origin + egui::vec2(-h.x, -h.y) * 10.0;
                painter.line_segment([origin, tip], egui::Stroke::new(1.5_f32, you));
                painter.rect_stroke(
                    outer,
                    3.0,
                    egui::Stroke::new(2.0_f32, edge),
                    egui::StrokeKind::Inside,
                );
            });
    }
}

/// World XY sampled by map pixel `(px, py)`.
///
/// Screen-up is world +Y; screen-left is world +X, matching the
/// left-handed chase camera.
pub fn sample_world(center: Vec2, px: u32, py: u32, size: u32, scale: f32) -> Vec2 {
    let half = size as f32 * 0.5;
    Vec2::new(
        center.x - (px as f32 + 0.5 - half) * scale,
        center.y - (py as f32 + 0.5 - half) * scale,
    )
}

/// Pixel offset from the map centre for a world point, or `None` if it
/// is more than half the window away (after wrapping).
pub fn mark_pixel(level: &Level, center: Vec2, world: Vec2, size: u32, scale: f32) -> Option<Vec2> {
    if scale <= 0.0 {
        return None;
    }
    let d = level.shortest_xy(center.extend(0.0), world.extend(0.0));
    let p = Vec2::new(-d.x / scale, -d.y / scale);
    let half = size as f32 * 0.5;
    if p.x.abs() >= half || p.y.abs() >= half {
        None
    } else {
        Some(p)
    }
}

fn blit(level: &Level, center: Vec2, size: u32, scale: f32, out: &mut [u8]) {
    let n = size as usize;
    for py in 0..n {
        for px in 0..n {
            let w = sample_world(center, px as u32, py as u32, size, scale);
            let c = sample_rgba(level, w.x as i32, w.y as i32);
            let i = (py * n + px) * 4;
            out[i..i + 4].copy_from_slice(&c);
        }
    }
}

fn sample_rgba(level: &Level, x: i32, y: i32) -> [u8; 4] {
    let (h, ty) = match level.get((x, y)) {
        Texel::Single(Point(h, ty)) => (h, ty),
        Texel::Dual {
            high: Point(h, ty), ..
        } => (h, ty),
    };
    let max_h = level.geometry.height.max(1) as f32;
    let t = (h / max_h).clamp(0.0, 1.0);
    let (lo, hi) = level
        .terrains
        .get(ty as usize)
        .map(|tc| (tc.colors.start as u32, tc.colors.end as u32))
        .unwrap_or((0, 255));
    let span = hi.saturating_sub(lo);
    let idx = (lo as f32 + span as f32 * t) as usize;
    // Palette bytes are sRGB, same as the terrain texture. egui's
    // ColorImage is sRGB too, so copy them as-is — a gamma-space shade
    // multiply here made the radar darker than the world.
    let mut c = level.palette[idx.min(255)];
    c[3] = 255;
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{self, Level};

    fn dummy(w: i32, h: i32) -> Level {
        let n = (w * h) as usize;
        Level {
            size: (w, h),
            flood_map: vec![0; 4].into_boxed_slice(),
            height: vec![0; n].into_boxed_slice(),
            meta: vec![0; n].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: vec![level::TerrainConfig::default(); 8].into_boxed_slice(),
            geometry: settings::Geometry::default(),
        }
    }

    #[test]
    fn a_mark_across_the_west_seam_sits_just_right_of_centre() {
        let level = dummy(32, 32);
        let center = Vec2::new(0.0, 0.0);
        let world = Vec2::new(31.0, 0.0);
        let p = mark_pixel(&level, center, world, 16, 1.0).unwrap();
        assert!(
            p.x > 0.4 && p.x < 2.0,
            "wrapped west is screen-right (left-handed view), got {p:?}"
        );
        assert!(p.y.abs() < 0.6, "same Y, got {p:?}");
    }

    #[test]
    fn sample_left_of_centre_looks_east() {
        let w = sample_world(Vec2::ZERO, 0, 8, 16, 1.0);
        // half=8, px=0 -> x = 0 - (0.5-8)*1 = 7.5
        assert!(
            (w.x - 7.5).abs() < 0.01,
            "leftmost pixel should look east of centre, got {w:?}"
        );
    }

    #[test]
    fn a_pixel_left_of_centre_reads_world_plus_x() {
        let mut level = dummy(32, 32);
        level.height[1] = 200;
        let w = sample_world(Vec2::ZERO, 1, 2, 5, 1.0);
        // half=2.5, px=1 -> x = 0 - (1.5-2.5)*1 = 1
        assert!(
            (w.x - 1.0).abs() < 0.01,
            "left-of-centre pixel should look at x=+1, got {w:?}"
        );
        assert_eq!(level.get((w.x as i32, w.y as i32)).high(), 200.0);
    }
}
