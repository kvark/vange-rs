//! Gameplay HUD in the 1998 Vangers style.
//!
//! The original's actint clocks sit in the corners: `IND_NRG` (armour +
//! energy) top-left, `IND_SPIRAL` bottom-right with tick marks. The painted
//! BML frames are proprietary game data and are not always in the remake's
//! packs, so this module draws the clocks procedurally in egui — native and
//! web share the same painter. Nothing here changes multiplayer behaviour
//! or the under-slab camera.
//!
//! Gauges never rely on colour alone: each has a distinct shape, a fill
//! level (arc or discrete wedges), and an `N/M` readout.

use crate::combat::ARMOR_UNIT;
use egui::epaint::{PathShape, PathStroke};
use egui::{Align2, Color32, Context, FontId, Order, Pos2, Rect, Sense, Stroke, Vec2};

/// Shows or hides the Tweaks / Settings debug panel. Chosen because it
/// is free on both native (`road`) and web (web already uses F2 for the
/// net-debug overlay).
pub const TOGGLE_PANEL_KEY: &str = "F8";

/// One frame of hull and spiral values, in whole `car.prm` points.
pub struct Status<'a> {
    pub armor: u32,
    pub max_armor: u32,
    pub energy: Option<(u32, u32)>,
    pub spiral: Option<(u8, u8)>,
    pub warning: Option<&'a str>,
    pub info: Option<&'a str>,
}

impl<'a> Status<'a> {
    /// Convert the simulation's hundredths-of-a-point armour and energy
    /// into the original's whole-point scale, the same one `hud_line` uses.
    pub fn from_units(
        armor: u32,
        max_armor: u32,
        energy: Option<(u32, u32)>,
        spiral: Option<(u8, u8)>,
    ) -> Self {
        Self {
            armor: armor / ARMOR_UNIT,
            max_armor: max_armor / ARMOR_UNIT,
            energy: energy.map(|(v, m)| (v / ARMOR_UNIT, m / ARMOR_UNIT)),
            spiral,
            warning: None,
            info: None,
        }
    }
}

/// Fraction of a gauge that should be filled, clamped to `0..=1`.
pub fn fill_fraction(value: u32, max: u32) -> f32 {
    if max == 0 {
        0.0
    } else {
        (value as f32 / max as f32).clamp(0.0, 1.0)
    }
}

// Bronze / olive bezel, close to the 1998 actint clocks.
const BEZEL_OUTER: Color32 = Color32::from_rgb(42, 36, 26);
const BEZEL_MID: Color32 = Color32::from_rgb(132, 112, 72);
const BEZEL_HIGH: Color32 = Color32::from_rgb(214, 196, 148);
const WELL: Color32 = Color32::from_rgb(14, 12, 10);
const PLATE: Color32 = Color32::from_rgba_premultiplied(13, 11, 8, 210);
const LABEL: Color32 = Color32::from_rgb(230, 220, 190);
const ARMOUR_FILL: Color32 = Color32::from_rgb(214, 126, 40);
const ARMOUR_EMPTY: Color32 = Color32::from_rgb(56, 34, 16);
const ENERGY_FILL: Color32 = Color32::from_rgb(72, 168, 96);
const ENERGY_EMPTY: Color32 = Color32::from_rgb(22, 42, 26);
const SPIRAL_FILL: Color32 = Color32::from_rgb(196, 48, 40);
const SPIRAL_EMPTY: Color32 = Color32::from_rgb(52, 18, 16);
const WARNING: Color32 = Color32::from_rgb(255, 140, 70);
const INFO: Color32 = Color32::from_rgb(210, 200, 170);

const CLOCK_R: f32 = 44.0;

/// Draw the gameplay gauges over the world view.
pub fn draw(ctx: &Context, status: &Status<'_>) {
    draw_nrg(ctx, status);
    if let Some((charge, capacity)) = status.spiral {
        draw_spiral(ctx, charge, capacity);
    }
}

fn draw_nrg(ctx: &Context, status: &Status<'_>) {
    let note_lines = status.warning.is_some() as i32 + status.info.is_some() as i32;
    let height = CLOCK_R * 2.0 + 24.0 + note_lines as f32 * 18.0;
    let width = CLOCK_R * 2.0 + 150.0;
    egui::Area::new(egui::Id::new("hud-nrg"))
        .anchor(Align2::LEFT_TOP, Vec2::new(10.0, 10.0))
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let painter = ui.painter();
            let plate = Rect::from_min_size(
                rect.min,
                Vec2::new(width, CLOCK_R * 2.0 + 16.0 + note_lines as f32 * 18.0),
            );
            draw_plate(painter, plate);

            let center = rect.min + Vec2::new(CLOCK_R + 8.0, CLOCK_R + 8.0);
            draw_bezel(painter, center, CLOCK_R);

            // Outer ring: energy. Inner ring: armour. Distinct radii so
            // the two can be told apart even in greyscale.
            let energy = status.energy.unwrap_or((0, 0));
            let energy_frac = if status.energy.is_some() {
                fill_fraction(energy.0, energy.1)
            } else {
                0.0
            };
            ring(
                painter,
                center,
                CLOCK_R * 0.72,
                CLOCK_R * 0.92,
                1.0,
                ENERGY_EMPTY,
            );
            ring(
                painter,
                center,
                CLOCK_R * 0.72,
                CLOCK_R * 0.92,
                energy_frac,
                ENERGY_FILL,
            );
            ring(
                painter,
                center,
                CLOCK_R * 0.42,
                CLOCK_R * 0.64,
                1.0,
                ARMOUR_EMPTY,
            );
            ring(
                painter,
                center,
                CLOCK_R * 0.42,
                CLOCK_R * 0.64,
                fill_fraction(status.armor, status.max_armor),
                ARMOUR_FILL,
            );
            ticks(painter, center, CLOCK_R * 0.92, CLOCK_R * 1.0, 12);
            ticks(painter, center, CLOCK_R * 0.64, CLOCK_R * 0.70, 8);
            shield_glyph(painter, center, CLOCK_R * 0.22);

            let text_x = center.x + CLOCK_R + 12.0;
            let font = FontId::proportional(14.0);
            painter.text(
                Pos2::new(text_x, center.y - 18.0),
                Align2::LEFT_CENTER,
                format!("Armour {}/{}", status.armor, status.max_armor),
                font.clone(),
                LABEL,
            );
            let energy_line = match status.energy {
                Some((v, m)) => format!("Energy {v}/{m}"),
                None => "Energy --/--".to_string(),
            };
            painter.text(
                Pos2::new(text_x, center.y + 4.0),
                Align2::LEFT_CENTER,
                energy_line,
                font.clone(),
                LABEL,
            );

            let mut note_y = rect.min.y + CLOCK_R * 2.0 + 14.0;
            if let Some(text) = status.warning {
                painter.text(
                    Pos2::new(rect.min.x + 8.0, note_y),
                    Align2::LEFT_CENTER,
                    text,
                    font.clone(),
                    WARNING,
                );
                note_y += 18.0;
            }
            if let Some(text) = status.info {
                painter.text(
                    Pos2::new(rect.min.x + 8.0, note_y),
                    Align2::LEFT_CENTER,
                    text,
                    font,
                    INFO,
                );
            }
        });
}

fn draw_spiral(ctx: &Context, charge: u8, capacity: u8) {
    let width = CLOCK_R * 2.0 + 130.0;
    let height = CLOCK_R * 2.0 + 16.0;
    egui::Area::new(egui::Id::new("hud-spiral"))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-10.0, -10.0))
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let painter = ui.painter();
            draw_plate(painter, rect);
            let center = Pos2::new(rect.max.x - CLOCK_R - 8.0, rect.center().y);
            draw_bezel(painter, center, CLOCK_R);
            wedges(
                painter,
                center,
                CLOCK_R * 0.38,
                CLOCK_R * 0.88,
                charge,
                capacity,
            );
            ticks(
                painter,
                center,
                CLOCK_R * 0.88,
                CLOCK_R * 0.98,
                capacity.max(1) as usize,
            );
            spiral_glyph(painter, center, CLOCK_R * 0.22);
            let font = FontId::proportional(14.0);
            painter.text(
                Pos2::new(center.x - CLOCK_R - 12.0, center.y),
                Align2::RIGHT_CENTER,
                format!("Spiral {charge}/{capacity}"),
                font,
                LABEL,
            );
        });
}

fn draw_plate(painter: &egui::Painter, rect: Rect) {
    let r = rect.expand(4.0);
    painter.rect_filled(r, 16.0, PLATE);
    painter.rect_stroke(
        r,
        16.0,
        Stroke::new(2.0_f32, BEZEL_MID),
        egui::StrokeKind::Inside,
    );
    painter.rect_stroke(
        r.shrink(2.0),
        14.0,
        Stroke::new(1.0_f32, BEZEL_HIGH),
        egui::StrokeKind::Inside,
    );
}

fn draw_bezel(painter: &egui::Painter, center: Pos2, radius: f32) {
    // Organic lugs / rivets around the clock, then the metal ring.
    for i in 0..8 {
        let a = i as f32 * std::f32::consts::TAU / 8.0;
        let p = center + Vec2::angled(a) * (radius + 4.0);
        painter.circle_filled(p, 5.0, BEZEL_OUTER);
        if i % 2 == 0 {
            painter.circle_filled(p, 1.7, BEZEL_HIGH);
            painter.circle_stroke(p, 2.4, Stroke::new(1.0_f32, BEZEL_MID));
        }
    }
    painter.circle_filled(center, radius + 4.0, BEZEL_OUTER);
    painter.circle_stroke(center, radius + 2.5, Stroke::new(3.0_f32, BEZEL_MID));
    painter.circle_stroke(center, radius + 0.8, Stroke::new(1.4_f32, BEZEL_HIGH));
    painter.circle_filled(center, radius - 1.0, WELL);
}

/// Annular sector, clockwise from 12 o'clock, as a thick stroked arc.
/// A ring sector is not convex, so we stroke the mid-radius instead of
/// filling a path.
fn ring(painter: &egui::Painter, center: Pos2, inner: f32, outer: f32, frac: f32, color: Color32) {
    let frac = frac.clamp(0.0, 1.0);
    if frac <= 0.0 {
        return;
    }
    let mid = (inner + outer) * 0.5;
    let width = (outer - inner).max(1.0);
    let start = -std::f32::consts::FRAC_PI_2;
    // egui y grows down, so a positive angle from `Vec2::angled` is clockwise.
    let end = start + std::f32::consts::TAU * frac;
    let steps = ((end - start).abs() * mid / 2.0).ceil().max(2.0) as usize;
    let mut points = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = start + (end - start) * t;
        points.push(center + Vec2::angled(a) * mid);
    }
    painter.add(PathShape::line(points, PathStroke::new(width, color)));
}

fn ticks(painter: &egui::Painter, center: Pos2, inner: f32, outer: f32, count: usize) {
    let count = count.max(1);
    for i in 0..count {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (i as f32 / count as f32);
        let dir = Vec2::angled(a);
        painter.line_segment(
            [center + dir * inner, center + dir * outer],
            Stroke::new(1.2_f32, BEZEL_HIGH),
        );
    }
}

/// Discrete capacity wedges for the spiral, with a gap so the count reads
/// without colour.
fn wedges(painter: &egui::Painter, center: Pos2, inner: f32, outer: f32, charge: u8, capacity: u8) {
    let slots = capacity.max(1) as usize;
    let filled = (charge as usize).min(slots);
    let gap = 0.08_f32;
    let span = std::f32::consts::TAU / slots as f32;
    let start0 = -std::f32::consts::FRAC_PI_2;
    for i in 0..slots {
        let a0 = start0 + span * i as f32 + gap * 0.5;
        let a1 = start0 + span * (i as f32 + 1.0) - gap * 0.5;
        let color = if i < filled {
            SPIRAL_FILL
        } else {
            SPIRAL_EMPTY
        };
        let steps = 6;
        let mut points = Vec::with_capacity((steps + 1) * 2);
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            let a = a0 + (a1 - a0) * t;
            points.push(center + Vec2::angled(a) * outer);
        }
        for s in (0..=steps).rev() {
            let t = s as f32 / steps as f32;
            let a = a0 + (a1 - a0) * t;
            points.push(center + Vec2::angled(a) * inner);
        }
        // Split into triangles from the first outer point so each piece
        // is convex (`PathShape` fill requires it).
        if points.len() >= 3 {
            let mid_r = (inner + outer) * 0.5;
            let mid_a = (a0 + a1) * 0.5;
            let mid = center + Vec2::angled(mid_a) * mid_r;
            for w in points.windows(2) {
                painter.add(PathShape::convex_polygon(
                    vec![mid, w[0], w[1]],
                    color,
                    PathStroke::NONE,
                ));
            }
        }
    }
}

fn shield_glyph(painter: &egui::Painter, center: Pos2, size: f32) {
    let pts = vec![
        center + Vec2::new(-size * 0.7, -size * 0.55),
        center + Vec2::new(size * 0.7, -size * 0.55),
        center + Vec2::new(size * 0.7, size * 0.05),
        center + Vec2::new(0.0, size * 0.85),
        center + Vec2::new(-size * 0.7, size * 0.05),
    ];
    painter.add(PathShape::convex_polygon(
        pts,
        ARMOUR_FILL,
        PathStroke::new(1.0_f32, BEZEL_HIGH),
    ));
}

fn spiral_glyph(painter: &egui::Painter, center: Pos2, size: f32) {
    let mut points = Vec::with_capacity(32);
    for i in 0..32 {
        let t = i as f32 / 31.0;
        let a = t * std::f32::consts::TAU * 2.2 - std::f32::consts::FRAC_PI_2;
        let r = size * (0.15 + 0.85 * t);
        points.push(center + Vec2::angled(a) * r);
    }
    painter.add(PathShape::line(
        points,
        PathStroke::new(1.8_f32, SPIRAL_FILL),
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_fraction_clamps() {
        assert_eq!(fill_fraction(0, 100), 0.0);
        assert_eq!(fill_fraction(50, 100), 0.5);
        assert_eq!(fill_fraction(100, 100), 1.0);
        assert_eq!(fill_fraction(150, 100), 1.0);
        assert_eq!(fill_fraction(10, 0), 0.0);
    }

    #[test]
    fn from_units_uses_the_original_scale() {
        let status = Status::from_units(10_000, 10_000, Some((5_000, 8_000)), Some((2, 4)));
        assert_eq!(status.armor, 100);
        assert_eq!(status.max_armor, 100);
        assert_eq!(status.energy, Some((50, 80)));
        assert_eq!(status.spiral, Some((2, 4)));
    }
}
