//! What the cars leave behind them.
//!
//! Two mechanics of the original, both of which reshape the altitude plane
//! under a car that is being driven.
//!
//! [`Tread`] is a port of `pixSetR` (`src/terra/land.cpp`) and
//! `DrawMechosWheelUp` (`src/units/hobj.cpp`): every wheel rolling over
//! soft ground stamps a tread pattern along the stretch it covered since
//! the last quant - a short bar across the track, raised once every
//! [`Tread::period`] texels and cut everywhere else. The pattern is
//! deliberately lopsided, two texels cut for each one raised, so driving
//! somewhere lowers it and circling the same patch digs a bowl out of it.
//!
//! [`Grader`] is a port of `dastPoly3D::make_dast` (`src/dast/poly3d.cpp`),
//! the "TerraMover" the mechos `.prm` files still carry parameters for. A
//! blade across the car's leading edge scrapes off everything standing
//! above it, carries the spoil along, drops it back as it goes and heaps
//! what is left into a berm ahead. That code never shipped - the call site
//! in `Object::analyse_dynamics` is commented out, and so are `make_dast`
//! itself and the `sqr3` and `max_len_mech` it needs, so it would not even
//! compile. What is ported here is its mechanism, not its arithmetic;
//! where the dead code contradicted itself the choice is noted.
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

/// Everything a car may do to the ground.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Config {
    pub tread: Tread,
    pub grader: Grader,
    pub press: Press,
    pub molehills: Molehills,
}

/// Tunables of the tread pattern.
///
/// The defaults are the constants the original passes to
/// `DrawMechosWheelUp(x0, y0, x1, y1, 8, 3, -1, nx, ny, 3)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tread {
    /// Master switch. Off restores terrain that only the moving land edits.
    pub enabled: bool,
    /// Altitude units one stamp moves the surface.
    pub depth: i32,
    /// Texels along the track between two raised bars. The other
    /// `period - 1` texels of each period are cut.
    pub period: u8,
    /// Stamps in one bar, laid out across the track.
    pub bar: u8,
    /// Texels between those stamps. The original's `8/3` spread a
    /// three-stamp bar over eight texels, about the width of a wheel;
    /// here it is halved so the track reads about half as wide.
    pub spacing: f32,
}

impl Default for Tread {
    fn default() -> Self {
        Tread {
            enabled: true,
            depth: 1,
            period: 3,
            bar: 3,
            // `DrawMechosWheelUp`'s `8/3`, halved: a bar that reads about
            // half the stock width, centred on the wheel.
            spacing: 4.0 / 3.0,
        }
    }
}

/// Terrains a blast is allowed to move, as a bit per type.
///
/// `SmoothTerrainMask` of the original, which every crater call site sets
/// before firing: the plain ground, the sand and the rock give way, while
/// the special surfaces do not.
pub const DESTRUCTIBLE: u16 = 0b0100_1111;

/// The shape a spot's altitude change takes from its centre to its rim.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    /// `DestroySpot`: straight sides, coming to a point at the middle.
    #[default]
    Cone,
    /// `xDestroySpot`: a quarter turn of a sine, so the middle is flat and
    /// the sides ease off into the rim. What the lava spots swell with.
    Dome,
}

/// One cone of `DestroySpot`: the shape a blast leaves.
///
/// Craters in the original are two of these together - a shallow wide one
/// that throws a rim up, and a deep narrow one that digs the bowl out
/// inside it. [`crater`] pairs them the way the game does.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    /// Texels from the centre to the edge.
    pub radius: i32,
    /// How far the ground moves, in 8.8 fixed point. A [`Profile::Cone`]
    /// reads it per texel in from the rim, so `512` over a radius of ten
    /// digs twenty; a [`Profile::Dome`] reads it as the depth at the middle
    /// outright. The original draws the same distinction between its two
    /// spot routines.
    pub delta: i32,
    /// Which way the sides fall away.
    pub profile: Profile,
    /// How frayed the edge is. `rfactor` of the original: `0` is a clean
    /// circle, and each step up doubles the reach of the dice roll that
    /// decides whether a texel is touched at all.
    pub ragged: u8,
    /// Terrain to leave behind, or `None` to keep what was there. The
    /// original spells the second one `83`, a type that does not exist.
    pub terrain: Option<u8>,
    /// Terrains this is allowed to move.
    pub mask: u16,
    /// Which side of a slab it works on.
    pub surface: Surface,
}

impl Default for Spot {
    fn default() -> Self {
        Spot {
            radius: 10,
            delta: -(1 << 11),
            profile: Profile::Cone,
            ragged: 1,
            terrain: None,
            mask: DESTRUCTIBLE,
            surface: Surface::Upper,
        }
    }
}

/// Blows a crater at `at`: a rim thrown up, and a bowl dug out inside it.
///
/// The two spots are `MAP_POINT_CRATER01` of the original, scaled to
/// `radius`. Returns the entries it touched through `regions`.
pub fn crater(level: &mut Level, at: (i32, i32), radius: i32, regions: &mut Vec<Region>) {
    let rim = Spot {
        radius,
        delta: 512,
        ragged: 1,
        ..Spot::default()
    };
    let bowl = Spot {
        radius: (radius * 4 / 5).max(1),
        delta: -(1 << 11),
        ragged: 1,
        ..Spot::default()
    };
    apply_spot(level, at, &rim, regions);
    apply_spot(level, at, &bowl, regions);
}

/// `DestroySpot`: moves the ground inside a circle by a cone, fraying the
/// edge as it goes.
pub fn apply_spot(level: &mut Level, at: (i32, i32), spot: &Spot, regions: &mut Vec<Region>) {
    if spot.radius <= 0 || spot.delta == 0 {
        return;
    }
    // `rfactor` picks how wide the dice are rolled; wider dice mean more
    // texels near the rim are skipped.
    // `rfactor` 4 rolls the dice over the whole radius; each step either
    // side halves or doubles that.
    let dice = match spot.ragged {
        0 => 0,
        n if n < 4 => spot.radius >> (4 - n as i32),
        n => spot.radius << (n as i32 - 4),
    };
    // The original rolls `RND`, so no two blasts in the same spot match.
    // Seeding from the centre instead keeps a level reproducible, which
    // matters more here than it did there.
    let mut rng = Rng::seeded(at, spot.radius as u32);

    let bits = level.terrain_bits();
    let terrain = spot.terrain.map(|t| bits.write(t & bits.mask));
    let mut bounds = Bounds::default();

    for (dx, dy, ring) in rings(spot.radius) {
        // The original shifts the 8.8 down by eight, which rounds towards
        // the floor and so gives a negative delta one more unit than the
        // matching positive one. Dividing keeps the two the same size,
        // which is what lets a lava spot sink back into the ground it came
        // out of without leaving a dent.
        let step = match spot.profile {
            Profile::Cone => (spot.radius - ring) * spot.delta / 256,
            Profile::Dome => {
                // A quarter turn from the middle out, so the sides ease
                // into the rim instead of meeting it at an angle.
                let turn =
                    std::f32::consts::FRAC_PI_2 * (spot.radius - ring) as f32 / spot.radius as f32;
                (spot.delta as f32 * turn.sin()) as i32 / 256
            }
        };
        if step == 0 {
            continue;
        }
        if dice > 0 && rng.below(dice as u32) >= (spot.radius - ring) as u32 {
            continue;
        }
        let (x, y) = (at.0 + dx, at.1 + dy);
        let i = match movable_on(level, x, y, spot.surface) {
            Some(i) => i,
            None => continue,
        };
        if spot.mask & (1 << bits.read(level.meta[i])) == 0 {
            continue;
        }
        level.height[i] = (level.height[i] as i32 + step).clamp(0, 255) as u8;
        if let Some(t) = terrain {
            level.meta[i] = (level.meta[i] & !bits.write(bits.mask)) | t;
        }
        reflood(level, i, x, y);
        bounds.add(x, y);
    }

    bounds.push(regions, level.size);
}

/// Every offset within `radius`, with the ring it falls in.
///
/// `RadiusDestroyX`/`RadiusDestroyY` of the original are this list bucketed
/// by ring and built once at startup. A crater is small enough that walking
/// the square and taking the distance costs less than the tables did.
fn rings(radius: i32) -> impl Iterator<Item = (i32, i32, i32)> {
    (-radius..=radius).flat_map(move |dy| {
        (-radius..=radius).filter_map(move |dx| {
            let ring = ((dx * dx + dy * dy) as f64).sqrt() as i32;
            (ring < radius).then_some((dx, dy, ring))
        })
    })
}

/// A tiny xorshift, so a blast frays the same way every time it is run.
struct Rng(u32);

impl Rng {
    fn seeded(at: (i32, i32), salt: u32) -> Self {
        let mixed = (at.0 as u32)
            .wrapping_mul(0x9E37_79B9)
            .wrapping_add((at.1 as u32).wrapping_mul(0x85EB_CA6B))
            .wrapping_add(salt.wrapping_mul(0xC2B2_AE35));
        Rng(mixed | 1)
    }

    fn below(&mut self, bound: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        if bound == 0 { 0 } else { self.0 % bound }
    }
}

/// Tunables of the mounds a burrowing car throws up.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Molehills {
    pub enabled: bool,
    /// Texels across one mound.
    pub radius: i32,
    /// How high a mound stands on low ground, before the altitude taper.
    pub height: i32,
}

impl Default for Molehills {
    fn default() -> Self {
        Molehills {
            // `aciMoleMounds` style switch in the original, off unless asked
            // for: burrowing carves the ground, which is not wanted by default.
            enabled: false,
            radius: 6,
            height: 12,
        }
    }
}

/// A stretch of burrow, as the line across the car that is under it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Burrow {
    pub left: glam::Vec3,
    pub right: glam::Vec3,
}

/// Throws up the mounds a burrowing car leaves behind it.
///
/// `dastPoly3D::make_mole`. Mounds go up along the line across the car,
/// two or so of eight places along it, and only where the ground is low
/// enough to be pushed aside - high ground swallows the burrow whole.
///
/// The original stamps one of a handful of authored height sprites from
/// `resource/bml/mole.bml`. That file is in no data set this can reach, so
/// the mound is raised with the same cone the blasts use, which is the
/// shape those sprites hold anyway.
pub fn apply_burrow(
    level: &mut Level,
    config: &Molehills,
    burrow: &Burrow,
    regions: &mut Vec<Region>,
) {
    if !config.enabled || config.radius <= 0 || config.height <= 0 {
        return;
    }
    let right = (burrow.right.x.round() as i32, burrow.right.y.round() as i32);
    // `dz` of the original: how high the ground already is where the car
    // went under, which decides how much of a mound gets through.
    let ground = match movable_on(level, right.0, right.1, Surface::Lower) {
        Some(i) => level.height[i] as i32,
        None => return,
    };
    let height = match ground {
        d if d < 50 => config.height,
        d if d < 120 => config.height / 2,
        d if d < 200 => config.height / 4,
        _ => return,
    };
    if height <= 0 {
        return;
    }

    let mut rng = Rng::seeded(right, ground as u32);
    let mut i = 1;
    while i < 8 {
        if rng.below(5) == 0 {
            // Eighths along the line from the right end to the left.
            let t = i as f32 / 8.0;
            let p = burrow.right + (burrow.left - burrow.right) * t;
            let spot = Spot {
                radius: config.radius,
                // A dome `height` tall, which is the shape the sprites the
                // original stamps here hold.
                delta: height << 8,
                profile: Profile::Dome,
                ragged: 2,
                terrain: None,
                mask: DESTRUCTIBLE,
                surface: Surface::Lower,
            };
            apply_spot(
                level,
                (p.x.round() as i32, p.y.round() as i32),
                &spot,
                regions,
            );
            // The original skips the next place after every mound it makes.
            i += 1;
        }
        i += 1;
    }
}

/// Tunables of a car's own weight on the ground.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Press {
    /// Master switch. `aciGroundPressingEnabled` of the original, which is
    /// a setting there too.
    pub enabled: bool,
    /// `ground_pressing_z_offset` - how far above the hull's underside the
    /// ground is allowed to stand before it is pushed down. Slack here
    /// keeps a car from scouring perfectly flat ground it is merely
    /// resting on.
    pub clearance: i32,
}

impl Default for Press {
    fn default() -> Self {
        Press {
            // `aciGroundPressingEnabled` in `moveland.cpp` ships as 0: the
            // original only presses the ground under a car when asked to.
            enabled: false,
            // `ground_pressing_z_offset` in `common.prm`.
            clearance: 5,
        }
    }
}

/// The underside of a car, as the four corners of the box it sits in.
///
/// The original renders the car's model into a small height buffer from
/// above and presses the ground down to that silhouette, wheels included.
/// Here the hull is taken as its bounding box instead: the same footprint
/// and the same height, but square at the corners where the model would
/// be rounded. Getting the true outline back means rasterising the
/// collision mesh, which is a lot of machinery for the difference between
/// a car-shaped hollow and a car-sized one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hull {
    /// Front-left, front-right, back-right, back-left. Anticlockwise or
    /// clockwise, as long as they go round.
    pub corners: [glam::Vec3; 4],
}

/// Presses a car's underside into whatever is standing proud of it.
///
/// `Object::ground_pressing`. Unlike the blade this banks nothing: the
/// ground it pushes down simply goes, which is what the original does.
pub fn apply_press(level: &mut Level, config: &Press, hull: &Hull, regions: &mut Vec<Region>) {
    if !config.enabled {
        return;
    }
    let origin = hull.corners[0];
    let size = level.size;
    let rel = |p: glam::Vec3| {
        glam::Vec3::new(
            wrap_delta_f(p.x - origin.x, size.0),
            wrap_delta_f(p.y - origin.y, size.1),
            p.z + config.clearance as f32,
        )
    };
    let c = hull.corners.map(rel);

    // Steps along each edge, at the same half-texel pitch the blade uses.
    let span = |a: glam::Vec3, b: glam::Vec3| (b - a).truncate().length();
    let across = (span(c[0], c[1]).max(span(c[3], c[2])) * SUB).ceil() as usize;
    let along = (span(c[0], c[3]).max(span(c[1], c[2])) * SUB).ceil() as usize;
    if across == 0 || along == 0 || across > MAX_BLADE_STEPS || along > MAX_BLADE_STEPS {
        return;
    }

    let mut bounds = Bounds::default();
    for i in 0..=along {
        let t = i as f32 / along as f32;
        let (a, b) = (c[0] + (c[3] - c[0]) * t, c[1] + (c[2] - c[1]) * t);
        for j in 0..=across {
            let p = a + (b - a) * (j as f32 / across as f32);
            let (x, y) = (
                (p.x + origin.x).round() as i32,
                (p.y + origin.y).round() as i32,
            );
            if scrape(level, x, y, p.z) > 0.0 {
                bounds.add(x, y);
            }
        }
    }
    bounds.push(regions, level.size);
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
    /// Where the grader blade was at the end of the previous step.
    last_blade: Option<(glam::Vec3, glam::Vec3)>,
    sweeps: Vec<Sweep>,
    /// Where the car's hull is resting, if it is resting on anything.
    hull: Option<Hull>,
    /// Where the car last went under, for the burrow it is digging.
    last_burrow: Option<(i32, i32)>,
    burrows: Vec<Burrow>,
    /// Double-level texels the hull punched through from below.
    ceilings: Vec<(i32, i32)>,
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
        self.raise_blade();
    }

    /// Records where the grader blade is now.
    ///
    /// Like a wheel, the blade needs a previous position before it can
    /// sweep anything, so the first call after a lift only arms it.
    pub fn blade(&mut self, left: glam::Vec3, right: glam::Vec3) {
        if let Some(from) = self.last_blade {
            self.sweeps.push(Sweep {
                from,
                to: (left, right),
            });
        }
        self.last_blade = Some((left, right));
    }

    /// Records that the blade is not cutting - the car is airborne, or
    /// coasting with the motor off.
    pub fn raise_blade(&mut self) {
        self.last_blade = None;
    }

    /// Records where the car's hull is bearing on the ground.
    pub fn press(&mut self, hull: Hull) {
        self.hull = Some(hull);
    }

    /// Hands over the hull, if it was on the ground this step.
    pub fn take_hull(&mut self) -> Option<Hull> {
        self.hull.take()
    }

    /// Records the line across a car that is burrowing.
    ///
    /// `MoleProcessQuant` only throws a mound when the car has moved a
    /// couple of texels since the last one, so a mole sitting still does
    /// not pile the ground up on top of itself.
    pub fn burrow(&mut self, left: glam::Vec3, right: glam::Vec3) {
        let at = (
            ((left.x + right.x) * 0.5).round() as i32,
            ((left.y + right.y) * 0.5).round() as i32,
        );
        let moved = match self.last_burrow {
            None => true,
            Some(from) => {
                let dx = at.0.saturating_sub(from.0);
                let dy = at.1.saturating_sub(from.1);
                dx.saturating_mul(dx).saturating_add(dy.saturating_mul(dy)) > 4
            }
        };
        if moved {
            self.last_burrow = Some(at);
            self.burrows.push(Burrow { left, right });
        }
    }

    /// Records that the car is no longer underground.
    pub fn surface(&mut self) {
        self.last_burrow = None;
    }

    /// Hands over the burrow stretches recorded since the last drain.
    pub fn drain_burrows(&mut self) -> std::vec::Drain<'_, Burrow> {
        self.burrows.drain(..)
    }

    /// Original `destroy_double_level`: a fast hit on a cave roof.
    pub fn smash_ceiling(&mut self, at: (i32, i32)) {
        self.ceilings.push(at);
    }

    pub fn drain_ceilings(&mut self) -> std::vec::Drain<'_, (i32, i32)> {
        self.ceilings.drain(..)
    }

    /// Forgets every wheel's contact and any track not yet drawn, so that
    /// the next contact starts fresh. Needed whenever the car is moved
    /// rather than driven.
    pub fn reset(&mut self) {
        self.lift_all();
        self.raise_blade();
        self.pending.clear();
        self.sweeps.clear();
        self.hull = None;
        self.last_burrow = None;
        self.burrows.clear();
        self.ceilings.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
            && self.sweeps.is_empty()
            && self.hull.is_none()
            && self.burrows.is_empty()
            && self.ceilings.is_empty()
    }

    /// Hands over the stretches recorded since the last drain.
    pub fn drain(&mut self) -> std::vec::Drain<'_, Track> {
        self.pending.drain(..)
    }

    /// Hands over the blade sweeps recorded since the last drain.
    pub fn drain_sweeps(&mut self) -> std::vec::Drain<'_, Sweep> {
        self.sweeps.drain(..)
    }
}

/// Apply ceiling smashes and, if enabled, the blade/tread/press/mole
/// edits recorded on `tracks`. Returns the wheel stretches so a caller
/// can spawn dust from them.
pub fn apply_vehicle(
    level: &mut Level,
    config: &Config,
    tracks: &mut Tracks,
    smash_radius: i32,
    regions: &mut Vec<Region>,
) -> Vec<Track> {
    for at in tracks.drain_ceilings() {
        smash_ceiling(level, at, smash_radius, regions);
    }
    let editing = config.tread.enabled
        || config.grader.enabled
        || config.press.enabled
        || config.molehills.enabled;
    if !editing {
        let treads: Vec<Track> = tracks.drain().collect();
        tracks.reset();
        return treads;
    }
    for burrow in tracks.drain_burrows() {
        apply_burrow(level, &config.molehills, &burrow, regions);
    }
    if let Some(hull) = tracks.take_hull() {
        apply_press(level, &config.press, &hull, regions);
    }
    for sweep in tracks.drain_sweeps() {
        apply_grader(level, &config.grader, &sweep, regions);
    }
    let mut treads = Vec::new();
    for track in tracks.drain() {
        apply_tread(level, &config.tread, &track, regions);
        treads.push(track);
    }
    treads
}

/// Cuts `track` into the level, pushing what it touched onto `regions`.
pub fn apply_tread(level: &mut Level, config: &Tread, track: &Track, regions: &mut Vec<Region>) {
    if !config.enabled || config.depth == 0 || config.period == 0 || config.bar == 0 {
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
    let mut mask = config.period;
    for i in 0..steps {
        let x = track.from.0 + (fx * i as f32).round() as i32;
        let y = track.from.1 + (fy * i as f32).round() as i32;
        let delta = if mask == 0 {
            mask = config.period;
            config.depth
        } else {
            -config.depth
        };
        mask -= 1;

        // The bar is centred on the wheel, so the track sits under the tyre
        // rather than off its trailing side.
        let mid = (config.bar as f32 - 1.0) * 0.5;
        for k in 0..config.bar as i32 {
            let reach = config.spacing * (k as f32 - mid);
            let bx = x + (track.across.0 * reach).round() as i32;
            let by = y + (track.across.1 * reach).round() as i32;
            if press(level, bx, by, delta) {
                bounds.add(bx, by);
            }
        }
    }

    bounds.push(regions, level.size);
}

/// Tunables of the grader blade.
///
/// Off by default. The original ships with `make_dast` commented out, so a
/// stock game does not bulldoze itself, and turning every car into a
/// digger is a large enough change to gameplay to be asked for rather than
/// assumed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Grader {
    /// Master switch.
    pub enabled: bool,
    /// How far along the blade spoil creeps each step, in slots. `_boder_`
    /// of the original is 8, and it spreads over twice that either way.
    pub spread: u8,
    /// Altitude units one slot of the blade may drop in one step. Spoil
    /// above this stays banked, which is what makes the blade carry a load
    /// instead of putting it straight back down.
    pub lift: i32,
    /// Texels the leftover berm reaches ahead of the blade, per unit of
    /// its height. The original ramps down over `8 * height` steps of half
    /// a texel each.
    pub reach: u8,
}

impl Default for Grader {
    fn default() -> Self {
        Grader {
            enabled: false,
            spread: 8,
            lift: 20,
            reach: 4,
        }
    }
}

/// One step of the blade: where it was, and where it is now.
///
/// The two lines bound the quad the blade swept, which is `p_array` of the
/// original. Taking the previous line rather than `velocity * dt` costs one
/// `Vec3` of state and gets the corners right when the car is turning, and
/// leaves no gap between one step and the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    /// Left and right ends of the blade at the end of the previous step.
    pub from: (glam::Vec3, glam::Vec3),
    /// Left and right ends of it now.
    pub to: (glam::Vec3, glam::Vec3),
}

/// Sub-steps per texel along the blade and along the sweep. The original
/// walks both at `2`, so that a blade lying at 45 degrees still touches
/// every texel it crosses.
const SUB: f32 = 2.0;

/// Ceilings on that sub-stepping, so that an enormous car or a bad sweep
/// cannot turn one quant into an unbounded amount of work.
const MAX_BLADE_STEPS: usize = 512;
const MAX_SWEEP_STEPS: usize = 64;

/// Drives the blade through `sweep`, pushing what it touched onto
/// `regions`.
///
/// `make_dast` of the original. Each step across the swept quad does three
/// things in turn: spread the spoil the blade is already carrying along its
/// length, drop as much of it as [`Grader::lift`] allows, then scrape off
/// whatever ground still stands above the blade and add it to the load.
/// What is left when the blade stops becomes a berm in front of it.
pub fn apply_grader(level: &mut Level, config: &Grader, sweep: &Sweep, regions: &mut Vec<Region>) {
    if !config.enabled || config.lift <= 0 {
        return;
    }

    // Everything is measured from one corner, so the level seam is crossed
    // once here rather than at every texel.
    let origin = sweep.from.1;
    let size = level.size;
    let rel = |p: glam::Vec3| {
        glam::Vec3::new(
            wrap_delta_f(p.x - origin.x, size.0),
            wrap_delta_f(p.y - origin.y, size.1),
            p.z,
        )
    };
    let (r0, l0) = (rel(sweep.from.1), rel(sweep.from.0));
    let (r1, l1) = (rel(sweep.to.1), rel(sweep.to.0));

    let span = |a: glam::Vec3, b: glam::Vec3| (b - a).truncate().length();
    let blade = (span(r0, l0).max(span(r1, l1)) * SUB).ceil() as usize;
    let travel = (span(r0, r1).max(span(l0, l1)) * SUB).ceil() as usize;
    if blade == 0 || blade > MAX_BLADE_STEPS || travel > MAX_SWEEP_STEPS {
        return;
    }

    // Spoil creeps sideways as the blade carries it, so the slots it can
    // reach run past both ends of the blade itself.
    let fringe = 2 * config.spread as usize;
    let slots = blade + 1 + 2 * fringe;
    let mut load = vec![0.0f32; slots];
    let mut next = vec![0.0f32; slots];
    // Slot `fringe` is the blade's right end, slot `fringe + blade` its left.
    let at = |a: glam::Vec3, b: glam::Vec3, slot: usize| {
        let t = (slot as f32 - fringe as f32) / blade as f32;
        a + (b - a) * t
    };

    // The interpolation runs in the frame `rel` set up; only the level is
    // addressed in absolute texels.
    let texel = |p: glam::Vec3| {
        (
            (p.x + origin.x).round() as i32,
            (p.y + origin.y).round() as i32,
        )
    };

    let mut bounds = Bounds::default();
    let put = |level: &mut Level, p: glam::Vec3, amount: f32, bounds: &mut Bounds| {
        let (x, y) = texel(p);
        let stuck = heap(level, x, y, amount, p.z);
        if stuck > 0.0 {
            bounds.add(x, y);
        }
        stuck
    };

    for step in 0..=travel {
        let t = if travel == 0 {
            1.0
        } else {
            step as f32 / travel as f32
        };
        let (a, b) = (r0 + (r1 - r0) * t, l0 + (l1 - l0) * t);

        spread(&load, &mut next, config.spread as usize);
        std::mem::swap(&mut load, &mut next);

        for (slot, carried) in load.iter_mut().enumerate() {
            let want = carried.min(config.lift as f32);
            if want < 1.0 {
                continue;
            }
            *carried -= put(level, at(a, b, slot), want, &mut bounds);
        }

        for (slot, carried) in load.iter_mut().enumerate().skip(fringe).take(blade + 1) {
            let p = at(a, b, slot);
            let (x, y) = texel(p);
            let spoil = scrape(level, x, y, p.z);
            if spoil > 0.0 {
                bounds.add(x, y);
                *carried += spoil;
            }
        }
    }

    berm(
        level,
        config,
        &load,
        &(r1, l1),
        origin,
        fringe,
        blade,
        &mut bounds,
    );
    bounds.push(regions, level.size);
}

/// Heaps whatever the blade is still carrying into a ridge in front of it.
///
/// The original takes the cube root of each slot's load for the height of
/// the pile, which keeps a big load from turning into a spike, and ramps it
/// down over `8 * height` steps ahead. Both are kept; what is dropped is
/// its habit of adding to the same texel several times over, which made the
/// berm's height depend on the angle the blade happened to be travelling.
#[allow(clippy::too_many_arguments)]
fn berm(
    level: &mut Level,
    config: &Grader,
    load: &[f32],
    blade_line: &(glam::Vec3, glam::Vec3),
    origin: glam::Vec3,
    fringe: usize,
    blade: usize,
    bounds: &mut Bounds,
) {
    let (a, b) = *blade_line;
    let ahead = {
        let dir = glam::Vec2::new(b.y - a.y, a.x - b.x);
        dir.normalize_or_zero()
    };
    if ahead == glam::Vec2::ZERO {
        return;
    }

    for (slot, &amount) in load.iter().enumerate() {
        let height = amount.cbrt().floor();
        if height < 1.0 {
            continue;
        }
        let t = (slot as f32 - fringe as f32) / blade as f32;
        let line = a + (b - a) * t;
        let base = line.truncate() + origin.truncate();
        let base_z = line.z;
        // One texel per step, tapering to nothing over `reach * height`.
        let steps = (config.reach as f32 * height) as i32;
        let (mut last_x, mut last_y) = (i32::MIN, i32::MIN);
        for k in 0..=steps {
            let p = base + ahead * k as f32;
            let (x, y) = (p.x.round() as i32, p.y.round() as i32);
            if (x, y) == (last_x, last_y) {
                continue;
            }
            last_x = x;
            last_y = y;
            let taper = height * (1.0 - k as f32 / (steps + 1) as f32);
            if heap(level, x, y, taper, base_z) > 0.0 {
                bounds.add(x, y);
            }
        }
    }
}

/// Smears each slot's load along the blade: a quarter stays put and the
/// rest goes evenly to the `2 * width` slots either side, which is the
/// kernel `make_dast` applies between its steps.
///
/// Written as a sliding window rather than the original's `2 * width + 1`
/// taps per slot, because the blade of a large car is hundreds of slots
/// long and this runs every step of every sweep of every car.
fn spread(load: &[f32], out: &mut [f32], width: usize) {
    let reach = 2 * width;
    let share = 0.75 / (2 * reach) as f32;
    let mut window: f32 = load.iter().take(reach.min(load.len())).sum();
    for i in 0..load.len() {
        // `window` holds `load[i - reach ..= i + reach]`.
        if let Some(&v) = load.get(i + reach) {
            window += v;
        }
        out[i] = 0.25 * load[i] + share * (window - load[i]);
        if i >= reach {
            window -= load[i - reach];
        }
    }
}

/// Shortest signed distance across a level that wraps, for a position
/// rather than a texel.
fn wrap_delta_f(d: f32, total: i32) -> f32 {
    let total = total as f32;
    let d = d.rem_euclid(total);
    if d * 2.0 > total { d - total } else { d }
}

/// The index whose altitude a wheel or a blade is allowed to move at
/// `(x, y)`, if there is one.
///
/// Only the top surface is drivable, so that is the only one either of them
/// can reshape. On a double-level pair the altitude that moves lives in the
/// odd half, which is why the even one is left alone rather than written
/// twice - the same rule `pixSetR` opens with.
fn movable(level: &Level, x: i32, y: i32) -> Option<usize> {
    movable_on(level, x, y, Surface::Upper)
}

/// Which surface of a double-level pair an edit works on.
///
/// `get_up_ground` and `get_down_ground` of the original. Everything a car
/// does on top of the world moves the upper one; a mole burrowing under a
/// slab moves the floor it is tunnelling through.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Surface {
    #[default]
    Upper,
    Lower,
}

fn movable_on(level: &Level, x: i32, y: i32, surface: Surface) -> Option<usize> {
    let i = level.wrap((x, y));
    let i = if level.meta[i] & DOUBLE_LEVEL != 0 {
        // A pair is addressed by one of its halves: the odd one carries
        // the altitude above the slab, the even one the floor below it.
        // Either way the other half is the same surface seen twice, so it
        // is skipped rather than written again.
        match surface {
            Surface::Upper if x & 1 == 0 => return None,
            Surface::Lower if x & 1 != 0 => return None,
            Surface::Upper => i,
            Surface::Lower => i & !1,
        }
    } else {
        i
    };
    if level.terrain_bits().read(level.meta[i]) != MAIN_TERRAIN {
        return None;
    }
    Some(i)
}

/// `pixSetR` of the original: moves the upper surface of one texel by
/// `delta`, and returns whether it wrote anything.
fn press(level: &mut Level, x: i32, y: i32, delta: i32) -> bool {
    let i = match movable(level, x, y) {
        Some(i) => i,
        None => return false,
    };

    let height = (level.height[i] as i32 + delta).clamp(0, 255);
    if level.meta[i] & DOUBLE_LEVEL != 0 && collapses(level, i, height) {
        collapse(level, i, x, y);
        return true;
    }
    level.height[i] = height as u8;

    reflood(level, i, x, y);
    true
}

/// Scrapes one texel down to `floor`, and returns the spoil that came off.
///
/// The blade only reaches what stands proud of it, so a texel already at or
/// below `floor` gives nothing back. A roof cut too thin comes down, and
/// then the spoil is whatever the collapse settled away.
fn scrape(level: &mut Level, x: i32, y: i32, floor: f32) -> f32 {
    let i = match movable(level, x, y) {
        Some(i) => i,
        None => return 0.0,
    };
    let dual = level.meta[i] & DOUBLE_LEVEL != 0;
    if dual {
        // Under the slab rather than on top of it: the blade is inside the
        // cave, and its roof is not the surface being graded.
        let ceiling = ceiling_of(level, i) as f32;
        if floor < ceiling {
            return 0.0;
        }
    }

    let was = level.height[i] as i32;
    let cut = floor.floor().clamp(0.0, 255.0) as i32;
    if cut >= was {
        return 0.0;
    }
    if dual && collapses(level, i, cut) {
        collapse(level, i, x, y);
        return (was - level.height[i] as i32).max(0) as f32;
    }
    level.height[i] = cut as u8;
    reflood(level, i, x, y);
    (was - cut) as f32
}

/// Heaps `amount` of spoil onto one texel, and returns how much of it
/// stuck. A texel that has reached the top of the level takes no more, and
/// the rest stays banked rather than vanishing.
///
/// `at` is the height of the blade dropping it, which decides whether the
/// slab overhead is something to pile against or something to pile on top
/// of - spoil dropped inside a cave must not land on its roof.
fn heap(level: &mut Level, x: i32, y: i32, amount: f32, at: f32) -> f32 {
    if amount < 1.0 {
        return 0.0;
    }
    let i = match movable(level, x, y) {
        Some(i) => i,
        None => return 0.0,
    };
    if level.meta[i] & DOUBLE_LEVEL != 0 && at < ceiling_of(level, i) as f32 {
        return 0.0;
    }
    let was = level.height[i] as i32;
    let put = (amount as i32).min(255 - was);
    if put <= 0 {
        return 0.0;
    }
    level.height[i] = (was + put) as u8;
    reflood(level, i, x, y);
    put as f32
}

/// The altitude of a cave's ceiling, from the delta bits of both halves of
/// the pair.
fn ceiling_of(level: &Level, i: usize) -> i32 {
    let (lo, hi) = (i & !1, i | 1);
    let delta = ((level.meta[lo] & DELTA_MASK) << DELTA_BITS) | (level.meta[hi] & DELTA_MASK);
    level.height[lo] as i32 + ((delta as i32) << level.geometry.delta_power as u32)
}

/// Whether cutting the roof of a cave down to `height` breaks through it.
///
/// The roof is everything between the ceiling - `low` plus the pair's delta
/// bits - and the top. The original allows itself one delta step of margin
/// before it gives up on the slab, so a roof does not thin out to nothing
/// first.
fn collapses(level: &Level, i: usize, height: i32) -> bool {
    ceiling_of(level, i) + (1 << level.geometry.delta_power as u32) >= height
}

/// `Object::destroy_double_level`: collapse dual-level texels around `at`.
pub fn smash_ceiling(level: &mut Level, at: (i32, i32), radius: i32, regions: &mut Vec<Region>) {
    let mut bounds = Bounds::default();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            if dx * dx + dy * dy > radius * radius {
                continue;
            }
            let (x, y) = (at.0 + dx, at.1 + dy);
            let i = level.wrap((x, y));
            if level.meta[i] & DOUBLE_LEVEL == 0 {
                continue;
            }
            collapse(level, i, x, y);
            bounds.add(x, y);
        }
    }
    bounds.push(regions, level.size);
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
        test_level_of(SIZE)
    }

    fn test_level_of(size: i32) -> Level {
        let total = (size * size) as usize;
        let bits = TerrainBits::new(8);
        Level {
            size: (size, size),
            flood_map: vec![0; size as usize].into_boxed_slice(),
            height: vec![100u8; total].into_boxed_slice(),
            meta: vec![bits.write(MAIN_TERRAIN); total].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    /// A straight track along +X with no across-track spread, so that each
    /// step marks exactly one texel and the pattern is easy to read off.
    fn straight(level: &mut Level, len: i32, config: &Tread) -> Vec<Region> {
        let mut regions = Vec::new();
        apply_tread(
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

    fn bar_only() -> Tread {
        Tread {
            bar: 1,
            ..Tread::default()
        }
    }

    /// One texel per stamp, so a bar's extent is easy to read off.
    fn tight() -> Tread {
        Tread {
            spacing: 1.0,
            ..Tread::default()
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
        apply_tread(
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
        for k in -1..=1 {
            assert_eq!(at(10, 10 + k), 99, "the whole bar is stamped");
        }
        assert_eq!(at(10, 13), 100, "and no further to the +y side");
        assert_eq!(at(10, 8), 100, "nor to the -y side of the wheel");
    }

    #[test]
    fn a_track_reports_what_it_touched() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_tread(
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
                y: 9,
                w: 4,
                h: 3
            }]
        );
    }

    #[test]
    fn a_track_across_the_seam_is_split_rather_than_stretched() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_tread(
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
        apply_tread(
            &mut level,
            &Tread::default(),
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
        apply_tread(
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
        apply_tread(
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
    fn smashing_a_cave_roof_opens_the_room() {
        let mut level = test_level();
        dual_level(&mut level, 10, 5, 40, 200, 4);
        let mut regions = Vec::new();
        smash_ceiling(&mut level, (10, 5), 3, &mut regions);
        let i = level.wrap((10, 5));
        assert_eq!(level.meta[i] & DOUBLE_LEVEL, 0, "the slab is still there");
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
        apply_tread(
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
        apply_tread(
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
        apply_tread(
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
        let config = Tread {
            enabled: false,
            ..Tread::default()
        };
        let regions = straight(&mut level, 9, &config);
        assert!(regions.is_empty());
        assert!(level.height.iter().all(|&h| h == 100));
    }

    // -- the grader ------------------------------------------------------

    fn grader() -> Grader {
        Grader {
            enabled: true,
            ..Grader::default()
        }
    }

    /// A blade lying across x, swept along +y from `y0` to `y1` at height
    /// `z`, spanning `x0..=x1`.
    fn blade(x0: f32, x1: f32, y0: f32, y1: f32, z: f32) -> Sweep {
        use glam::Vec3;
        Sweep {
            from: (Vec3::new(x0, y0, z), Vec3::new(x1, y0, z)),
            to: (Vec3::new(x0, y1, z), Vec3::new(x1, y1, z)),
        }
    }

    fn total_altitude(level: &Level) -> i64 {
        level.height.iter().map(|&h| h as i64).sum()
    }

    #[test]
    fn the_blade_leaves_a_flat_floor_at_its_own_height() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 40.0, 20.0, 30.0, 60.0),
            &mut regions,
        );
        // Every row the blade crossed, bar the last, which carries the berm.
        for y in 20..30 {
            for x in 20..=40 {
                assert_eq!(
                    level.height[level.wrap((x, y))],
                    60,
                    "({}, {}) is not level with the blade",
                    x,
                    y
                );
            }
        }
        assert!(!regions.is_empty());
    }

    #[test]
    fn the_spoil_is_thrown_into_windrows_along_the_cut() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 40.0, 20.0, 30.0, 60.0),
            &mut regions,
        );
        let at = |x: i32, y: i32| level.height[level.wrap((x, y))] as i32;
        for side in [19, 41] {
            assert!(
                at(side, 25) > 100,
                "no windrow beside the cut at x = {}: {}",
                side,
                at(side, 25)
            );
        }
        // The blade keeps sweeping spoil outwards, so the windrow is taller
        // where the blade had been carrying a load for longer.
        assert!(
            at(19, 29) > at(19, 21),
            "the windrow should build up along the sweep"
        );
    }

    #[test]
    fn the_blade_leaves_ground_that_is_already_below_it_alone() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 30.0, 20.0, 26.0, 200.0),
            &mut regions,
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn the_spoil_goes_somewhere_rather_than_nowhere() {
        let mut level = test_level();
        let before = total_altitude(&level);
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 40.0, 20.0, 30.0, 60.0),
            &mut regions,
        );
        let after = total_altitude(&level);
        let cut = (40 - 20 + 1) as i64 * (30 - 20 + 1) as i64 * (100 - 60);
        assert!(after < before, "the blade has to remove ground");
        assert!(
            before - after < cut,
            "and put most of it back: cut {}, lost {}",
            cut,
            before - after
        );
    }

    #[test]
    fn the_spoil_is_heaped_ahead_of_the_blade() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 40.0, 20.0, 30.0, 60.0),
            &mut regions,
        );
        let raised = (0..SIZE)
            .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
            .filter(|&(x, y)| level.height[level.wrap((x, y))] > 100)
            .collect::<Vec<_>>();
        assert!(!raised.is_empty(), "nothing was heaped up at all");
        // The blade started at y = 20 and stopped at y = 30. It drops spoil
        // all along the way, but never behind where it set off, and what it
        // is still carrying at the end goes out in front.
        for &(x, y) in raised.iter() {
            assert!(y >= 20, "({}, {}) was raised behind the start", x, y);
        }
        assert!(
            raised.iter().any(|&(_, y)| y > 30),
            "nothing was left ahead of the blade"
        );
    }

    #[test]
    fn a_blade_that_never_moved_still_cuts_its_own_line() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 30.0, 20.0, 20.0, 80.0),
            &mut regions,
        );
        // Cut to the blade, with the berm it was left holding on top.
        assert!(level.height[level.wrap((25, 20))] < 100);
        assert!(!regions.is_empty());
    }

    #[test]
    fn only_the_main_terrain_is_graded() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        for x in 25..=30 {
            for y in 20..=26 {
                level.meta[level.wrap((x, y))] = bits.write(4);
            }
        }
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 30.0, 20.0, 26.0, 80.0),
            &mut regions,
        );
        assert_eq!(level.height[level.wrap((22, 22))], 80, "the soft half");
        assert_eq!(level.height[level.wrap((27, 22))], 100, "and the hard one");
    }

    #[test]
    fn the_blade_does_not_reach_the_roof_of_a_cave_it_is_driving_through() {
        let mut level = test_level();
        // A tall slab with a cave under it, and the blade down in the cave.
        dual_level(&mut level, 24, 22, 40, 200, 3);
        let i = level.wrap((25, 22));
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 30.0, 20.0, 26.0, 60.0),
            &mut regions,
        );
        assert_eq!(level.height[i], 200, "the roof is not the blade's to cut");
        assert_ne!(level.meta[i] & DOUBLE_LEVEL, 0);
    }

    #[test]
    fn an_absurdly_long_blade_is_refused() {
        let mut level = test_level_of(1024);
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 420.0, 20.0, 26.0, 60.0),
            &mut regions,
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn a_teleport_is_not_a_sweep() {
        let mut level = test_level_of(1024);
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &grader(),
            &blade(20.0, 30.0, 20.0, 300.0, 60.0),
            &mut regions,
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn the_grader_is_off_unless_it_is_asked_for() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        apply_grader(
            &mut level,
            &Grader::default(),
            &blade(20.0, 30.0, 20.0, 26.0, 60.0),
            &mut regions,
        );
        assert!(!Grader::default().enabled);
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn spreading_a_load_neither_creates_nor_destroys_it() {
        let mut load = vec![0.0f32; 80];
        load[40] = 1000.0;
        let mut out = vec![0.0f32; 80];
        spread(&load, &mut out, 8);
        let total: f32 = out.iter().sum();
        assert!(
            (total - 1000.0).abs() < 1.0,
            "the kernel has to conserve: {}",
            total
        );
        assert!(out[40] > out[45], "and keep most of it near where it was");
        assert_eq!(out[10], 0.0, "without reaching past its width");
    }

    #[test]
    fn spreading_matches_the_kernel_it_replaces() {
        // The sliding window has to agree with the original's per-slot taps.
        let width = 3usize;
        let reach = 2 * width;
        let load = (0..40).map(|i| (i * 7 % 13) as f32).collect::<Vec<_>>();
        let mut plain = vec![0.0f32; load.len()];
        let share = 0.75 / (2 * reach) as f32;
        for (i, &v) in load.iter().enumerate() {
            plain[i] += 0.25 * v;
            for k in 1..=reach {
                if i + k < load.len() {
                    plain[i + k] += share * v;
                }
                if i >= k {
                    plain[i - k] += share * v;
                }
            }
        }
        let mut fast = vec![0.0f32; load.len()];
        spread(&load, &mut fast, width);
        for (a, b) in plain.iter().zip(&fast) {
            assert!((a - b).abs() < 1e-3, "{} vs {}", a, b);
        }
    }

    // -- the hull -------------------------------------------------------

    fn press() -> Press {
        Press {
            enabled: true,
            clearance: 0,
        }
    }

    fn molehills() -> Molehills {
        Molehills {
            enabled: true,
            ..Molehills::default()
        }
    }

    /// A hull covering `x0..=x1` by `y0..=y1`, with its underside at `z`.
    fn hull(x0: f32, x1: f32, y0: f32, y1: f32, z: f32) -> Hull {
        use glam::Vec3;
        Hull {
            corners: [
                Vec3::new(x0, y0, z),
                Vec3::new(x1, y0, z),
                Vec3::new(x1, y1, z),
                Vec3::new(x0, y1, z),
            ],
        }
    }

    #[test]
    fn the_hull_flattens_what_stands_proud_of_it() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_press(
            &mut level,
            &press(),
            &hull(20.0, 30.0, 20.0, 26.0, 80.0),
            &mut regions,
        );
        for y in 20..=26 {
            for x in 20..=30 {
                assert_eq!(
                    level.height[level.wrap((x, y))],
                    80,
                    "({}, {}) still stands above the hull",
                    x,
                    y
                );
            }
        }
        assert!(!regions.is_empty());
    }

    #[test]
    fn the_hull_does_not_touch_ground_it_is_resting_on() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        // Sitting exactly on the plain, and then above it.
        apply_press(
            &mut level,
            &press(),
            &hull(20.0, 30.0, 20.0, 26.0, 100.0),
            &mut regions,
        );
        apply_press(
            &mut level,
            &press(),
            &hull(20.0, 30.0, 20.0, 26.0, 140.0),
            &mut regions,
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn clearance_is_slack_the_ground_is_allowed() {
        let mut level = test_level();
        let mut regions = Vec::new();
        let config = Press {
            enabled: true,
            clearance: 10,
        };
        apply_press(
            &mut level,
            &config,
            &hull(20.0, 30.0, 20.0, 26.0, 80.0),
            &mut regions,
        );
        assert_eq!(
            level.height[level.wrap((25, 23))],
            90,
            "pressed to the hull plus its clearance, not through it"
        );
    }

    #[test]
    fn the_hull_leaves_the_ground_beside_it_alone() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_press(
            &mut level,
            &press(),
            &hull(20.0, 30.0, 20.0, 26.0, 80.0),
            &mut regions,
        );
        assert_eq!(level.height[level.wrap((18, 23))], 100);
        assert_eq!(level.height[level.wrap((32, 23))], 100);
        assert_eq!(level.height[level.wrap((25, 18))], 100);
        assert_eq!(level.height[level.wrap((25, 28))], 100);
    }

    #[test]
    fn a_tilted_hull_leaves_a_tilted_hollow() {
        use glam::Vec3;
        let mut level = test_level();
        let mut regions = Vec::new();
        // Nose down: the far edge digs in, the near edge barely does.
        let tilted = Hull {
            corners: [
                Vec3::new(20.0, 20.0, 60.0),
                Vec3::new(30.0, 20.0, 60.0),
                Vec3::new(30.0, 30.0, 98.0),
                Vec3::new(20.0, 30.0, 98.0),
            ],
        };
        apply_press(&mut level, &press(), &tilted, &mut regions);
        let at = |x: i32, y: i32| level.height[level.wrap((x, y))] as i32;
        assert_eq!(at(25, 20), 60, "the buried edge");
        assert!(at(25, 25) > 60 && at(25, 25) < 98, "and a ramp between");
        assert!(at(25, 29) > at(25, 25), "rising towards the raised edge");
    }

    #[test]
    fn only_the_main_terrain_takes_a_hull_print() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        for x in 25..=30 {
            for y in 20..=26 {
                level.meta[level.wrap((x, y))] = bits.write(4);
            }
        }
        let mut regions = Vec::new();
        apply_press(
            &mut level,
            &press(),
            &hull(20.0, 30.0, 20.0, 26.0, 80.0),
            &mut regions,
        );
        assert_eq!(level.height[level.wrap((22, 22))], 80);
        assert_eq!(
            level.height[level.wrap((27, 22))],
            100,
            "the hard half holds"
        );
    }

    #[test]
    fn switching_the_press_off_leaves_the_ground_alone() {
        let mut level = test_level();
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        let config = Press {
            enabled: false,
            ..press()
        };
        apply_press(
            &mut level,
            &config,
            &hull(20.0, 30.0, 20.0, 26.0, 60.0),
            &mut regions,
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn a_hull_across_the_seam_still_prints() {
        let mut level = test_level();
        let mut regions = Vec::new();
        apply_press(
            &mut level,
            &press(),
            &hull(SIZE as f32 - 4.0, SIZE as f32 + 4.0, 20.0, 24.0, 80.0),
            &mut regions,
        );
        assert_eq!(level.height[level.wrap((SIZE - 2, 22))], 80);
        assert_eq!(level.height[level.wrap((2, 22))], 80, "and past the seam");
        assert_eq!(
            regions.len(),
            2,
            "reported as two rectangles, not one wide one"
        );
    }

    // -- craters --------------------------------------------------------

    #[test]
    fn a_crater_is_a_bowl_inside_a_rim() {
        let mut level = test_level();
        let mut regions = Vec::new();
        crater(&mut level, (30, 30), 12, &mut regions);
        let at = |x: i32, y: i32| level.height[level.wrap((x, y))] as i32;

        assert!(
            at(30, 30) < 100 - 40,
            "the middle is dug right out: {}",
            at(30, 30)
        );
        // Somewhere between the bowl's edge and the rim's, the ground has
        // been thrown up rather than taken away.
        let raised = (10..=12).any(|r| at(30 + r, 30) > 100);
        assert!(raised, "no rim was thrown up");
        assert_eq!(at(30, 30 + 13), 100, "and nothing outside the radius");
        assert!(!regions.is_empty());
    }

    #[test]
    fn a_spot_is_deepest_in_the_middle_and_nothing_at_the_edge() {
        let mut level = test_level();
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 16,
            delta: -(1 << 8),
            ragged: 0,
            ..Spot::default()
        };
        apply_spot(&mut level, (30, 30), &spot, &mut regions);
        let at = |d: i32| level.height[level.wrap((30 + d, 30))] as i32;
        assert_eq!(at(0), 100 - 16, "a unit per texel in, sixteen deep");
        assert_eq!(at(8), 100 - 8);
        assert_eq!(at(15), 100 - 1);
        assert_eq!(at(16), 100, "the rim itself is untouched");
    }

    #[test]
    fn a_clean_spot_touches_every_texel_inside_it() {
        let mut level = test_level();
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 10,
            delta: -(1 << 8),
            ragged: 0,
            ..Spot::default()
        };
        apply_spot(&mut level, (30, 30), &spot, &mut regions);
        for dy in -9..=9i32 {
            for dx in -9..=9i32 {
                if ((dx * dx + dy * dy) as f64).sqrt() as i32 >= 10 {
                    continue;
                }
                assert!(
                    level.height[level.wrap((30 + dx, 30 + dy))] < 100,
                    "({}, {}) was skipped by a clean spot",
                    dx,
                    dy
                );
            }
        }
    }

    #[test]
    fn a_ragged_spot_frays_at_the_edge_but_not_at_the_middle() {
        let mut level = test_level();
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 20,
            delta: -(1 << 8),
            ragged: 4,
            ..Spot::default()
        };
        apply_spot(&mut level, (40, 40), &spot, &mut regions);
        let touched = |lo: i32, hi: i32| {
            let mut hit = 0;
            let mut total = 0;
            for dy in -20..=20i32 {
                for dx in -20..=20i32 {
                    let r = ((dx * dx + dy * dy) as f64).sqrt() as i32;
                    if r < lo || r >= hi {
                        continue;
                    }
                    total += 1;
                    if level.height[level.wrap((40 + dx, 40 + dy))] != 100 {
                        hit += 1;
                    }
                }
            }
            hit as f32 / total as f32
        };
        // The dice are rolled over the whole radius, so a texel `r` rings
        // out is skipped with probability `r / radius`: certain at the
        // centre, thinning steadily, mostly gone by the rim.
        assert_eq!(touched(0, 1), 1.0, "the centre is never skipped");
        assert!(touched(0, 5) > 0.8, "and the middle is nearly solid");
        assert!(touched(16, 20) < 0.3, "while the edge is mostly gone");
    }

    #[test]
    fn a_blast_frays_the_same_way_every_time() {
        let shape = |seed: (i32, i32)| {
            let mut level = test_level();
            let mut regions = Vec::new();
            let spot = Spot {
                radius: 14,
                delta: -(1 << 8),
                ragged: 4,
                ..Spot::default()
            };
            apply_spot(&mut level, seed, &spot, &mut regions);
            level.height.to_vec()
        };
        assert_eq!(shape((30, 30)), shape((30, 30)), "same spot, same crater");
        assert_ne!(
            shape((30, 30)),
            shape((31, 30)),
            "but a different spot frays differently"
        );
    }

    #[test]
    fn a_blast_leaves_the_terrains_it_cannot_move_alone() {
        let mut level = test_level();
        let bits = level.terrain_bits();
        // Terrain 5 is not in the destructible set.
        for x in 30..50 {
            for y in 20..50 {
                level.meta[level.wrap((x, y))] = bits.write(5);
            }
        }
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 12,
            delta: -(1 << 8),
            ragged: 0,
            ..Spot::default()
        };
        apply_spot(&mut level, (30, 30), &spot, &mut regions);
        assert!(
            level.height[level.wrap((25, 30))] < 100,
            "the soft side went"
        );
        assert_eq!(
            level.height[level.wrap((35, 30))],
            100,
            "the hard side held"
        );
    }

    #[test]
    fn a_blast_can_scorch_the_ground_it_moves() {
        let mut level = test_level();
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 8,
            delta: -(1 << 8),
            ragged: 0,
            terrain: Some(3),
            ..Spot::default()
        };
        apply_spot(&mut level, (30, 30), &spot, &mut regions);
        let bits = level.terrain_bits();
        assert_eq!(bits.read(level.meta[level.wrap((30, 30))]), 3);
        assert_eq!(
            bits.read(level.meta[level.wrap((30, 40))]),
            MAIN_TERRAIN,
            "and only inside itself"
        );
    }

    #[test]
    fn a_blast_never_digs_through_the_floor() {
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 3);
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 10,
            delta: -(1 << 11),
            ragged: 0,
            ..Spot::default()
        };
        apply_spot(&mut level, (30, 30), &spot, &mut regions);
        assert_eq!(level.height[level.wrap((30, 30))], 0);
    }

    #[test]
    fn a_blast_across_the_seam_is_reported_in_pieces() {
        let mut level = test_level();
        let mut regions = Vec::new();
        let spot = Spot {
            radius: 6,
            delta: -(1 << 8),
            ragged: 0,
            ..Spot::default()
        };
        apply_spot(&mut level, (0, 30), &spot, &mut regions);
        assert!(level.height[level.wrap((SIZE - 3, 30))] < 100);
        assert!(level.height[level.wrap((3, 30))] < 100);
        assert_eq!(regions.len(), 2);
    }

    // -- the burrow -----------------------------------------------------

    fn burrow(x0: f32, x1: f32, y: f32) -> Burrow {
        use glam::Vec3;
        Burrow {
            left: Vec3::new(x0, y, 0.0),
            right: Vec3::new(x1, y, 0.0),
        }
    }

    #[test]
    fn a_burrow_throws_mounds_up_along_the_car() {
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 20);
        let mut regions = Vec::new();
        apply_burrow(
            &mut level,
            &molehills(),
            &burrow(20.0, 40.0, 30.0),
            &mut regions,
        );

        let raised = (15..45)
            .filter(|&x| level.height[level.wrap((x, 30))] > 20)
            .collect::<Vec<_>>();
        assert!(!raised.is_empty(), "the burrow threw nothing up");
        assert!(
            raised.iter().all(|&x| (14..46).contains(&x)),
            "and nothing far off the line: {:?}",
            raised
        );
        assert!(!regions.is_empty());
    }

    #[test]
    fn a_mound_only_ever_goes_up() {
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 20);
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        apply_burrow(
            &mut level,
            &molehills(),
            &burrow(20.0, 40.0, 30.0),
            &mut regions,
        );
        assert!(
            before.iter().zip(level.height.iter()).all(|(a, b)| b >= a),
            "a burrow must not dig the surface down"
        );
    }

    #[test]
    fn high_ground_swallows_the_burrow() {
        // `dz >= 200` in `make_mole`: nothing gets through.
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 220);
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        apply_burrow(
            &mut level,
            &molehills(),
            &burrow(20.0, 40.0, 30.0),
            &mut regions,
        );
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }

    #[test]
    fn deeper_ground_lets_less_of_the_mound_through() {
        let peak = |ground: u8| {
            let mut level = test_level();
            level.height.iter_mut().for_each(|h| *h = ground);
            let mut regions = Vec::new();
            apply_burrow(
                &mut level,
                &molehills(),
                &burrow(20.0, 40.0, 30.0),
                &mut regions,
            );
            level
                .height
                .iter()
                .map(|&h| h as i32 - ground as i32)
                .max()
                .unwrap()
        };
        let (low, mid, high) = (peak(20), peak(100), peak(150));
        assert!(low > mid, "low ground should give the tallest mound");
        assert!(
            mid > high,
            "and it should keep tapering: {} {} {}",
            low,
            mid,
            high
        );
    }

    #[test]
    fn a_mole_sitting_still_does_not_bury_itself() {
        use glam::Vec3;
        let mut tracks = Tracks::default();
        tracks.burrow(Vec3::new(20.0, 30.0, 0.0), Vec3::new(40.0, 30.0, 0.0));
        assert_eq!(
            tracks.drain_burrows().count(),
            1,
            "the first one always counts"
        );

        // Shuffling about within a couple of texels earns nothing.
        for _ in 0..10 {
            tracks.burrow(Vec3::new(21.0, 30.0, 0.0), Vec3::new(41.0, 30.0, 0.0));
        }
        assert_eq!(tracks.drain_burrows().count(), 0);

        tracks.burrow(Vec3::new(26.0, 30.0, 0.0), Vec3::new(46.0, 30.0, 0.0));
        assert_eq!(tracks.drain_burrows().count(), 1, "but moving on does");
    }

    #[test]
    fn surfacing_starts_the_next_burrow_fresh() {
        use glam::Vec3;
        let mut tracks = Tracks::default();
        tracks.burrow(Vec3::new(20.0, 30.0, 0.0), Vec3::new(40.0, 30.0, 0.0));
        let _ = tracks.drain_burrows().count();
        tracks.surface();
        // Same spot, but a new burrow, so it counts again.
        tracks.burrow(Vec3::new(20.0, 30.0, 0.0), Vec3::new(40.0, 30.0, 0.0));
        assert_eq!(tracks.drain_burrows().count(), 1);
    }

    #[test]
    fn a_burrow_under_a_slab_raises_the_floor_not_the_roof() {
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 20);
        // A cave over the whole line, floor at 20 and roof up at 200.
        for x in (14..48).step_by(2) {
            dual_level(&mut level, x, 30, 20, 200, 1);
        }
        let mut regions = Vec::new();
        apply_burrow(
            &mut level,
            &molehills(),
            &burrow(20.0, 40.0, 30.0),
            &mut regions,
        );

        let mut floors = 0;
        for x in (14..48).step_by(2) {
            let i = level.wrap((x, 30));
            assert_eq!(level.height[i | 1], 200, "the roof at {} moved", x);
            if level.height[i & !1] > 20 {
                floors += 1;
            }
        }
        assert!(floors > 0, "and the floor never rose");
    }

    #[test]
    fn switching_the_mounds_off_leaves_the_ground_alone() {
        let mut level = test_level();
        level.height.iter_mut().for_each(|h| *h = 20);
        let before = level.height.to_vec();
        let mut regions = Vec::new();
        let config = Molehills {
            enabled: false,
            ..Molehills::default()
        };
        apply_burrow(&mut level, &config, &burrow(20.0, 40.0, 30.0), &mut regions);
        assert_eq!(level.height.to_vec(), before);
        assert!(regions.is_empty());
    }
}
