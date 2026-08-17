//! Interactive moving-land controls for the level viewer.
//!
//! Loads the world's `data.vot` and `location.lst` and lets you play the
//! animations by hand: send any location to any of its key phases, hold a
//! sensor down to work the door it drives, and fly the camera to whatever you
//! just triggered.
//!
//! Nothing runs until you ask it to. The level opens exactly as it is on
//! disk: every location parked at phase 0, the clock stopped, and the
//! location engines not simulated - otherwise the cyclic ones (34 of them on
//! Fostral) would start cycling the moment the level appeared. Pressing a
//! trigger starts the clock for you.
//!
//! To put a location back, send it to key phase 0. A moving land is a closed
//! cycle, so that restores the surface rather than merely stopping it.

use vangers::level::{
    Level, LevelConfig,
    moving::{FREE_RUNNING, MovingLand, Region},
    trigger::{Kind, Triggers},
};

/// Never run more than this many quants in one frame.
///
/// Deliberately small. Catching up after a slow frame means running several
/// quants, each of which walks every animating patch, which makes the next
/// frame slower still - a spiral that shows up as the viewer juddering
/// instead of merely animating below the nominal rate. A viewer would much
/// rather play the animation slowly than stall, so the backlog is dropped.
const MAX_QUANTS_PER_FRAME: u32 = 2;

pub struct MovingLandUi {
    land: MovingLand,
    triggers: Triggers,
    /// Sensors the user is holding down, parallel to `triggers.sensors`.
    held: Vec<bool>,
    /// Whether the clock is running. Off until something is triggered.
    playing: bool,
    /// Whether the location engines are simulated. Off by default: the
    /// cyclic ones drive themselves, and a viewer that starts by animating
    /// half the map is not showing you the level.
    run_engines: bool,
    /// Quants per second. The game runs 20.
    rate: f32,
    /// Fractional quants carried between frames.
    carry: f32,
    /// Set by the Step button, consumed by the next update.
    step_once: bool,
    /// Whether anything has been triggered yet, so the "nothing is running"
    /// hint stops once the user has found the controls.
    ever_triggered: bool,
    selected: Option<usize>,
    filter: String,
    /// Set by a "Look" button; the app consumes it to move the camera.
    pub focus: Option<(glam::Vec3, f32)>,
    /// Rectangles the last update touched, for the caller to upload.
    regions: Vec<Region>,
    /// Order the list by distance from what the camera is looking at, so the
    /// patch you are staring at is the one at the top.
    sort_by_distance: bool,
    /// Scratch for the sorted order, rebuilt each frame it is needed.
    order: Vec<usize>,
    /// Rolling cost of the last update, so it is obvious from the panel
    /// whether the moving land is what is costing the frame.
    last_cost_ms: f32,
    last_quants: u32,
}

impl MovingLandUi {
    /// Returns `None` for a level with no moving land at all.
    pub fn load(config: &LevelConfig) -> Option<Self> {
        let data_vot = config.path_moving_land()?;
        let world_dir = data_vot.parent()?.to_path_buf();
        let mut land = MovingLand::load_dir(&data_vot, config.terrains.len() as i32);
        if land.is_empty() {
            return None;
        }
        let triggers = Triggers::load(&world_dir, &data_vot, &land);
        triggers.reset_locations(&mut land);
        Some(MovingLandUi {
            held: vec![false; triggers.sensors.len()],
            land,
            triggers,
            playing: false,
            run_engines: false,
            rate: 20.0,
            carry: 0.0,
            step_once: false,
            ever_triggered: false,
            selected: None,
            filter: String::new(),
            focus: None,
            regions: Vec::new(),
            sort_by_distance: true,
            order: Vec::new(),
            last_cost_ms: 0.0,
            last_quants: 0,
        })
    }

    /// Advances the clock and returns the rectangles that changed.
    pub fn update(&mut self, level: &mut Level, delta: f32) -> &[Region] {
        self.regions.clear();

        let mut quants = 0;
        if self.playing {
            self.carry += delta * self.rate;
            quants = self.carry as u32;
            self.carry -= quants as f32;
        }
        if self.step_once {
            self.step_once = false;
            quants = quants.max(1);
        }

        let quants = quants.min(MAX_QUANTS_PER_FRAME);
        self.last_quants = quants;
        let started = std::time::Instant::now();

        for _ in 0..quants {
            if self.run_engines {
                for (index, &held) in self.held.iter().enumerate() {
                    if held {
                        self.triggers.touch_sensor(index);
                    }
                }
                self.triggers.update(&mut self.land);
            }
            self.land.update(level, &mut self.regions);
        }

        // A frame keeps redrawing its rectangle for its whole period, so the
        // same region comes back once per quant.
        self.regions.sort_unstable();
        self.regions.dedup();

        // Smoothed, because most frames run no quants at all and the raw
        // number would flicker between zero and the cost of one quant.
        let cost = started.elapsed().as_secs_f32() * 1000.0;
        self.last_cost_ms += (cost - self.last_cost_ms) * 0.1;
        &self.regions
    }

    /// Where a location sits, in level texels, wrapped into the map.
    fn location_pos(&self, index: usize, level: &Level) -> (i32, i32) {
        let location = &self.land.locations[index];
        let frame = &location.source.frames[0];
        let (w, h) = location.source.max_frame_size();
        (
            (frame.pos.0 + location.offset.0 + w / 2).rem_euclid(level.size.0),
            (frame.pos.1 + location.offset.1 + h / 2).rem_euclid(level.size.1),
        )
    }

    /// Squared distance from `focus` to a location, across the level seams.
    fn distance2(&self, index: usize, level: &Level, focus: glam::Vec2) -> i64 {
        let (x, y) = self.location_pos(index, level);
        let wrap = |d: i32, size: i32| -> i64 {
            let d = d.rem_euclid(size);
            if d * 2 > size { d - size } else { d }.into()
        };
        let dx = wrap(x - focus.x as i32, level.size.0);
        let dy = wrap(y - focus.y as i32, level.size.1);
        dx * dx + dy * dy
    }

    /// The order to list locations in: nearest to the camera first, or the
    /// order they loaded in.
    fn listing_order(&mut self, level: &Level, focus: glam::Vec2) {
        self.order.clear();
        self.order.extend(0..self.land.locations.len());
        if self.sort_by_distance {
            // Keyed by location index, so the lookup stays right as the
            // order is permuted.
            let keys = (0..self.land.locations.len())
                .map(|i| self.distance2(i, level, focus))
                .collect::<Vec<_>>();
            self.order.sort_by_key(|&i| keys[i]);
        }
    }

    /// Centre of a location, for the camera to fly to.
    fn location_centre(&self, index: usize, level: &Level) -> (glam::Vec3, f32) {
        let location = &self.land.locations[index];
        let frame = &location.source.frames[0];
        let (w, h) = location.source.max_frame_size();
        let x = frame.pos.0 + location.offset.0 + w / 2;
        let y = frame.pos.1 + location.offset.1 + h / 2;
        let texel = (x.rem_euclid(level.size.0), y.rem_euclid(level.size.1));
        let z = level.get(texel).high();
        (
            glam::Vec3::new(texel.0 as f32, texel.1 as f32, z),
            // Frame the whole patch with a bit of room around it.
            (w.max(h) as f32 * 1.8).max(120.0),
        )
    }

    pub fn draw_ui(&mut self, ui: &mut egui::Ui, level: &Level, focus: glam::Vec2) {
        self.listing_order(level, focus);
        ui.horizontal(|ui| {
            let label = if self.playing { "⏸" } else { "▶" };
            if ui.button(label).on_hover_text("Play / pause").clicked() {
                self.playing = !self.playing;
            }
            if ui
                .button("Step")
                .on_hover_text("Advance exactly one quant")
                .clicked()
            {
                self.step_once = true;
            }
            if ui
                .button("Stop all")
                .on_hover_text("Park every location where it stands and stop the clock")
                .clicked()
            {
                for location in self.land.locations.iter_mut() {
                    location.park();
                }
                self.playing = false;
                self.run_engines = false;
                self.held.iter_mut().for_each(|h| *h = false);
            }
        });
        ui.horizontal(|ui| {
            if ui
                .checkbox(&mut self.run_engines, "Run engines")
                .on_hover_text(
                    "Simulate location.lst. Cyclic engines start cycling on \
                     their own, and sensors begin working doors",
                )
                .changed()
                && self.run_engines
            {
                self.playing = true;
            }
        });
        ui.add(
            egui::Slider::new(&mut self.rate, 1.0..=60.0)
                .text("quants/s")
                .clamping(egui::SliderClamping::Always),
        )
        .on_hover_text("The game runs the moving land at 20 quants a second");

        let moving = self
            .land
            .locations
            .iter()
            .filter(|l| l.go_phase() == FREE_RUNNING || !l.is_go_finish())
            .count();
        ui.label(format!(
            "{} locations ({moving} moving), {} engines, {} sensors",
            self.land.locations.len(),
            self.triggers.engines.len(),
            self.triggers.sensors.len(),
        ));
        if !self.ever_triggered && !self.run_engines {
            ui.weak("Nothing is running - pick a location and send it to a key phase.");
        }

        if self.last_quants != 0 || self.last_cost_ms > 0.01 {
            ui.weak(format!(
                "stepping {:.2} ms/frame, {} quant(s), {} region(s)",
                self.last_cost_ms,
                self.last_quants,
                self.regions.len()
            ));
        }

        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.text_edit_singleline(&mut self.filter);
        });
        ui.checkbox(&mut self.sort_by_distance, "Nearest first")
            .on_hover_text("Order the list by distance from what the camera is looking at");

        self.draw_locations(ui, level, focus);
        self.draw_engines(ui);
        self.draw_sensors(ui);
    }

    fn draw_locations(&mut self, ui: &mut egui::Ui, level: &Level, focus: glam::Vec2) {
        egui::CollapsingHeader::new("Locations")
            .default_open(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .id_salt("ml-locations")
                    .show(ui, |ui| {
                        let filter = self.filter.to_lowercase();
                        let order = std::mem::take(&mut self.order);
                        for &index in order.iter() {
                            let name = self.land.locations[index].source.name.clone();
                            if !filter.is_empty() && !name.to_lowercase().contains(&filter) {
                                continue;
                            }
                            self.draw_location(ui, index, &name, level, focus);
                        }
                        self.order = order;
                    });
            });
    }

    fn draw_location(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        name: &str,
        level: &Level,
        focus: glam::Vec2,
    ) {
        let (summary, status, moving) = {
            let location = &self.land.locations[index];
            let go = location.go_phase();
            let away = (self.distance2(index, level, focus) as f64).sqrt();
            let summary = format!(
                "{name}  frame {}/{}  phase {}/{}  {}",
                location.current_frame(),
                location.source.frames.len(),
                location.current_phase(),
                location.source.max_stage(),
                if away < 1000.0 {
                    format!("{away:.0} away")
                } else {
                    format!("{:.1}k away", away / 1000.0)
                },
            );
            let status = if go == FREE_RUNNING {
                "looping".to_string()
            } else if location.is_go_finish() {
                format!("parked at {go}")
            } else {
                format!("heading to {go}")
            };
            let moving = go == FREE_RUNNING || !location.is_go_finish();
            (summary, status, moving)
        };

        let open = self.selected == Some(index);
        let header = if moving {
            egui::RichText::new(&summary).strong()
        } else {
            egui::RichText::new(&summary)
        };
        if ui.selectable_label(open, header).clicked() {
            self.selected = if open { None } else { Some(index) };
        }
        if !open {
            return;
        }

        ui.indent(("ml-loc", index), |ui| {
            ui.label(status);
            ui.horizontal_wrapped(|ui| {
                let keys = self.land.locations[index].source.key_phases;
                for (slot, &phase) in keys.iter().enumerate() {
                    if phase < 0 {
                        continue;
                    }
                    if ui
                        .button(format!("Key {slot}"))
                        .on_hover_text(format!(
                            "Run to phase {phase} and park.{}",
                            if slot == 0 {
                                " Key 0 is the start, so this puts the surface back."
                            } else {
                                ""
                            }
                        ))
                        .clicked()
                    {
                        self.land.locations[index].go_key_phase(slot as i32);
                        self.playing = true;
                        self.ever_triggered = true;
                    }
                }
                if ui
                    .button("Loop")
                    .on_hover_text("Run continuously, ignoring key phases")
                    .clicked()
                {
                    self.land.locations[index].set_go_phase(FREE_RUNNING);
                    self.playing = true;
                    self.ever_triggered = true;
                }
                if ui
                    .button("Freeze")
                    .on_hover_text("Park right where it is")
                    .clicked()
                {
                    self.land.locations[index].park();
                }
                if ui
                    .button("Look")
                    .on_hover_text("Fly the camera here")
                    .clicked()
                {
                    self.focus = Some(self.location_centre(index, level));
                }
            });

            let location = &self.land.locations[index];
            let ml = &location.source;
            ui.label(format!(
                "{:?} mode, dry terrain {}, impulse {}, keys {:?}",
                ml.mode, ml.dry_terrain, ml.impulse, ml.key_phases
            ));
            if location.offset != (0, 0) {
                ui.label(format!("clone, offset {:?}", location.offset));
            }
            egui::CollapsingHeader::new("Frames")
                .id_salt(("ml-frames", index))
                .show(ui, |ui| {
                    for (n, frame) in ml.frames.iter().enumerate() {
                        let marker = if n == location.current_frame() {
                            "▶"
                        } else {
                            " "
                        };
                        ui.label(format!(
                            "{marker} {n:3}  at {:?} {}x{}  period {}  surf {}{}",
                            frame.pos,
                            frame.size.0,
                            frame.size.1,
                            frame.period,
                            frame.surface_type,
                            if frame.terrain.is_empty() {
                                ""
                            } else {
                                "  +terrain"
                            },
                        ));
                    }
                });
        });
    }

    fn draw_engines(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Engines").show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .id_salt("ml-engines")
                .show(ui, |ui| {
                    for index in 0..self.triggers.engines.len() {
                        self.draw_engine(ui, index);
                    }
                });
        });
    }

    fn draw_engine(&mut self, ui: &mut egui::Ui, index: usize) {
        let engine = &self.triggers.engines[index];
        let kind = match engine.kind {
            Kind::Door { lock, .. } => {
                if lock == 0 {
                    "Door".to_string()
                } else {
                    format!("Door (lock {lock})")
                }
            }
            Kind::Tiristor { .. } => "Tiristor".to_string(),
            Kind::Cyclic { .. } => "Cyclic".to_string(),
            Kind::Unsupported(t) => format!("type {t}, not driven"),
        };
        let target = match engine.location {
            Some(i) => self.land.locations[i].source.name.clone(),
            None => "<unlinked>".to_string(),
        };
        let sensors = engine.sensors().to_vec();
        let text = format!(
            "{kind} on {target}  {:?}  touching {}",
            engine.mode(),
            engine.touch_count(),
        );

        if sensors.is_empty() {
            ui.label(text);
            return;
        }
        ui.horizontal(|ui| {
            // One toggle for the whole engine: holding all of its sensors is
            // what standing on the pad does.
            let mut all = sensors.iter().all(|&s| self.held[s]);
            if ui
                .toggle_value(&mut all, "Hold")
                .on_hover_text("Keep something standing on this engine's sensors")
                .changed()
            {
                for s in sensors {
                    self.held[s] = all;
                }
                if all {
                    // Holding a sensor is pointless unless the engines run.
                    self.run_engines = true;
                    self.playing = true;
                    self.ever_triggered = true;
                }
            }
            ui.label(text);
        });
    }

    fn draw_sensors(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("Sensors").show(ui, |ui| {
            egui::ScrollArea::vertical()
                .max_height(200.0)
                .id_salt("ml-sensors")
                .show(ui, |ui| {
                    for index in 0..self.triggers.sensors.len() {
                        let sensor = &self.triggers.sensors[index];
                        let name = if sensor.name.is_empty() {
                            format!("<{index}>")
                        } else {
                            sensor.name.clone()
                        };
                        let text = format!(
                            "{name}  kind {}  r {}  at {:?}",
                            sensor.kind, sensor.radius, sensor.pos
                        );
                        let owned = self.triggers.sensor_owner(index).is_some();
                        ui.horizontal(|ui| {
                            let mut held = self.held[index];
                            let changed = ui
                                .add_enabled_ui(owned, |ui| {
                                    ui.toggle_value(&mut held, "Hold")
                                        .on_hover_text(if owned {
                                            "Keep something standing in this sensor"
                                        } else {
                                            "No engine listens to this sensor"
                                        })
                                        .changed()
                                })
                                .inner;
                            if changed {
                                self.held[index] = held;
                                if held {
                                    self.run_engines = true;
                                    self.playing = true;
                                    self.ever_triggered = true;
                                }
                            }
                            if self.triggers.sensor_enabled(index) {
                                ui.label(text);
                            } else {
                                ui.weak(format!("{text}  (disabled)"));
                            }
                        });
                    }
                });
        });
    }
}
