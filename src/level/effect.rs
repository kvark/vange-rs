//! Terrain effects that run for a while and then are done with.
//!
//! Two of the `MAP_POINT` handlers in `LocalMapProcess::Quant`, both of
//! which reshape the ground over a number of quants rather than all at
//! once:
//!
//! * [`LavaSpot`] is `MapLavaSpot` - a dome that swells out of the ground,
//!   holds, and sinks back, putting the terrain back as it found it. What
//!   the original throws up under a lava flow or an item going off.
//! * [`landslide`] is `LandSlideProcess` - a cave-in. Every cave inside a
//!   quad has its roof brought down onto its own floor, so a tunnel
//!   becomes solid ground.
//!
//! Neither has anything to set it off yet. Every caller in the original is
//! a sensor, an item or the train, and none of those are ported; what is
//! here is the terrain half, the same position the craters are in.

use super::terraform::{DESTRUCTIBLE, Profile, Spot, Surface};
use super::{DELTA_MASK, DOUBLE_LEVEL, Level, Region};

/// A dome swelling out of the ground and sinking back.
///
/// The original walks radius and depth through two straight runs - out to
/// one size over `MaxPhase1` quants, then on to another over `MaxPhase2` -
/// and un-stamps the previous shape before stamping the next, so only one
/// is ever standing. It restores the last one on its way out too, which is
/// what makes the whole thing transient.
pub struct LavaSpot {
    at: (i32, i32),
    terrain: Option<u8>,
    /// Radius and depth now, and what was last stamped, in 8.8.
    radius: i32,
    delta: i32,
    last: Option<(i32, i32)>,
    /// Steps per run, and how far through.
    first_run: i32,
    second_run: i32,
    phase: i32,
    /// Per-step change during each run.
    d_radius: [i32; 2],
    d_delta: [i32; 2],
}

/// How a lava spot grows: where it starts, what it reaches, and how long
/// each leg takes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Swell {
    /// Radius and depth it starts at.
    pub from: (i32, i32),
    /// What it reaches at the end of the first run.
    pub peak: (i32, i32),
    /// And at the end of the second, which is usually back to nothing.
    pub to: (i32, i32),
    /// Quants in each run.
    pub first_run: i32,
    pub second_run: i32,
    /// Terrain to leave behind while it stands, or `None` to keep what is
    /// there. The original spells the second one `83`.
    pub terrain: Option<u8>,
}

impl Default for Swell {
    fn default() -> Self {
        // `MapD.CreateLavaSpot(R_curr, 5, 5, 20, 10, 0, 0, 1, 8, 83, ..)`
        // of `items.cpp`: out to a broad shallow dome, then back to
        // nothing.
        Swell {
            from: (5, 5),
            peak: (20, 10),
            to: (0, 0),
            first_run: 1,
            second_run: 8,
            terrain: None,
        }
    }
}

impl LavaSpot {
    pub fn new(at: (i32, i32), swell: &Swell) -> Self {
        let first = swell.first_run.max(1);
        let second = swell.second_run.max(1);
        let (r0, d0) = (swell.from.0 << 8, swell.from.1 << 8);
        LavaSpot {
            at,
            terrain: swell.terrain,
            radius: r0,
            delta: d0,
            last: None,
            first_run: first,
            second_run: second,
            phase: 0,
            d_radius: [
                ((swell.peak.0 << 8) - r0) / first,
                ((swell.to.0 - swell.peak.0) << 8) / second,
            ],
            d_delta: [
                ((swell.peak.1 << 8) - d0) / first,
                ((swell.to.1 - swell.peak.1) << 8) / second,
            ],
        }
    }

    /// Whether the spot still has anything left to do.
    pub fn is_alive(&self) -> bool {
        self.phase <= self.first_run + self.second_run
    }

    /// One quant. Takes the previous shape back out of the ground, puts the
    /// next one in, and reports what it touched. Returns whether the spot
    /// is still going.
    pub fn quant(&mut self, level: &mut Level, regions: &mut Vec<Region>) -> bool {
        if !self.is_alive() {
            return false;
        }
        // Out with the last shape first, so only one is ever standing.
        if let Some((radius, delta)) = self.last.take() {
            self.stamp(level, radius, -delta, None, regions);
        }
        if self.phase > self.first_run + self.second_run {
            self.phase += 1;
            return false;
        }

        self.stamp(level, self.radius, self.delta, self.terrain, regions);
        self.last = Some((self.radius, self.delta));

        let leg = if self.phase < self.first_run { 0 } else { 1 };
        self.radius += self.d_radius[leg];
        self.delta += self.d_delta[leg];
        self.phase += 1;
        true
    }

    /// Takes the spot back out of the ground, wherever it has got to.
    pub fn cancel(&mut self, level: &mut Level, regions: &mut Vec<Region>) {
        if let Some((radius, delta)) = self.last.take() {
            self.stamp(level, radius, -delta, None, regions);
        }
        self.phase = self.first_run + self.second_run + 1;
    }

    fn stamp(
        &self,
        level: &mut Level,
        radius: i32,
        delta: i32,
        terrain: Option<u8>,
        regions: &mut Vec<Region>,
    ) {
        let radius = radius >> 8;
        if radius <= 0 {
            return;
        }
        let spot = Spot {
            radius,
            delta,
            profile: Profile::Dome,
            // Every stamp has to hit the same texels as the one it undoes,
            // so the dice never come out. The original only rolls them on
            // the very first restore, which is stamping a shape that was
            // never put down in the first place.
            ragged: 0,
            terrain,
            mask: DESTRUCTIBLE,
            surface: Surface::Upper,
        };
        super::terraform::apply_spot(level, self.at, &spot, regions);
    }
}

/// Brings the roof down on every cave inside `quad`.
///
/// `LandSlideProcess` and the `LandSlideLine` under it: a double-level pair
/// whose upper surface is below `ceiling` stops being one, both halves
/// dropping to the cave's own floor plus a little rubble. Anything already
/// solid is left alone, which is why a slide over open ground does nothing.
///
/// Returns how many pairs came down.
pub fn landslide(
    level: &mut Level,
    quad: &[(i32, i32); 4],
    ceiling: u8,
    rubble: u8,
    regions: &mut Vec<Region>,
) -> usize {
    let (mut lo, mut hi) = (quad[0], quad[0]);
    for &(x, y) in quad.iter() {
        lo = (lo.0.min(x), lo.1.min(y));
        hi = (hi.0.max(x), hi.1.max(y));
    }
    // A quad wider than half the level is a mistake, not a tunnel.
    if hi.0 - lo.0 > level.size.0 / 2 || hi.1 - lo.1 > level.size.1 / 2 {
        return 0;
    }

    let bits = level.terrain_bits();
    let mut rng = Rubble::seeded(lo);
    let mut brought_down = 0;
    let mut touched: Option<(i32, i32, i32, i32)> = None;

    for y in lo.1..=hi.1 {
        for x in lo.0..=hi.0 {
            if !inside(quad, x, y) {
                continue;
            }
            // Pairs are addressed by their even half.
            if x & 1 != 0 {
                continue;
            }
            let i = level.wrap((x, y));
            let (low, high) = (i & !1, i | 1);
            if level.meta[low] & DOUBLE_LEVEL == 0 {
                continue;
            }
            if level.height[high] >= ceiling {
                continue;
            }
            let filled = level.height[low].saturating_add(rng.below(rubble));
            level.height[low] = filled;
            level.height[high] = filled;
            // The pair stops being double-level, and the delta bits that
            // described its ceiling go with it.
            let keep = !(DOUBLE_LEVEL | DELTA_MASK);
            let terrain = bits.write(bits.read(level.meta[low]));
            level.meta[low] = (level.meta[low] & keep & !bits.write(bits.mask)) | terrain;
            level.meta[high] = (level.meta[high] & keep & !bits.write(bits.mask)) | terrain;

            brought_down += 1;
            touched = Some(match touched {
                None => (x, y, x + 1, y),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y)),
            });
        }
    }

    if let Some((x0, y0, x1, y1)) = touched {
        Region::push_wrapped(regions, x0, y0, x1 - x0 + 1, y1 - y0 + 1, level.size);
    }
    brought_down
}

/// Whether a texel centre falls inside the quad, by the winding of its
/// edges. The original walks the quad scanline by scanline; this is the
/// same set of texels, said in a way that does not care which corner is
/// highest.
fn inside(quad: &[(i32, i32); 4], x: i32, y: i32) -> bool {
    let side = |a: (i32, i32), b: (i32, i32)| {
        let (abx, aby) = ((b.0 - a.0) as i64, (b.1 - a.1) as i64);
        let (apx, apy) = ((x - a.0) as i64, (y - a.1) as i64);
        abx * apy - aby * apx
    };
    let mut negative = false;
    let mut positive = false;
    for i in 0..4 {
        match side(quad[i], quad[(i + 1) % 4]) {
            d if d > 0 => positive = true,
            d if d < 0 => negative = true,
            _ => {}
        }
    }
    !(negative && positive)
}

/// A little noise for the rubble, seeded so a slide fills the same way
/// twice.
struct Rubble(u32);

impl Rubble {
    fn seeded(at: (i32, i32)) -> Self {
        Rubble(
            (at.0 as u32)
                .wrapping_mul(0x9E37_79B9)
                .wrapping_add((at.1 as u32).wrapping_mul(0x85EB_CA6B))
                | 1,
        )
    }

    fn below(&mut self, bound: u8) -> u8 {
        if bound == 0 {
            return 0;
        }
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 % bound as u32) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{LevelConfig, TerrainBits, terraform::MAIN_TERRAIN};

    const SIZE: i32 = 96;

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

    fn cave(level: &mut Level, x: i32, y: i32, low: u8, high: u8) {
        let bits = level.terrain_bits();
        let i = level.wrap((x, y)) & !1;
        level.height[i] = low;
        level.height[i | 1] = high;
        level.meta[i] = DOUBLE_LEVEL | bits.write(MAIN_TERRAIN);
        level.meta[i | 1] = DOUBLE_LEVEL | bits.write(MAIN_TERRAIN) | 1;
    }

    // -- lava spots -----------------------------------------------------

    fn swell() -> Swell {
        Swell {
            from: (4, 8),
            peak: (16, 24),
            to: (0, 0),
            first_run: 4,
            second_run: 8,
            terrain: None,
        }
    }

    #[test]
    fn a_lava_spot_swells_and_then_sinks_back() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut spot = LavaSpot::new((48, 48), &swell());
        let mut regions = Vec::new();

        let mut peak = 0i32;
        let mut steps = 0;
        while spot.quant(&mut level, &mut regions) {
            let here = level.height[level.wrap((48, 48))] as i32 - 100;
            peak = peak.max(here);
            steps += 1;
            assert!(steps < 100, "the spot never finished");
        }
        assert!(peak > 10, "it never swelled: {}", peak);
        assert_eq!(
            level.height.to_vec(),
            before,
            "and it did not put the ground back"
        );
        assert!(!regions.is_empty());
    }

    #[test]
    fn only_one_shape_of_a_lava_spot_stands_at_a_time() {
        let mut level = test_level();
        let mut spot = LavaSpot::new((48, 48), &swell());
        let mut regions = Vec::new();
        // Run it to the top of its swell and check the ground has one dome
        // on it, not a stack of them.
        for _ in 0..4 {
            spot.quant(&mut level, &mut regions);
        }
        let raised = level.height[level.wrap((48, 48))] as i32 - 100;
        assert!(raised > 0);
        assert!(
            raised <= 24,
            "several stamps are standing at once: {}",
            raised
        );
    }

    #[test]
    fn a_cancelled_lava_spot_leaves_nothing_behind() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut spot = LavaSpot::new((48, 48), &swell());
        let mut regions = Vec::new();
        for _ in 0..5 {
            spot.quant(&mut level, &mut regions);
        }
        assert_ne!(level.height.to_vec(), before, "nothing was raised at all");
        spot.cancel(&mut level, &mut regions);
        assert_eq!(level.height.to_vec(), before);
        assert!(!spot.is_alive());
    }

    #[test]
    fn a_lava_spot_can_scorch_while_it_stands() {
        let mut level = test_level();
        let mut spot = LavaSpot::new(
            (48, 48),
            &Swell {
                terrain: Some(3),
                ..swell()
            },
        );
        let mut regions = Vec::new();
        for _ in 0..4 {
            spot.quant(&mut level, &mut regions);
        }
        let bits = level.terrain_bits();
        assert_eq!(bits.read(level.meta[level.wrap((48, 48))]), 3);
    }

    // -- landslides -----------------------------------------------------

    fn quad(x0: i32, y0: i32, x1: i32, y1: i32) -> [(i32, i32); 4] {
        [(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
    }

    #[test]
    fn a_slide_brings_the_caves_inside_it_down() {
        let mut level = test_level();
        for y in 20..30 {
            for x in (20..40).step_by(2) {
                cave(&mut level, x, y, 40, 180);
            }
        }
        let mut regions = Vec::new();
        let down = landslide(&mut level, &quad(20, 20, 39, 29), 250, 4, &mut regions);
        assert!(down > 0, "nothing came down");

        for y in 20..30 {
            for x in (20..40).step_by(2) {
                let i = level.wrap((x, y));
                assert_eq!(
                    level.meta[i] & DOUBLE_LEVEL,
                    0,
                    "({}, {}) is still a cave",
                    x,
                    y
                );
                assert_eq!(level.meta[i | 1] & DOUBLE_LEVEL, 0);
                assert!(
                    level.height[i] >= 40 && level.height[i] < 45,
                    "the roof did not come down to the floor: {}",
                    level.height[i]
                );
                assert_eq!(level.height[i], level.height[i | 1], "the pair is uneven");
            }
        }
        assert!(!regions.is_empty());
    }

    #[test]
    fn a_slide_over_open_ground_does_nothing() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        assert_eq!(
            landslide(&mut level, &quad(20, 20, 40, 30), 250, 4, &mut regions),
            0
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn a_slide_leaves_the_caves_outside_it_alone() {
        let mut level = test_level();
        cave(&mut level, 24, 24, 40, 180);
        cave(&mut level, 60, 60, 40, 180);
        let mut regions = Vec::new();
        landslide(&mut level, &quad(20, 20, 30, 30), 250, 4, &mut regions);
        assert_eq!(level.meta[level.wrap((24, 24))] & DOUBLE_LEVEL, 0, "inside");
        assert_ne!(
            level.meta[level.wrap((60, 60))] & DOUBLE_LEVEL,
            0,
            "the far cave should still be standing"
        );
    }

    #[test]
    fn a_cave_whose_roof_is_above_the_ceiling_holds() {
        let mut level = test_level();
        cave(&mut level, 24, 24, 40, 200);
        let mut regions = Vec::new();
        // The slide only reaches what is below it.
        let down = landslide(&mut level, &quad(20, 20, 30, 30), 150, 4, &mut regions);
        assert_eq!(down, 0);
        assert_ne!(level.meta[level.wrap((24, 24))] & DOUBLE_LEVEL, 0);
    }

    #[test]
    fn a_slide_fills_the_same_way_twice() {
        let fill = || {
            let mut level = test_level();
            for x in (20..40).step_by(2) {
                cave(&mut level, x, 24, 40, 180);
            }
            let mut regions = Vec::new();
            landslide(&mut level, &quad(20, 20, 39, 30), 250, 8, &mut regions);
            level.height.to_vec()
        };
        assert_eq!(fill(), fill());
    }

    #[test]
    fn an_absurd_quad_is_refused() {
        let mut level = test_level();
        cave(&mut level, 24, 24, 40, 180);
        let mut regions = Vec::new();
        let down = landslide(&mut level, &quad(0, 0, SIZE, SIZE), 250, 4, &mut regions);
        assert_eq!(down, 0);
        assert_ne!(level.meta[level.wrap((24, 24))] & DOUBLE_LEVEL, 0);
    }

    #[test]
    fn a_slanted_quad_only_takes_what_is_under_it() {
        let mut level = test_level();
        for y in 20..40 {
            for x in (20..40).step_by(2) {
                cave(&mut level, x, y, 40, 180);
            }
        }
        // A triangle-ish quad leaning across the field of caves.
        let slanted = [(20, 20), (38, 20), (38, 24), (20, 38)];
        let mut regions = Vec::new();
        let down = landslide(&mut level, &slanted, 250, 4, &mut regions);
        assert!(down > 0);
        // The far corner of the box is outside the quad and must stand.
        assert_ne!(
            level.meta[level.wrap((36, 36))] & DOUBLE_LEVEL,
            0,
            "a cave outside the quad came down"
        );
    }
}
