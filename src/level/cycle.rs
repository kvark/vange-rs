//! The story cycles a world moves through, and the colours each one paints
//! it in.
//!
//! A port of `uvsBunch` (`src/uvs/univang.cpp`) and the `CPAL_CHANGE_CYCLE`
//! arm of `PalPoint` (`src/units/hobj.cpp`). Every escave belongs to a
//! bunch, and a bunch runs through a handful of named stages - "Plump-up",
//! "Eleerection", "Gulp-down" on Fostral - each with its own palette file
//! and its own share of the world's light.
//!
//! The stage advances when enough *cirt* has been delivered to the escave.
//! Cirt is gathered by driving near a stage's dolly, carried in a
//! cirtainer, and handed over on arrival; meanwhile the stages that are not
//! current lose half their stock on their own schedule, so a stage nobody
//! feeds slips back. When one does come round, the whole palette
//! cross-fades to the new stage's over a hundred quants and the world's
//! light ramps with it.
//!
//! Two things the original does that are not reproduced. Its dollies wander
//! and are placed in a *different* world from their escave, reachable only
//! by travelling between worlds; here they sit still in the world their
//! escave is in, since there is nowhere else to go yet. And it starts each
//! bunch on a random stage, where this starts on the first, so that a level
//! looks the same from one run to the next.

use super::{Level, Texel, read_palette_bytes};
use crate::config::{bunches, escaves};

use std::ops::Range;

/// `V_CIRT_R` - beyond this there is no cirt at all. The ladder inside it
/// is `uvsDolly::getCirt`'s.
pub const CIRT_RADIUS: i32 = 1 << 12;

/// The most one dolly can give a single cirtainer, which is what three bits
/// per dolly buys the original.
pub const CIRT_PER_DOLLY: i32 = 7;

/// How close to the escave counts as arriving.
pub const DELIVERY_RADIUS: i32 = 64;

/// Quants a cycle change takes to fade, from `PalCD.Set(CPAL_CHANGE_CYCLE, 100, ..)`.
pub const FADE_QUANTS: i32 = 100;

/// `WorldLightParam` of the original, out of 256. The three worlds that
/// have cycles all share this row: the world brightens into its second
/// stage and darkens well below the first in its third.
const STAGE_LIGHT: [i32; 3] = [205, 256, 160];

/// Fixed-point bits the fade interpolates in, as the original does.
const FADE_BITS: u32 = 16;

/// One stage of a bunch.
pub struct Stage {
    pub name: String,
    /// `cirtMAX` - cirt needed to see this stage out.
    pub cirt_max: i32,
    /// `time` - quants between halvings of this stage's stock while some
    /// other stage is the current one.
    pub decay: i32,
    /// The colours this stage paints the world in.
    pub palette: Box<[[u8; 4]; 0x100]>,
    /// Scale on the world's light while this stage is current.
    pub light: f32,
    /// Where this stage's cirt comes from.
    pub dolly: (i32, i32),
}

/// What one car is carrying, a count per stage.
///
/// The original packs three bits per dolly into one item field and folds
/// new gatherings in with a bitwise or, so a cirtainer holds what the best
/// spot visited was worth rather than a running total. That is kept.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cirtainer {
    held: Vec<i32>,
}

impl Cirtainer {
    pub fn is_empty(&self) -> bool {
        self.held.iter().all(|&c| c == 0)
    }

    pub fn held(&self) -> &[i32] {
        &self.held
    }

    pub fn clear(&mut self) {
        self.held.iter_mut().for_each(|c| *c = 0);
    }
}

/// A world's bunch: its stages, which one is current, and what has been
/// delivered towards each.
pub struct Bunch {
    pub escave: String,
    pub escave_pos: (i32, i32),
    pub stages: Vec<Stage>,
    current: usize,
    /// `cirtQ` per stage.
    banked: Vec<i32>,
    /// Quants elapsed, which the decay schedule counts against.
    counter: i32,
    fade: Option<Fade>,
    light: f32,
    size: (i32, i32),
}

/// A cycle change in progress.
struct Fade {
    /// Colour being interpolated, in [`FADE_BITS`] fixed point.
    at: Vec<[i32; 3]>,
    delta: Vec<[i32; 3]>,
    light_at: i32,
    light_delta: i32,
    left: i32,
    target: usize,
}

/// Reads `bunches.prm` from a path, for callers that have no `Settings`.
pub fn load_bunches(path: &std::path::Path) -> Option<Vec<bunches::Bunch>> {
    Some(bunches::load(std::fs::File::open(path).ok()?))
}

/// Reads `escaves.prm` the same way.
pub fn load_escaves(path: &std::path::Path) -> Option<Vec<escaves::Escave>> {
    Some(escaves::load(std::fs::File::open(path).ok()?))
}

impl Bunch {
    /// Builds the bunch of the world `level_name` names, if it has one.
    ///
    /// Three of the shipped worlds do; the rest keep the single palette
    /// their INI points at and never change cycle.
    pub fn load(
        level_name: &str,
        level: &Level,
        bunches: &[bunches::Bunch],
        escaves: &[escaves::Escave],
        mut palette: impl FnMut(&str) -> Option<Vec<u8>>,
    ) -> Option<Self> {
        let escave = escaves
            .iter()
            .find(|e| e.world.eq_ignore_ascii_case(level_name))?;
        let bunch = bunches.iter().find(|b| b.escave == escave.name)?;

        let stages = bunch
            .cycles
            .iter()
            .enumerate()
            .map(|(i, cycle)| {
                let bytes = palette(&cycle.palette_path)?;
                Some(Stage {
                    name: cycle.name.clone(),
                    cirt_max: cycle.cirt_max.max(1) as i32,
                    decay: cycle.radiance_time.max(1) as i32,
                    palette: Box::new(read_palette_bytes(&bytes, Some(&level.terrains))),
                    light: STAGE_LIGHT.get(i).copied().unwrap_or(256) as f32 / 256.0,
                    dolly: place_dolly(level, escave.coordinates, i, bunch.cycles.len()),
                })
            })
            .collect::<Option<Vec<_>>>()?;
        if stages.is_empty() {
            return None;
        }

        log::info!(
            "{} runs {} cycles: {}",
            escave.name,
            stages.len(),
            stages
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let light = stages[0].light;
        Some(Bunch {
            escave: escave.name.clone(),
            escave_pos: escave.coordinates,
            banked: vec![0; stages.len()],
            stages,
            current: 0,
            counter: 0,
            fade: None,
            light,
            size: level.size,
        })
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn stage(&self) -> &Stage {
        &self.stages[self.current]
    }

    /// How much cirt each stage has towards its own `cirt_max`.
    pub fn banked(&self) -> &[i32] {
        &self.banked
    }

    /// The scale the current cycle puts on the world's light.
    pub fn light(&self) -> f32 {
        self.light
    }

    /// Whether a cycle change is being faded through, during which the
    /// dynamic palette has to stand aside - the original suspends
    /// `pal_iter` for exactly as long.
    pub fn is_fading(&self) -> bool {
        self.fade.is_some()
    }

    /// The palette this bunch has settled on, once nothing is fading.
    pub fn settled_palette(&self) -> &[[u8; 4]; 0x100] {
        &self.stages[self.current].palette
    }

    /// Tops up what a car at `pos` is carrying.
    pub fn gather(&self, pos: (i32, i32), into: &mut Cirtainer) {
        into.held.resize(self.stages.len(), 0);
        for (stage, held) in self.stages.iter().zip(into.held.iter_mut()) {
            // `p1 |= getCirt(..)`: a cirtainer keeps the best it has seen
            // rather than adding up, so parking on a dolly is no better
            // than driving past it.
            *held |= cirt_at(distance(pos, stage.dolly, self.size));
        }
    }

    /// Empties a car's cirtainer into the escave, if it has arrived and has
    /// anything to hand over. Returns what was delivered.
    pub fn deliver(&mut self, pos: (i32, i32), from: &mut Cirtainer) -> i32 {
        if from.is_empty() || distance(pos, self.escave_pos, self.size) > DELIVERY_RADIUS {
            return 0;
        }
        let mut total = 0;
        for (banked, held) in self.banked.iter_mut().zip(from.held.iter()) {
            *banked += *held;
            total += *held;
        }
        from.clear();
        total
    }

    /// One quant of the bunch. Writes into `level.palette` while a cycle
    /// change is fading, and returns the entries that moved.
    pub fn quant(&mut self, level: &mut Level) -> Range<u32> {
        if let Some(range) = self.step_fade(level) {
            return range;
        }

        self.counter += 1;
        // `QuantCirt`: a stage nobody is feeding loses half its stock.
        for (i, stage) in self.stages.iter().enumerate() {
            if i != self.current && self.counter % stage.decay == 0 {
                self.banked[i] >>= 1;
            }
        }

        if self.banked[self.current] < self.stages[self.current].cirt_max {
            return 0..0;
        }
        self.banked[self.current] -= self.stages[self.current].cirt_max;
        let target = (self.current + 1) % self.stages.len();
        log::info!(
            "{}: cycle {} -> {}",
            self.escave,
            self.stages[self.current].name,
            self.stages[target].name
        );
        self.start_fade(level, target);
        0..0
    }

    /// Jumps straight to a cycle without the fade. For a menu, and for
    /// tests that care about the destination rather than the journey.
    pub fn set_cycle(&mut self, index: usize, level: &mut Level) -> Range<u32> {
        if index >= self.stages.len() {
            return 0..0;
        }
        self.fade = None;
        self.current = index;
        self.light = self.stages[index].light;
        level.palette = *self.stages[index].palette;
        0..0x100
    }

    fn start_fade(&mut self, level: &Level, target: usize) {
        let to = &self.stages[target].palette;
        let at = level
            .palette
            .iter()
            .map(|c| {
                [
                    (c[0] as i32) << FADE_BITS,
                    (c[1] as i32) << FADE_BITS,
                    (c[2] as i32) << FADE_BITS,
                ]
            })
            .collect::<Vec<_>>();
        let delta = at
            .iter()
            .zip(to.iter())
            .map(|(from, want)| {
                let mut d = [0i32; 3];
                for c in 0..3 {
                    d[c] = (((want[c] as i32) << FADE_BITS) - from[c]) / FADE_QUANTS;
                }
                d
            })
            .collect();
        let light_at = (self.light * (1 << FADE_BITS) as f32) as i32;
        let light_want = (self.stages[target].light * (1 << FADE_BITS) as f32) as i32;
        self.fade = Some(Fade {
            at,
            delta,
            light_at,
            light_delta: (light_want - light_at) / FADE_QUANTS,
            left: FADE_QUANTS,
            target,
        });
    }

    /// Advances a fade by one quant. `None` when there is nothing fading.
    fn step_fade(&mut self, level: &mut Level) -> Option<Range<u32>> {
        let fade = self.fade.as_mut()?;
        fade.left -= 1;
        if fade.left <= 0 {
            let target = fade.target;
            self.fade = None;
            self.current = target;
            self.light = self.stages[target].light;
            level.palette = *self.stages[target].palette;
            return Some(0..0x100);
        }

        for (entry, (at, delta)) in fade.at.iter_mut().zip(fade.delta.iter()).enumerate() {
            for c in 0..3 {
                at[c] += delta[c];
                level.palette[entry][c] = (at[c] >> FADE_BITS).clamp(0, 255) as u8;
            }
        }
        fade.light_at += fade.light_delta;
        self.light = fade.light_at as f32 / (1 << FADE_BITS) as f32;
        Some(0..0x100)
    }
}

/// `getCirt`'s ladder: cirt is thin everywhere in the world and thickens
/// towards the dolly.
pub fn cirt_at(distance: i32) -> i32 {
    const LADDER: [(i32, i32); 7] = [
        (CIRT_RADIUS, 0),
        (CIRT_RADIUS / 2, 1),
        (CIRT_RADIUS / 4, 2),
        (CIRT_RADIUS / 6, 3),
        (CIRT_RADIUS / 8, 4),
        (CIRT_RADIUS / 16, 5),
        (CIRT_RADIUS / 40, 6),
    ];
    for (from, amount) in LADDER {
        if distance >= from {
            return amount;
        }
    }
    CIRT_PER_DOLLY
}

/// Distance across a level that wraps.
fn distance(a: (i32, i32), b: (i32, i32), size: (i32, i32)) -> i32 {
    let span = |d: i32, total: i32| {
        let d = d.abs() % total;
        d.min(total - d)
    };
    let (dx, dy) = (span(a.0 - b.0, size.0), span(a.1 - b.1, size.1));
    ((dx as i64 * dx as i64 + dy as i64 * dy as i64) as f64).sqrt() as i32
}

/// Puts a stage's dolly out at arm's length from the escave, spaced evenly
/// around it so the stages send you to different quarters of the world.
///
/// The original scatters dollies at random through the *other* worlds and
/// lets them wander. With one world loaded there is nowhere else to put
/// them, and a fixed spot is at least somewhere a player can learn.
fn place_dolly(level: &Level, escave: (i32, i32), index: usize, count: usize) -> (i32, i32) {
    let reach = level.size.0.min(level.size.1) as f32 * 0.3;
    let turn = std::f32::consts::TAU * index as f32 / count.max(1) as f32;
    let wanted = (
        escave.0 + (reach * turn.cos()) as i32,
        escave.1 + (reach * turn.sin()) as i32,
    );
    dry_land_near(level, wanted)
}

/// Nudges a spot off the water, which is no use to anyone driving.
fn dry_land_near(level: &Level, wanted: (i32, i32)) -> (i32, i32) {
    let is_water = |coord: (i32, i32)| {
        let terrain = match level.get(coord) {
            Texel::Single(p) => p.1,
            Texel::Dual { high, .. } => high.1,
        };
        terrain == 0
    };
    if !is_water(wanted) {
        return wrap(level, wanted);
    }
    for ring in 1..40 {
        let step = ring * 16;
        for (dx, dy) in [(step, 0), (0, step), (-step, 0), (0, -step)] {
            let coord = (wanted.0 + dx, wanted.1 + dy);
            if !is_water(coord) {
                return wrap(level, coord);
            }
        }
    }
    wrap(level, wanted)
}

fn wrap(level: &Level, coord: (i32, i32)) -> (i32, i32) {
    (
        coord.0.rem_euclid(level.size.0),
        coord.1.rem_euclid(level.size.1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::TerrainConfig;

    const SIZE: i32 = 512;

    fn test_level() -> Level {
        let total = (SIZE * SIZE) as usize;
        let terrains = (0..8u8)
            .map(|i| TerrainConfig {
                colors: (i * 16)..(i * 16 + 15),
                ..TerrainConfig::default()
            })
            .collect::<Vec<_>>();
        Level {
            size: (SIZE, SIZE),
            flood_map: vec![0; SIZE as usize].into_boxed_slice(),
            height: vec![100; total].into_boxed_slice(),
            // Terrain 1, so nothing reads as water.
            meta: vec![1 << 3; total].into_boxed_slice(),
            palette: [[10, 10, 10, 0xFF]; 0x100],
            terrains: terrains.into_boxed_slice(),
            geometry: settings::Geometry::default(),
        }
    }

    fn stage(name: &str, cirt_max: i32, decay: i32, shade: u8, dolly: (i32, i32)) -> Stage {
        Stage {
            name: name.to_string(),
            cirt_max,
            decay,
            palette: Box::new([[shade, shade, shade, 0xFF]; 0x100]),
            light: shade as f32 / 255.0,
            dolly,
        }
    }

    fn bunch() -> Bunch {
        let stages = vec![
            stage("first", 8, 10, 60, (100, 100)),
            stage("second", 8, 10, 200, (400, 100)),
            stage("third", 8, 10, 120, (100, 400)),
        ];
        Bunch {
            escave: "Test".to_string(),
            escave_pos: (256, 256),
            banked: vec![0; stages.len()],
            light: stages[0].light,
            stages,
            current: 0,
            counter: 0,
            fade: None,
            size: (SIZE, SIZE),
        }
    }

    #[test]
    fn cirt_thickens_towards_the_dolly() {
        assert_eq!(cirt_at(CIRT_RADIUS), 0, "out of reach entirely");
        assert_eq!(cirt_at(CIRT_RADIUS - 1), 1, "just inside, barely worth it");
        assert_eq!(cirt_at(0), CIRT_PER_DOLLY, "standing on it");
        // Every step of the ladder has to be reachable and monotone.
        let mut last = CIRT_PER_DOLLY + 1;
        for d in (0..CIRT_RADIUS).step_by(16) {
            let here = cirt_at(d);
            assert!(here <= last, "cirt must not rise with distance");
            last = here;
        }
    }

    #[test]
    fn a_cirtainer_keeps_the_best_spot_rather_than_a_running_total() {
        let b = bunch();
        let mut held = Cirtainer::default();
        b.gather((100, 100), &mut held);
        let first = held.held()[0];
        assert_eq!(first, CIRT_PER_DOLLY, "sitting on the first dolly");
        for _ in 0..10 {
            b.gather((100, 100), &mut held);
        }
        assert_eq!(held.held()[0], first, "loitering earns nothing more");
    }

    #[test]
    fn every_stage_gathers_from_its_own_dolly() {
        let b = bunch();
        let mut held = Cirtainer::default();
        b.gather((400, 100), &mut held);
        assert_eq!(held.held()[1], CIRT_PER_DOLLY, "on the second dolly");
        assert!(held.held()[0] < CIRT_PER_DOLLY, "and away from the first");
    }

    #[test]
    fn cirt_is_only_handed_over_at_the_escave() {
        let mut b = bunch();
        let mut held = Cirtainer::default();
        b.gather((100, 100), &mut held);

        assert_eq!(b.deliver((100, 100), &mut held), 0, "not there yet");
        assert!(!held.is_empty(), "and still carrying it");

        let delivered = b.deliver((256, 256), &mut held);
        assert!(delivered > 0);
        assert!(held.is_empty(), "the cirtainer is emptied");
        assert_eq!(b.banked()[0], CIRT_PER_DOLLY);
    }

    #[test]
    fn an_empty_cirtainer_is_not_a_delivery() {
        let mut b = bunch();
        let mut held = Cirtainer::default();
        assert_eq!(b.deliver((256, 256), &mut held), 0);
    }

    #[test]
    fn enough_cirt_turns_the_cycle_over() {
        let mut level = test_level();
        let mut b = bunch();
        let mut held = Cirtainer::default();
        for _ in 0..2 {
            b.gather((100, 100), &mut held);
            b.deliver((256, 256), &mut held);
        }
        assert!(b.banked()[0] >= b.stages[0].cirt_max);

        assert_eq!(b.current(), 0);
        b.quant(&mut level);
        assert!(b.is_fading(), "the change fades rather than snapping");
        assert_eq!(b.current(), 0, "and holds the old cycle until it lands");

        for _ in 0..FADE_QUANTS {
            b.quant(&mut level);
        }
        assert!(!b.is_fading());
        assert_eq!(b.current(), 1);
        assert_eq!(level.palette[0], [200, 200, 200, 0xFF], "the new colours");
    }

    #[test]
    fn the_fade_walks_the_colours_across_rather_than_cutting() {
        let mut level = test_level();
        let mut b = bunch();
        b.banked[0] = 8;
        b.quant(&mut level);

        let mut seen = Vec::new();
        for _ in 0..FADE_QUANTS {
            b.quant(&mut level);
            seen.push(level.palette[0][0]);
        }
        assert!(
            seen[0] > 10 && seen[0] < 40,
            "starts near the old: {:?}",
            seen[0]
        );
        assert_eq!(*seen.last().unwrap(), 200, "and arrives exactly");
        assert!(
            seen.windows(2).all(|w| w[1] >= w[0]),
            "and only ever moves towards it"
        );
    }

    #[test]
    fn the_light_ramps_with_the_colours() {
        let mut level = test_level();
        let mut b = bunch();
        let before = b.light();
        b.banked[0] = 8;
        b.quant(&mut level);
        b.quant(&mut level);
        let midway = b.light();
        assert!(
            midway > before,
            "it should be climbing: {} -> {}",
            before,
            midway
        );

        for _ in 0..FADE_QUANTS {
            b.quant(&mut level);
        }
        assert!((b.light() - b.stages[1].light).abs() < 0.01);
    }

    #[test]
    fn a_stage_nobody_feeds_slips_back() {
        let mut level = test_level();
        let mut b = bunch();
        b.banked[1] = 64;
        // Ten quants to the halving, and the current stage is exempt.
        b.banked[0] = 4;
        for _ in 0..10 {
            b.quant(&mut level);
        }
        assert_eq!(b.banked()[1], 32, "the idle stage lost half");
        assert_eq!(b.banked()[0], 4, "the current one keeps its stock");
    }

    #[test]
    fn the_cycles_come_round_again() {
        let mut level = test_level();
        let mut b = bunch();
        for expected in [1, 2, 0] {
            let now = b.current();
            b.banked[now] = b.stages[now].cirt_max;
            b.quant(&mut level);
            for _ in 0..FADE_QUANTS {
                b.quant(&mut level);
            }
            assert_eq!(b.current(), expected);
        }
    }

    #[test]
    fn nothing_fades_while_the_cycle_is_short_of_cirt() {
        let mut level = test_level();
        let mut b = bunch();
        b.banked[0] = 7;
        for _ in 0..50 {
            assert_eq!(b.quant(&mut level), 0..0);
        }
        assert!(!b.is_fading());
        assert_eq!(level.palette[0], [10, 10, 10, 0xFF]);
    }

    #[test]
    fn setting_a_cycle_outright_skips_the_fade() {
        let mut level = test_level();
        let mut b = bunch();
        b.set_cycle(2, &mut level);
        assert_eq!(b.current(), 2);
        assert!(!b.is_fading());
        assert_eq!(level.palette[0], [120, 120, 120, 0xFF]);
        assert_eq!(b.light(), b.stages[2].light);
    }

    #[test]
    fn dollies_are_spread_out_and_stay_off_the_water() {
        let mut level = test_level();
        // A lake right where the first dolly wants to be.
        let bits = level.terrain_bits();
        for y in 300..420 {
            for x in 380..500 {
                let i = level.wrap((x, y));
                level.meta[i] = bits.write(0);
            }
        }
        let places = (0..3)
            .map(|i| place_dolly(&level, (256, 256), i, 3))
            .collect::<Vec<_>>();
        for (i, &p) in places.iter().enumerate() {
            let terrain = match level.get(p) {
                Texel::Single(t) => t.1,
                Texel::Dual { high, .. } => high.1,
            };
            assert_ne!(terrain, 0, "dolly {} was left in the water at {:?}", i, p);
            assert!(
                distance(p, (256, 256), (SIZE, SIZE)) > 32,
                "dolly {} is on top of the escave",
                i
            );
        }
        for (i, a) in places.iter().enumerate() {
            for b in places.iter().skip(i + 1) {
                assert!(distance(*a, *b, (SIZE, SIZE)) > 32, "dollies overlap");
            }
        }
    }

    #[test]
    fn the_level_seam_is_not_a_wall() {
        let b = bunch();
        // Two texels apart across the seam, not most of a level.
        assert_eq!(distance((1, 10), (SIZE - 1, 10), b.size), 2);
    }
}
