//! Ground insects / beebs.

use crate::level::Level;
use crate::render::debug::LineBuffer;

use glam::Vec3;

/// `INSECT_PRICE_DATA` of the original: cheap, middling, gold.
pub const TIER_PRICES: [i32; 3] = [1, 10, 100];

/// `MAX_INSECT_UNIT` of the original.
pub const MAX_INSECTS: usize = 30;

/// Fostral (`Map Power X=11, Y=14`) is what that count was authored for.
const FOSTRAL_AREA: i64 = 2048 * 16384;

/// How far from the camera an insect is simulated and drawn.
pub const ACTIVE_RADIUS: f32 = 768.0;

pub const MAX_SPEED: f32 = 0.7;

/// `MECHOS_ROT_DELTA` analogue: how far heading can change in one quant.
const TURN_RATE: f32 = 0.22;

/// `p->radius * 5` separation distance in InitEnvironment.
const SEPARATION: f32 = 48.0;

/// How far an insect looks for a new wander target, `INSECT_RADIUS`.
const WANDER_RADIUS: f32 = 2500.0;

const TRACTION: u32 = 256;
const SLOPE_SAMPLE: f32 = 6.0;
const MARK_LIFT: f32 = 0.0;
const MARK_SPAN: f32 = 12.0;
/// Steeper than this (rise/run) is a wall or a cliff, not a walkable hill.
const STEEP: f32 = 4.0;
const FALL_ACCEL: f32 = 0.7;
const FALL_MAX: f32 = 10.0;

const TIER_COLORS: [u32; 3] = [0xFF66_CC88, 0xFF44_88CC, 0xFF22_CCFF];

/// One beeb on the ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Insect {
    pub pos: Vec3,
    /// Where it is currently walking towards, on the level plane.
    pub target: (f32, f32),
    /// Facing about world Z, for the Bug mesh.
    pub heading: f32,
    /// Look up/down along the slope. Positive is nose up.
    pub pitch: f32,
    /// `Object::i_model`. The drawn frame is this `>> 8`.
    pub i_model: u32,
    /// Vertical speed while falling. Zero when walking.
    pub vz: f32,
    /// 0, 1 or 2 - indexes [`TIER_PRICES`].
    pub tier: u8,
}

impl Insect {
    pub fn at(pos: Vec3, tier: u8) -> Self {
        Insect {
            pos,
            target: (pos.x, pos.y),
            heading: 0.0,
            pitch: 0.0,
            i_model: 0,
            vz: 0.0,
            tier: tier.min(2),
        }
    }

    pub fn price(&self) -> i32 {
        TIER_PRICES[self.tier as usize]
    }

    /// Frame of the Bug animation. Original `Object::draw` indexes
    /// `models[(i_model >> 8) % n_models]`.
    pub fn frame(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        ((self.i_model >> 8) as usize) % n
    }

    /// Mesh `+Y` forward, `+Z` up: heading on the ground, then pitch so
    /// the nose follows the slope.
    pub fn rotation(&self) -> glam::Quat {
        glam::Quat::from_rotation_z(self.heading) * glam::Quat::from_rotation_x(self.pitch)
    }
}

/// Every insect of one world, plus the RNG that places and relocates them.
pub struct Swarm {
    insects: Vec<Insect>,
    size: (i32, i32),
    seed: u32,
}

impl Swarm {
    pub fn new(size: (i32, i32)) -> Self {
        Swarm {
            insects: Vec::new(),
            size,
            seed: 83838383,
        }
    }

    pub fn insects(&self) -> &[Insect] {
        &self.insects
    }

    pub fn insects_mut(&mut self) -> &mut [Insect] {
        &mut self.insects
    }

    pub fn is_empty(&self) -> bool {
        self.insects.is_empty()
    }

    pub fn push(&mut self, insect: Insect) {
        self.insects.push(insect);
    }

    pub fn len(&self) -> usize {
        self.insects.len()
    }

    /// Twenty times the original Fostral count of 30, scaled by map area.
    pub fn count_for_size(size: (i32, i32)) -> usize {
        let area = size.0.max(1) as i64 * size.1.max(1) as i64;
        let authored = MAX_INSECTS as i64 * 20;
        let n = (authored * area + FOSTRAL_AREA / 2) / FOSTRAL_AREA;
        n.clamp(1, 4_000) as usize
    }

    /// `InsectList::Init`: scatter `count` insects across the level,
    /// picking tiers the original way (the first one comes out gold because
    /// `RND(0)` is 0).
    pub fn populate(&mut self, count: usize, level: &Level) {
        let mut counts = [0i32; 3];
        for _ in 0..count {
            let tier = pick_tier(&mut self.seed, &counts);
            counts[tier as usize] += 1;
            let x = self.rng(self.size.0.max(1) as u32) as f32;
            let y = self.rng(self.size.1.max(1) as u32) as f32;
            let z = level.get((x as i32, y as i32)).high() + MARK_LIFT;
            let mut insect = Insect::at(Vec3::new(x, y, z), tier);
            insect.target = self.wander_target(insect.pos);
            self.insects.push(insect);
        }
    }

    /// `HideAction` toward a wander target, not the player. Only insects
    /// near `eye` step, so a few hundred on Fostral stay cheap.
    pub fn quant(&mut self, level: &Level, eye: Option<Vec3>) {
        let size = self.size;
        let r2 = ACTIVE_RADIUS * ACTIVE_RADIUS;
        let n = self.insects.len();
        let mut active = Vec::new();
        for i in 0..n {
            if let Some(at) = eye {
                let pos = self.insects[i].pos;
                let dx = wrap_delta(pos.x - at.x, size.0 as f32);
                let dy = wrap_delta(pos.y - at.y, size.1 as f32);
                if dx * dx + dy * dy > r2 {
                    continue;
                }
            }
            active.push(i);
        }
        for &i in &active {
            let pos = self.insects[i].pos;
            let (tx, ty) = self.insects[i].target;
            let mut dx = wrap_delta(tx - pos.x, size.0 as f32);
            let mut dy = wrap_delta(ty - pos.y, size.1 as f32);
            let dist = (dx * dx + dy * dy).sqrt();
            if dist < 24.0 {
                self.insects[i].target = self.wander_target(pos);
                let slope = surface_slope(level, pos, self.insects[i].heading);
                self.insects[i].pitch = pitch_from_slope(slope);
                continue;
            }
            for &j in &active {
                if i == j {
                    continue;
                }
                let other = self.insects[j].pos;
                let ox = wrap_delta(pos.x - other.x, size.0 as f32);
                let oy = wrap_delta(pos.y - other.y, size.1 as f32);
                let od = (ox * ox + oy * oy).sqrt();
                if od > 0.1 && od < SEPARATION {
                    let f = (SEPARATION - od) / SEPARATION;
                    dx += ox / od * f * MAX_SPEED;
                    dy += oy / od * f * MAX_SPEED;
                }
            }
            // Mesh forward is +Y; `from_rotation_z` takes that to
            // `(-sin h, cos h)`. Steer and walk in that frame, not +X.
            let desired = (-dx).atan2(dy);
            let mut dh = wrap_delta(desired - self.insects[i].heading, std::f32::consts::TAU);
            dh = dh.clamp(-TURN_RATE, TURN_RATE);
            self.insects[i].heading += dh;
            let heading = self.insects[i].heading;
            let slope = surface_slope(level, pos, heading);
            self.insects[i].pitch = pitch_from_slope(slope);
            let step = MAX_SPEED * (1.0 - 0.5 * (dh.abs() / TURN_RATE));
            let (fx, fy) = (-heading.sin(), heading.cos());
            let (nx, ny, z, vz) =
                crawl_or_fall(level, pos, fx, fy, step, slope, self.insects[i].vz);
            self.insects[i].vz = vz;
            self.insects[i].i_model = self.insects[i].i_model.wrapping_add(TRACTION);
            self.insects[i].pos = Vec3::new(nx, ny, z);
        }
    }

    /// Insects within `radius` of `eye`, wrapping the level.
    pub fn near(&self, eye: Vec3, radius: f32) -> impl Iterator<Item = &Insect> {
        let size = self.size;
        let r2 = radius * radius;
        self.insects.iter().filter(move |insect| {
            let dx = wrap_delta(insect.pos.x - eye.x, size.0 as f32);
            let dy = wrap_delta(insect.pos.y - eye.y, size.1 as f32);
            dx * dx + dy * dy <= r2
        })
    }

    /// Original `test_wheels_to_sphere`: a wheel has to actually pass
    /// through the insect. `points` are the wheel positions in the world.
    pub fn crush(&mut self, points: &[Vec3], radius: f32) -> Crush {
        let mut awarded = 0;
        let mut at = Vec::new();
        let reach = radius * radius;
        let size = self.size;
        let mut hits = Vec::new();
        for (i, insect) in self.insects.iter().enumerate() {
            for &pos in points {
                let dx = wrap_delta(insect.pos.x - pos.x, size.0 as f32);
                let dy = wrap_delta(insect.pos.y - pos.y, size.1 as f32);
                let dz = insect.pos.z - pos.z;
                if dx * dx + dy * dy + dz * dz <= reach {
                    hits.push(i);
                    break;
                }
            }
        }
        for i in hits {
            awarded += self.insects[i].price();
            at.push(self.insects[i].pos);
            let nx = self.rng(size.0.max(1) as u32) as f32;
            let ny = self.rng(size.1.max(1) as u32) as f32;
            self.insects[i].pos.x = nx;
            self.insects[i].pos.y = ny;
            self.insects[i].vz = 0.0;
            self.insects[i].target = self.wander_target(self.insects[i].pos);
        }
        Crush { awarded, at }
    }

    pub fn draw(&self, lines: &mut LineBuffer) {
        self.draw_near(lines, Vec3::ZERO, f32::MAX);
    }

    pub fn draw_near(&self, lines: &mut LineBuffer, eye: Vec3, radius: f32) {
        for insect in self.near(eye, radius) {
            let c = TIER_COLORS[insect.tier as usize];
            let p = insect.pos;
            let z = p.z + 0.5;
            let s = MARK_SPAN;
            lines.add([p.x - s, p.y - s, z], [p.x + s, p.y + s, z], c);
            lines.add([p.x - s, p.y + s, z], [p.x + s, p.y - s, z], c);
            lines.add([p.x, p.y, z], [p.x, p.y, z + s], c);
            lines.add([p.x - s, p.y, z], [p.x + s, p.y, z], c);
            lines.add([p.x, p.y - s, z], [p.x, p.y + s, z], c);
        }
    }

    fn wander_target(&mut self, from: Vec3) -> (f32, f32) {
        let span = WANDER_RADIUS * 2.0;
        let x = wrap(
            from.x + self.unit() * span - WANDER_RADIUS,
            self.size.0 as f32,
        );
        let y = wrap(
            from.y + self.unit() * span - WANDER_RADIUS,
            self.size.1 as f32,
        );
        (x, y)
    }

    fn rng(&mut self, modulus: u32) -> u32 {
        self.seed ^= self.seed >> 3;
        self.seed ^= self.seed << 28;
        self.seed &= 0x7FFF_FFFF;
        if modulus == 0 { 0 } else { self.seed % modulus }
    }

    fn unit(&mut self) -> f32 {
        self.rng(10_000) as f32 / 10_000.0
    }
}

/// What a crush collected.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Crush {
    /// Beebs to add to the player's wallet.
    pub awarded: i32,
    /// Where the insects were, for a particle burst.
    pub at: Vec<Vec3>,
}

fn surface_height(level: &Level, x: f32, y: f32) -> f32 {
    level.get((x as i32, y as i32)).high()
}

/// Rise over run along `heading`, sampled `SLOPE_SAMPLE` texels out.
fn surface_slope(level: &Level, pos: Vec3, heading: f32) -> f32 {
    let (s, c) = heading.sin_cos();
    let z0 = surface_height(level, pos.x, pos.y);
    let z1 = surface_height(level, pos.x - s * SLOPE_SAMPLE, pos.y + c * SLOPE_SAMPLE);
    (z1 - z0) / SLOPE_SAMPLE
}

/// Original `insect_analysis` flattens a normal steeper than 45°.
fn pitch_from_slope(slope: f32) -> f32 {
    slope
        .atan()
        .clamp(-std::f32::consts::FRAC_PI_4, std::f32::consts::FRAC_PI_4)
}

/// Walk a medium slope at constant 3D speed, climb a wall, or fall off a
/// cliff. Never snaps Z by more than `step` in one quant.
fn crawl_or_fall(
    level: &Level,
    pos: Vec3,
    fx: f32,
    fy: f32,
    step: f32,
    slope: f32,
    vz: f32,
) -> (f32, f32, f32, f32) {
    let ground = surface_height(level, pos.x, pos.y) + MARK_LIFT;
    if pos.z > ground + 1.5 {
        let vz = (vz - FALL_ACCEL).max(-FALL_MAX);
        let nx = pos.x + fx * step * 0.35;
        let ny = pos.y + fy * step * 0.35;
        let z = pos.z + vz;
        let g = surface_height(level, nx, ny) + MARK_LIFT;
        if z <= g {
            return (nx, ny, g, 0.0);
        }
        return (nx, ny, z, vz);
    }
    if slope > STEEP {
        let top = surface_height(level, pos.x + fx * SLOPE_SAMPLE, pos.y + fy * SLOPE_SAMPLE);
        let z = pos.z + step;
        if z >= top + MARK_LIFT {
            let nx = pos.x + fx * step * 0.5;
            let ny = pos.y + fy * step * 0.5;
            return (nx, ny, surface_height(level, nx, ny) + MARK_LIFT, 0.0);
        }
        return (pos.x, pos.y, z, 0.0);
    }
    if slope < -STEEP {
        let nx = pos.x + fx * step;
        let ny = pos.y + fy * step;
        let g = surface_height(level, nx, ny) + MARK_LIFT;
        if g < pos.z - step {
            let vz = -step;
            return (nx, ny, pos.z + vz, vz);
        }
        return (nx, ny, g, 0.0);
    }
    let xy = step / (1.0 + slope * slope).sqrt();
    let nx = pos.x + fx * xy;
    let ny = pos.y + fy * xy;
    let want = surface_height(level, nx, ny) + MARK_LIFT;
    let z = pos.z + (want - pos.z).clamp(-step, step);
    (nx, ny, z, 0.0)
}

fn pick_tier(seed: &mut u32, counts: &[i32; 3]) -> u8 {
    // `CreateInsect`: RND(n * price) == 0 picks that tier. n starts at 0,
    // and RND(0) is 0, so the first insect is always gold.
    let rng = |m: u32, seed: &mut u32| {
        *seed ^= *seed >> 3;
        *seed ^= *seed << 28;
        *seed &= 0x7FFF_FFFF;
        if m == 0 { 0 } else { *seed % m }
    };
    if rng(
        (counts[2] as u32).saturating_mul(TIER_PRICES[2] as u32),
        seed,
    ) == 0
    {
        2
    } else if rng(
        (counts[1] as u32).saturating_mul(TIER_PRICES[1] as u32),
        seed,
    ) == 0
    {
        1
    } else {
        0
    }
}

fn wrap(v: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return v;
    }
    v.rem_euclid(span)
}

fn wrap_delta(d: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return d;
    }
    let half = span * 0.5;
    (d + half).rem_euclid(span) - half
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{LevelConfig, TerrainBits, terraform::MAIN_TERRAIN};

    fn test_level() -> Level {
        let size = 256i32;
        let total = (size * size) as usize;
        let bits = TerrainBits::new(8);
        Level {
            size: (size, size),
            flood_map: vec![0; size as usize].into_boxed_slice(),
            height: vec![40u8; total].into_boxed_slice(),
            meta: vec![bits.write(MAIN_TERRAIN); total].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    #[test]
    fn they_walk_the_way_the_mesh_faces() {
        let level = test_level();
        let mut swarm = Swarm::new(level.size);
        let mut bug = Insect::at(Vec3::new(80.0, 80.0, 10.0), 0);
        bug.target = (80.0, 180.0);
        swarm.push(bug);
        swarm.quant(&level, None);
        let now = swarm.insects()[0].pos;
        assert!(now.y > 80.5, "heading 0 should be +Y, got {now:?}");
        assert!((now.x - 80.0).abs() < 1.0);
        assert_eq!(swarm.insects()[0].frame(4), 1);
    }

    #[test]
    fn crush_is_only_under_a_wheel() {
        let mut swarm = Swarm::new((256, 256));
        swarm.push(Insect::at(Vec3::new(50.0, 50.0, 10.0), 0));
        swarm.push(Insect::at(Vec3::new(200.0, 200.0, 10.0), 2));
        assert_eq!(swarm.crush(&[Vec3::new(80.0, 50.0, 10.0)], 8.0).awarded, 0);
        let crush = swarm.crush(&[Vec3::new(50.0, 50.0, 10.0)], 8.0);
        assert_eq!(crush.awarded, TIER_PRICES[0]);
        assert_eq!(swarm.insects()[1].pos, Vec3::new(200.0, 200.0, 10.0));
    }

    #[test]
    fn they_pitch_on_a_slope_and_fall_off_a_cliff() {
        let mut slope = test_level();
        let (w, h) = slope.size;
        for y in 0..h {
            for x in 0..w {
                slope.height[(y * w + x) as usize] = (40 + y).clamp(0, 255) as u8;
            }
        }
        let mut swarm = Swarm::new(slope.size);
        let start = Vec3::new(80.0, 80.0, surface_height(&slope, 80.0, 80.0));
        let mut bug = Insect::at(start, 0);
        bug.target = (80.0, 180.0);
        swarm.push(bug);
        swarm.quant(&slope, None);
        assert!(swarm.insects()[0].pitch > 0.6);
        assert!(swarm.insects()[0].pos.y - start.y < MAX_SPEED * 0.85);

        let mut cliff = test_level();
        for y in 0..h {
            let altitude = if y < 40 { 200 } else { 40 };
            for x in 0..w {
                cliff.height[(y * w + x) as usize] = altitude;
            }
        }
        let mut swarm = Swarm::new(cliff.size);
        let start = Vec3::new(80.0, 38.0, surface_height(&cliff, 80.0, 38.0));
        let mut bug = Insect::at(start, 0);
        bug.target = (80.0, 80.0);
        swarm.push(bug);
        swarm.quant(&cliff, None);
        assert!(swarm.insects()[0].pos.z > start.z - MAX_SPEED - 0.2);
        for _ in 0..40 {
            swarm.quant(&cliff, None);
        }
        assert!(swarm.insects()[0].pos.z < 50.0);
    }

    #[test]
    fn density_and_culling() {
        assert_eq!(Swarm::count_for_size((2048, 16384)), MAX_INSECTS * 20);
        let level = test_level();
        let mut swarm = Swarm::new((8192, 8192));
        swarm.push(Insect::at(Vec3::new(10.0, 10.0, 10.0), 0));
        swarm.quant(&level, Some(Vec3::new(7000.0, 7000.0, 10.0)));
        assert_eq!(swarm.insects()[0].pos, Vec3::new(10.0, 10.0, 10.0));
    }

    #[test]
    fn walking_west_does_not_fold_the_seam() {
        let level = test_level();
        let mut swarm = Swarm::new(level.size);
        let mut bug = Insect::at(Vec3::new(2.0, 40.0, 10.0), 0);
        bug.target = (-80.0, 40.0);
        bug.heading = std::f32::consts::FRAC_PI_2;
        swarm.push(bug);
        for _ in 0..20 {
            swarm.quant(&level, None);
        }
        let now = swarm.insects()[0].pos;
        assert!(now.x < 0.0 && now.x > -40.0, "{now:?}");
    }
}
