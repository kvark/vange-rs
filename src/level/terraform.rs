//! What the wheels leave behind them.
//!
//! This is a port of `pixSetR` (`src/terra/land.cpp`) and
//! `DrawMechosWheelUp` (`src/units/hobj.cpp`) of the original game, which
//! together are its tyre tracks. Every wheel that is rolling over soft
//! ground stamps a tread pattern into the altitude plane along the stretch
//! it covered since the last quant: a short bar across the track, raised
//! once every [`Config::tread`] texels and cut everywhere else.
//!
//! The pattern is deliberately lopsided - two texels are cut for each one
//! raised - so the net effect of driving somewhere is that the ground sinks.
//! Circling the same patch digs a bowl out of it, which is the original's
//! whole terrain-deformation mechanic and not a side effect.
//!
//! Like the moving land, everything here mutates [`Level`] in place and
//! reports the touched rectangles so the renderer can re-upload them.

use super::{DELTA_BITS, DELTA_MASK, DOUBLE_LEVEL, Level, Region, Texel};

/// The terrain type a wheel is allowed to disturb.
///
/// `MAIN_TERRAIN_INDEX` of `src/terra/world.h`: the plain drivable ground.
/// Everything else - water, lava, the various special surfaces - keeps its
/// shape no matter how hard it is driven over, which is what stops a track
/// from being cut across a river.
pub const MAIN_TERRAIN: u8 = 1;

/// How far above the surface a wheel still counts as touching it.
///
/// `get_upper_height(...) < round(rg.z) + 15` of `Object::analyse_dynamics`.
pub const MAX_CONTACT_HEIGHT: f32 = 15.0;

/// Tunables of the tread pattern.
///
/// The defaults are the constants the original passes to
/// `DrawMechosWheelUp(x0, y0, x1, y1, 8, 3, -1, nx, ny, 3)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Config {
    /// Master switch. Off restores terrain that only the moving land edits.
    pub enabled: bool,
    /// Altitude units one stamp moves the surface.
    pub depth: i32,
    /// Texels along the track between two raised bars. The other
    /// `tread - 1` texels of each period are cut.
    pub tread: u8,
    /// Stamps in one bar, laid out across the track.
    pub bar: u8,
    /// Texels between those stamps. `8/3` spreads a three-stamp bar over
    /// eight texels, about the width of a wheel.
    pub spacing: f32,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            enabled: true,
            depth: 1,
            tread: 3,
            bar: 3,
            spacing: 8.0 / 3.0,
        }
    }
}

/// One stretch of ground a single wheel rolled over during a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Track {
    /// Where the wheel was at the end of the previous step.
    pub from: (i32, i32),
    /// Where it is now.
    pub to: (i32, i32),
    /// The car's lateral axis, flattened onto the level. `track_nx` and
    /// `track_ny` of the original: unit length on the level, and shortened
    /// by the car's pitch, so a wheel on a slope lays a narrower bar.
    pub across: (f32, f32),
}

/// Per-agent track state: where each wheel last touched the ground, and the
/// stretches it has covered since the terrain was last updated.
///
/// The physics fills this in while it runs - in parallel, over an immutable
/// level - and the game drains it afterwards, when it can borrow the level
/// mutably. That split is the only reason the two halves are separate.
#[derive(Default)]
pub struct Tracks {
    /// Last contact per wheel. `None` once the wheel has left the ground,
    /// so that a jump does not draw a track across everything it flew over.
    last: Vec<Option<(i32, i32)>>,
    pending: Vec<Track>,
}

impl Tracks {
    /// Records that wheel `index` is touching the ground at `pos`.
    ///
    /// The first contact after a lift only arms the wheel; a track needs
    /// two points. `PrevWheelY[n] != 0` of the original serves the same
    /// purpose, by way of a level row no wheel can legitimately sit on.
    pub fn touch(&mut self, index: usize, pos: (i32, i32), across: (f32, f32)) {
        if self.last.len() <= index {
            self.last.resize(index + 1, None);
        }
        if let Some(from) = self.last[index]
            && from != pos
        {
            self.pending.push(Track {
                from,
                to: pos,
                across,
            });
        }
        self.last[index] = Some(pos);
    }

    /// Records that wheel `index` is off the ground.
    pub fn lift(&mut self, index: usize) {
        if let Some(slot) = self.last.get_mut(index) {
            *slot = None;
        }
    }

    /// Records that no wheel is on the ground - the car is airborne, or
    /// lying on its side and no longer rolling.
    pub fn lift_all(&mut self) {
        self.last.iter_mut().for_each(|slot| *slot = None);
    }

    /// Forgets every wheel's contact and any track not yet drawn, so that
    /// the next contact starts fresh. Needed whenever the car is moved
    /// rather than driven.
    pub fn reset(&mut self) {
        self.lift_all();
        self.pending.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Hands over the stretches recorded since the last drain.
    pub fn drain(&mut self) -> std::vec::Drain<'_, Track> {
        self.pending.drain(..)
    }
}

/// Cuts `track` into the level, pushing what it touched onto `regions`.
pub fn apply(level: &mut Level, config: &Config, track: &Track, regions: &mut Vec<Region>) {
    if !config.enabled || config.depth == 0 || config.tread == 0 || config.bar == 0 {
        return;
    }

    // `getDistX`/`getDistY` of the original: a wheel that crossed the seam
    // moved one texel, not a level's width.
    let dx = wrap_delta(track.to.0 - track.from.0, level.size.0);
    let dy = wrap_delta(track.to.1 - track.from.1, level.size.1);
    // A stretch this long is a teleport, not a drive.
    if dx.abs() > level.size.0 / 4 || dy.abs() > level.size.1 / 4 {
        return;
    }

    let steps = dx.abs().max(dy.abs());
    if steps == 0 {
        return;
    }
    let (fx, fy) = (dx as f32 / steps as f32, dy as f32 / steps as f32);

    let mut bounds = Bounds::default();
    // `mask` counts down from `step`, and the bar is raised only on the
    // wrap-around. Starting it at `tread` puts the first raised bar one
    // full period in, exactly as the original's `mask = step` does.
    let mut mask = config.tread;
    for i in 0..steps {
        let x = track.from.0 + (fx * i as f32).round() as i32;
        let y = track.from.1 + (fy * i as f32).round() as i32;
        let delta = if mask == 0 {
            mask = config.tread;
            config.depth
        } else {
            -config.depth
        };
        mask -= 1;

        for k in 0..config.bar as i32 {
            let reach = config.spacing * k as f32;
            let bx = x + (track.across.0 * reach).round() as i32;
            let by = y + (track.across.1 * reach).round() as i32;
            if press(level, bx, by, delta) {
                bounds.add(bx, by);
            }
        }
    }

    bounds.push(regions, level.size);
}

/// `pixSetR` of the original: moves the upper surface of one texel by
/// `delta`, and returns whether it wrote anything.
///
/// Only the top surface is drivable, so that is the only one a wheel can
/// mark. On a double-level pair the altitude that moves lives in the odd
/// half, which is why the even one is left alone rather than written twice.
fn press(level: &mut Level, x: i32, y: i32, delta: i32) -> bool {
    let i = level.wrap((x, y));
    let bits = level.terrain_bits();

    if level.meta[i] & DOUBLE_LEVEL != 0 {
        // The pair is addressed by its odd half; the even one is the same
        // surface seen from the other side.
        if x & 1 == 0 {
            return false;
        }
        if bits.read(level.meta[i]) != MAIN_TERRAIN {
            return false;
        }
        let height = (level.height[i] as i32 + delta).clamp(0, 255);
        if collapses(level, i, height) {
            collapse(level, i, x, y);
            return true;
        }
        level.height[i] = height as u8;
    } else {
        if bits.read(level.meta[i]) != MAIN_TERRAIN {
            return false;
        }
        level.height[i] = (level.height[i] as i32 + delta).clamp(0, 255) as u8;
    }

    reflood(level, i, x, y);
    true
}

/// Whether cutting the roof of a cave down to `height` breaks through it.
///
/// The roof is everything between the ceiling - `low` plus the pair's delta
/// bits - and the top. The original allows itself one delta step of margin
/// before it gives up on the slab, so a roof does not thin out to nothing
/// first.
fn collapses(level: &Level, i: usize, height: i32) -> bool {
    let (lo, hi) = (i & !1, i | 1);
    let delta = ((level.meta[lo] & DELTA_MASK) << DELTA_BITS) | (level.meta[hi] & DELTA_MASK);
    let ceiling =
        level.height[lo] as i32 + (((delta + 1) as i32) << level.geometry.delta_power as u32);
    ceiling >= height
}

/// Drops a cave roof that has been driven through, leaving flat ground.
///
/// The pair stops being double-level, the surviving terrain type is the one
/// the cave floor had, and the altitude settles between that floor and the
/// ground next door - `pixSetR`'s own recovery, which keeps the collapsed
/// texel from standing out as a spike.
fn collapse(level: &mut Level, i: usize, x: i32, y: i32) {
    let (lo, hi) = (i & !1, i | 1);
    let floor = level.height[lo] as i32;
    let settled = ((floor + raw_low(level, (x + 1, y))) / 2).clamp(0, 255);

    let bits = level.terrain_bits();
    let terrain = bits.write(bits.read(level.meta[lo]));
    let keep = !(DOUBLE_LEVEL | DELTA_MASK | bits.write(bits.mask));
    level.meta[lo] = (level.meta[lo] & keep) | terrain;
    level.meta[hi] = (level.meta[hi] & keep) | terrain;
    level.height[lo] = settled as u8;
    level.height[hi] = settled as u8;

    reflood(level, hi, x, y);
}

/// Keeps the water line honest after a texel has moved.
///
/// A pit dug below the flood level next to water fills up with it, and
/// ground raised back above the line stops being water. The original only
/// spreads from a neighbour that is already water, so a hole in the middle
/// of dry land stays dry however deep it gets.
fn reflood(level: &mut Level, i: usize, x: i32, y: i32) {
    let bits = level.terrain_bits();
    // The original only compiles this in for the eight-terrain worlds; the
    // sixteen-terrain ones have no single water type to spread.
    if bits.mask != 0x7 {
        return;
    }
    let flood = match level.flood_map.len() {
        0 => return,
        len => {
            level.flood_map[(y.rem_euclid(level.size.1) as usize * len) / level.size.1 as usize]
                as i32
        }
    };

    if (level.height[i] as i32) < flood {
        let wet = [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)]
            .iter()
            .any(|&c| bits.read(level.meta[level.wrap(c)]) == 0);
        if wet {
            level.meta[i] &= !bits.write(bits.mask);
        }
    } else if bits.read(level.meta[i]) == 0 {
        level.meta[i] = (level.meta[i] & !bits.write(bits.mask)) | bits.write(MAIN_TERRAIN);
    }
}

/// The stored altitude of the ground under any cave at `coord`.
fn raw_low(level: &Level, coord: (i32, i32)) -> i32 {
    let i = level.wrap(coord);
    let i = if level.meta[i] & DOUBLE_LEVEL != 0 {
        i & !1
    } else {
        i
    };
    level.height[i] as i32
}

/// Shortest signed distance across a level that wraps.
fn wrap_delta(d: i32, total: i32) -> i32 {
    let d = d.rem_euclid(total);
    if d * 2 > total { d - total } else { d }
}

/// The texels one track touched, as an unwrapped box.
#[derive(Default)]
struct Bounds {
    range: Option<(i32, i32, i32, i32)>,
}

impl Bounds {
    fn add(&mut self, x: i32, y: i32) {
        self.range = Some(match self.range {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }

    fn push(&self, regions: &mut Vec<Region>, size: (i32, i32)) {
        if let Some((x0, y0, x1, y1)) = self.range {
            Region::push_wrapped(regions, x0, y0, x1 - x0 + 1, y1 - y0 + 1, size);
        }
    }
}

/// The topmost surface at `coord`, in world units.
pub fn surface_height(level: &Level, coord: (i32, i32)) -> f32 {
    match level.get(coord) {
        Texel::Single(p) => p.0,
        Texel::Dual { high, .. } => high.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{LevelConfig, TerrainBits};

    const SIZE: i32 = 64;

    fn test_level() -> Level {
        let total = (SIZE * SIZE) as usize;
        let bits = TerrainBits::new(8);
        Level {
            size: (SIZE, SIZE),
            flood_map: vec![0; SIZE as usize].into_boxed_slice(),
            height: vec![100u8; total].into_boxed_slice(),
            meta: vec![bits.write(MAIN_TERRAIN); total].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    /// A straight track along +X with no across-track spread, so that each
    /// step marks exactly one texel and the pattern is easy to read off.
    fn straight(level: &mut Level, len: i32, config: &Config) -> Vec<Region> {
        let mut regions = Vec::new();
        apply(
            level,
            config,
            &Track {
                from: (0, 0),
                to: (len, 0),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        regions
    }

    fn row(level: &Level, len: i32) -> Vec<u8> {
        (0..len).map(|x| level.height[x as usize]).collect()
    }

    fn bar_only() -> Config {
        Config {
            bar: 1,
            ..Config::default()
        }
    }

    /// One texel per stamp, so a bar's extent is easy to read off.
    fn tight() -> Config {
        Config {
            spacing: 1.0,
            ..Config::default()
        }
    }

    #[test]
    fn the_tread_cuts_twice_for_every_ridge_it_raises() {
        let mut level = test_level();
        straight(&mut level, 9, &bar_only());
        assert_eq!(
            row(&level, 9),
            [99, 99, 99, 101, 99, 99, 101, 99, 99],
            "three cuts, then a ridge every third texel"
        );
    }

    #[test]
    fn driving_over_the_same_ground_digs_it_out() {
        let mut level = test_level();
        let config = bar_only();
        for _ in 0..30 {
            straight(&mut level, 9, &config);
        }
        let after = row(&level, 9);
        let sunk = after.iter().filter(|&&h| h < 100).count();
        assert!(
            sunk >= 6,
            "most of the track should sit below the plain: {:?}",
            after
        );
        assert_eq!(after[0], 70, "a cut texel sinks one unit per pass");
        assert_eq!(after[3], 130, "a ridge texel rises one unit per pass");
    }

    #[test]
    fn a_track_never_digs_through_the_floor() {
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 1);
        let config = bar_only();
        for _ in 0..5 {
            straight(&mut level, 9, &config);
        }
        assert_eq!(level.height[0], 0, "clamped rather than wrapping around");
    }

    #[test]
    fn only_the_main_terrain_takes_a_track() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        // Turn the second half of the track into something a wheel cannot
        // disturb - a lava flow keeps its shape however hard it is driven on.
        for x in 5..9 {
            level.meta[x] = bits.write(4);
        }
        straight(&mut level, 9, &bar_only());
        assert_eq!(row(&level, 9), [99, 99, 99, 101, 99, 100, 100, 100, 100]);
    }

    #[test]
    fn the_bar_lies_across_the_track() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply(
            &mut level,
            &tight(),
            &Track {
                from: (10, 10),
                to: (13, 10),
                across: (0.0, 1.0),
            },
            &mut regions,
        );
        let at = |x: i32, y: i32| level.height[(y * SIZE + x) as usize];
        for k in 0..3 {
            assert_eq!(at(10, 10 + k), 99, "the whole bar is stamped");
        }
        assert_eq!(at(10, 13), 100, "and no further");
    }

    #[test]
    fn a_track_reports_what_it_touched() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply(
            &mut level,
            &tight(),
            &Track {
                from: (10, 10),
                to: (14, 10),
                across: (0.0, 1.0),
            },
            &mut regions,
        );
        assert_eq!(
            regions,
            vec![Region {
                x: 10,
                y: 10,
                w: 4,
                h: 3
            }]
        );
    }

    #[test]
    fn a_track_across_the_seam_is_split_rather_than_stretched() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply(
            &mut level,
            &bar_only(),
            &Track {
                from: (SIZE - 2, 0),
                to: (2, 0),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        assert_eq!(level.height[(SIZE - 2) as usize], 99);
        assert_eq!(level.height[0], 99, "the track carries on past the seam");
        assert_eq!(
            regions,
            vec![
                Region {
                    x: SIZE - 2,
                    y: 0,
                    w: 2,
                    h: 1
                },
                Region {
                    x: 0,
                    y: 0,
                    w: 2,
                    h: 1
                }
            ]
        );
    }

    #[test]
    fn a_teleport_leaves_no_track() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply(
            &mut level,
            &Config::default(),
            &Track {
                from: (0, 0),
                to: (SIZE / 2, SIZE / 2),
                across: (0.0, 1.0),
            },
            &mut regions,
        );
        assert!(regions.is_empty());
        assert!(level.height.iter().all(|&h| h == 100));
    }

    #[test]
    fn a_wheel_needs_two_contacts_before_it_marks_anything() {
        let mut tracks = Tracks::default();
        tracks.touch(0, (10, 10), (0.0, 1.0));
        assert!(tracks.is_empty(), "the first contact only arms the wheel");
        tracks.touch(0, (11, 10), (0.0, 1.0));
        assert_eq!(tracks.drain().count(), 1);
    }

    #[test]
    fn a_jump_does_not_draw_a_track_across_what_it_flew_over() {
        let mut tracks = Tracks::default();
        tracks.touch(0, (10, 10), (0.0, 1.0));
        tracks.lift(0);
        tracks.touch(0, (20, 10), (0.0, 1.0));
        assert!(tracks.is_empty());
        tracks.touch(0, (21, 10), (0.0, 1.0));
        assert_eq!(
            tracks.drain().next().unwrap().from,
            (20, 10),
            "the track picks up where the wheel landed"
        );
    }

    #[test]
    fn wheels_keep_their_own_tracks() {
        let mut tracks = Tracks::default();
        for wheel in 0..4 {
            tracks.touch(wheel, (10, 10 * wheel as i32), (0.0, 1.0));
        }
        assert!(tracks.is_empty());
        for wheel in 0..4 {
            tracks.touch(wheel, (11, 10 * wheel as i32), (0.0, 1.0));
        }
        assert_eq!(tracks.drain().count(), 4);
    }

    fn dual_level(level: &mut Level, x: i32, y: i32, low: u8, high: u8, delta: u8) {
        let bits = level.terrain_bits();
        let i = level.wrap((x, y)) & !1;
        level.height[i] = low;
        level.height[i | 1] = high;
        level.meta[i] = DOUBLE_LEVEL | bits.write(MAIN_TERRAIN) | (delta >> DELTA_BITS);
        level.meta[i | 1] = DOUBLE_LEVEL | bits.write(MAIN_TERRAIN) | (delta & DELTA_MASK);
    }

    #[test]
    fn a_cave_roof_survives_while_it_is_thick_enough() {
        let mut level = test_level();
        dual_level(&mut level, 10, 5, 40, 200, 1);
        let mut regions = Vec::new();
        apply(
            &mut level,
            &bar_only(),
            &Track {
                from: (9, 5),
                to: (13, 5),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        let i = level.wrap((11, 5));
        assert_ne!(level.meta[i] & DOUBLE_LEVEL, 0, "the cave is still there");
        assert_eq!(level.height[i], 199, "and its roof took the track");
    }

    #[test]
    fn driving_through_a_thin_cave_roof_brings_it_down() {
        let mut level = test_level();
        // A roof one unit above the ceiling: the next cut goes through it.
        dual_level(&mut level, 10, 5, 40, 49, 1);
        let mut regions = Vec::new();
        apply(
            &mut level,
            &bar_only(),
            &Track {
                from: (9, 5),
                to: (13, 5),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        let i = level.wrap((11, 5));
        assert_eq!(level.meta[i] & DOUBLE_LEVEL, 0, "the cave collapsed");
        assert_eq!(level.meta[i & !1] & DOUBLE_LEVEL, 0, "both halves of it");
        assert_eq!(
            level.height[i], 70,
            "and settled between its floor and the ground next door"
        );
        assert!(!regions.is_empty());
    }

    #[test]
    fn a_pit_dug_below_the_water_line_next_to_water_fills_up() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        level.flood_map.iter_mut().for_each(|f| *f = 100);
        // A shore: everything left of x = 5 is already water.
        for y in 0..SIZE {
            for x in 0..5 {
                level.meta[level.wrap((x, y))] = bits.write(0);
            }
        }
        let mut regions = Vec::new();
        apply(
            &mut level,
            &bar_only(),
            &Track {
                from: (5, 5),
                to: (9, 5),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        let i = level.wrap((5, 5));
        assert_eq!(level.height[i], 99, "the wheel cut below the water line");
        assert_eq!(bits.read(level.meta[i]), 0, "so the sea came in");
        // Each cut texel sees the one behind it already flooded, so the
        // water follows the rut inland for as long as it stays deep enough.
        assert_eq!(bits.read(level.meta[level.wrap((7, 5))]), 0);
        assert_eq!(
            bits.read(level.meta[level.wrap((8, 5))]),
            MAIN_TERRAIN,
            "the tread's ridge stands above the line and dams it"
        );
    }

    #[test]
    fn a_hole_dug_away_from_any_water_stays_dry() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        level.flood_map.iter_mut().for_each(|f| *f = 100);
        let mut regions = Vec::new();
        apply(
            &mut level,
            &bar_only(),
            &Track {
                from: (30, 30),
                to: (34, 30),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        let i = level.wrap((30, 30));
        assert_eq!(level.height[i], 99, "below the water line");
        assert_eq!(
            bits.read(level.meta[i]),
            MAIN_TERRAIN,
            "but with nothing to fill it"
        );
    }

    #[test]
    fn ground_raised_back_out_of_the_water_stops_being_water() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        level.flood_map.iter_mut().for_each(|f| *f = 100);
        let i = level.wrap((3, 5));
        level.height[i] = 100;
        let mut regions = Vec::new();
        // The fourth texel of a track is the one the tread raises.
        apply(
            &mut level,
            &bar_only(),
            &Track {
                from: (0, 5),
                to: (9, 5),
                across: (0.0, 0.0),
            },
            &mut regions,
        );
        assert_eq!(level.height[i], 101);
        assert_eq!(bits.read(level.meta[i]), MAIN_TERRAIN);
    }

    #[test]
    fn switching_it_off_leaves_the_ground_alone() {
        let mut level = test_level();
        let config = Config {
            enabled: false,
            ..Config::default()
        };
        let regions = straight(&mut level, 9, &config);
        assert!(regions.is_empty());
        assert!(level.height.iter().all(|&h| h == 100));
    }
}
