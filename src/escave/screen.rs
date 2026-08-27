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

/// Full-screen talk/trade. The road is covered; this is the visit.
pub fn draw_interior(
    ctx: &egui::Context,
    visit: &Visit,
    shop: &Shop,
    inventory: &Inventory,
    beebs: i32,
    note: Option<&str>,
    selected: &mut Option<String>,
    spin: Option<&SpinMesh>,
    see_through: bool,
) -> InteriorAction {
    let mut action = InteriorAction::default();
    let rect = ctx.content_rect();
    let veil = if see_through {
        egui::Color32::from_rgba_unmultiplied(8, 6, 4, 70)
    } else {
        BG
    };
    let panel = if see_through {
        egui::Color32::from_rgba_unmultiplied(36, 26, 16, 210)
    } else {
        PANEL
    };
    egui::Area::new(egui::Id::new("escave-interior"))
        .fixed_pos(rect.left_top())
        .order(egui::Order::Middle)
        .show(ctx, |ui| {
            ui.painter().rect_filled(rect, 0.0, veil);
            ui.set_min_size(rect.size());
            egui::Frame::new()
                .inner_margin(egui::Margin::same(16))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(10.0, 8.0);
                    ui.horizontal(|ui| {
                        if ui.add(egui::Button::new(rich("Leave", ACCENT))).clicked() {
                            action.leave = true;
                        }
                        ui.label(
                            egui::RichText::new(visit.name.to_uppercase())
                                .color(ACCENT)
                                .size(22.0),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(rich(format!("Beebs: {beebs}"), INK));
                        });
                    });
                    if let Some(note) = note {
                        ui.label(rich(note, MUTED));
                    }
                    ui.add_space(4.0);
                    ui.columns(2, |cols| {
                        egui::Frame::new()
                            .fill(panel)
                            .inner_margin(egui::Margin::same(10))
                            .show(&mut cols[0], |ui| {
                                draw_shop(ui, shop, inventory, selected, spin, &mut action);
                            });
                        egui::Frame::new()
                            .fill(panel)
                            .inner_margin(egui::Margin::same(10))
                            .show(&mut cols[1], |ui| {
                                draw_mechos(ui, inventory, selected, &mut action);
                            });
                    });
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(panel)
                        .inner_margin(egui::Margin::same(10))
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
    ui.label(rich("Counselor", ACCENT).size(16.0));
    ui.add_space(6.0);
    let Some(ref session) = visit.session else {
        ui.label(rich("The counselor is silent.", MUTED));
        return;
    };
    let phrase = session.last_phrase().unwrap_or("");
    egui::ScrollArea::vertical()
        .max_height(140.0)
        .id_salt("escave-phrase")
        .show(ui, |ui| {
            ui.add(
                egui::Label::new(rich(phrase, INK).size(15.0))
                    .wrap()
                    .selectable(false),
            );
        });
    ui.add_space(8.0);
    if !session.ended() && ui.button(rich("Next", INK)).clicked() {
        action.next_phrase = true;
    }
    if session.queries().is_empty() {
        return;
    }
    ui.add_space(8.0);
    ui.label(rich("Ask", ACCENT));
    ui.horizontal_wrapped(|ui| {
        for q in session.queries() {
            let label = session.query_prompt(q);
            if ui.button(rich(label, INK)).clicked() {
                action.ask = Some(q.clone());
            }
        }
    });
}

const CELL: f32 = 22.0;
const CELL_EMPTY: egui::Color32 = egui::Color32::from_rgb(24, 18, 12);
const CELL_FULL: egui::Color32 = egui::Color32::from_rgb(72, 52, 28);
const CELL_BAY: egui::Color32 = egui::Color32::from_rgb(48, 36, 22);
const CELL_DEAD: egui::Color32 = egui::Color32::from_rgb(12, 9, 6);

fn draw_shop(
    ui: &mut egui::Ui,
    shop: &Shop,
    inventory: &Inventory,
    selected: &mut Option<String>,
    spin: Option<&SpinMesh>,
    action: &mut InteriorAction,
) {
    ui.label(rich("Shop", ACCENT).size(16.0));
    ui.label(rich(
        "Drag a ware onto the mechos to buy. Drag cargo here to sell.",
        MUTED,
    ));
    ui.add_space(4.0);
    let shop_frame = egui::Frame::new()
        .fill(egui::Color32::from_rgb(28, 20, 12))
        .inner_margin(egui::Margin::same(6));
    let (inner, dropped) = ui.dnd_drop_zone::<Hand, ()>(shop_frame, |ui| {
        if shop.stock().is_empty() {
            ui.label(rich("Nothing on the counter.", MUTED));
            return;
        }
        for good in shop.stock() {
            let id = egui::Id::new(("shop-stock", good.id.as_str()));
            let payload = Hand::Shop {
                id: good.id.clone(),
            };
            let response = ui
                .dnd_drag_source(id, payload, |ui| {
                    let kind = if good.is_weapon() { "gun" } else { "ware" };
                    let marked = selected.as_deref() == Some(good.id.as_str());
                    let color = if marked { ACCENT } else { INK };
                    ui.label(rich(
                        format!("{} ({kind})  {} beebs", good.display_name(), good.buy_price),
                        color,
                    ));
                })
                .response;
            if response.clicked() {
                *selected = Some(good.id.clone());
            }
        }
    });
    let _ = inner;
    if let Some(hand) = dropped.as_deref() {
        action.drop = Some((hand.clone(), DropTarget::Shop));
    }

    ui.add_space(10.0);
    draw_preview(ui, shop, inventory, selected.as_deref(), spin);
}

fn draw_mechos(
    ui: &mut egui::Ui,
    inventory: &Inventory,
    selected: &mut Option<String>,
    action: &mut InteriorAction,
) {
    ui.label(rich("Mechos", ACCENT).size(16.0));
    ui.label(rich("Hex board of this vehicle.", MUTED));
    ui.add_space(4.0);
    let layout = inventory.layout();
    let cell_size = egui::vec2(CELL, CELL);
    for y in 0..layout.height {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0, 2.0);
            if Layout::row_offset(y) {
                ui.add_space(CELL * 0.5);
            }
            for x in 0..layout.width {
                draw_board_cell(
                    ui,
                    inventory,
                    layout.cell(x, y),
                    (x, y),
                    cell_size,
                    selected,
                    action,
                );
            }
        });
    }
}

fn draw_board_cell(
    ui: &mut egui::Ui,
    inventory: &Inventory,
    cell: Cell,
    origin: (i32, i32),
    cell_size: egui::Vec2,
    selected: &mut Option<String>,
    action: &mut InteriorAction,
) {
    match cell {
        Cell::Empty => {
            let (rect, _) = ui.allocate_exact_size(cell_size, egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, CELL_DEAD);
        }
        Cell::Cargo => {
            let occupant = inventory.occupant(origin);
            let is_origin = occupant.is_some_and(|(_, p)| p.origin == origin);
            let fill = if occupant.is_some() {
                CELL_FULL
            } else {
                CELL_EMPTY
            };
            let frame = egui::Frame::new().inner_margin(egui::Margin::same(1));
            let (inner, dropped) = ui.dnd_drop_zone::<Hand, ()>(frame, |ui| {
                let (rect, _) = ui.allocate_exact_size(cell_size, egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, fill);
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                    if let Some((index, placed)) = occupant {
                        let payload = Hand::Cargo { index };
                        let id = egui::Id::new(("cargo-cell", origin.0, origin.1, index));
                        let response = ui
                            .dnd_drag_source(id, payload, |ui| {
                                if is_origin {
                                    let marked =
                                        selected.as_deref() == Some(placed.good.id.as_str());
                                    let color = if marked { ACCENT } else { INK };
                                    ui.label(
                                        rich(short_id(placed.good.display_name()), color).small(),
                                    );
                                }
                            })
                            .response;
                        if response.clicked() {
                            *selected = Some(placed.good.id.clone());
                        }
                    }
                });
            });
            let _ = inner;
            if let Some(hand) = dropped.as_deref() {
                action.drop = Some((hand.clone(), DropTarget::Cargo { origin }));
            }
        }
        Cell::Bay(index) => {
            let good = inventory.bay(index);
            let fill = if good.is_some() { CELL_FULL } else { CELL_BAY };
            let frame = egui::Frame::new()
                .fill(fill)
                .inner_margin(egui::Margin::same(1));
            let (inner, dropped) = ui.dnd_drop_zone::<Hand, ()>(frame, |ui| {
                let (rect, _) = ui.allocate_exact_size(cell_size, egui::Sense::hover());
                ui.painter().rect_filled(rect, 2.0, fill);
                ui.scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| match good {
                    Some(good) => {
                        let payload = Hand::Bay { index };
                        let id = egui::Id::new(("bay-cell", origin.0, origin.1, index));
                        let response = ui
                            .dnd_drag_source(id, payload, |ui| {
                                let marked = selected.as_deref() == Some(good.id.as_str());
                                let color = if marked { ACCENT } else { INK };
                                ui.label(rich(short_id(good.display_name()), color).small());
                            })
                            .response;
                        if response.clicked() {
                            *selected = Some(good.id.clone());
                        }
                    }
                    None => {
                        ui.label(rich(format!("W{index}"), MUTED).small());
                    }
                });
            });
            let _ = inner;
            if let Some(hand) = dropped.as_deref() {
                action.drop = Some((hand.clone(), DropTarget::Bay { index }));
            }
        }
    }
}

fn draw_preview(
    ui: &mut egui::Ui,
    shop: &Shop,
    inventory: &Inventory,
    selected: Option<&str>,
    spin: Option<&SpinMesh>,
) {
    ui.label(rich("Preview", ACCENT));
    let Some(id) = selected else {
        ui.label(rich("Select a ware to see its stats.", MUTED));
        return;
    };
    let live = shop
        .stock()
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
        });
    let stats = match live {
        Some(good) => preview_good(good),
        None => match preview(id) {
            Some(p) => p,
            None => Preview {
                id: id.to_string(),
                name: id.to_string(),
                kind: Kind::Ware,
                buy_price: 0,
                sell_price: 0,
                description: String::new(),
            },
        },
    };
    let kind = if stats.kind == Kind::Weapon {
        "gun"
    } else {
        "ware"
    };
    ui.label(rich(format!("{} ({kind})", stats.name), INK));
    ui.label(rich(
        format!("Buy {}   Sell {}", stats.buy_price, stats.sell_price),
        MUTED,
    ));
    if let Some(mesh) = spin {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(180.0, 140.0), egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 4.0, egui::Color32::from_rgb(20, 14, 10));
        let angle = ui.input(|i| i.time) as f32;
        let painter = ui.painter();
        mesh.paint(painter, rect, angle, ACCENT);
        ui.ctx().request_repaint();
    }
    if stats.description.is_empty() {
        ui.label(rich("No description.", MUTED));
    } else {
        ui.add(
            egui::Label::new(rich(&stats.description, INK))
                .wrap()
                .selectable(false),
        );
    }
}

fn short_id(id: &str) -> String {
    let mut chars = id.chars();
    let take: String = chars.by_ref().take(4).collect();
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
}
