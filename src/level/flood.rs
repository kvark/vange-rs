//! The water level, and the way it drifts.
//!
//! A port of the tide in `LoadVPR` (`src/terra/vmap.cpp`), which the
//! original added late and only ever runs in a network game: a world's
//! water sits a little higher or lower depending on the day, on a rhythm
//! of that world's own.
//!
//! Two things are done differently here, both noted where they happen: the
//! tide is evaluated as the game runs rather than once when the world
//! loads, so the water actually moves; and it scales every band of the
//! flood map rather than only the first, so the sea stays level with
//! itself.
//!
//! The terrain reclassification that goes with it in the original - the
//! shore turning to water and back as the level passes it - is not ported.
//! It rewrites the terrain type of every texel it touches, which the
//! original can afford because it happens as map sections stream in and
//! there is no streaming here. It is also not needed to see the tide: the
//! water is drawn as a full plane at the flood height and depth-tested
//! against the ground, so raising it floods the low ground on its own.

use super::Level;

/// Seconds of play per day of tide.
///
/// The original's clock is real time since a fixed date, which would move
/// the water by less than a unit an hour. Something has to give for the
/// drift to be visible at all, and this is the honest place to put it.
pub const DEFAULT_SECONDS_PER_DAY: f64 = 90.0;

/// How far into a swing the tide has to be before it moves the water:
/// `sin(PI/4)` of the original, so a world spends about half its cycle at
/// its ordinary level.
const THRESHOLD: f64 = std::f64::consts::FRAC_1_SQRT_2;

/// One world's tide, from the switch in `LoadVPR`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tide {
    /// Days in a full swing from high, through low, and back.
    pub period: f64,
    /// Where in that swing the world starts.
    pub phase: f64,
    /// Fraction the water rises by at the top of a high.
    pub high: f64,
    /// Fraction it falls by at the bottom of a low.
    pub low: f64,
    /// Whether this world's water moves at all. The original gives every
    /// world a period but only lets some of them act on it.
    pub dynamic: bool,
}

impl Default for Tide {
    fn default() -> Self {
        Tide {
            period: 10.0,
            phase: 0.0,
            high: 0.3,
            low: 0.3,
            dynamic: false,
        }
    }
}

/// The tide of the world by that name, matched the way the original's
/// `switch(CurrentWorld)` does.
pub fn tide_of(world: &str) -> Tide {
    let name = world.trim().to_ascii_lowercase();
    let base = |period: f64| Tide {
        period,
        ..Tide::default()
    };
    match name.as_str() {
        "fostral" => Tide {
            period: 4.0,
            high: 0.8,
            low: 0.6,
            dynamic: true,
            ..Tide::default()
        },
        "glorx" => Tide {
            period: 3.0,
            high: 0.6,
            low: 0.6,
            dynamic: true,
            ..Tide::default()
        },
        "necross" => Tide {
            period: 5.0,
            high: 0.8,
            low: 0.6,
            dynamic: true,
            ..Tide::default()
        },
        "xplo" => base(6.0),
        "boozeena" => base(8.0),
        "weexow" => Tide {
            period: 7.0,
            dynamic: true,
            ..Tide::default()
        },
        "threall" => Tide {
            period: 10.0,
            high: 0.6,
            low: 0.6,
            dynamic: true,
            ..Tide::default()
        },
        // The data folder spells this one `ark-a-znoy`.
        n if n.starts_with("ark") => Tide {
            period: 6.0,
            phase: std::f64::consts::PI,
            ..Tide::default()
        },
        _ => base(10.0),
    }
}

/// A world's water, and where the tide has taken it.
pub struct Flood {
    /// The flood map as the world's `.vpr` has it, which the tide is
    /// measured against rather than compounded onto.
    base: Box<[u8]>,
    tide: Tide,
    /// Days since the epoch the original counts from.
    at: f64,
    /// Seconds of play to a day of tide.
    pub seconds_per_day: f64,
    pub enabled: bool,
}

impl Flood {
    pub fn new(level: &Level, world: &str) -> Self {
        let tide = tide_of(world);
        // The original starts from real days since 2007-02-27, so which way
        // the water is leaning depends on when you play. Keeping that.
        let at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| (d.as_secs_f64() - 1_172_609_523.0) / 86_400.0)
            .unwrap_or(0.0);
        Flood {
            base: level.flood_map.clone(),
            tide,
            at,
            seconds_per_day: DEFAULT_SECONDS_PER_DAY,
            enabled: tide.dynamic,
        }
    }

    /// Whether this world's water moves at all.
    pub fn is_dynamic(&self) -> bool {
        self.tide.dynamic
    }

    pub fn tide(&self) -> &Tide {
        &self.tide
    }

    /// Days since the epoch, as the tide currently sees it.
    pub fn days(&self) -> f64 {
        self.at
    }

    pub fn set_days(&mut self, days: f64) {
        self.at = days;
    }

    /// How far through its swing the world is, in `-1..=1`.
    pub fn swing(&self) -> f64 {
        swing_at(&self.tide, self.at)
    }

    /// What the water is multiplied by right now.
    pub fn scale(&self) -> f64 {
        if !self.enabled {
            return 1.0;
        }
        scale_at(&self.tide, self.at)
    }

    /// Advances the tide by `delta` seconds of play and, if the water has
    /// moved, rewrites `level.flood_map`. Returns whether it moved.
    pub fn step(&mut self, level: &mut Level, delta: f32) -> bool {
        if self.enabled && self.seconds_per_day > 0.0 {
            self.at += delta as f64 / self.seconds_per_day;
        }
        self.apply(level)
    }

    /// Writes the current level into `level.flood_map`, if it differs from
    /// what is already there.
    pub fn apply(&mut self, level: &mut Level) -> bool {
        let scale = self.scale();
        // Every band moves together. The original only ever rewrites the
        // first, which would leave the sea stepped from one band to the
        // next; it gets away with it because almost everything that reads
        // the level reads band zero.
        let changed = self
            .base
            .iter()
            .zip(level.flood_map.iter())
            .any(|(&base, &now)| scaled(base, scale) != now);
        if !changed {
            return false;
        }
        for (dst, &base) in level.flood_map.iter_mut().zip(self.base.iter()) {
            *dst = scaled(base, scale);
        }
        true
    }
}

fn scaled(base: u8, scale: f64) -> u8 {
    (base as f64 * scale).round().clamp(0.0, 255.0) as u8
}

/// `zMod_cycle`: where the world is in its own long swing.
fn swing_at(tide: &Tide, days: f64) -> f64 {
    (days * std::f64::consts::TAU / tide.period + tide.phase).sin()
}

/// What the water is multiplied by on a given day.
///
/// The long swing decides whether the tide is running at all and which way;
/// the daily term decides how far it has got. Between the two thresholds
/// the world sits at the level its `.vpr` gives it.
fn scale_at(tide: &Tide, days: f64) -> f64 {
    if !tide.dynamic {
        return 1.0;
    }
    let swing = swing_at(tide, days);
    if swing.abs() <= THRESHOLD {
        return 1.0;
    }
    // `zMod_flood_level_delta`, which carries the sign of the swing.
    let delta = swing.signum() * (1.0 + (days * std::f64::consts::TAU).cos()) / 2.0;
    let reach = if swing > 0.0 { tide.high } else { tide.low };
    (1.0 + delta * reach).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{LevelConfig, TerrainBits};

    const BANDS: usize = 8;

    fn test_level(level: u8) -> Level {
        Level {
            size: (64, 64),
            flood_map: vec![level; BANDS].into_boxed_slice(),
            height: vec![0; 64 * 64].into_boxed_slice(),
            meta: vec![TerrainBits::new(8).write(1); 64 * 64].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    fn flood(world: &str, level: u8) -> (Level, Flood) {
        let lvl = test_level(level);
        let mut f = Flood::new(&lvl, world);
        f.set_days(0.0);
        (lvl, f)
    }

    #[test]
    fn every_world_the_original_names_keeps_its_own_rhythm() {
        assert_eq!(tide_of("Fostral").period, 4.0);
        assert_eq!(tide_of("Glorx").period, 3.0);
        assert_eq!(tide_of("Necross").period, 5.0);
        assert_eq!(tide_of("Threall").period, 10.0);
        assert_eq!(tide_of("Boozeena").period, 8.0);
        assert_eq!(tide_of("ark-a-znoy").phase, std::f64::consts::PI);
        // Case is however the settings happen to spell it.
        assert_eq!(tide_of("FOSTRAL"), tide_of("fostral"));
        // A world the original does not name gets the fallback and stays put.
        let other = tide_of("somewhere else");
        assert_eq!(other.period, 10.0);
        assert!(!other.dynamic);
    }

    #[test]
    fn a_world_the_tide_does_not_touch_never_moves() {
        let (mut level, mut f) = flood("Boozeena", 100);
        assert!(!f.is_dynamic());
        for _ in 0..1000 {
            f.step(&mut level, 1.0);
        }
        assert!(level.flood_map.iter().all(|&v| v == 100));
    }

    #[test]
    fn the_water_sits_at_its_own_level_between_the_swings() {
        let (_, mut f) = flood("Fostral", 100);
        // A quarter of the way into a four-day swing is the peak, so step
        // back to where the swing is still shallow.
        f.set_days(0.0);
        assert_eq!(f.scale(), 1.0, "at the crossing the tide is not running");
        assert!(f.swing().abs() <= THRESHOLD);
    }

    #[test]
    fn a_high_swing_lifts_the_water_and_a_low_one_drops_it() {
        let tide = tide_of("Fostral");
        // Peak of the long swing is a quarter period in, and the daily term
        // is at its highest on a whole day.
        let high = (1..4000)
            .map(|i| i as f64 * 0.001)
            .filter(|&d| swing_at(&tide, d) > THRESHOLD)
            .map(|d| scale_at(&tide, d))
            .fold(f64::MIN, f64::max);
        let low = (1..8000)
            .map(|i| i as f64 * 0.001)
            .filter(|&d| swing_at(&tide, d) < -THRESHOLD)
            .map(|d| scale_at(&tide, d))
            .fold(f64::MAX, f64::min);
        assert!(high > 1.0, "a high swing has to raise it: {}", high);
        assert!(low < 1.0, "and a low one lower it: {}", low);
        assert!(high <= 1.0 + tide.high + 1e-9, "no further than its reach");
        assert!(low >= 1.0 - tide.low - 1e-9);
    }

    #[test]
    fn the_water_never_goes_below_nothing() {
        let tide = Tide {
            period: 2.0,
            phase: 0.0,
            high: 4.0,
            low: 4.0,
            dynamic: true,
        };
        for i in 0..4000 {
            let scale = scale_at(&tide, i as f64 * 0.001);
            assert!(scale >= 0.0, "scale went negative at day {}", i);
        }
    }

    #[test]
    fn the_level_is_clamped_to_what_a_byte_holds() {
        assert_eq!(scaled(200, 4.0), 255);
        assert_eq!(scaled(200, -1.0), 0);
        assert_eq!(scaled(100, 1.0), 100);
    }

    #[test]
    fn every_band_moves_together() {
        let mut level = test_level(100);
        // Bands that start out different stay in proportion.
        for (i, band) in level.flood_map.iter_mut().enumerate() {
            *band = 80 + i as u8 * 4;
        }
        let mut f = Flood::new(&level, "Fostral");
        let before = level.flood_map.to_vec();
        // Wind to a day where the tide is definitely running.
        let tide = tide_of("Fostral");
        let day = (1..4000)
            .map(|i| i as f64 * 0.001)
            .find(|&d| swing_at(&tide, d) > THRESHOLD && scale_at(&tide, d) > 1.2)
            .expect("no high day found");
        f.set_days(day);
        assert!(f.apply(&mut level), "the water should have moved");

        let scale = f.scale();
        for (i, (&now, &was)) in level.flood_map.iter().zip(before.iter()).enumerate() {
            assert_eq!(now, scaled(was, scale), "band {} is out of step", i);
        }
        assert!(level.flood_map.iter().zip(&before).all(|(n, w)| n > w));
    }

    #[test]
    fn a_step_that_does_not_move_the_water_reports_nothing() {
        let (mut level, mut f) = flood("Fostral", 100);
        f.seconds_per_day = 0.0;
        f.apply(&mut level);
        for _ in 0..20 {
            assert!(
                !f.step(&mut level, 1.0),
                "a still tide keeps reporting moves"
            );
        }
    }

    #[test]
    fn switching_it_off_puts_the_water_back_where_the_world_had_it() {
        let mut level = test_level(100);
        let mut f = Flood::new(&level, "Fostral");
        let tide = tide_of("Fostral");
        let day = (1..4000)
            .map(|i| i as f64 * 0.001)
            .find(|&d| scale_at(&tide, d) > 1.2)
            .unwrap();
        f.set_days(day);
        f.apply(&mut level);
        assert_ne!(level.flood_map[0], 100);

        f.enabled = false;
        assert!(f.apply(&mut level));
        assert!(level.flood_map.iter().all(|&v| v == 100));
    }

    #[test]
    fn the_tide_carries_on_over_a_long_run() {
        let (mut level, mut f) = flood("Fostral", 120);
        f.seconds_per_day = 1.0;
        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..400 {
            f.step(&mut level, 0.05);
            seen.insert(level.flood_map[0]);
        }
        assert!(
            seen.len() > 3,
            "the water should visit several levels, saw {:?}",
            seen
        );
        assert!(seen.contains(&120), "including the one the world set");
    }
}
