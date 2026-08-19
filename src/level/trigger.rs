//! Location engines - the state machines that drive the moving land.
//!
//! Port of the `LocationEngine` family of `src/units/sensor.cpp`. A level's
//! `location.lst` names one engine per moving-land patch: which sensors open
//! it, which key phases count as open and closed, and how long it waits. Once
//! a vehicle drives into a sensor the engine sends its location to the active
//! key phase, and the moving land animates the bridge, door or lift there.
//!
//! Three engine types are implemented, the ones whose behaviour is entirely
//! moving land plus proximity:
//!
//! - [`Kind::Door`] opens while something is standing on a sensor and closes
//!   again once everything leaves;
//! - [`Kind::Tiristor`] is a latch - it opens on the first touch and stays
//!   open;
//! - [`Kind::Cyclic`] ignores sensors and cycles on a timer.
//!
//! The rest (escaves, passages, trains, item generators) hang off quest and
//! inventory systems this port does not have, so they are parsed far enough
//! to be skipped and reported.

use crate::level::moving::MovingLand;
use crate::level::vlc::{self, Sensor};
use crate::vfs::Vfs;

use std::path::Path;

/// `EngineTypeList` of the original.
mod engine_type {
    pub const DOOR: i32 = 0;
    pub const CYCLIC: i32 = 4;
    pub const TIRISTOR: i32 = 7;
}

/// `DOOR_OPEN_LOCK`/`DOOR_CLOSE_LOCK` - authored flags that pin a door in
/// one direction.
const DOOR_OPEN_LOCK: i32 = 1;
const DOOR_CLOSE_LOCK: i32 = 2;

/// `EngineModeList`, trimmed to the states the implemented engines use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `WAIT` - closed and idle.
    Wait,
    /// `OPEN`.
    Open,
    /// The four states a cyclic engine walks around.
    AcceptProcess,
    AcceptDelay,
    AcceptProcessEnd,
    SendProcess,
}

impl Mode {
    /// `ActionMode` in `location.lst` is one of these, by number.
    pub fn from_index(index: i32) -> Option<Self> {
        Some(match index {
            0 => Mode::AcceptProcess,
            1 => Mode::AcceptDelay,
            2 => Mode::AcceptProcessEnd,
            3 => Mode::SendProcess,
            6 => Mode::Wait,
            7 => Mode::Open,
            _ => return None,
        })
    }
}

/// What a particular engine does with its sensors.
#[derive(Debug)]
pub enum Kind {
    /// Held open by proximity, closes when everything leaves.
    Door {
        sensors: Vec<usize>,
        /// Bit 0 pins it closed, bit 1 pins it open.
        lock: i32,
    },
    /// Opens on the first touch and never closes itself.
    Tiristor { sensors: Vec<usize> },
    /// Runs on a timer. `actions` are sensors it enables while it sits in
    /// `action_mode`, which is how the original gates a passage on a bridge
    /// having finished extending.
    Cyclic {
        actions: Vec<usize>,
        action_mode: Option<Mode>,
    },
    /// Parsed but not driven - the engine types that need systems this port
    /// does not have.
    Unsupported(i32),
}

/// One entry of `location.lst`.
pub struct Engine {
    pub kind: Kind,
    /// Index into [`MovingLand::locations`], if `MLName` resolved.
    pub location: Option<usize>,
    /// Key phase indices, not phases: they go through `go_key_phase`.
    pub active_phase: i32,
    pub deactive_phase: i32,
    pub active_time: i32,
    pub deactive_time: i32,
    pub sound_id: i32,
    pub enabled: bool,
    mode: Mode,
    time: i32,
    /// Touches accumulated since the last quant.
    touch_count: u32,
}

impl Engine {
    /// `NumTouchObject` - how many objects were on the sensors last quant.
    pub fn touch_count(&self) -> u32 {
        self.touch_count
    }

    pub fn is_open(&self) -> bool {
        self.mode == Mode::Open
    }

    /// Which state of its cycle the engine is in.
    pub fn mode(&self) -> Mode {
        self.mode
    }

    /// Sensors this engine listens to, as indices into [`Triggers::sensors`].
    pub fn sensors(&self) -> &[usize] {
        match self.kind {
            Kind::Door { ref sensors, .. } | Kind::Tiristor { ref sensors } => sensors,
            Kind::Cyclic { .. } | Kind::Unsupported(_) => &[],
        }
    }

    /// `LocationEngine::Touch` - something is inside one of our sensors.
    fn touch(&mut self) {
        if self.enabled && self.location.is_some() {
            self.touch_count += 1;
        }
    }

    /// One `Quant` of the engine's state machine.
    ///
    /// Returns the sensor-enable changes the engine wants, as
    /// `(sensor index, enabled)`, which only cyclic engines produce.
    fn update(&mut self, land: &mut MovingLand, enables: &mut Vec<(usize, bool)>) {
        let Some(index) = self.location else {
            return;
        };
        if !self.enabled {
            return;
        }
        // The original also bails out while the location's rows are paged
        // out (`MLLink->frozen`). Levels are fully resident here, so that
        // branch can never be taken.
        match self.kind {
            Kind::Door { lock, .. } => self.update_door(&mut land.locations[index], lock),
            Kind::Tiristor { .. } => self.update_tiristor(&mut land.locations[index]),
            Kind::Cyclic { .. } => {
                self.update_cyclic(&mut land.locations[index]);
                self.publish_actions(enables);
            }
            Kind::Unsupported(_) => {}
        }
        self.touch_count = 0;
    }

    fn update_door(&mut self, location: &mut crate::level::moving::Location, lock: i32) {
        if !location.is_go_finish() {
            return;
        }
        if self.time > 0 {
            self.time -= 1;
            return;
        }
        if self.touch_count > 0 {
            if self.mode != Mode::Open && lock & DOOR_OPEN_LOCK == 0 {
                self.mode = Mode::Open;
                location.go_key_phase(self.active_phase);
            }
        } else if self.mode != Mode::Wait && lock & DOOR_CLOSE_LOCK == 0 {
            self.mode = Mode::Wait;
            location.go_key_phase(self.deactive_phase);
        }
    }

    fn update_tiristor(&mut self, location: &mut crate::level::moving::Location) {
        if !location.is_go_finish() {
            return;
        }
        if self.touch_count > 0 && self.mode != Mode::Open {
            self.mode = Mode::Open;
            location.go_key_phase(self.active_phase);
        }
    }

    fn update_cyclic(&mut self, location: &mut crate::level::moving::Location) {
        if !location.is_go_finish() {
            return;
        }
        match self.mode {
            Mode::AcceptProcess => {
                self.time += 1;
                if self.time > self.deactive_time {
                    location.go_key_phase(self.active_phase);
                    self.time = 0;
                    self.mode = Mode::AcceptDelay;
                }
            }
            Mode::AcceptDelay => self.mode = Mode::AcceptProcessEnd,
            Mode::AcceptProcessEnd => {
                self.time += 1;
                if self.time > self.active_time {
                    location.go_key_phase(self.deactive_phase);
                    self.time = 0;
                    self.mode = Mode::SendProcess;
                }
            }
            Mode::SendProcess => self.mode = Mode::AcceptProcess,
            Mode::Wait | Mode::Open => {}
        }
    }

    /// A cyclic engine enables its action sensors only while it sits in the
    /// authored mode.
    fn publish_actions(&self, enables: &mut Vec<(usize, bool)>) {
        if let Kind::Cyclic {
            ref actions,
            action_mode: Some(action_mode),
        } = self.kind
        {
            let on = self.mode == action_mode;
            enables.extend(actions.iter().map(|&s| (s, on)));
        }
    }
}

/// Every sensor and engine of one level.
#[derive(Default)]
pub struct Triggers {
    pub sensors: Vec<Sensor>,
    pub engines: Vec<Engine>,
    /// Engine owning each sensor, parallel to `sensors`.
    owners: Vec<Option<usize>>,
    /// `SensorDataType::Enable`, parallel to `sensors`.
    enabled: Vec<bool>,
    /// Scratch for the enable changes an engine quant produces.
    enables: Vec<(usize, bool)>,
}

impl Triggers {
    /// Loads `snstable.vlc` out of `data_vot` and `location.lst` out of
    /// `world_dir`, resolving every name against `land`.
    pub fn load(world_dir: &Path, data_vot: &Path, land: &MovingLand) -> Self {
        let sensors = vlc::load_sensors(data_vot);
        let text = match std::fs::read(world_dir.join("location.lst")) {
            Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("Unable to read {:?}: {}", world_dir.join("location.lst"), e);
                }
                None
            }
        };
        Self::from_sensors_and_list(sensors, text.as_deref(), land)
    }

    /// Same as [`load`], from a VFS that already has the level zip mounted.
    /// `data_vot` is the archive folder (`"data.vot"`); `location.lst` is
    /// read from the VFS root, next to `world.ini`.
    pub fn load_from_vfs(vfs: &Vfs, data_vot: &str, land: &MovingLand) -> Self {
        let sensor_key = format!("{}/snstable.vlc", data_vot.trim_end_matches('/'));
        let sensors = vfs
            .read(&sensor_key)
            .map(|bytes| vlc::load_sensors_from_bytes(&bytes, &sensor_key))
            .unwrap_or_default();
        let text = vfs
            .read("location.lst")
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        Self::from_sensors_and_list(sensors, text.as_deref(), land)
    }

    fn from_sensors_and_list(sensors: Vec<Sensor>, text: Option<&str>, land: &MovingLand) -> Self {
        let mut triggers = Triggers {
            owners: vec![None; sensors.len()],
            enabled: vec![true; sensors.len()],
            sensors,
            engines: Vec::new(),
            enables: Vec::new(),
        };
        if let Some(text) = text {
            triggers.parse(text, land);
        }
        log::info!(
            "Loaded {} location engines and {} sensors",
            triggers.engines.len(),
            triggers.sensors.len()
        );
        triggers
    }

    fn parse(&mut self, text: &str, land: &MovingLand) {
        let mut unsupported = Vec::new();
        for part in Part::split(text) {
            let mut scan = part;
            let Some(kind_index) = scan.int_after("EngineType") else {
                continue;
            };

            let ml_name = scan.name_after("MLName").unwrap_or("None");
            let location = if ml_name == "None" {
                None
            } else {
                let found = land.find(ml_name);
                if found.is_none() {
                    log::warn!("Location engine refers to unknown moving land '{ml_name}'");
                }
                found
            };

            let active_phase = scan.int_after("ActivePhase").unwrap_or(0);
            let deactive_phase = scan.int_after("DeactivePhase").unwrap_or(0);
            let active_time = scan.int_after("ActiveTime").unwrap_or(0);
            let deactive_time = scan.int_after("DeactiveTime").unwrap_or(0);
            let sound_id = scan.int_after("SoundID").unwrap_or(0);

            let engine_index = self.engines.len();
            let kind = match kind_index {
                engine_type::DOOR => {
                    let sensors = self.link_sensors(
                        &mut scan,
                        "NumSensorLink",
                        "SensorName",
                        engine_index,
                        true,
                    );
                    Kind::Door {
                        sensors,
                        lock: scan.int_after("LockFlag").unwrap_or(0),
                    }
                }
                engine_type::TIRISTOR => {
                    let sensors = self.link_sensors(
                        &mut scan,
                        "NumSensorLink",
                        "SensorName",
                        engine_index,
                        true,
                    );
                    Kind::Tiristor { sensors }
                }
                engine_type::CYCLIC => {
                    let raw_mode = scan.int_after("ActionMode").unwrap_or(-1);
                    let actions = if raw_mode > -1 {
                        // Action sensors start disabled; the engine turns
                        // them on when it reaches `ActionMode`.
                        self.link_sensors(
                            &mut scan,
                            "NumActionLink",
                            "ActionName",
                            engine_index,
                            false,
                        )
                    } else {
                        Vec::new()
                    };
                    Kind::Cyclic {
                        actions,
                        action_mode: Mode::from_index(raw_mode),
                    }
                }
                other => {
                    unsupported.push(other);
                    Kind::Unsupported(other)
                }
            };

            // Both doors and cyclic engines start parked at the closed phase.
            let mode = match kind {
                Kind::Cyclic { .. } => Mode::SendProcess,
                _ => Mode::Wait,
            };
            self.engines.push(Engine {
                kind,
                location,
                active_phase,
                deactive_phase,
                active_time,
                deactive_time,
                sound_id,
                enabled: true,
                mode,
                time: 0,
                touch_count: 0,
            });
        }

        if !unsupported.is_empty() {
            unsupported.sort_unstable();
            unsupported.dedup();
            log::info!("Skipped location engines of types {unsupported:?}");
        }
    }

    /// Reads a `Num...`/`...Name` sensor list and records this engine as the
    /// owner of each one.
    fn link_sensors(
        &mut self,
        scan: &mut Part<'_>,
        count_key: &str,
        name_key: &str,
        engine: usize,
        enabled: bool,
    ) -> Vec<usize> {
        let count = scan.int_after(count_key).unwrap_or(0).max(0);
        let mut linked = Vec::with_capacity(count as usize);
        for _ in 0..count {
            let Some(name) = scan.name_after(name_key) else {
                break;
            };
            match self.sensors.iter().position(|s| s.name == name) {
                Some(index) => {
                    self.owners[index] = Some(engine);
                    self.enabled[index] = enabled;
                    linked.push(index);
                }
                None => log::warn!("Location engine refers to unknown sensor '{name}'"),
            }
        }
        linked
    }

    /// Sends every engine's location to its closed phase, which is what the
    /// `Open` of each engine type does once the level is up.
    ///
    /// Locations start parked at phase 0 already, so this only matters for an
    /// engine whose `DeactivePhase` names some other key. Engine types this
    /// port skips are left alone, matching the original: of those, only
    /// `SignPlayEngine` parks its location at all, and every `DeactivePhase`
    /// in the shipped levels is key 0 regardless.
    pub fn reset_locations(&self, land: &mut MovingLand) {
        for engine in self.engines.iter() {
            let Some(index) = engine.location else {
                continue;
            };
            if matches!(engine.kind, Kind::Unsupported(_)) {
                continue;
            }
            land.locations[index].go_key_phase(engine.deactive_phase);
        }
    }

    /// Registers an object standing at `pos` with radius `radius`, in level
    /// texel units. Mirrors the sensor test of `VangerUnit::Quant`: a cylinder
    /// in the plane and a band in altitude, both widened by the object.
    pub fn touch(&mut self, pos: (i32, i32, i32), radius: i32, size: (i32, i32)) {
        for (index, sensor) in self.sensors.iter().enumerate() {
            if !self.enabled[index] {
                continue;
            }
            let Some(engine) = self.owners[index] else {
                continue;
            };
            let reach = radius + sensor.radius;
            let dx = wrap_delta(sensor.pos.0 - pos.0, size.0);
            if dx.abs() >= reach {
                continue;
            }
            if pos.2 <= sensor.z_range.0 - radius || pos.2 >= sensor.z_range.1 + radius {
                continue;
            }
            let dy = wrap_delta(sensor.pos.1 - pos.1, size.1);
            if dx * dx + dy * dy < reach * reach {
                self.engines[engine].touch();
            }
        }
    }

    /// Fires one sensor directly, as if something were standing inside it -
    /// `SensorDataType::Touch` without the proximity test. This is what a
    /// tool uses to work a door without a vehicle to drive onto the pad.
    pub fn touch_sensor(&mut self, index: usize) {
        if !self.enabled[index] {
            return;
        }
        if let Some(engine) = self.owners[index] {
            self.engines[engine].touch();
        }
    }

    /// The engine a sensor belongs to, if any claimed it.
    pub fn sensor_owner(&self, index: usize) -> Option<usize> {
        self.owners[index]
    }

    /// Runs every engine for one quant.
    ///
    /// Unlike the original this does not gate on the location being near the
    /// camera: the whole level is resident, and a bridge that keeps its cycle
    /// while off screen is closer to what the level author drew.
    pub fn update(&mut self, land: &mut MovingLand) {
        profiling::scope!("Location engines");
        for engine in self.engines.iter_mut() {
            self.enables.clear();
            engine.update(land, &mut self.enables);
            for &(sensor, on) in self.enables.iter() {
                self.enabled[sensor] = on;
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.engines.is_empty()
    }

    /// Whether a sensor is currently listening.
    pub fn sensor_enabled(&self, index: usize) -> bool {
        self.enabled[index]
    }
}

/// Shortest signed distance on a torus of `size`, like `getDistX` of the
/// original.
fn wrap_delta(delta: i32, size: i32) -> i32 {
    let wrapped = delta.rem_euclid(size);
    if wrapped * 2 > size {
        wrapped - size
    } else {
        wrapped
    }
}

/// One `Part` block of `location.lst`, as a forward-scanning token cursor.
///
/// The original's `Parser::search_name` scans ahead for a key and reads the
/// value after it, so unknown keys are simply stepped over. Splitting on
/// `Part` first keeps a missing key inside one engine from swallowing the
/// next engine's values.
struct Part<'a> {
    tokens: Vec<&'a str>,
    pos: usize,
}

impl<'a> Part<'a> {
    fn split(text: &'a str) -> Vec<Part<'a>> {
        let tokens = text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .flat_map(|line| line.split_whitespace())
            .collect::<Vec<_>>();

        let starts = tokens
            .iter()
            .enumerate()
            .filter(|&(_, &t)| t == "Part")
            .map(|(i, _)| i)
            .collect::<Vec<_>>();

        starts
            .iter()
            .enumerate()
            .map(|(n, &start)| {
                let end = starts.get(n + 1).copied().unwrap_or(tokens.len());
                Part {
                    tokens: tokens[start..end].to_vec(),
                    pos: 0,
                }
            })
            .collect()
    }

    /// Advances past the next occurrence of `key` and returns what follows.
    fn value_after(&mut self, key: &str) -> Option<&'a str> {
        let found = self.tokens[self.pos..].iter().position(|&t| t == key)?;
        self.pos += found + 1;
        let value = self.tokens.get(self.pos)?;
        self.pos += 1;
        Some(value)
    }

    fn int_after(&mut self, key: &str) -> Option<i32> {
        self.value_after(key)?.parse().ok()
    }

    fn name_after(&mut self, key: &str) -> Option<&'a str> {
        // The original strips the quotes that names are usually written with.
        Some(self.value_after(key)?.trim_matches('"'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::level::moving::{Location, MobileLocation, Mode as VotMode};
    use std::sync::Arc;
    use vot::Frame;

    const SIZE: (i32, i32) = (256, 256);

    fn level_for(size: (i32, i32)) -> crate::level::Level {
        use crate::config::settings;
        use crate::level::LevelConfig;
        let total = (size.0 * size.1) as usize;
        crate::level::Level {
            size,
            flood_map: vec![0; size.1 as usize].into_boxed_slice(),
            height: vec![100u8; total].into_boxed_slice(),
            meta: vec![0u8; total].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    fn land_with(names: &[&str]) -> MovingLand {
        let mut land = MovingLand::default();
        for name in names {
            let frames = (0..4)
                .map(|i| Frame {
                    pos: (0, 0),
                    size: (1, 1),
                    period: 1,
                    surface_type: -1,
                    delta: vec![i as u8 + 1],
                    terrain: Vec::new(),
                    sign_bits: vec![0],
                })
                .collect();
            land.locations.push(Location::new(
                Arc::new(MobileLocation {
                    name: name.to_string(),
                    mode: VotMode::Absolute,
                    dry_terrain: 0,
                    impulse: 0,
                    // Key phase 1 is the far end of the loop, 0 the start.
                    key_phases: [0, 2, 0, 0],
                    frames,
                }),
                (0, 0),
            ));
        }
        land
    }

    fn sensor(name: &str, x: i32, y: i32, radius: i32) -> Sensor {
        Sensor {
            pos: (x, y, 100),
            kind: vlc::sensor_kind::SENSOR,
            radius,
            name: name.to_string(),
            z_range: (50, 150),
            direction: (0, 0, 0),
            power: 0,
            data5: 0,
            data6: 0,
        }
    }

    fn triggers_with(sensors: Vec<Sensor>, text: &str, land: &MovingLand) -> Triggers {
        let mut triggers = Triggers {
            owners: vec![None; sensors.len()],
            enabled: vec![true; sensors.len()],
            sensors,
            engines: Vec::new(),
            enables: Vec::new(),
        };
        triggers.parse(text, land);
        triggers
    }

    const DOOR_LST: &str = "\
NumEngine 1
Part 0
EngineType 0
MLName bridge
ActivePhase 1
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 3
NumSensorLink 2
SensorName west
SensorName east
LockFlag 0
Luck 0
";

    #[test]
    fn parses_a_door() {
        let land = land_with(&["bridge"]);
        let triggers = triggers_with(
            vec![sensor("west", 10, 10, 5), sensor("east", 40, 10, 5)],
            DOOR_LST,
            &land,
        );

        assert_eq!(triggers.engines.len(), 1);
        let engine = &triggers.engines[0];
        assert_eq!(engine.location, Some(0));
        assert_eq!(engine.active_phase, 1);
        assert_eq!(engine.deactive_phase, 0);
        assert_eq!(engine.sound_id, 3);
        match engine.kind {
            Kind::Door { ref sensors, lock } => {
                assert_eq!(sensors, &[0, 1]);
                assert_eq!(lock, 0);
            }
            _ => panic!("expected a door"),
        }
        assert_eq!(triggers.owners, [Some(0), Some(0)]);
    }

    #[test]
    fn touch_opens_and_release_closes() {
        let mut land = land_with(&["bridge"]);
        let mut triggers = triggers_with(vec![sensor("west", 10, 10, 5)], DOOR_LST, &land);
        triggers.reset_locations(&mut land);
        assert_eq!(land.locations[0].go_phase(), 0);

        // Nothing near the sensor: the door stays shut.
        triggers.update(&mut land);
        assert!(!triggers.engines[0].is_open());

        // Drive in. The sensor radius is 5 and the car's is 3.
        triggers.touch((12, 11, 100), 3, SIZE);
        assert_eq!(triggers.engines[0].touch_count(), 1);
        triggers.update(&mut land);
        assert!(triggers.engines[0].is_open());
        assert_eq!(land.locations[0].go_phase(), 2, "sent to key phase 1");
        assert_eq!(triggers.engines[0].touch_count(), 0, "counter is consumed");

        // The door has to finish moving before it reacts again.
        let mut regions = Vec::new();
        for _ in 0..8 {
            land.update(&mut level_for(SIZE), &mut regions);
        }
        assert!(land.locations[0].is_go_finish());

        // Drive out: no touches this quant, so it closes.
        triggers.update(&mut land);
        assert!(!triggers.engines[0].is_open());
        assert_eq!(land.locations[0].go_phase(), 0);
    }

    #[test]
    fn touch_respects_range_and_altitude() {
        let land = land_with(&["bridge"]);
        let mut triggers = triggers_with(vec![sensor("west", 10, 10, 5)], DOOR_LST, &land);

        // Too far in the plane.
        triggers.touch((30, 10, 100), 3, SIZE);
        assert_eq!(triggers.engines[0].touch_count(), 0);
        // In range, but flying way above the band.
        triggers.touch((12, 11, 400), 3, SIZE);
        assert_eq!(triggers.engines[0].touch_count(), 0);
        // In range and in the band.
        triggers.touch((12, 11, 100), 3, SIZE);
        assert_eq!(triggers.engines[0].touch_count(), 1);
    }

    #[test]
    fn touch_wraps_around_the_level() {
        let land = land_with(&["bridge"]);
        let mut triggers = triggers_with(vec![sensor("west", 2, 2, 6)], DOOR_LST, &land);
        // Standing just the other side of the seam.
        triggers.touch((SIZE.0 - 2, SIZE.1 - 1, 100), 3, SIZE);
        assert_eq!(triggers.engines[0].touch_count(), 1);
    }

    #[test]
    fn a_sensor_no_engine_claims_never_fires() {
        let land = land_with(&["bridge"]);
        let mut triggers = triggers_with(
            vec![sensor("west", 10, 10, 5), sensor("stray", 10, 10, 5)],
            DOOR_LST,
            &land,
        );
        triggers.touch((10, 10, 100), 3, SIZE);
        // Both sensors cover the spot, but only the linked one counts.
        assert_eq!(triggers.engines[0].touch_count(), 1);
    }

    const TIRISTOR_LST: &str = "\
Part 0
EngineType 7
MLName gate
ActivePhase 1
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 0
NumSensorLink 1
SensorName pad
LockFlag 0
Luck 0
";

    #[test]
    fn tiristor_latches_open() {
        let mut land = land_with(&["gate"]);
        let mut triggers = triggers_with(vec![sensor("pad", 10, 10, 5)], TIRISTOR_LST, &land);
        triggers.reset_locations(&mut land);

        triggers.touch((10, 10, 100), 3, SIZE);
        triggers.update(&mut land);
        assert!(triggers.engines[0].is_open());

        // Let it finish, then stop touching. A door would close; this does not.
        let mut regions = Vec::new();
        for _ in 0..8 {
            land.update(&mut level_for(SIZE), &mut regions);
        }
        triggers.update(&mut land);
        assert!(triggers.engines[0].is_open());
    }

    const CYCLIC_LST: &str = "\
Part 0
EngineType 4
MLName lift
ActivePhase 1
DeactivePhase 0
ActiveTime 1
DeactiveTime 1
SoundID 0
ActionMode 2
NumActionLink 1
ActionName gateway
";

    #[test]
    fn cyclic_runs_on_its_own_and_gates_action_sensors() {
        let mut land = land_with(&["lift"]);
        let mut triggers = triggers_with(vec![sensor("gateway", 10, 10, 5)], CYCLIC_LST, &land);
        match triggers.engines[0].kind {
            Kind::Cyclic {
                ref actions,
                action_mode,
            } => {
                assert_eq!(actions, &[0]);
                assert_eq!(action_mode, Some(Mode::AcceptProcessEnd));
            }
            _ => panic!("expected a cyclic engine"),
        }
        // Action sensors start disabled.
        assert!(!triggers.sensor_enabled(0));

        triggers.reset_locations(&mut land);
        let mut level = level_for(SIZE);
        let mut regions = Vec::new();

        // Drive it far enough to reach the phase that opens the gateway.
        let mut opened = false;
        for _ in 0..40 {
            triggers.update(&mut land);
            land.update(&mut level, &mut regions);
            opened |= triggers.sensor_enabled(0);
        }
        assert!(opened, "the cyclic engine never reached its action mode");
    }

    #[test]
    fn unknown_engine_types_are_skipped_not_misparsed() {
        let land = land_with(&["bridge"]);
        let text = "\
Part 0
EngineType 5
MLName bridge
ActivePhase 1
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 0
PassageName somewhere
Part 1
EngineType 0
MLName bridge
ActivePhase 3
DeactivePhase 2
ActiveTime 0
DeactiveTime 0
SoundID 0
NumSensorLink 0
LockFlag 0
Luck 0
";
        let triggers = triggers_with(Vec::new(), text, &land);
        assert_eq!(triggers.engines.len(), 2);
        assert!(matches!(triggers.engines[0].kind, Kind::Unsupported(5)));
        // The door after it parsed its own values, not the passage's.
        assert!(matches!(triggers.engines[1].kind, Kind::Door { .. }));
        assert_eq!(triggers.engines[1].active_phase, 3);
        assert_eq!(triggers.engines[1].deactive_phase, 2);
    }

    #[test]
    fn missing_names_are_reported_not_fatal() {
        let land = land_with(&["bridge"]);
        let text = "\
Part 0
EngineType 0
MLName nosuchland
ActivePhase 1
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 0
NumSensorLink 1
SensorName nosuchsensor
LockFlag 0
Luck 0
";
        let triggers = triggers_with(vec![sensor("other", 0, 0, 1)], text, &land);
        assert_eq!(triggers.engines.len(), 1);
        assert_eq!(triggers.engines[0].location, None);
        match triggers.engines[0].kind {
            Kind::Door { ref sensors, .. } => assert!(sensors.is_empty()),
            _ => panic!("expected a door"),
        }
        // An engine without a location does nothing.
        let mut land = land;
        triggers.reset_locations(&mut land);
    }

    #[test]
    fn none_ml_name_leaves_the_engine_unlinked() {
        let land = land_with(&["bridge"]);
        let text = "\
Part 0
EngineType 0
MLName None
ActivePhase 0
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 0
NumSensorLink 0
LockFlag 0
Luck 0
";
        let triggers = triggers_with(Vec::new(), text, &land);
        assert_eq!(triggers.engines[0].location, None);
    }

    #[test]
    fn locations_without_an_engine_sit_still() {
        let mut level = level_for(SIZE);
        let mut land = land_with(&["bridge", "waterfall"]);
        let triggers = triggers_with(Vec::new(), DOOR_LST, &land);
        triggers.reset_locations(&mut land);

        // Both are parked at phase 0 - the engine-less one included, which is
        // where `checkQuant` leaves every location in the original.
        assert_eq!(land.locations[0].go_phase(), 0);
        assert_eq!(land.locations[1].go_phase(), 0);

        let mut regions = Vec::new();
        for _ in 0..10 {
            land.update(&mut level, &mut regions);
        }
        assert!(regions.is_empty(), "nothing should animate unprompted");
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let land = land_with(&["bridge"]);
        let text = "\
// a comment
NumEngine 1

Part 0
// another one
EngineType 0
MLName bridge
ActivePhase 1
DeactivePhase 0
ActiveTime 0
DeactiveTime 0
SoundID 0
NumSensorLink 0
LockFlag 2
Luck 0
";
        let triggers = triggers_with(Vec::new(), text, &land);
        assert_eq!(triggers.engines.len(), 1);
        match triggers.engines[0].kind {
            Kind::Door { lock, .. } => assert_eq!(lock, DOOR_CLOSE_LOCK),
            _ => panic!("expected a door"),
        }
    }

    #[test]
    fn a_locked_door_never_moves() {
        let land = land_with(&["bridge"]);
        let text = DOOR_LST.replace("LockFlag 0", "LockFlag 1");
        let mut land = land;
        let mut triggers = triggers_with(vec![sensor("west", 10, 10, 5)], &text, &land);
        triggers.reset_locations(&mut land);

        triggers.touch((10, 10, 100), 3, SIZE);
        triggers.update(&mut land);
        assert!(!triggers.engines[0].is_open(), "open lock holds it shut");
    }

    #[test]
    fn wrap_delta_takes_the_short_way() {
        assert_eq!(wrap_delta(3, 256), 3);
        assert_eq!(wrap_delta(-3, 256), -3);
        assert_eq!(wrap_delta(255, 256), -1);
        assert_eq!(wrap_delta(-255, 256), 1);
    }
}
