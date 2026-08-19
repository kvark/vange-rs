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
    /// Texels between those stamps. `8/3` spreads a three-stamp bar over
    /// eight texels, about the width of a wheel.
    pub spacing: f32,
}

impl Default for Tread {
    fn default() -> Self {
        Tread {
            enabled: true,
            depth: 1,
            period: 3,
            bar: 3,
            spacing: 8.0 / 3.0,
        }
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
            enabled: true,
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

    /// Forgets every wheel's contact and any track not yet drawn, so that
    /// the next contact starts fresh. Needed whenever the car is moved
    /// rather than driven.
    pub fn reset(&mut self) {
        self.lift_all();
        self.raise_blade();
        self.pending.clear();
        self.sweeps.clear();
        self.hull = None;
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty() && self.sweeps.is_empty() && self.hull.is_none()
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
    let i = level.wrap((x, y));
    if level.meta[i] & DOUBLE_LEVEL != 0 && x & 1 == 0 {
        return None;
    }
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
        for k in 0..3 {
            assert_eq!(at(10, 10 + k), 99, "the whole bar is stamped");
        }
        assert_eq!(at(10, 13), 100, "and no further");
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
}
