//! Cosmetic tyre tracks for multiplayer clients.
//!
//! Online, the level a client steps its car over has to stay the server's:
//! the server never edits its copy, and a client that cut its own tracks
//! drove over different ground and was corrected
//! (`terraform::drain_vehicle`). The tracks are still worth seeing, so the
//! client cuts them into this overlay instead: a sparse set of 64x64 tiles
//! holding the drawn height and meta of the texels around the tracks. The
//! renderer composites it over the level when it uploads the terrain
//! texture, which every terrain renderer (the mesh included) reads its
//! heights from, so the tracks show everywhere while the physics never
//! sees them.
//!
//! A tread is cut by swapping the overlay's texels into the level around
//! the track, cutting it there with [`terraform::apply_tread`] (so a track
//! over an older track deepens it, as offline), copying the result back
//! into the overlay and restoring the level, all in one call: the level
//! comes out bit for bit as it went in.
//!
//! The overlay is capped at [`MAX_BYTES`]; past that the tiles touched
//! longest ago go first, and the level shows through again. A real edit of
//! the level (moving land, a blast) drops the tiles it touches, since their
//! copy of the ground under the tracks is stale.

use super::{
    Level, Region,
    terraform::{self, Track, Tracks, Tread},
};
use std::collections::HashMap;

/// Tile side, in texels.
pub const TILE: i32 = 64;
/// Bytes of one tile: a height and a meta byte per texel.
pub const TILE_BYTES: usize = 2 * (TILE * TILE) as usize;
/// The overlay's budget.
pub const MAX_BYTES: usize = 4 << 20;
/// Longest wheel stretch cut, in texels per axis: anything longer is a
/// teleport or a stall, and would pull in a field of tiles.
const MAX_STRETCH: i32 = TILE;

struct Tile {
    height: Box<[u8]>,
    meta: Box<[u8]>,
    /// When the tile was last cut into; the smallest goes first.
    touched: u64,
}

pub struct TrackOverlay {
    tiles: HashMap<(i32, i32), Tile>,
    cap_tiles: usize,
    clock: u64,
    /// Size of the level the tiles were cut from.
    size: (i32, i32),
    /// The level's texels saved while a track is cut.
    scratch: Vec<(u8, u8)>,
}

impl Default for TrackOverlay {
    fn default() -> Self {
        Self::with_cap(MAX_BYTES / TILE_BYTES)
    }
}

impl TrackOverlay {
    /// An overlay holding at most `cap_tiles` tiles.
    pub fn with_cap(cap_tiles: usize) -> Self {
        TrackOverlay {
            tiles: HashMap::new(),
            cap_tiles: cap_tiles.max(1),
            clock: 0,
            size: (0, 0),
            scratch: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    /// Tiles held.
    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    /// Bytes of tile data held (the map's own bookkeeping aside).
    pub fn bytes(&self) -> usize {
        self.tiles.len() * TILE_BYTES
    }

    /// Texels the overlay draws differently from `level`: the tracks.
    pub fn changed_texels(&self, level: &Level) -> usize {
        let mut changed = 0;
        for (&(tx, ty), tile) in &self.tiles {
            for y in 0..TILE {
                let base = ((ty * TILE + y) * level.size.0 + tx * TILE) as usize;
                for x in 0..TILE as usize {
                    let t = (y * TILE) as usize + x;
                    if (tile.height[t], tile.meta[t])
                        != (level.height[base + x], level.meta[base + x])
                    {
                        changed += 1;
                    }
                }
            }
        }
        changed
    }

    /// The drawn height and meta at `(x, y)` (wrapped), where the overlay
    /// has a tile.
    pub fn get(&self, level_size: (i32, i32), x: i32, y: i32) -> Option<(u8, u8)> {
        let (x, y) = (x.rem_euclid(level_size.0), y.rem_euclid(level_size.1));
        let tile = self.tiles.get(&(x / TILE, y / TILE))?;
        let i = ((y % TILE) * TILE + x % TILE) as usize;
        Some((tile.height[i], tile.meta[i]))
    }

    /// Overwrites the texels of row `y` from `x0` that the overlay holds in
    /// `line`, interleaved height and meta (the terrain texture's layout).
    /// The span must not wrap.
    pub fn patch_row(&self, y: usize, x0: usize, line: &mut [u8]) {
        if self.tiles.is_empty() || line.len() < 2 {
            return;
        }
        let (y, x0) = (y as i32, x0 as i32);
        let x1 = x0 + (line.len() / 2) as i32;
        let row = ((y % TILE) * TILE) as usize;
        let mut tx = x0 / TILE;
        while tx * TILE < x1 {
            if let Some(tile) = self.tiles.get(&(tx, y / TILE)) {
                let from = (tx * TILE).max(x0);
                let to = ((tx + 1) * TILE).min(x1);
                for x in from..to {
                    let i = row + (x % TILE) as usize;
                    let o = 2 * (x - x0) as usize;
                    line[o] = tile.height[i];
                    line[o + 1] = tile.meta[i];
                }
            }
            tx += 1;
        }
    }

    /// The online counterpart of [`terraform::apply_vehicle`]: the treads go
    /// into the overlay, everything else a vehicle recorded (smashed
    /// ceilings, burrows, the hull press, grader sweeps) is dropped as
    /// [`terraform::drain_vehicle`] does, and the level is left as it was.
    /// `regions` gets what to re-upload. Returns the wheel stretches, for
    /// the dust.
    pub fn cut_vehicle(
        &mut self,
        level: &mut Level,
        config: &terraform::Config,
        tracks: &mut Tracks,
        regions: &mut Vec<Region>,
    ) -> Vec<Track> {
        let treads = terraform::drain_vehicle(tracks);
        if self.size != level.size {
            self.clear(regions);
            self.size = level.size;
        }
        self.clock += 1;
        for track in &treads {
            self.cut_tread(level, &config.tread, track, regions);
        }
        self.enforce_cap(regions);
        treads
    }

    /// Cuts one wheel stretch into the overlay.
    fn cut_tread(
        &mut self,
        level: &mut Level,
        config: &Tread,
        track: &Track,
        regions: &mut Vec<Region>,
    ) {
        if !config.enabled || config.depth == 0 || config.period == 0 || config.bar == 0 {
            return;
        }
        let size = level.size;
        let dx = wrap_delta(track.to.0 - track.from.0, size.0);
        let dy = wrap_delta(track.to.1 - track.from.1, size.1);
        if (dx == 0 && dy == 0) || dx.abs() > MAX_STRETCH || dy.abs() > MAX_STRETCH {
            return;
        }
        // Everything `apply_tread` may write: the stretch, the bar across
        // it (`across` is at most unit length) and the other half of a
        // double-level pair.
        let margin = (config.spacing * config.bar as f32 * 0.5).ceil() as i32 + 2;
        let x0 = track.from.0.min(track.from.0 + dx) - margin;
        let x1 = track.from.0.max(track.from.0 + dx) + margin;
        let y0 = track.from.1.min(track.from.1 + dy) - margin;
        let y1 = track.from.1.max(track.from.1 + dy) + margin;

        let mut fresh = Vec::new();
        for ty in y0.div_euclid(TILE)..=y1.div_euclid(TILE) {
            for tx in x0.div_euclid(TILE)..=x1.div_euclid(TILE) {
                fresh.extend(self.touch(level, (tx, ty)));
            }
        }

        // Swap the drawn ground in, cut, keep the result, swap back.
        self.scratch.clear();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = level.wrap((x, y));
                self.scratch.push((level.height[i], level.meta[i]));
                let (h, m) = self.get(size, x, y).expect("tile was just touched");
                level.height[i] = h;
                level.meta[i] = m;
            }
        }
        let before = regions.len();
        terraform::apply_tread(level, config, track, regions);
        let mut saved = self.scratch.iter();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = level.wrap((x, y));
                let (wx, wy) = (x.rem_euclid(size.0), y.rem_euclid(size.1));
                let tile = self.tiles.get_mut(&(wx / TILE, wy / TILE)).unwrap();
                let t = ((wy % TILE) * TILE + wx % TILE) as usize;
                tile.height[t] = level.height[i];
                tile.meta[t] = level.meta[i];
                let &(h, m) = saved.next().unwrap();
                level.height[i] = h;
                level.meta[i] = m;
            }
        }
        // A tile the tread made but did not bite into (a road, a stretch
        // that only grazed it) would only cost memory.
        let cut = &regions[before..];
        for key in fresh {
            let hit = cut.iter().any(|r| {
                r.x < (key.0 + 1) * TILE
                    && r.x + r.w > key.0 * TILE
                    && r.y < (key.1 + 1) * TILE
                    && r.y + r.h > key.1 * TILE
            });
            if !hit {
                self.tiles.remove(&key);
            }
        }
    }

    /// Makes sure the tile at `key` (unwrapped tile coordinates) exists,
    /// copied from the level, and marks it as cut into now. Returns the
    /// wrapped key of a tile it had to make.
    fn touch(&mut self, level: &Level, key: (i32, i32)) -> Option<(i32, i32)> {
        let key = (
            key.0.rem_euclid(level.size.0 / TILE),
            key.1.rem_euclid(level.size.1 / TILE),
        );
        let clock = self.clock;
        let mut made = None;
        let tile = self.tiles.entry(key).or_insert_with(|| {
            made = Some(key);
            let n = (TILE * TILE) as usize;
            let (mut height, mut meta) = (vec![0; n], vec![0; n]);
            for ty in 0..TILE {
                let base = ((key.1 * TILE + ty) * level.size.0 + key.0 * TILE) as usize;
                let row = (ty * TILE) as usize..((ty + 1) * TILE) as usize;
                height[row.clone()].copy_from_slice(&level.height[base..base + TILE as usize]);
                meta[row].copy_from_slice(&level.meta[base..base + TILE as usize]);
            }
            Tile {
                height: height.into_boxed_slice(),
                meta: meta.into_boxed_slice(),
                touched: clock,
            }
        });
        tile.touched = clock;
        made
    }

    /// Past the cap, the tiles cut into longest ago go, and the level shows
    /// through there again.
    fn enforce_cap(&mut self, regions: &mut Vec<Region>) {
        while self.tiles.len() > self.cap_tiles {
            let oldest = self
                .tiles
                .iter()
                .map(|(&key, tile)| (tile.touched, key))
                .min()
                .map(|(_, key)| key)
                .unwrap();
            self.tiles.remove(&oldest);
            push_tile(regions, oldest, self.size);
        }
    }

    /// The level changed for real under `edited`: the tiles there hold a
    /// stale copy of the ground, so they go (their tracks with them).
    /// `regions` gets the dropped tiles, to re-upload from the level.
    pub fn invalidate(&mut self, edited: &[Region], regions: &mut Vec<Region>) {
        if self.tiles.is_empty() {
            return;
        }
        let size = self.size;
        for r in edited {
            if r.w <= 0 || r.h <= 0 {
                continue;
            }
            for ty in r.y.div_euclid(TILE)..=(r.y + r.h - 1).div_euclid(TILE) {
                for tx in r.x.div_euclid(TILE)..=(r.x + r.w - 1).div_euclid(TILE) {
                    let key = (tx.rem_euclid(size.0 / TILE), ty.rem_euclid(size.1 / TILE));
                    if self.tiles.remove(&key).is_some() {
                        push_tile(regions, key, size);
                    }
                }
            }
        }
    }

    /// Drops every tile (a new level, the session over); `regions` gets
    /// them, to re-upload from the level.
    pub fn clear(&mut self, regions: &mut Vec<Region>) {
        for (key, _) in self.tiles.drain() {
            push_tile(regions, key, self.size);
        }
    }
}

fn push_tile(regions: &mut Vec<Region>, key: (i32, i32), size: (i32, i32)) {
    Region::push_wrapped(regions, key.0 * TILE, key.1 * TILE, TILE, TILE, size);
}

/// The shorter way round the torus.
fn wrap_delta(d: i32, size: i32) -> i32 {
    let d = d.rem_euclid(size);
    if d > size / 2 { d - size } else { d }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{LevelConfig, TerrainBits, terraform::MAIN_TERRAIN};

    const SIZE: i32 = 512;

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

    fn copy(level: &Level) -> Level {
        Level {
            size: level.size,
            flood_map: level.flood_map.clone(),
            height: level.height.clone(),
            meta: level.meta.clone(),
            palette: level.palette,
            terrains: level.terrains.clone(),
            geometry: level.geometry.clone(),
        }
    }

    fn stretch(from: (i32, i32)) -> Track {
        Track {
            from,
            to: (from.0 + 12, from.1 + 3),
            across: (0.24, -0.97),
        }
    }

    fn one_frame(track: Track) -> Tracks {
        let mut tracks = Tracks::default();
        tracks.touch(0, track.from, track.across);
        tracks.touch(0, track.to, track.across);
        tracks
    }

    /// What the overlay draws over `level` at `(x, y)`.
    fn drawn(overlay: &TrackOverlay, level: &Level, x: i32, y: i32) -> (u8, u8) {
        let i = level.wrap((x, y));
        overlay
            .get(level.size, x, y)
            .unwrap_or((level.height[i], level.meta[i]))
    }

    #[test]
    fn cut_leaves_the_level_and_draws_the_offline_track() {
        let mut level = test_level();
        let pristine = copy(&level);
        let mut offline = copy(&level);
        let mut overlay = TrackOverlay::default();
        let config = terraform::Config::default();
        // Twice over the same ground, and across a tile corner and the
        // seam, so stacking and wrapping are both covered.
        let tracks = [
            stretch((60, 62)),
            stretch((58, 63)),
            stretch((SIZE - 6, SIZE - 2)),
        ];
        let mut regions = Vec::new();
        for &track in &tracks {
            terraform::apply_tread(&mut offline, &config.tread, &track, &mut Vec::new());
            let out = overlay.cut_vehicle(&mut level, &config, &mut one_frame(track), &mut regions);
            assert_eq!(out, [track]);
        }
        assert!(level.height == pristine.height && level.meta == pristine.meta);
        assert!(!regions.is_empty());
        assert_ne!(offline.height, pristine.height);
        for y in 0..SIZE {
            for x in 0..SIZE {
                let i = level.wrap((x, y));
                assert_eq!(
                    drawn(&overlay, &level, x, y),
                    (offline.height[i], offline.meta[i]),
                    "at ({x},{y})"
                );
            }
        }

        // A row patched for upload, across a tile edge, shows it too.
        for y in 55..75 {
            let x0 = 40usize;
            let mut line = vec![0u8; 2 * 48];
            for (k, px) in line.chunks_mut(2).enumerate() {
                let i = level.wrap(((x0 + k) as i32, y));
                px[0] = level.height[i];
                px[1] = level.meta[i];
            }
            overlay.patch_row(y as usize, x0, &mut line);
            for (k, px) in line.chunks(2).enumerate() {
                let i = level.wrap(((x0 + k) as i32, y));
                assert_eq!((px[0], px[1]), (offline.height[i], offline.meta[i]));
            }
        }
    }

    #[test]
    fn oldest_tiles_go_first_past_the_cap() {
        let mut level = test_level();
        let mut overlay = TrackOverlay::with_cap(2);
        let config = terraform::Config::default();
        let mut regions = Vec::new();
        let at = |tx: i32| (tx * TILE + 20, TILE + 30);
        for tx in [1, 3, 5] {
            regions.clear();
            overlay.cut_vehicle(
                &mut level,
                &config,
                &mut one_frame(stretch(at(tx))),
                &mut regions,
            );
            assert!(overlay.len() <= 2);
        }
        // The first tile went, and is re-uploaded from the level.
        assert!(regions.contains(&Region {
            x: TILE,
            y: TILE,
            w: TILE,
            h: TILE
        }));
        assert!(overlay.get(level.size, at(1).0, at(1).1).is_none());
        assert!(overlay.get(level.size, at(3).0, at(3).1).is_some());
        assert!(overlay.get(level.size, at(5).0, at(5).1).is_some());

        // Cutting into the older of the two again makes it the newest.
        overlay.cut_vehicle(
            &mut level,
            &config,
            &mut one_frame(stretch(at(3))),
            &mut regions,
        );
        overlay.cut_vehicle(
            &mut level,
            &config,
            &mut one_frame(stretch(at(7))),
            &mut regions,
        );
        assert!(overlay.get(level.size, at(3).0, at(3).1).is_some());
        assert!(overlay.get(level.size, at(5).0, at(5).1).is_none());
        assert!(overlay.get(level.size, at(7).0, at(7).1).is_some());
        assert_eq!(overlay.bytes(), 2 * TILE_BYTES);
    }

    #[test]
    fn real_edits_drop_the_tiles_under_them() {
        let mut level = test_level();
        let mut overlay = TrackOverlay::default();
        let config = terraform::Config::default();
        let track = stretch((100, 100));
        overlay.cut_vehicle(&mut level, &config, &mut one_frame(track), &mut Vec::new());
        assert!(!overlay.is_empty());
        let mut regions = Vec::new();
        let edit = Region {
            x: 101,
            y: 101,
            w: 1,
            h: 1,
        };
        overlay.invalidate(&[edit], &mut regions);
        assert!(overlay.get(level.size, 101, 101).is_none());
        assert!(regions.contains(&Region {
            x: 64,
            y: 64,
            w: 64,
            h: 64
        }));
        regions.clear();
        overlay.clear(&mut regions);
        assert!(overlay.is_empty());
    }

    #[test]
    fn the_cap_is_four_megabytes() {
        assert_eq!(TrackOverlay::default().cap_tiles * TILE_BYTES, MAX_BYTES);
        assert_eq!(MAX_BYTES / TILE_BYTES, 512);
    }
}
