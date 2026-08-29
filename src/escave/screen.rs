//! In and out of an escave: gate shutters, then talk and trade.
//!
//! The original swaps the road for a 2.5D iscreen (`RTO_ESCAVE_ID`) after
//! `EXTERNAL_MODE_ESCAVE_IN` waits out the gate. Space opens the hatch;
//! the visit starts once the car has fallen in. The interior is shop on
//! the left, the mechos hex on the right, counselor along the bottom.

use super::layout::{Cell, Layout};
use super::preview::SpinMesh;
use super::{
    DropTarget, Hand, Inventory, Kind, Preview, Shop, Visit, drop_held, preview, preview_good,
};
use std::collections::HashMap;
use std::path::Path;

/// Camera dive, then the gates close. Seconds.
pub const ENTER_SECS: f32 = 1.6;
/// Gates close over the interior, then open onto the road. Seconds.
pub const LEAVE_SECS: f32 = 0.9;

const DIVE_END: f32 = 0.45;
const CLOSE_END: f32 = 0.70;
const LEAVE_CLOSE: f32 = 0.45;

const BG: egui::Color32 = egui::Color32::from_rgb(16, 12, 8);
const PANEL: egui::Color32 = egui::Color32::from_rgb(36, 26, 16);
const INK: egui::Color32 = egui::Color32::from_rgb(232, 208, 144);
const MUTED: egui::Color32 = egui::Color32::from_rgb(168, 140, 96);
const ACCENT: egui::Color32 = egui::Color32::from_rgb(200, 140, 48);
const GATE: egui::Color32 = egui::Color32::from_rgb(6, 4, 3);
const GATE_EDGE: egui::Color32 = egui::Color32::from_rgb(180, 120, 40);

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn ramp(t: f32, a: f32, b: f32) -> f32 {
    if b <= a {
        return 1.0;
    }
    smooth((t - a) / (b - a))
}

enum Phase {
    World,
    Enter {
        name: String,
        t: f32,
        visit: Option<Visit>,
    },
    Inside(Visit),
    Leave {
        t: f32,
        visit: Option<Visit>,
    },
}

/// Occupies the screen while the player is going into, sitting in, or
/// leaving an escave.
pub struct Screen {
    phase: Phase,
}

impl Default for Screen {
    fn default() -> Self {
        Screen {
            phase: Phase::World,
        }
    }
}

impl Screen {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_world(&self) -> bool {
        matches!(self.phase, Phase::World)
    }

    /// Car, fauna, and the rest of the road freeze for the visit.
    /// After the interior hides on the way out, physics runs so the
    /// impulse can throw the car while the gates are still opening.
    pub fn blocks_drive(&self) -> bool {
        match self.phase {
            Phase::World => false,
            Phase::Leave { ref visit, .. } => visit.is_some(),
            Phase::Enter { .. } | Phase::Inside(_) => true,
        }
    }

    /// Chase-camera while the gates open back onto the road, and on the
    /// surface. Frozen while diving in and while talking.
    pub fn follows_camera(&self) -> bool {
        match self.phase {
            Phase::World => true,
            Phase::Leave { ref visit, .. } => visit.is_none(),
            Phase::Enter { .. } | Phase::Inside(_) => false,
        }
    }

    pub fn visit(&self) -> Option<&Visit> {
        match self.phase {
            Phase::World => None,
            Phase::Inside(ref visit) => Some(visit),
            Phase::Enter { ref visit, .. } | Phase::Leave { ref visit, .. } => visit.as_ref(),
        }
    }

    /// Escave we are entering or sitting in, for the iscreen map.
    pub fn cave_name(&self) -> Option<&str> {
        match self.phase {
            Phase::Enter { ref name, .. } => Some(name.as_str()),
            Phase::Inside(ref visit) => Some(visit.name.as_str()),
            Phase::Leave { ref visit, .. } => visit.as_ref().map(|v| v.name.as_str()),
            Phase::World => None,
        }
    }

    pub fn visit_mut(&mut self) -> Option<&mut Visit> {
        match self.phase {
            Phase::World => None,
            Phase::Inside(ref mut visit) => Some(visit),
            Phase::Enter { ref mut visit, .. } | Phase::Leave { ref mut visit, .. } => {
                visit.as_mut()
            }
        }
    }

    /// 0 = gates open, 1 = fully shut.
    pub fn shutter(&self) -> f32 {
        match self.phase {
            Phase::World | Phase::Inside(_) => 0.0,
            Phase::Enter { t, ref visit, .. } => {
                if visit.is_none() {
                    ramp(t, DIVE_END, CLOSE_END)
                } else {
                    1.0 - ramp(t, CLOSE_END, 1.0)
                }
            }
            Phase::Leave { t, ref visit, .. } => {
                if visit.is_some() {
                    ramp(t, 0.0, LEAVE_CLOSE)
                } else {
                    1.0 - ramp(t, LEAVE_CLOSE, 1.0)
                }
            }
        }
    }

    /// Smooth 0..1 blend from the chase camera to the pad during the dive.
    /// `None` means leave the camera alone.
    pub fn camera_blend(&self) -> Option<f32> {
        match self.phase {
            Phase::Enter { t, .. } => Some(smooth((t / DIVE_END).min(1.0))),
            _ => None,
        }
    }

    pub fn begin_enter(&mut self, name: String) {
        if !matches!(self.phase, Phase::World) {
            return;
        }
        self.phase = Phase::Enter {
            name,
            t: 0.0,
            visit: None,
        };
    }

    pub fn begin_leave(&mut self) {
        let visit = match self.phase {
            Phase::Inside(_) | Phase::Enter { visit: Some(_), .. } => {
                match std::mem::replace(&mut self.phase, Phase::World) {
                    Phase::Inside(visit) => visit,
                    Phase::Enter {
                        visit: Some(visit), ..
                    } => visit,
                    other => {
                        self.phase = other;
                        return;
                    }
                }
            }
            _ => return,
        };
        self.phase = Phase::Leave {
            t: 0.0,
            visit: Some(visit),
        };
    }

    pub fn step(&mut self, dt: f32, data_path: &Path) {
        match self.phase {
            Phase::Enter { .. } => self.step_enter(dt, data_path),
            Phase::Leave { .. } => self.step_leave(dt),
            Phase::World | Phase::Inside(_) => {}
        }
    }

    fn step_enter(&mut self, dt: f32, data_path: &Path) {
        let done = {
            let &mut Phase::Enter {
                ref mut t,
                ref name,
                ref mut visit,
            } = &mut self.phase
            else {
                return;
            };
            *t = (*t + dt / ENTER_SECS).min(1.0);
            if *t >= CLOSE_END && visit.is_none() {
                *visit = Some(open_visit(name, data_path));
            }
            *t >= 1.0
        };
        if !done {
            return;
        }
        let visit = match std::mem::replace(&mut self.phase, Phase::World) {
            Phase::Enter { visit, name, .. } => visit.unwrap_or(Visit {
                name,
                session: None,
            }),
            other => {
                self.phase = other;
                return;
            }
        };
        self.phase = Phase::Inside(visit);
    }

    fn step_leave(&mut self, dt: f32) {
        let done = {
            let &mut Phase::Leave {
                ref mut t,
                ref mut visit,
            } = &mut self.phase
            else {
                return;
            };
            *t = (*t + dt / LEAVE_SECS).min(1.0);
            if *t >= LEAVE_CLOSE {
                *visit = None;
            }
            *t >= 1.0
        };
        if done {
            self.phase = Phase::World;
        }
    }
}

fn open_visit(name: &str, data_path: &Path) -> Visit {
    let mut visit = Visit::enter(name, data_path);
    if let Some(ref mut session) = visit.session {
        session.next_phrase();
    }
    visit
}

/// Clicks and drops on the interior this frame.
#[derive(Clone, Debug, Default)]
pub struct InteriorAction {
    pub next_phrase: bool,
    pub ask: Option<String>,
    pub buy: Option<String>,
    pub sell: Option<usize>,
    pub equip: Option<(usize, usize)>,
    pub unequip: Option<usize>,
    pub drop: Option<(Hand, DropTarget)>,
    pub leave: bool,
}

impl InteriorAction {
    /// Talk and trade. The status line is for the shop; `true` means a
    /// weapon bay changed and the mechos slots need hanging again.
    pub fn apply(
        &self,
        visit: &mut Visit,
        shop: &mut Shop,
        inventory: &mut Inventory,
        credits: &mut i32,
    ) -> (Option<String>, bool) {
        if self.next_phrase
            && let Some(ref mut session) = visit.session
        {
            session.next_phrase();
        }
        if let Some(ref q) = self.ask
            && let Some(ref mut session) = visit.session
        {
            let _ = session.answer(q);
        }
        let mut note = None;
        let mut slots = false;
        if let Some((hand, target)) = self.drop.clone() {
            let touches_bay =
                matches!(hand, Hand::Bay { .. }) || matches!(target, DropTarget::Bay { .. });
            match drop_held(shop, inventory, credits, hand.clone(), target.clone()) {
                Ok(()) => {
                    let text = drop_note(&hand, &target);
                    if !text.is_empty() {
                        note = Some(text);
                    }
                    slots = touches_bay;
                }
                Err(err) => note = Some(err.to_string()),
            }
        }
        if self.drop.is_none() {
            if let Some(ref id) = self.buy {
                note = Some(match shop.buy(id, inventory, credits) {
                    Ok(()) => format!("Bought {id}"),
                    Err(err) => err.to_string(),
                });
            }
            if let Some(i) = self.sell {
                note = Some(match shop.sell(i, inventory, credits) {
                    Ok(()) => "Sold".to_string(),
                    Err(err) => err.to_string(),
                });
            }
            if let Some((cargo, bay)) = self.equip
                && inventory.equip(cargo, bay).is_ok()
            {
                slots = true;
            }
            if let Some(bay) = self.unequip
                && inventory.unequip(bay).is_ok()
            {
                slots = true;
            }
        }
        (note, slots)
    }
}

fn drop_note(hand: &Hand, target: &DropTarget) -> String {
    match (hand, target) {
        (&Hand::Shop { ref id }, &DropTarget::Cargo { .. })
        | (&Hand::Shop { ref id }, &DropTarget::Bay { .. }) => format!("Bought {id}"),
        (&Hand::Cargo { .. }, &DropTarget::Shop) | (&Hand::Bay { .. }, &DropTarget::Shop) => {
            "Sold".to_string()
        }
        (&Hand::Cargo { .. }, &DropTarget::Bay { .. })
        | (&Hand::Bay { .. }, &DropTarget::Cargo { .. })
        | (&Hand::Bay { .. }, &DropTarget::Bay { .. }) => "Equipped".to_string(),
        (&Hand::Cargo { .. }, &DropTarget::Cargo { .. }) => "Moved".to_string(),
        (&Hand::Shop { .. }, &DropTarget::Shop) => String::new(),
    }
}

/// Talk/trade overlay. Background order so Tweaks/Settings stay clickable.
///
/// Original 800×600 shop: spinning AVI top-left, item list under it,
/// mechos matrix on the right, price/info along the bottom. Scaled to
/// the current window; the cave map shows through the gaps.
pub fn draw_interior(
    ctx: &egui::Context,
    visit: &Visit,
    shop: &Shop,
    inventory: &Inventory,
    beebs: i32,
    note: Option<&str>,
    selected: &mut Option<String>,
    spins: &HashMap<String, SpinMesh>,
    see_through: bool,
) -> InteriorAction {
    if selected.is_none() {
        *selected = shop.stock().first().map(|g| g.id.clone());
    }
    let spin = selected.as_ref().and_then(|id| spins.get(id.as_str()));
    let mut action = InteriorAction::default();
    let rect = ctx.content_rect();
    let veil = if see_through {
        egui::Color32::from_rgba_unmultiplied(8, 6, 4, 40)
    } else {
        BG
    };
    let panel = if see_through {
        egui::Color32::from_rgba_unmultiplied(36, 26, 16, 220)
    } else {
        PANEL
    };
    egui::Area::new(egui::Id::new("escave-interior"))
        .fixed_pos(rect.left_top())
        .order(egui::Order::Background)
        .show(ctx, |ui| {
            ui.painter().rect_filled(rect, 0.0, veil);
            ui.set_min_size(rect.size());
            egui::Frame::new()
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(rich("Leave", ACCENT))).clicked() {
                            action.leave = true;
                        }
                        ui.label(
                            egui::RichText::new(visit.name.to_uppercase())
                                .color(ACCENT)
                                .size(20.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(rich(format!("Beebs: {beebs}"), INK));
                            if let Some(note) = note {
                                ui.label(rich(note, MUTED));
                            }
                        });
                    });
                    let avail = ui.available_size();
                    let talk_h = 96.0;
                    let body_h = (avail.y - talk_h - 8.0).max(220.0);
                    let left_w = (avail.x * 0.36).clamp(280.0, 440.0);
                    let mech = mechos_panel_size(inventory.layout());
                    let gap = (avail.x - left_w - mech.x - 12.0).max(16.0);
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(
                            egui::vec2(left_w, body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                draw_shop(ui, panel, shop, inventory, selected, spin, &mut action);
                            },
                        );
                        ui.add_space(gap);
                        ui.allocate_ui_with_layout(
                            egui::vec2(mech.x, body_h),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                egui::Frame::new()
                                    .fill(panel)
                                    .inner_margin(egui::Margin::same(10))
                                    .show(ui, |ui| {
                                        draw_mechos(ui, inventory, selected, &mut action);
                                    });
                            },
                        );
                    });
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(panel)
                        .inner_margin(egui::Margin::same(8))
                        .show(ui, |ui| {
                            draw_talk(ui, visit, &mut action);
                        });
                });
        });
    action
}

fn rich(text: impl Into<String>, color: egui::Color32) -> egui::RichText {
    egui::RichText::new(text).color(color)
}

fn draw_talk(ui: &mut egui::Ui, visit: &Visit, action: &mut InteriorAction) {
    ui.horizontal(|ui| {
        ui.label(rich("Counselor", ACCENT).size(14.0));
        let Some(ref session) = visit.session else {
            ui.label(rich("The counselor is silent.", MUTED));
            return;
        };
        if !session.ended() && ui.button(rich("Next", INK)).clicked() {
            action.next_phrase = true;
        }
        for q in session.queries() {
            let label = session.query_prompt(q);
            if ui.button(rich(label, INK)).clicked() {
                action.ask = Some(q.clone());
            }
        }
    });
    let Some(ref session) = visit.session else {
        return;
    };
    let phrase = session.last_phrase().unwrap_or("");
    egui::ScrollArea::vertical()
        .max_height(52.0)
        .id_salt("escave-phrase")
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(rich(phrase, INK).size(14.0))
                    .wrap()
                    .selectable(false),
            );
        });
}

/// Pointy-top hex radius in UI points. Original matrix sat ~300 px wide
/// for 8 columns on an 800-wide screen.
const HEX_R: f32 = 18.0;
const CELL_EMPTY: egui::Color32 = egui::Color32::from_rgb(42, 30, 18);
const CELL_BAY: egui::Color32 = egui::Color32::from_rgb(64, 46, 26);
const CELL_DEAD: egui::Color32 = egui::Color32::from_rgb(22, 16, 12);

fn hex_pitch() -> (f32, f32) {
    (HEX_R * 3.0_f32.sqrt(), HEX_R * 1.5)
}

/// Inclusive (min_x, min_y, max_x, max_y) of cells that are part of the mechos.
fn layout_bounds(layout: &Layout) -> Option<(i32, i32, i32, i32)> {
    let mut min_x = layout.width;
    let mut min_y = layout.height;
    let mut max_x = -1;
    let mut max_y = -1;
    for y in 0..layout.height {
        for x in 0..layout.width {
            if layout.cell(x, y) == Cell::Empty {
                continue;
            }
            min_x = min_x.min(x);
            min_y = min_y.min(y);
            max_x = max_x.max(x);
            max_y = max_y.max(y);
        }
    }
    if max_x < 0 {
        None
    } else {
        Some((min_x, min_y, max_x, max_y))
    }
}

fn mechos_panel_size(layout: &Layout) -> egui::Vec2 {
    let Some((x0, y0, x1, y1)) = layout_bounds(layout) else {
        return egui::vec2(220.0, 220.0);
    };
    let cols = (x1 - x0 + 1) as f32;
    let rows = (y1 - y0 + 1) as f32;
    let (dx, dy) = hex_pitch();
    egui::vec2(
        HEX_R * 2.0 + (cols - 1.0) * dx + dx * 0.5 + 28.0,
        HEX_R * 2.0 + (rows - 1.0) * dy + 56.0,
    )
}

fn hex_center(origin: egui::Pos2, x: i32, y: i32) -> egui::Pos2 {
    let (dx, dy) = hex_pitch();
    let odd = Layout::row_offset(y);
    egui::pos2(
        origin.x + HEX_R + x as f32 * dx + if odd { dx * 0.5 } else { 0.0 },
        origin.y + HEX_R + y as f32 * dy,
    )
}

fn hex_points(center: egui::Pos2) -> Vec<egui::Pos2> {
    (0..6)
        .map(|i| {
            let a = (i as f32 + 0.5) * std::f32::consts::FRAC_PI_3;
            egui::pos2(center.x + HEX_R * a.cos(), center.y + HEX_R * a.sin())
        })
        .collect()
}

fn ware_fill(id: &str) -> egui::Color32 {
    let mut h = 0u32;
    for b in id.bytes() {
        h = h.wrapping_mul(16777619) ^ u32::from(b);
    }
    let r = 70 + (h & 0x3f) as u8;
    let g = 42 + ((h >> 6) & 0x2f) as u8;
    let b = 20 + ((h >> 12) & 0x1f) as u8;
    egui::Color32::from_rgb(r, g, b)
}

fn draw_shop(
    ui: &mut egui::Ui,
    panel: egui::Color32,
    shop: &Shop,
    inventory: &Inventory,
    selected: &mut Option<String>,
    spin: Option<&SpinMesh>,
    action: &mut InteriorAction,
) {
    let inner = ui.available_size();
    let video_h = (inner.x * 0.72).clamp(200.0, 280.0);
    egui::Frame::new()
        .fill(panel)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            draw_video(ui, selected.as_deref(), spin, video_h);
            ui.add_space(6.0);
            draw_stats(ui, shop, inventory, selected.as_deref());
        });
    ui.add_space(8.0);
    egui::Frame::new()
        .fill(panel)
        .inner_margin(egui::Margin::same(8))
        .show(ui, |ui| {
            draw_stock_list(ui, shop, selected, action);
        });
}

fn draw_video(ui: &mut egui::Ui, selected: Option<&str>, spin: Option<&SpinMesh>, height: f32) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), egui::Sense::hover());
    ui.painter()
        .rect_filled(rect, 6.0, egui::Color32::from_rgb(28, 20, 12));
    ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.5)),
        egui::StrokeKind::Inside,
    );
    if let Some(mesh) = spin {
        let angle = ui.input(|i| i.time) as f32;
        mesh.paint(ui.painter(), rect.shrink(10.0), angle, ACCENT);
        ui.ctx().request_repaint();
    } else if selected.is_some() {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "No mesh",
            egui::FontId::proportional(14.0),
            MUTED,
        );
    }
}

fn draw_stats(ui: &mut egui::Ui, shop: &Shop, inventory: &Inventory, selected: Option<&str>) {
    let Some(id) = selected else {
        ui.label(rich("Select a ware.", MUTED));
        return;
    };
    let stats = lookup_stats(shop, inventory, id);
    let kind = if stats.kind == Kind::Weapon {
        "gun"
    } else {
        "ware"
    };
    ui.label(rich(format!("{}  ·  {kind}", stats.name), INK).size(16.0));
    ui.label(rich(
        format!(
            "Buy {} beebs    Sell {} beebs",
            stats.buy_price, stats.sell_price
        ),
        ACCENT,
    ));
    if stats.description.is_empty() {
        ui.label(rich("No description.", MUTED));
    } else {
        ui.add(
            egui::Label::new(rich(&stats.description, INK).size(13.0))
                .wrap()
                .selectable(false),
        );
    }
}

fn lookup_stats(shop: &Shop, inventory: &Inventory, id: &str) -> Preview {
    shop.stock()
        .iter()
        .find(|g| g.id == id)
        .or_else(|| {
            inventory
                .cargo()
                .iter()
                .map(|p| &p.good)
                .find(|g| g.id == id)
        })
        .or_else(|| {
            inventory
                .bays()
                .iter()
                .filter_map(Option::as_ref)
                .find(|g| g.id == id)
        })
        .map(preview_good)
        .or_else(|| preview(id))
        .unwrap_or_else(|| Preview {
            id: id.to_string(),
            name: id.to_string(),
            kind: Kind::Ware,
            buy_price: 0,
            sell_price: 0,
            description: String::new(),
        })
}

fn draw_stock_list(
    ui: &mut egui::Ui,
    shop: &Shop,
    selected: &mut Option<String>,
    action: &mut InteriorAction,
) {
    ui.label(rich("Shop", ACCENT).size(14.0));
    ui.label(rich(
        "Click to preview. Drag onto the mechos to buy.",
        MUTED,
    ));
    let shop_frame = egui::Frame::new()
        .fill(egui::Color32::from_rgb(22, 16, 10))
        .inner_margin(egui::Margin::same(4));
    let (inner, dropped) = ui.dnd_drop_zone::<Hand, ()>(shop_frame, |ui| {
        if shop.stock().is_empty() {
            ui.label(rich("Nothing on the counter.", MUTED));
            return;
        }
        egui::ScrollArea::vertical()
            .id_salt("escave-stock")
            .max_height(ui.available_height().max(80.0))
            .show(ui, |ui| {
                for good in shop.stock() {
                    let marked = selected.as_deref() == Some(good.id.as_str());
                    let id = egui::Id::new(("shop-stock", good.id.as_str()));
                    let payload = Hand::Shop {
                        id: good.id.clone(),
                    };
                    let response = ui
                        .dnd_drag_source(id, payload, |ui| {
                            let color = if marked { ACCENT } else { INK };
                            let kind = if good.is_weapon() { "gun" } else { "ware" };
                            let label = format!(
                                "{}  {} · {} beebs",
                                good.display_name(),
                                kind,
                                good.buy_price
                            );
                            let fill = if marked {
                                egui::Color32::from_rgb(56, 38, 18)
                            } else {
                                egui::Color32::TRANSPARENT
                            };
                            egui::Frame::new()
                                .fill(fill)
                                .inner_margin(egui::Margin::symmetric(6, 4))
                                .show(ui, |ui| {
                                    ui.set_min_width(ui.available_width());
                                    ui.label(rich(label, color));
                                });
                        })
                        .response;
                    if response.clicked() {
                        *selected = Some(good.id.clone());
                    }
                }
            });
    });
    let _ = inner;
    if let Some(hand) = dropped.as_deref() {
        action.drop = Some((hand.clone(), DropTarget::Shop));
    }
}

fn draw_mechos(
    ui: &mut egui::Ui,
    inventory: &Inventory,
    selected: &mut Option<String>,
    action: &mut InteriorAction,
) {
    ui.label(rich("Mechos", ACCENT).size(16.0));
    ui.label(rich("Drag a shop ware onto a hex to buy.", MUTED));
    ui.add_space(6.0);
    let layout = inventory.layout();
    let Some((x0, y0, x1, y1)) = layout_bounds(layout) else {
        ui.label(rich("No board for this mechos.", MUTED));
        return;
    };
    let (dx, dy) = hex_pitch();
    let cols = (x1 - x0 + 1) as f32;
    let rows = (y1 - y0 + 1) as f32;
    let board_w = HEX_R * 2.0 + (cols - 1.0) * dx + dx * 0.5;
    let board_h = HEX_R * 2.0 + (rows - 1.0) * dy;
    let (board, _) = ui.allocate_exact_size(egui::vec2(board_w, board_h), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(board);
    for y in y0..=y1 {
        for x in x0..=x1 {
            let cell = layout.cell(x, y);
            if cell == Cell::Empty {
                continue;
            }
            let center = hex_center(board.min, x - x0, y - y0);
            let points = hex_points(center);
            let bbox = egui::Rect::from_center_size(
                center,
                egui::vec2(dx.max(HEX_R * 2.0) - 1.0, dy + HEX_R * 0.2),
            )
            .intersect(board);
            draw_hex_cell(
                ui,
                &painter,
                inventory,
                cell,
                (x, y),
                points,
                bbox,
                selected,
                action,
            );
        }
    }
}

fn draw_hex_cell(
    ui: &mut egui::Ui,
    painter: &egui::Painter,
    inventory: &Inventory,
    cell: Cell,
    origin: (i32, i32),
    points: Vec<egui::Pos2>,
    bbox: egui::Rect,
    selected: &mut Option<String>,
    action: &mut InteriorAction,
) {
    if bbox.width() < 4.0 || bbox.height() < 4.0 {
        return;
    }
    match cell {
        Cell::Empty => {
            painter.add(egui::Shape::convex_polygon(
                points,
                CELL_DEAD,
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(40, 28, 18)),
            ));
        }
        Cell::Cargo => {
            let occupant = inventory.occupant(origin);
            let is_origin = occupant.is_some_and(|(_, p)| p.origin == origin);
            let marked = occupant
                .map(|(_, p)| selected.as_deref() == Some(p.good.id.as_str()))
                .unwrap_or(false);
            let fill = occupant
                .map(|(_, p)| ware_fill(&p.good.id))
                .unwrap_or(CELL_EMPTY);
            let stroke = if marked {
                egui::Stroke::new(2.0_f32, ACCENT)
            } else {
                egui::Stroke::new(1.0_f32, egui::Color32::from_rgb(60, 42, 26))
            };
            painter.add(egui::Shape::convex_polygon(points, fill, stroke));
            if let Some((_, placed)) = occupant
                && is_origin
            {
                painter.text(
                    bbox.center(),
                    egui::Align2::CENTER_CENTER,
                    short_id(placed.good.display_name()),
                    egui::FontId::proportional(11.0),
                    if marked { ACCENT } else { INK },
                );
            }
            ui.scope_builder(egui::UiBuilder::new().max_rect(bbox), |ui| {
                let frame = egui::Frame::new().inner_margin(0);
                let (inner, dropped) = ui.dnd_drop_zone::<Hand, ()>(frame, |ui| {
                    if let Some((index, placed)) = occupant {
                        let payload = Hand::Cargo { index };
                        let id = egui::Id::new(("cargo-hex", origin.0, origin.1, index));
                        let response = ui
                            .dnd_drag_source(id, payload, |ui| {
                                ui.allocate_exact_size(bbox.size(), egui::Sense::click());
                            })
                            .response;
                        response.clone().on_hover_text(placed.good.display_name());
                        if response.clicked() {
                            *selected = Some(placed.good.id.clone());
                        }
                    } else {
                        ui.allocate_exact_size(bbox.size(), egui::Sense::hover());
                    }
                });
                let _ = inner;
                if let Some(hand) = dropped.as_deref() {
                    action.drop = Some((hand.clone(), DropTarget::Cargo { origin }));
                }
            });
        }
        Cell::Bay(index) => {
            let good = inventory.bay(index);
            let marked = good
                .map(|g| selected.as_deref() == Some(g.id.as_str()))
                .unwrap_or(false);
            let fill = good.map(|g| ware_fill(&g.id)).unwrap_or(CELL_BAY);
            let stroke = if marked {
                egui::Stroke::new(2.0_f32, ACCENT)
            } else {
                egui::Stroke::new(1.5_f32, egui::Color32::from_rgb(180, 140, 48))
            };
            painter.add(egui::Shape::convex_polygon(points, fill, stroke));
            let label = match good {
                Some(g) => short_id(g.display_name()),
                None => format!("W{}", index + 1),
            };
            painter.text(
                bbox.center(),
                egui::Align2::CENTER_CENTER,
                label,
                egui::FontId::proportional(11.0),
                if marked { ACCENT } else { INK },
            );
            ui.scope_builder(egui::UiBuilder::new().max_rect(bbox), |ui| {
                let frame = egui::Frame::new().inner_margin(0);
                let (inner, dropped) = ui.dnd_drop_zone::<Hand, ()>(frame, |ui| {
                    if let Some(good) = good {
                        let payload = Hand::Bay { index };
                        let id = egui::Id::new(("bay-hex", origin.0, origin.1, index));
                        let response = ui
                            .dnd_drag_source(id, payload, |ui| {
                                ui.allocate_exact_size(bbox.size(), egui::Sense::click());
                            })
                            .response;
                        response.clone().on_hover_text(good.display_name());
                        if response.clicked() {
                            *selected = Some(good.id.clone());
                        }
                    } else {
                        ui.allocate_exact_size(bbox.size(), egui::Sense::hover());
                    }
                });
                let _ = inner;
                if let Some(hand) = dropped.as_deref() {
                    action.drop = Some((hand.clone(), DropTarget::Bay { index }));
                }
            });
        }
    }
}

#[cfg(test)]
fn draw_preview(
    ui: &mut egui::Ui,
    shop: &Shop,
    inventory: &Inventory,
    selected: Option<&str>,
    spin: Option<&SpinMesh>,
) {
    draw_video(ui, selected, spin, 140.0);
    draw_stats(ui, shop, inventory, selected);
}

fn short_id(id: &str) -> String {
    let mut chars = id.chars();
    let take: String = chars.by_ref().take(5).collect();
    if chars.next().is_some() {
        format!("{take}…")
    } else {
        take
    }
}

/// Left and right gate leaves. `amount` 0 is open, 1 is shut.
pub fn draw_shutters(ctx: &egui::Context, amount: f32) {
    if amount <= 0.001 {
        return;
    }
    let rect = ctx.content_rect();
    let width = rect.width() * 0.5 * amount.clamp(0.0, 1.0);
    egui::Area::new(egui::Id::new("escave-shutters"))
        .fixed_pos(rect.left_top())
        .order(egui::Order::Foreground)
        .interactable(amount > 0.95)
        .show(ctx, |ui| {
            let painter = ui.painter();
            let left = egui::Rect::from_min_max(
                rect.left_top(),
                egui::pos2(rect.left() + width, rect.bottom()),
            );
            let right = egui::Rect::from_min_max(
                egui::pos2(rect.right() - width, rect.top()),
                rect.right_bottom(),
            );
            painter.rect_filled(left, 0.0, GATE);
            painter.rect_filled(right, 0.0, GATE);
            if amount > 0.02 {
                let stroke = egui::Stroke::new(2.0_f32, GATE_EDGE);
                painter.line_segment(
                    [
                        egui::pos2(left.right(), rect.top()),
                        egui::pos2(left.right(), rect.bottom()),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        egui::pos2(right.left(), rect.top()),
                        egui::pos2(right.left(), rect.bottom()),
                    ],
                    stroke,
                );
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cave_name_tracks_the_visit() {
        let mut screen = Screen::new();
        assert!(screen.cave_name().is_none());
        screen.begin_enter("Podish".into());
        assert_eq!(screen.cave_name(), Some("Podish"));
        screen.step(ENTER_SECS + 0.05, Path::new("."));
        assert_eq!(screen.cave_name(), Some("Podish"));
    }

    #[test]
    fn entering_closes_the_gates_then_opens_the_interior() {
        let mut screen = Screen::new();
        screen.begin_enter("Podish".into());
        assert_eq!(screen.shutter(), 0.0);
        screen.step(ENTER_SECS * 0.60, Path::new("."));
        assert!(
            screen.shutter() > 0.5,
            "gates should be closing, got {}",
            screen.shutter()
        );
        assert!(screen.visit().is_none());
        screen.step(ENTER_SECS, Path::new("."));
        assert!(screen.visit().is_some(), "interior after the gates open");
        assert!(screen.shutter() < 0.05);
        assert!(screen.blocks_drive());
        assert!(!screen.follows_camera());
    }

    #[test]
    fn leaving_returns_to_the_road() {
        let mut screen = Screen::new();
        screen.begin_enter("Podish".into());
        screen.step(ENTER_SECS + 0.05, Path::new("."));
        assert!(screen.visit().is_some());
        screen.begin_leave();
        assert!(screen.blocks_drive());
        screen.step(LEAVE_SECS * 0.50, Path::new("."));
        assert!(
            !screen.blocks_drive(),
            "car must be free to fly out while the gates open"
        );
        assert!(!screen.is_world());
        screen.step(LEAVE_SECS, Path::new("."));
        assert!(screen.is_world());
        assert_eq!(screen.shutter(), 0.0);
    }

    #[test]
    fn buying_from_the_interior_moves_stock() {
        let mut visit = Visit {
            name: "Podish".into(),
            session: None,
        };
        let mut shop = Shop::fostral();
        let mut inventory = Inventory::default();
        let mut beebs = 100;
        let action = InteriorAction {
            buy: Some("Nymbos".into()),
            ..InteriorAction::default()
        };
        let (note, slots) = action.apply(&mut visit, &mut shop, &mut inventory, &mut beebs);
        assert!(!slots);
        assert!(inventory.contains("Nymbos"));
        assert_eq!(beebs, 88);
        assert_eq!(note.as_deref(), Some("Bought Nymbos"));
    }

    fn viewport_input() -> egui::RawInput {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0));
        egui::RawInput {
            screen_rect: Some(rect),
            ..egui::RawInput::default()
        }
    }

    fn is_hex_path(shape: &egui::Shape) -> bool {
        #[allow(clippy::pattern_type_mismatch)]
        match shape {
            egui::Shape::Path(path) => {
                path.points.len() >= 6
                    && path.points.len() <= 7
                    && (path.points[0] - path.points[1]).length() > 4.0
            }
            _ => false,
        }
    }

    fn hex_paths(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|clipped| is_hex_path(&clipped.shape))
            .count()
    }

    fn painted_images(output: &egui::FullOutput) -> usize {
        output
            .shapes
            .iter()
            .filter(|clipped| matches!(clipped.shape, egui::Shape::Mesh(_)))
            .count()
    }

    fn paint_mechos(inventory: &Inventory) -> egui::FullOutput {
        let ctx = egui::Context::default();
        let mut selected = None;
        let mut action = InteriorAction::default();
        ctx.run_ui(viewport_input(), |ui| {
            draw_mechos(ui, inventory, &mut selected, &mut action);
        })
    }

    #[test]
    fn the_interior_paints_the_hex_board() {
        let inventory = Inventory::default();
        assert!(inventory.layout().width > 0);
        let output = paint_mechos(&inventory);
        let cells = hex_paths(&output);
        assert!(
            cells >= (inventory.layout().width * inventory.layout().height) as usize,
            "expected a painted hex cell per board slot, got {cells}"
        );
    }

    #[test]
    fn an_empty_board_paints_no_hex_cells() {
        let inventory = Inventory::for_car("Nobody", &super::super::Catalog::default());
        assert_eq!(inventory.layout().width, 0);
        assert_eq!(hex_paths(&paint_mechos(&inventory)), 0);
    }

    #[test]
    fn a_selected_ware_paints_on_the_turntable() {
        let mesh = SpinMesh::triangle();
        let shop = Shop::fostral();
        let inventory = Inventory::default();
        let ctx = egui::Context::default();
        let output = ctx.run_ui(viewport_input(), |ui| {
            draw_preview(ui, &shop, &inventory, Some("Nymbos"), Some(&mesh));
        });
        assert!(
            painted_images(&output) > 0,
            "the shop preview must paint the spinning mesh"
        );
    }

    #[test]
    fn opening_the_shop_selects_the_first_ware() {
        let visit = Visit {
            name: "Podish".into(),
            session: None,
        };
        let shop = Shop::fostral();
        let inventory = Inventory::default();
        let mut selected = None;
        let ctx = egui::Context::default();
        #[expect(deprecated)]
        let _ = ctx.run(viewport_input(), |ctx| {
            let _ = draw_interior(
                ctx,
                &visit,
                &shop,
                &inventory,
                200,
                None,
                &mut selected,
                &HashMap::new(),
                false,
            );
        });
        assert_eq!(selected.as_deref(), Some("Nymbos"));
    }

    #[test]
    fn dropping_on_the_interior_buys_through_apply() {
        let mut visit = Visit {
            name: "Podish".into(),
            session: None,
        };
        let mut shop = Shop::fostral();
        let mut inventory = Inventory::default();
        let mut beebs = 100;
        let action = InteriorAction {
            drop: Some((
                Hand::Shop {
                    id: "Nymbos".into(),
                },
                DropTarget::Cargo { origin: (1, 2) },
            )),
            ..InteriorAction::default()
        };
        let (note, slots) = action.apply(&mut visit, &mut shop, &mut inventory, &mut beebs);
        assert!(!slots);
        assert_eq!(beebs, 88);
        assert_eq!(inventory.cargo()[0].origin, (1, 2));
        assert_eq!(note.as_deref(), Some("Bought Nymbos"));
        let preview = preview("Nymbos").unwrap();
        assert_eq!(preview.kind, Kind::Ware);
        assert!(!preview.description.is_empty());
    }

    fn data_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../Vangers/data")
    }

    fn preview_mesh() -> SpinMesh {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        for rel in [
            "../Vangers/data/resource/m3d/items/i4.m3d",
            "../VangersData/resource/m3d/items/i4.m3d",
        ] {
            if let Some(mesh) = SpinMesh::load_path(&root.join(rel)) {
                return mesh;
            }
        }
        SpinMesh::triangle()
    }

    #[test]
    fn shop_layout_preview_png() {
        let visit = Visit {
            name: "Podish".into(),
            session: None,
        };
        let shop = Shop::fostral();
        let mut inventory = {
            let cat = super::super::Catalog::load(&data_root());
            if cat.is_empty() {
                Inventory::default()
            } else {
                Inventory::for_car("OxidizeMonk", &cat)
            }
        };
        if let Some(origin) = inventory.first_fit(shop.stock()[0].shape()) {
            let _ = inventory.place(shop.stock()[0].clone(), origin);
        }
        let mut selected = Some("Nymbos".into());
        let mut spins = HashMap::new();
        spins.insert("Nymbos".into(), preview_mesh());
        let ctx = egui::Context::default();
        let mut input = viewport_input();
        input.time = Some(0.8);
        #[expect(deprecated)]
        let _ = ctx.run(input.clone(), |ctx| {
            let _ = draw_interior(
                ctx,
                &visit,
                &shop,
                &inventory,
                188,
                Some("Bought Nymbos"),
                &mut selected,
                &spins,
                true,
            );
        });
        input.time = Some(0.8);
        #[expect(deprecated)]
        let output = ctx.run(input, |ctx| {
            let _ = draw_interior(
                ctx,
                &visit,
                &shop,
                &inventory,
                188,
                Some("Bought Nymbos"),
                &mut selected,
                &spins,
                true,
            );
        });
        let primitives = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
        let pixels = crate::escave::shot::rasterize(1280, 800, &output, &primitives);
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/shop-preview.png");
        crate::escave::shot::write_png(&path, 1280, 800, &pixels).expect("write shop preview");
        assert!(
            path.metadata().map(|m| m.len()).unwrap_or(0) > 8_000,
            "shop preview png should have pixels"
        );
    }
}
