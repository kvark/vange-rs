//! Gameplay HUD in the 1998 Vangers style.
//!
//! The original's actint clocks sit tight in the corners: `IND_NRG`
//! (armour + energy) top-left, `IND_SPIRAL` bottom-right with tick marks.
//! The painted BML frames are proprietary and are not always in the remake's
//! packs, so this module draws the clocks procedurally in egui — native and
//! web share the same painter. Nothing here changes multiplayer behaviour
//! or the under-slab camera.
//!
//! Gauges never rely on colour alone: each has a pie-slice fill angle,
//! tick marks, and an `N/M` readout.

use crate::combat::ARMOR_UNIT;
use egui::epaint::{PathShape, PathStroke};
use egui::{Align2, Color32, Context, FontId, Order, Pos2, Sense, Stroke, Vec2};

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

// Machined bronze / steel bezel, close to the 1998 actint clocks.
const BEZEL_CASE: Color32 = Color32::from_rgb(28, 24, 18);
const BEZEL_MID: Color32 = Color32::from_rgb(118, 100, 64);
const BEZEL_GROOVE: Color32 = Color32::from_rgb(16, 13, 10);
const BEZEL_LIP: Color32 = Color32::from_rgb(210, 192, 144);
const BEZEL_SHEEN: Color32 = Color32::from_rgb(168, 152, 110);
const WELL: Color32 = Color32::from_rgb(10, 9, 8);
const LABEL: Color32 = Color32::from_rgb(230, 220, 190);
const LABEL_SHADOW: Color32 = Color32::from_rgba_premultiplied(0, 0, 0, 200);
const ARMOUR_FILL: Color32 = Color32::from_rgb(214, 126, 40);
const ENERGY_FILL: Color32 = Color32::from_rgb(72, 168, 96);
const SPIRAL_FILL: Color32 = Color32::from_rgb(196, 48, 40);
const SPIRAL_EMPTY: Color32 = Color32::from_rgb(44, 16, 14);
const WARNING: Color32 = Color32::from_rgb(255, 140, 70);
const INFO: Color32 = Color32::from_rgb(210, 200, 170);

/// Small enough to sit in a corner the way the 800x600 actint clocks did
/// (radius 20 there), scaled up a little so the ticks and text stay readable
/// at 1280x800.
const CLOCK_R: f32 = 28.0;
const BEZEL: f32 = 7.0;
const MARGIN: f32 = 6.0;
const LABEL_W: f32 = 118.0;

/// Draw the gameplay gauges over the world view.
pub fn draw(ctx: &Context, status: &Status<'_>) {
    draw_nrg(ctx, status);
    if let Some((charge, capacity)) = status.spiral {
        draw_spiral(ctx, charge, capacity);
    }
}

fn draw_nrg(ctx: &Context, status: &Status<'_>) {
    let note_lines = status.warning.is_some() as i32 + status.info.is_some() as i32;
    let dial = CLOCK_R + BEZEL;
    let width = dial * 2.0 + 10.0 + LABEL_W;
    let height = (dial * 2.0).max(36.0 + note_lines as f32 * 16.0);
    egui::Area::new(egui::Id::new("hud-nrg"))
        .anchor(Align2::LEFT_TOP, Vec2::new(MARGIN, MARGIN))
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let painter = ui.painter();
            let center = rect.min + Vec2::new(dial, dial);
            draw_bezel(painter, center, CLOCK_R);

            // Energy is the outer pie, armour a smaller pie in the same
            // well — one clock, like `IND_NRG`, both readable as fill angles.
            if let Some((v, m)) = status.energy {
                pie(
                    painter,
                    center,
                    CLOCK_R - 1.0,
                    fill_fraction(v, m),
                    ENERGY_FILL,
                );
            }
            pie(
                painter,
                center,
                CLOCK_R * 0.55,
                fill_fraction(status.armor, status.max_armor),
                ARMOUR_FILL,
            );
            ticks(painter, center, CLOCK_R - 0.5, CLOCK_R + 2.2, 12);

            let font = FontId::proportional(12.0);
            let text_x = center.x + dial + 8.0;
            let armour_line = format!("Armour {}/{}", status.armor, status.max_armor);
            let energy_line = match status.energy {
                Some((v, m)) => format!("Energy {v}/{m}"),
                None => "Energy --/--".to_string(),
            };
            shield_glyph(painter, Pos2::new(text_x + 6.0, center.y - 14.0), 6.0);
            label(
                painter,
                Pos2::new(text_x + 16.0, center.y - 14.0),
                Align2::LEFT_CENTER,
                &armour_line,
                font.clone(),
                LABEL,
            );
            label(
                painter,
                Pos2::new(text_x + 16.0, center.y + 4.0),
                Align2::LEFT_CENTER,
                &energy_line,
                font.clone(),
                LABEL,
            );

            let mut note_y = center.y + dial + 6.0;
            if let Some(text) = status.warning {
                label(
                    painter,
                    Pos2::new(rect.min.x, note_y),
                    Align2::LEFT_TOP,
                    text,
                    font.clone(),
                    WARNING,
                );
                note_y += 16.0;
            }
            if let Some(text) = status.info {
                label(
                    painter,
                    Pos2::new(rect.min.x, note_y),
                    Align2::LEFT_TOP,
                    text,
                    font,
                    INFO,
                );
            }
        });
}

fn draw_spiral(ctx: &Context, charge: u8, capacity: u8) {
    let dial = CLOCK_R + BEZEL;
    let width = dial * 2.0 + 10.0 + LABEL_W;
    let height = dial * 2.0;
    egui::Area::new(egui::Id::new("hud-spiral"))
        .anchor(Align2::RIGHT_BOTTOM, Vec2::new(-MARGIN, -MARGIN))
        .order(Order::Foreground)
        .interactable(false)
        .show(ctx, |ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            let painter = ui.painter();
            let center = Pos2::new(rect.max.x - dial, rect.center().y);
            draw_bezel(painter, center, CLOCK_R);
            wedges(
                painter,
                center,
                CLOCK_R * 0.28,
                CLOCK_R - 1.0,
                charge,
                capacity,
            );
            ticks(
                painter,
                center,
                CLOCK_R - 0.5,
                CLOCK_R + 2.2,
                capacity.max(1) as usize,
            );
            let font = FontId::proportional(12.0);
            let text = format!("Spiral {charge}/{capacity}");
            let text_x = center.x - dial - 8.0;
            spiral_glyph(painter, Pos2::new(text_x - 10.0, center.y), 7.0);
            label(
                painter,
                Pos2::new(text_x - 20.0, center.y),
                Align2::RIGHT_CENTER,
                &text,
                font,
                LABEL,
            );
        });
}

/// Thick, turned-metal case: dark body, a groove, a bright inner lip, and a
/// short sheen on the upper-left — the 1998 clocks in brief.
fn draw_bezel(painter: &egui::Painter, center: Pos2, radius: f32) {
    painter.circle_filled(center, radius + BEZEL, BEZEL_CASE);
    painter.circle_stroke(
        center,
        radius + BEZEL - 1.0,
        Stroke::new(2.4_f32, BEZEL_MID),
    );
    painter.circle_stroke(
        center,
        radius + BEZEL - 3.2,
        Stroke::new(1.4_f32, BEZEL_GROOVE),
    );
    painter.circle_stroke(center, radius + 1.1, Stroke::new(2.0_f32, BEZEL_LIP));
    // Upper-left sheen, a short arc, so the rim reads as machined metal.
    let sheen_start = -std::f32::consts::PI * 0.95;
    let sheen_end = -std::f32::consts::PI * 0.45;
    let steps = 8;
    let mut pts = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let a = sheen_start + (sheen_end - sheen_start) * t;
        pts.push(center + Vec2::angled(a) * (radius + BEZEL - 1.6));
    }
    painter.add(PathShape::line(pts, PathStroke::new(1.6_f32, BEZEL_SHEEN)));
    painter.circle_filled(center, radius, WELL);
}

/// Pie slice, clockwise from 12 o'clock. Tessellated as triangles so each
/// piece stays convex (`PathShape` fill requires it).
fn pie(painter: &egui::Painter, center: Pos2, radius: f32, frac: f32, color: Color32) {
    let frac = frac.clamp(0.0, 1.0);
    if frac <= 0.0 || radius <= 0.0 {
        return;
    }
    if frac >= 1.0 {
        painter.circle_filled(center, radius, color);
        return;
    }
    let start = -std::f32::consts::FRAC_PI_2;
    let end = start + std::f32::consts::TAU * frac;
    let steps = ((end - start).abs() * radius / 1.8).ceil().max(2.0) as usize;
    let mut prev = center + Vec2::angled(start) * radius;
    for i in 1..=steps {
        let t = i as f32 / steps as f32;
        let a = start + (end - start) * t;
        let next = center + Vec2::angled(a) * radius;
        painter.add(PathShape::convex_polygon(
            vec![center, prev, next],
            color,
            PathStroke::NONE,
        ));
        prev = next;
    }
}

fn ticks(painter: &egui::Painter, center: Pos2, inner: f32, outer: f32, count: usize) {
    let count = count.max(1);
    for i in 0..count {
        let a = -std::f32::consts::FRAC_PI_2 + std::f32::consts::TAU * (i as f32 / count as f32);
        let dir = Vec2::angled(a);
        painter.line_segment(
            [center + dir * inner, center + dir * outer],
            Stroke::new(1.2_f32, BEZEL_LIP),
        );
    }
}

/// Discrete capacity wedges for the spiral, with a gap so the count reads
/// without colour.
fn wedges(painter: &egui::Painter, center: Pos2, inner: f32, outer: f32, charge: u8, capacity: u8) {
    let slots = capacity.max(1) as usize;
    let filled = (charge as usize).min(slots);
    let gap = 0.10_f32;
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
        // A slim ring-sector of pie triangles between inner and outer.
        let steps = 6;
        let mut prev_o = center + Vec2::angled(a0) * outer;
        let mut prev_i = center + Vec2::angled(a0) * inner;
        for s in 1..=steps {
            let t = s as f32 / steps as f32;
            let a = a0 + (a1 - a0) * t;
            let next_o = center + Vec2::angled(a) * outer;
            let next_i = center + Vec2::angled(a) * inner;
            painter.add(PathShape::convex_polygon(
                vec![prev_i, prev_o, next_o],
                color,
                PathStroke::NONE,
            ));
            painter.add(PathShape::convex_polygon(
                vec![prev_i, next_o, next_i],
                color,
                PathStroke::NONE,
            ));
            prev_o = next_o;
            prev_i = next_i;
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
        PathStroke::new(1.0_f32, BEZEL_LIP),
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
        PathStroke::new(1.6_f32, SPIRAL_FILL),
    ));
}

fn label(
    painter: &egui::Painter,
    pos: Pos2,
    align: Align2,
    text: &str,
    font: FontId,
    color: Color32,
) {
    painter.text(
        pos + Vec2::new(1.0, 1.0),
        align,
        text,
        font.clone(),
        LABEL_SHADOW,
    );
    painter.text(pos, align, text, font, color);
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
