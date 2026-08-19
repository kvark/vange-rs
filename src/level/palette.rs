//! The palette the terrain is drawn through, and the way it breathes.
//!
//! A port of `pal_iter` (`src/palette.cpp` of the original), which the game
//! runs once per quant. Two things happen to the colour ramps:
//!
//! * a glint travels along one terrain's ramp - the shimmer on water - by
//!   brightening a handful of consecutive entries and sliding them along;
//! * any number of other ramps swing up and down on a sine, on whichever of
//!   red, green and blue the world names.
//!
//! Both read from an untouched copy of the palette and write the live one,
//! so neither accumulates and the two never fight over the same entry
//! unless a world points them at the same terrain.
//!
//! The original has a third layer, `pal_iter0`: a second, shorter glint on
//! the same ramp. It never reaches the screen. Both it and `pal_iter1`
//! begin by restoring their range from the pristine palette, and
//! `pal_iter1` runs second, so it wipes whatever `pal_iter0` just drew.
//! What survives of `pal_iter0` is the ambient sound it triggers on its way
//! out. Only the glint that is actually visible is ported here.

use super::{Level, config::PaletteWave};
use crate::config::common::MAIN_LOOP_TIME;

use std::ops::Range;

/// Steps in a full turn - `PIx2` of the original, which indexes its sine
/// table with the low 12 bits of the phase.
const TURN: i32 = 1 << 12;

/// The travelling glint's kernel and period, from `pal_iter1`.
const GLINT: [i32; 10] = [1, 3, 5, 7, 10, 8, 6, 4, 2, 1];
const GLINT_PERIOD: i32 = 150;

/// The original stores colours as the VGA 0..=63 the hardware took, and
/// every amplitude in its INIs is in those units. [`super::read_palette`]
/// has already scaled the palette to 0..=255, so the swings have to be
/// scaled the same way.
const SCALE: i32 = 4;

/// The animation state of one level's palette.
pub struct Animation {
    /// `palbufOrg` - the palette as loaded, which every frame is built from
    /// rather than added to.
    original: Box<[[u8; 4]; 0x100]>,
    waves: Vec<PaletteWave>,
    /// Phase of each wave, in [`TURN`] steps.
    phase: Vec<i32>,
    glint: Option<Glint>,
    /// Colour ramps, indexed by terrain.
    ramps: Vec<Range<u8>>,
    /// Time carried over towards the next quant.
    time: f32,
    pub enabled: bool,
}

/// Where the travelling glint has got to.
struct Glint {
    terrain: u8,
    /// Index into the ramp of the kernel's first entry. Starts off the near
    /// end so the glint slides in rather than appearing.
    offset: i32,
    /// Quants left before it restarts from one end or the other.
    countdown: i32,
    /// Which way it is travelling, `1` or `-1`.
    direction: i32,
    /// The original picks the direction with `realRND`. Anything that keeps
    /// it from settling into a rhythm will do, and a counter keeps a level
    /// reproducible from one run to the next.
    restarts: u32,
}

impl Animation {
    pub fn new(level: &Level, config: &super::config::DynamicPalette) -> Self {
        let waves = config.waves.clone();
        let ramps = level
            .terrains
            .iter()
            .map(|t| t.colors.clone())
            .collect::<Vec<_>>();
        Animation {
            original: Box::new(level.palette),
            phase: vec![0; waves.len()],
            glint: config
                .wave_terrain
                .filter(|&t| (t as usize) < ramps.len())
                .map(|terrain| Glint {
                    terrain,
                    offset: 0,
                    countdown: GLINT_PERIOD,
                    direction: 1,
                    restarts: 0,
                }),
            waves,
            ramps,
            time: 0.0,
            enabled: true,
        }
    }

    /// After a long stall, catch up by this many quants at most rather than
    /// spinning the animation through the whole backlog.
    const MAX_CATCH_UP: u32 = 4;

    /// One rendered frame. Like the moving land, the animation runs on
    /// [`MAIN_LOOP_TIME`] quants rather than the frame rate, so it moves at
    /// the speed the original's INI numbers were chosen for.
    pub fn step(&mut self, level: &mut Level, delta: f32) -> Range<u32> {
        if !self.enabled || self.is_empty() {
            return 0..0;
        }
        self.time += delta;
        let quants = (self.time / MAIN_LOOP_TIME) as u32;
        if quants == 0 {
            return 0..0;
        }
        self.time -= quants as f32 * MAIN_LOOP_TIME;

        let mut range = 0..0;
        for _ in 0..quants.min(Self::MAX_CATCH_UP) {
            let step = self.tick(level);
            range = if range.start == range.end {
                step
            } else if step.start == step.end {
                range
            } else {
                range.start.min(step.start)..range.end.max(step.end)
            };
        }
        range
    }

    /// Whether this level has anything to animate at all.
    pub fn is_empty(&self) -> bool {
        self.waves.is_empty() && self.glint.is_none()
    }

    /// Replaces the palette every frame is built from. Needed when
    /// something else rewrites the palette wholesale - a cycle change, say -
    /// or the animation would keep dragging the old colours back.
    pub fn rebase(&mut self, palette: &[[u8; 4]; 0x100]) {
        *self.original = *palette;
    }

    /// The palette as it was before any animation.
    pub fn original(&self) -> &[[u8; 4]; 0x100] {
        &self.original
    }

    /// Advances one quant, writing into `level.palette`, and returns the
    /// entries that changed.
    pub fn tick(&mut self, level: &mut Level) -> Range<u32> {
        if !self.enabled || self.is_empty() {
            return 0..0;
        }
        let mut touched = Touched::default();

        for (wave, phase) in self.waves.iter().zip(self.phase.iter_mut()) {
            let ramp = match self.ramps.get(wave.terrain as usize) {
                Some(ramp) => ramp.clone(),
                None => continue,
            };
            *phase = (*phase + wave.speed) & (TURN - 1);
            // The original reads a fixed-point sine table; the difference
            // between that and the real thing is far below one colour step.
            let turns = *phase as f32 / TURN as f32 * std::f32::consts::TAU;
            let swing = (wave.amplitude as f32 * turns.sin()) as i32 * SCALE;

            // `ENDCOLOR` is inclusive.
            for entry in ramp.start as usize..=ramp.end as usize {
                let (from, to) = (self.original[entry], &mut level.palette[entry]);
                for channel in 0..3 {
                    to[channel] = if wave.channels[channel] {
                        (from[channel] as i32 + swing).clamp(0, 255) as u8
                    } else {
                        from[channel]
                    };
                }
            }
            touched.add(ramp);
        }

        if let Some(ref mut glint) = self.glint
            && let Some(ramp) = self.ramps.get(glint.terrain as usize)
        {
            // The original works from the entry after the ramp's first, and
            // over one fewer entry than the ramp holds.
            let base = ramp.start as usize + 1;
            let span = (ramp.end - ramp.start) as i32;
            for entry in base..base + span as usize {
                level.palette[entry][..3].copy_from_slice(&self.original[entry][..3]);
            }
            for (i, &bump) in GLINT.iter().enumerate() {
                let at = glint.offset + i as i32;
                if at < 0 || at >= span {
                    continue;
                }
                let to = &mut level.palette[base + at as usize];
                for channel in to[..3].iter_mut() {
                    *channel = (*channel as i32 + bump * SCALE).clamp(0, 255) as u8;
                }
            }
            glint.advance(span);
            touched.add(ramp.start..ramp.end);
        }

        touched.range()
    }
}

impl Glint {
    fn advance(&mut self, span: i32) {
        self.countdown -= 1;
        if self.countdown != 0 {
            self.offset += self.direction;
            return;
        }
        self.countdown = GLINT_PERIOD;
        self.restarts += 1;
        self.direction = if self.restarts.is_multiple_of(2) {
            1
        } else {
            -1
        };
        self.offset = if self.direction > 0 {
            -(GLINT.len() as i32)
        } else {
            span - 1
        };
    }
}

/// The span of palette entries an animation step wrote.
#[derive(Default)]
struct Touched {
    span: Option<(u32, u32)>,
}

impl Touched {
    fn add(&mut self, ramp: Range<u8>) {
        // `colors` is inclusive at the top, and the glint reaches one past
        // its ramp's first entry.
        let (start, end) = (ramp.start as u32, ramp.end as u32 + 1);
        self.span = Some(match self.span {
            None => (start, end),
            Some((a, b)) => (a.min(start), b.max(end)),
        });
    }

    fn range(&self) -> Range<u32> {
        match self.span {
            None => 0..0,
            Some((a, b)) => a..b.min(0x100),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{DynamicPalette, TerrainConfig};

    /// Two terrains with easy ramps: 0 covers entries 0..=15, 1 covers
    /// 16..=31, and every colour starts mid-grey so a swing can go either
    /// way without clamping.
    fn test_level() -> Level {
        let terrains = vec![
            TerrainConfig {
                colors: 0..15,
                ..TerrainConfig::default()
            },
            TerrainConfig {
                colors: 16..31,
                ..TerrainConfig::default()
            },
        ];
        Level {
            size: (4, 4),
            flood_map: vec![0; 4].into_boxed_slice(),
            height: vec![0; 16].into_boxed_slice(),
            meta: vec![0; 16].into_boxed_slice(),
            palette: [[128, 128, 128, 0xFF]; 0x100],
            terrains: terrains.into_boxed_slice(),
            geometry: settings::Geometry::default(),
        }
    }

    fn wave(terrain: u8, speed: i32, amplitude: i32, channels: [bool; 3]) -> PaletteWave {
        PaletteWave {
            terrain,
            speed,
            amplitude,
            channels,
        }
    }

    #[test]
    fn a_world_without_the_section_does_not_animate() {
        let mut level = test_level();
        let mut anim = Animation::new(&level, &DynamicPalette::default());
        assert!(anim.is_empty());
        assert_eq!(anim.tick(&mut level), 0..0);
        assert!(level.palette.iter().all(|c| c[0] == 128));
    }

    #[test]
    fn a_wave_swings_the_ramp_it_names_and_no_other() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: None,
            // A quarter turn per quant, so the first tick lands on the peak.
            waves: vec![wave(1, TURN / 4, 16, [true, false, false])],
        };
        let mut anim = Animation::new(&level, &config);
        anim.tick(&mut level);

        assert_eq!(level.palette[16][0], 128 + 16 * SCALE as u8, "at the peak");
        assert_eq!(level.palette[16][1], 128, "green was not asked for");
        assert_eq!(level.palette[31][0], 128 + 16 * SCALE as u8, "whole ramp");
        assert_eq!(level.palette[15][0], 128, "the neighbouring ramp is left");
    }

    #[test]
    fn a_wave_comes_back_to_where_it_started() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: None,
            waves: vec![wave(1, TURN / 8, 32, [true, true, true])],
        };
        let mut anim = Animation::new(&level, &config);
        // Eight steps to the turn, sampled after each one.
        let seen = (0..8)
            .map(|_| {
                anim.tick(&mut level);
                level.palette[20][0]
            })
            .collect::<Vec<_>>();
        assert!(seen[0] > 128, "an eighth of a turn in, on the way up");
        assert_eq!(seen[3], 128, "half a turn is back through the middle");
        assert!(seen[5] < 128, "three quarters in, at the bottom");
        assert_eq!(seen[7], 128, "a full turn is back where it started");
    }

    #[test]
    fn a_swing_never_leaves_the_channel_range() {
        let mut level = test_level();
        level.palette = [[250, 4, 128, 0xFF]; 0x100];
        let config = DynamicPalette {
            wave_terrain: None,
            waves: vec![wave(1, TURN / 4, 63, [true, true, true])],
        };
        let mut anim = Animation::new(&level, &config);
        for _ in 0..16 {
            anim.tick(&mut level);
            for entry in 16..=31 {
                let _ = level.palette[entry];
            }
        }
        // Clamping is the point; reaching here without a panic is the test,
        // but check the ends really did saturate rather than wrap. The tick
        // advances the phase before it reads it, so wind back a step.
        anim.phase[0] = 0;
        anim.tick(&mut level);
        assert_eq!(level.palette[20][0], 255);
        assert!(level.palette[20][1] > 4);
    }

    #[test]
    fn the_glint_brightens_a_run_of_the_ramp_and_moves_on() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: Some(0),
            waves: Vec::new(),
        };
        let mut anim = Animation::new(&level, &config);
        assert!(!anim.is_empty());

        anim.tick(&mut level);
        let lit = |level: &Level| {
            (0..16)
                .filter(|&e| level.palette[e][0] > 128)
                .collect::<Vec<_>>()
        };
        let first = lit(&level);
        assert!(!first.is_empty(), "the glint lit nothing");
        assert!(
            first.iter().all(|&e| (1..15).contains(&e)),
            "it strayed off the ramp: {:?}",
            first
        );

        for _ in 0..3 {
            anim.tick(&mut level);
        }
        let later = lit(&level);
        assert_ne!(first, later, "the glint has to travel");
        assert!(later.iter().min() > first.iter().min());
    }

    #[test]
    fn the_glint_leaves_nothing_behind_it() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: Some(0),
            waves: Vec::new(),
        };
        let mut anim = Animation::new(&level, &config);
        // Run it right off the end of the ramp.
        for _ in 0..30 {
            anim.tick(&mut level);
        }
        assert!(
            (1..15).all(|e| level.palette[e][0] == 128),
            "the ramp should be back to its own colours"
        );
    }

    #[test]
    fn the_glint_turns_round_when_its_period_runs_out() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: Some(0),
            waves: Vec::new(),
        };
        let mut anim = Animation::new(&level, &config);
        for _ in 0..GLINT_PERIOD {
            anim.tick(&mut level);
        }
        let glint = anim.glint.as_ref().unwrap();
        assert_eq!(glint.countdown, GLINT_PERIOD, "the period restarted");
        assert_eq!(glint.direction, -1, "it should have flipped");
        assert_eq!(glint.offset, 14, "and restarted from the far end");
    }

    #[test]
    fn the_reported_range_covers_everything_that_moved() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: Some(0),
            waves: vec![wave(1, TURN / 4, 32, [true, true, true])],
        };
        let mut anim = Animation::new(&level, &config);
        let before = level.palette;
        let range = anim.tick(&mut level);
        for (entry, was) in before.iter().enumerate() {
            if level.palette[entry] != *was {
                assert!(
                    range.contains(&(entry as u32)),
                    "entry {} changed outside {:?}",
                    entry,
                    range
                );
            }
        }
    }

    #[test]
    fn switching_it_off_holds_the_palette_still() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: Some(0),
            waves: vec![wave(1, TURN / 4, 32, [true, true, true])],
        };
        let mut anim = Animation::new(&level, &config);
        anim.enabled = false;
        assert_eq!(anim.tick(&mut level), 0..0);
        assert!(level.palette.iter().all(|c| c[0] == 128));
    }

    #[test]
    fn rebasing_moves_the_colours_the_animation_returns_to() {
        let mut level = test_level();
        let config = DynamicPalette {
            wave_terrain: None,
            waves: vec![wave(1, TURN / 2, 32, [true, true, true])],
        };
        let mut anim = Animation::new(&level, &config);
        anim.rebase(&[[64, 64, 64, 0xFF]; 0x100]);
        // Half a turn puts the sine at zero, so the ramp comes out as its
        // base colour - the new one.
        anim.tick(&mut level);
        assert_eq!(level.palette[20][0], 64);
    }
}
