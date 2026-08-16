//! Moving land - the animated surface patches loaded from `*.vot` files.
//!
//! This is a port of `MobileLocation::quant`/`MLFrame::quant` of
//! `src/units/moveland.cpp` in the original game. A location owns a looping
//! list of frames; each frame rewrites a rectangle of the level's altitude
//! (and optionally terrain) planes, either by interpolating a signed offset
//! over `period` quants, or by writing target altitudes straight through.
//!
//! Everything here mutates [`Level`] in place, and reports the touched
//! rectangles so the renderer can re-upload them.

use crate::level::{DOUBLE_LEVEL, Level};

use std::{io, path::Path, sync::Arc};

pub use vot::{Frame, MAX_KEY_PHASE, MobileLocation, Mode};

/// `MLPREC` of the original: the fractional bits of the interpolation
/// accumulator. Altitudes are bytes, so an `i32` leaves plenty of headroom.
const PRECISION: u32 = 16;

/// `goPh == -1` of the original: the location loops forever instead of
/// stopping at a phase.
pub const FREE_RUNNING: i32 = -1;

/// A rectangle of level texels, guaranteed not to wrap around the level.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Region {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

/// A single moving-land instance: the shared frame data plus the playback
/// state (`cFrame`/`cStage`/`steps`/`alt` of the original).
///
/// The frame data is shared so that clones - copies of a location placed at
/// a different spot - cost only their own playback state.
pub struct Location {
    pub source: Arc<MobileLocation>,
    /// `(dx, dy)` - offset of this instance, non-zero only for clones.
    pub offset: (i32, i32),
    /// Index of the frame currently being played.
    frame: usize,
    /// Per-frame quant counter. Reset when the frame hands over.
    steps: Vec<i32>,
    /// Fixed-point altitude accumulator, one entry per texel of the frame
    /// being interpolated. Sized for the largest frame.
    alt: Vec<i32>,
    /// `cStage` - quants elapsed in this loop, `-1` before the first one.
    stage: i32,
    /// `maxStage` - quants in a full loop.
    max_stage: i32,
    /// `goPh` - the phase to stop at, or [`FREE_RUNNING`].
    go_phase: i32,
}

impl Location {
    pub fn new(source: Arc<MobileLocation>, offset: (i32, i32)) -> Self {
        let (max_x, max_y) = source.max_frame_size();
        let steps = vec![0; source.frames.len()];
        let max_stage = source.max_stage();
        Location {
            source,
            offset,
            frame: 0,
            steps,
            alt: vec![0; max_x as usize * max_y as usize],
            stage: -1,
            max_stage,
            go_phase: FREE_RUNNING,
        }
    }

    /// Index of the frame being played.
    pub fn current_frame(&self) -> usize {
        self.frame
    }

    /// `getCurPhase` - the stage this location is about to enter. Phases and
    /// stages share a numbering, so this is what [`Self::set_go_phase`] is
    /// compared against.
    pub fn current_phase(&self) -> i32 {
        if self.stage == self.max_stage - 1 {
            0
        } else {
            self.stage + 1
        }
    }

    /// `isGoFinish` - the location has reached the phase it was sent to and
    /// is parked there.
    pub fn is_go_finish(&self) -> bool {
        self.go_phase == self.current_phase()
    }

    /// `goPhase` - park at `phase` once it comes around. [`FREE_RUNNING`]
    /// puts the location back into its endless loop.
    pub fn set_go_phase(&mut self, phase: i32) {
        self.go_phase = phase;
    }

    pub fn go_phase(&self) -> i32 {
        self.go_phase
    }

    /// `goKeyPhase` - park at one of the location's authored key phases.
    /// A negative index means [`FREE_RUNNING`].
    pub fn go_key_phase(&mut self, index: i32) {
        self.go_phase = match self.source.key_phases.get(index.max(0) as usize) {
            Some(&phase) if index >= 0 => phase,
            _ => FREE_RUNNING,
        };
    }

    /// Rewind to the very beginning without touching the level -
    /// `setPhase(0, noChange)` of the original.
    pub fn reset(&mut self) {
        self.frame = 0;
        self.stage = -1;
        self.steps.iter_mut().for_each(|s| *s = 0);
    }

    /// `setPhase` - jump straight to `frame`, applying every frame along the
    /// way in one go rather than interpolating them.
    pub fn set_phase(&mut self, frame: usize, level: &mut Level, regions: &mut Vec<Region>) {
        if frame >= self.source.frames.len() {
            return;
        }
        let saved = self.go_phase;
        self.go_phase = FREE_RUNNING;
        // Each frame takes two fast steps: one to apply, one to hand over.
        for _ in 0..2 * self.source.frames.len() + 2 {
            if self.frame == frame && self.steps[self.frame] == 0 {
                break;
            }
            self.stage += 1;
            if self.step_frame(level, regions, true) {
                self.advance_frame();
            }
        }
        self.go_phase = saved;
    }

    /// Advance the animation by one quant, writing into `level` and pushing
    /// the touched rectangles onto `regions`.
    ///
    /// A frame that has just run out of steps hands over to the next one
    /// within the same quant, exactly like `MobileLocation::quant` does.
    pub fn update(&mut self, level: &mut Level, regions: &mut Vec<Region>) {
        if self.source.frames.is_empty() || self.is_go_finish() {
            return;
        }
        self.stage += 1;
        if self.step_frame(level, regions, false) {
            self.advance_frame();
            self.step_frame(level, regions, false);
        }
    }

    fn advance_frame(&mut self) {
        self.frame += 1;
        if self.frame == self.source.frames.len() {
            self.frame = 0;
            self.stage = 0;
        }
    }

    /// Runs one quant of the current frame. Returns `true` when the frame is
    /// finished and the next one should take over.
    ///
    /// `fast` is `fastMode` of the original: the frame is applied in a single
    /// step, skipping the interpolation. It is how a location seeks to a
    /// phase without animating through it.
    fn step_frame(&mut self, level: &mut Level, regions: &mut Vec<Region>, fast: bool) -> bool {
        let frame = &self.source.frames[self.frame];
        let step = &mut self.steps[self.frame];
        // `quantAbs` ignores the period entirely, and so does a fast seek.
        let period = if self.source.mode.is_relative() && !fast {
            frame.period
        } else {
            1
        };
        let interpolated = period > 1;

        if interpolated && *step == 0 {
            snapshot(frame, self.offset, level, &mut self.alt);
        }

        *step += 1;
        if *step > period {
            *step = 0;
            return true;
        }

        // The terrain type only lands on the final step of the frame.
        let terrain = if *step == period {
            Terrain::of(frame, level)
        } else {
            Terrain::Keep
        };

        if self.source.mode.is_relative() {
            if interpolated {
                apply_interpolated(frame, self.offset, level, &mut self.alt, terrain);
            } else {
                apply_relative(frame, self.offset, level, terrain);
            }
        } else {
            apply_absolute(frame, self.offset, level, terrain);
        }

        push_regions(frame, self.offset, level, regions);
        false
    }
}

/// What to write into the terrain bits of every touched texel.
enum Terrain {
    /// Leave the terrain type alone.
    Keep,
    /// One type for the whole frame, already shifted into place.
    Uniform(u8),
    /// A per-texel plane, also already shifted.
    PerTexel,
}

impl Terrain {
    fn of(frame: &Frame, level: &Level) -> Self {
        if !frame.terrain.is_empty() {
            Terrain::PerTexel
        } else if frame.surface_type >= 0 {
            let bits = level.terrain_bits();
            Terrain::Uniform(bits.write(frame.surface_type as u8 & bits.mask))
        } else {
            Terrain::Keep
        }
    }
}

/// Iterates the texels of `frame`, yielding `(local index, level index,
/// wrapped x)` for each. The x parity drives the double-level rules, so it
/// has to be the wrapped, level-space column.
fn texels(
    frame: &Frame,
    offset: (i32, i32),
    size: (i32, i32),
) -> impl Iterator<Item = (usize, usize, i32)> + '_ {
    let (x0, y0) = (frame.pos.0 + offset.0, frame.pos.1 + offset.1);
    (0..frame.size.1).flat_map(move |j| {
        let y = (y0 + j).rem_euclid(size.1);
        let row = (y * size.0) as usize;
        let local_row = (j * frame.size.0) as usize;
        (0..frame.size.0).map(move |i| {
            let x = (x0 + i).rem_euclid(size.0);
            (local_row + i as usize, row + x as usize, x)
        })
    })
}

/// `SET_UP_ALT` - on a double-level texel pair only the odd (upper) half
/// carries the altitude that the moving land is allowed to move.
fn set_up_alt(level: &mut Level, index: usize, x: i32, height: u8) {
    if level.meta[index] & DOUBLE_LEVEL == 0 || x & 1 != 0 {
        level.height[index] = height;
    }
}

/// `SET_REAL_TERRAIN`, with the same upper-half rule as `set_up_alt`.
fn set_up_terrain(level: &mut Level, index: usize, x: i32, terrain: u8, mask: u8) {
    if level.meta[index] & DOUBLE_LEVEL == 0 || x & 1 != 0 {
        level.meta[index] = (level.meta[index] & !mask) | terrain;
    }
}

fn write_terrain(
    frame: &Frame,
    level: &mut Level,
    terrain: &Terrain,
    mask: u8,
    local: usize,
    index: usize,
    x: i32,
) {
    match *terrain {
        Terrain::Keep => {}
        Terrain::Uniform(value) => set_up_terrain(level, index, x, value, mask),
        Terrain::PerTexel => {
            let value = frame.terrain[local] & mask;
            set_up_terrain(level, index, x, value, mask)
        }
    }
}

/// Bits of `meta` that hold the terrain type.
fn terrain_mask(level: &Level) -> u8 {
    let bits = level.terrain_bits();
    bits.mask << bits.shift
}

/// `MLFrame::start` - seeds the accumulator with the current altitudes,
/// rounded to the middle of the texel so the interpolation doesn't drift.
fn snapshot(frame: &Frame, offset: (i32, i32), level: &Level, alt: &mut [i32]) {
    for (local, index, _) in texels(frame, offset, level.size) {
        alt[local] = ((level.height[index] as i32) << PRECISION) + (1 << (PRECISION - 1));
    }
}

/// The `period > 1` path: the total offset is spread evenly over the frame's
/// quants, and the accumulator remembers the sub-texel remainder.
fn apply_interpolated(
    frame: &Frame,
    offset: (i32, i32),
    level: &mut Level,
    alt: &mut [i32],
    terrain: Terrain,
) {
    let period = frame.period;
    let mask = terrain_mask(level);
    for (local, index, x) in texels(frame, offset, level.size) {
        let mut delta = frame.delta[local] as i32;
        if delta == 0 {
            continue;
        }
        if frame.is_negative(local) {
            delta = -delta;
        }
        // Undo the part of the offset already applied, then re-apply the
        // accumulated total - this keeps other terrain edits in the same
        // rectangle from being clobbered.
        let mut value = level.height[index] as i32 - (alt[local] >> PRECISION);
        alt[local] += (delta << PRECISION) / period;
        value += alt[local] >> PRECISION;
        if value < 1 {
            value = 0;
            alt[local] = 0;
        }
        set_up_alt(level, index, x, value as u8);
        write_terrain(frame, level, &terrain, mask, local, index, x);
    }
}

/// The `period == 1` path: a plain signed offset, applied in one go.
fn apply_relative(frame: &Frame, offset: (i32, i32), level: &mut Level, terrain: Terrain) {
    let mask = terrain_mask(level);
    for (local, index, x) in texels(frame, offset, level.size) {
        let mut delta = frame.delta[local] as i32;
        if delta == 0 {
            continue;
        }
        if frame.is_negative(local) {
            delta = -delta;
        }
        let value = (level.height[index] as i32 + delta).max(0);
        set_up_alt(level, index, x, value as u8);
        write_terrain(frame, level, &terrain, mask, local, index, x);
    }
}

/// `MLFrame::quantAbs` - the plane holds target altitudes, and zero is the
/// "don't touch" marker. Note that unlike the relative paths this one ignores
/// the double-level rules, matching the original.
fn apply_absolute(frame: &Frame, offset: (i32, i32), level: &mut Level, terrain: Terrain) {
    let mask = terrain_mask(level);
    for (local, index, _) in texels(frame, offset, level.size) {
        let value = frame.delta[local];
        if value == 0 {
            continue;
        }
        level.height[index] = value;
        match terrain {
            Terrain::Keep => {}
            Terrain::Uniform(t) => level.meta[index] = (level.meta[index] & !mask) | t,
            Terrain::PerTexel => {
                let t = frame.terrain[local] & mask;
                level.meta[index] = (level.meta[index] & !mask) | t;
            }
        }
    }
}

/// Splits the frame's rectangle at the level seams, so every emitted region
/// is a plain in-bounds rectangle.
fn push_regions(frame: &Frame, offset: (i32, i32), level: &Level, regions: &mut Vec<Region>) {
    let x0 = (frame.pos.0 + offset.0).rem_euclid(level.size.0);
    let y0 = (frame.pos.1 + offset.1).rem_euclid(level.size.1);
    let spans_x = split_span(x0, frame.size.0, level.size.0);
    let spans_y = split_span(y0, frame.size.1, level.size.1);
    for &(y, h) in spans_y.iter().flatten() {
        for &(x, w) in spans_x.iter().flatten() {
            regions.push(Region { x, y, w, h });
        }
    }
}

/// Cuts `[start, start + length)` into at most two non-wrapping spans.
fn split_span(start: i32, length: i32, total: i32) -> [Option<(i32, i32)>; 2] {
    let length = length.min(total);
    if length <= 0 {
        return [None, None];
    }
    if start + length <= total {
        [Some((start, length)), None]
    } else {
        [
            Some((start, total - start)),
            Some((0, start + length - total)),
        ]
    }
}

/// All the moving land of one level.
#[derive(Default)]
pub struct MovingLand {
    pub locations: Vec<Location>,
}

impl MovingLand {
    /// Loads every `*.vot` of `dir`, which is the level's `data.vot` folder.
    /// A missing folder just means the level has no moving land.
    pub fn load_dir(dir: &Path, terrain_count: i32) -> Self {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                if e.kind() != io::ErrorKind::NotFound {
                    log::warn!("Unable to scan {:?} for moving land: {}", dir, e);
                }
                return Self::default();
            }
        };

        // The original walks the directory in `alphasort` order; matching it
        // keeps overlapping locations resolving the same way.
        let mut paths = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("vot"))
            })
            .collect::<Vec<_>>();
        paths.sort();

        let mut locations = Vec::new();
        for path in paths {
            match Self::load_file(&path, terrain_count) {
                Ok(ml) => {
                    log::info!(
                        "Loaded moving land '{}' ({} frames, {:?})",
                        ml.name,
                        ml.frames.len(),
                        ml.mode
                    );
                    locations.push(Location::new(Arc::new(ml), (0, 0)));
                }
                Err(e) => log::error!("Unable to load {:?}: {}", path, e),
            }
        }

        MovingLand { locations }
    }

    fn load_file(path: &Path, terrain_count: i32) -> Result<MobileLocation, vot::Error> {
        let file = std::fs::File::open(path)?;
        MobileLocation::load(&mut io::BufReader::new(file), terrain_count)
    }

    pub fn is_empty(&self) -> bool {
        self.locations.is_empty()
    }

    /// Index of the location named `name`, which is how `location.lst`
    /// addresses them - `FindMobileLocation` of the original.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.locations.iter().position(|l| l.source.name == name)
    }

    /// `MobileLocation::cloning` - adds another instance of an already loaded
    /// location, shifted by `offset`, with its own playback state.
    pub fn add_clone(&mut self, index: usize, offset: (i32, i32)) -> usize {
        let source = Arc::clone(&self.locations[index].source);
        self.locations.push(Location::new(source, offset));
        self.locations.len() - 1
    }

    /// Advances every location by one quant, appending the touched rectangles
    /// to `regions`.
    pub fn update(&mut self, level: &mut Level, regions: &mut Vec<Region>) {
        profiling::scope!("Moving land");
        for location in self.locations.iter_mut() {
            location.update(level, regions);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::LevelConfig;

    const SIZE: i32 = 8;

    fn test_level() -> Level {
        let total = (SIZE * SIZE) as usize;
        Level {
            size: (SIZE, SIZE),
            flood_map: vec![0; SIZE as usize].into_boxed_slice(),
            height: vec![100u8; total].into_boxed_slice(),
            meta: vec![0u8; total].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    fn frame(pos: (i32, i32), size: (i32, i32), delta: Vec<u8>, period: i32) -> Frame {
        let words = delta.len() / 32 + 1;
        Frame {
            pos,
            size,
            period,
            surface_type: -1,
            delta,
            terrain: Vec::new(),
            sign_bits: vec![0; words],
        }
    }

    fn location(mode: Mode, frames: Vec<Frame>) -> Location {
        location_with_keys(mode, frames, [0; MAX_KEY_PHASE])
    }

    fn location_with_keys(
        mode: Mode,
        frames: Vec<Frame>,
        key_phases: [i32; MAX_KEY_PHASE],
    ) -> Location {
        Location::new(
            Arc::new(MobileLocation {
                name: "test".to_string(),
                mode,
                dry_terrain: 0,
                impulse: 0,
                key_phases,
                frames,
            }),
            (0, 0),
        )
    }

    fn heights(level: &Level, x: i32, y: i32, w: i32, h: i32) -> Vec<u8> {
        (0..h)
            .flat_map(|j| (0..w).map(move |i| (x + i, y + j)))
            .map(|(x, y)| level.height[(y.rem_euclid(SIZE) * SIZE + x.rem_euclid(SIZE)) as usize])
            .collect()
    }

    #[test]
    fn relative_single_step() {
        let mut level = test_level();
        let mut ml = location(Mode::Relative, vec![frame((2, 2), (2, 1), vec![10, 0], 1)]);
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(heights(&level, 2, 2, 2, 1), [110, 100]);
        assert_eq!(
            regions,
            [Region {
                x: 2,
                y: 2,
                w: 2,
                h: 1
            }]
        );

        // The next quant retires the frame and immediately starts the only
        // other one - which is the same frame again, so it applies twice.
        ml.update(&mut level, &mut regions);
        assert_eq!(heights(&level, 2, 2, 2, 1), [120, 100]);
    }

    #[test]
    fn negative_delta_clamps_at_zero() {
        let mut level = test_level();
        let mut f = frame((0, 0), (1, 1), vec![200], 1);
        f.sign_bits[0] = 1;
        let mut ml = location(Mode::Relative, vec![f]);
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 0);
    }

    #[test]
    fn interpolation_spreads_over_the_period() {
        let mut level = test_level();
        let mut ml = location(Mode::Relative, vec![frame((0, 0), (1, 1), vec![9], 3)]);
        let mut regions = Vec::new();

        // 9 units over 3 quants, with the accumulator carrying the remainder.
        for expected in [103, 106, 109] {
            ml.update(&mut level, &mut regions);
            assert_eq!(level.height[0], expected);
        }
    }

    #[test]
    fn interpolation_preserves_concurrent_edits() {
        let mut level = test_level();
        let mut ml = location(Mode::Relative, vec![frame((0, 0), (1, 1), vec![8], 4)]);
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 102);
        // Something else digs the same texel between quants.
        level.height[0] -= 50;
        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 54);
    }

    #[test]
    fn absolute_writes_through_and_skips_zeros() {
        let mut level = test_level();
        let mut ml = location(
            Mode::Absolute,
            vec![frame((1, 1), (3, 1), vec![7, 0, 9], 5)],
        );
        let mut regions = Vec::new();

        // The period is ignored in absolute mode: one quant does it all.
        ml.update(&mut level, &mut regions);
        assert_eq!(heights(&level, 1, 1, 3, 1), [7, 100, 9]);
    }

    #[test]
    fn frames_advance_in_order() {
        let mut level = test_level();
        let mut ml = location(
            Mode::Absolute,
            vec![
                frame((0, 0), (1, 1), vec![10], 1),
                frame((0, 0), (1, 1), vec![20], 1),
            ],
        );
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 10);
        assert_eq!(ml.current_frame(), 0);
        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 20);
        assert_eq!(ml.current_frame(), 1);
        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 10);
        assert_eq!(ml.current_frame(), 0);
    }

    #[test]
    fn double_level_only_moves_the_upper_half() {
        let mut level = test_level();
        // Texels 0 and 1 form a dual pair: 0 is the floor, 1 is the slab.
        level.meta[0] = DOUBLE_LEVEL;
        level.meta[1] = DOUBLE_LEVEL;
        let mut ml = location(Mode::Relative, vec![frame((0, 0), (2, 1), vec![5, 5], 1)]);
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 100, "the cave floor must stay put");
        assert_eq!(level.height[1], 105);
    }

    #[test]
    fn terrain_lands_on_the_last_step() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        let mut f = frame((0, 0), (1, 1), vec![3], 2);
        f.surface_type = 5;
        let mut ml = location(Mode::Relative, vec![f]);
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(bits.read(level.meta[0]), 0);
        ml.update(&mut level, &mut regions);
        assert_eq!(bits.read(level.meta[0]), 5);
    }

    #[test]
    fn per_texel_terrain_plane() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        let mut f = frame((0, 0), (2, 1), vec![1, 1], 1);
        // Anything at or above the terrain count means "read the plane".
        f.surface_type = 8;
        f.terrain = vec![bits.write(2), bits.write(6)];
        let mut ml = location(Mode::Relative, vec![f]);
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(bits.read(level.meta[0]), 2);
        assert_eq!(bits.read(level.meta[1]), 6);
    }

    #[test]
    fn wrapping_frame_is_split_into_regions() {
        let mut level = test_level();
        let mut ml = location(
            Mode::Absolute,
            vec![frame((SIZE - 1, SIZE - 1), (2, 2), vec![1, 2, 3, 4], 1)],
        );
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        assert_eq!(regions.len(), 4);
        for r in &regions {
            assert!(r.x >= 0 && r.x + r.w <= SIZE);
            assert!(r.y >= 0 && r.y + r.h <= SIZE);
        }
        // The four corners of the level got one texel each.
        assert_eq!(level.height[(SIZE * SIZE - 1) as usize], 1);
        assert_eq!(level.height[(SIZE * (SIZE - 1)) as usize], 2);
        assert_eq!(level.height[(SIZE - 1) as usize], 3);
        assert_eq!(level.height[0], 4);
    }

    /// The smallest well-formed VOT: one relative frame over a 2x1 rectangle.
    fn one_frame_file(name: &str, delta: [u8; 2]) -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(b"ML3");
        let mut raw_name = [0u8; 16];
        raw_name[..name.len()].copy_from_slice(name.as_bytes());
        data.extend_from_slice(&raw_name);
        for value in [
            1i32, /* frames */
            0,    /* dry */
            0,    /* impulse */
        ] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&[0, Mode::Relative as u8, 0, 0]);
        for value in [0i32; 4] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        // x0, y0, sx, sy, period, surfType, csd, cst, 2 reserved
        for value in [0i32, 0, 2, 1, 1, -1, 0, 0, 0, 0] {
            data.extend_from_slice(&value.to_le_bytes());
        }
        data.extend_from_slice(&delta);
        data.extend_from_slice(&0u32.to_le_bytes()); // sign bits
        data
    }

    #[test]
    fn load_dir_reads_every_vot() {
        let dir = std::env::temp_dir().join(format!("vange-rs-vot-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("b.vot"), one_frame_file("second", [2, 0])).unwrap();
        std::fs::write(dir.join("a.vot"), one_frame_file("first", [1, 0])).unwrap();
        // Anything that isn't a VOT is skipped.
        std::fs::write(dir.join("notes.txt"), b"ignore me").unwrap();

        let mut land = MovingLand::load_dir(&dir, 8);
        std::fs::remove_dir_all(&dir).unwrap();

        assert_eq!(land.locations.len(), 2);
        assert_eq!(land.locations[0].source.name, "first");
        assert_eq!(land.locations[1].source.name, "second");

        let mut level = test_level();
        let mut regions = Vec::new();
        land.update(&mut level, &mut regions);
        // Both locations write the same texel, in order.
        assert_eq!(level.height[0], 103);
    }

    #[test]
    fn missing_dir_is_not_an_error() {
        let land = MovingLand::load_dir(Path::new("/definitely/not/here"), 8);
        assert!(land.is_empty());
    }

    #[test]
    fn empty_location_is_harmless() {
        let mut level = test_level();
        let mut ml = location(Mode::Relative, Vec::new());
        let mut regions = Vec::new();
        ml.update(&mut level, &mut regions);
        assert!(regions.is_empty());
    }

    /// Three absolute frames, so one quant lands one frame and the stage
    /// counter tracks the frame index.
    fn three_step_stairs() -> Location {
        location_with_keys(
            Mode::Absolute,
            vec![
                frame((0, 0), (1, 1), vec![10], 1),
                frame((0, 0), (1, 1), vec![20], 1),
                frame((0, 0), (1, 1), vec![30], 1),
            ],
            [0, 2, 0, 0],
        )
    }

    #[test]
    fn free_running_never_parks() {
        let mut level = test_level();
        let mut ml = three_step_stairs();
        let mut regions = Vec::new();
        assert_eq!(ml.go_phase(), FREE_RUNNING);

        for expected in [10, 20, 30, 10, 20, 30] {
            ml.update(&mut level, &mut regions);
            assert_eq!(level.height[0], expected);
        }
    }

    #[test]
    fn go_phase_parks_and_releases() {
        let mut level = test_level();
        let mut ml = three_step_stairs();
        let mut regions = Vec::new();

        // Key phase 1 is stage 2, the last frame.
        ml.go_key_phase(1);
        for _ in 0..6 {
            ml.update(&mut level, &mut regions);
        }
        assert!(ml.is_go_finish());
        assert_eq!(level.height[0], 20, "parked before entering stage 2");

        // Further quants do nothing at all.
        regions.clear();
        ml.update(&mut level, &mut regions);
        assert!(regions.is_empty());

        // Sending it back to the start releases it.
        ml.go_key_phase(0);
        ml.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 30);
    }

    #[test]
    fn negative_key_phase_frees_the_location() {
        let mut ml = three_step_stairs();
        ml.go_key_phase(1);
        assert_eq!(ml.go_phase(), 2);
        ml.go_key_phase(-1);
        assert_eq!(ml.go_phase(), FREE_RUNNING);
    }

    #[test]
    fn set_phase_seeks_without_interpolating() {
        let mut level = test_level();
        // A long period would take 5 quants per frame when animated.
        let mut ml = location(
            Mode::Relative,
            vec![
                frame((0, 0), (1, 1), vec![10], 5),
                frame((0, 0), (1, 1), vec![20], 5),
            ],
        );
        let mut regions = Vec::new();

        ml.set_phase(1, &mut level, &mut regions);
        assert_eq!(ml.current_frame(), 1);
        // Frame 0 was applied whole rather than a fifth at a time.
        assert_eq!(level.height[0], 110);
    }

    #[test]
    fn set_phase_to_the_current_frame_is_a_no_op() {
        let mut level = test_level();
        let mut ml = three_step_stairs();
        let mut regions = Vec::new();

        ml.set_phase(0, &mut level, &mut regions);
        assert!(regions.is_empty());
        assert_eq!(level.height[0], 100);
    }

    #[test]
    fn set_phase_out_of_range_is_ignored() {
        let mut level = test_level();
        let mut ml = three_step_stairs();
        let mut regions = Vec::new();

        ml.set_phase(9, &mut level, &mut regions);
        assert_eq!(ml.current_frame(), 0);
        assert!(regions.is_empty());
    }

    #[test]
    fn reset_rewinds_without_touching_the_level() {
        let mut level = test_level();
        let mut ml = three_step_stairs();
        let mut regions = Vec::new();

        ml.update(&mut level, &mut regions);
        ml.update(&mut level, &mut regions);
        assert_eq!(ml.current_frame(), 1);

        let before = level.height[0];
        regions.clear();
        ml.reset();
        assert_eq!(ml.current_frame(), 0);
        assert_eq!(ml.current_phase(), 0);
        assert_eq!(level.height[0], before);
        assert!(regions.is_empty());
    }

    #[test]
    fn clones_share_frames_and_run_independently() {
        let mut level = test_level();
        let mut land = MovingLand::default();
        land.locations.push(location(
            Mode::Absolute,
            vec![
                frame((0, 0), (1, 1), vec![10], 1),
                frame((0, 0), (1, 1), vec![20], 1),
            ],
        ));
        let clone = land.add_clone(0, (4, 0));
        assert_eq!(clone, 1);
        assert!(Arc::ptr_eq(
            &land.locations[0].source,
            &land.locations[1].source
        ));

        // Park the clone right away; the original keeps going.
        land.locations[1].set_go_phase(0);
        let mut regions = Vec::new();
        land.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 10, "original ran");
        assert_eq!(level.height[4], 100, "clone is parked");

        land.locations[1].set_go_phase(FREE_RUNNING);
        land.update(&mut level, &mut regions);
        assert_eq!(level.height[0], 20);
        assert_eq!(level.height[4], 10, "clone applies at its own offset");
    }

    #[test]
    fn find_locates_by_name() {
        let mut land = MovingLand::default();
        land.locations.push(location(
            Mode::Absolute,
            vec![frame((0, 0), (1, 1), vec![1], 1)],
        ));
        assert_eq!(land.find("test"), Some(0));
        assert_eq!(land.find("absent"), None);
    }
}
