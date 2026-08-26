//! Ground insects / beebs: they wander the terrain, and a player vehicle
//! running one over awards beeb currency by tier and relocates it.
//!
//! Port of `InsectUnit` (`src/units/mechos.cpp`). Flocking constants are
//! not reproduced. The original Bug `.a3d` is drawn when `game.lst`
//! lists it; otherwise they are ticks.

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

/// Crawl speed, a little under original `MaxHideSpeed` of 3 so a quant
/// of 0.05s does not make them dart.
pub const MAX_SPEED: f32 = 0.7;

/// `MECHOS_ROT_DELTA` analogue: how far heading can change in one quant.
const TURN_RATE: f32 = 0.22;

/// `p->radius * 5` separation distance in InitEnvironment.
const SEPARATION: f32 = 48.0;

/// How far an insect looks for a new wander target, `INSECT_RADIUS`.
const WANDER_RADIUS: f32 = 2500.0;

/// Texels of walking per animation frame of the Bug `.a3d`.
const WALK_PER_FRAME: f32 = 6.0;

/// How far above the ground a mark is drawn, so the depth test does not
/// bury it in the terrain.
const MARK_LIFT: f32 = 2.0;
const MARK_SPAN: f32 = 12.0;

const TIER_COLORS: [u32; 3] = [0xFF66_CC88, 0xFF44_88CC, 0xFF22_CCFF];

/// One beeb on the ground.
#[derive(Clone, Debug, PartialEq)]
pub struct Insect {
    pub pos: Vec3,
    /// Where it is currently walking towards, on the level plane.
    pub target: (f32, f32),
    /// Facing, for the Bug mesh.
    pub heading: f32,
    /// Distance walked, for the `.a3d` frame.
    pub walk: f32,
    /// 0, 1 or 2 - indexes [`TIER_PRICES`].
    pub tier: u8,
}

impl Insect {
    pub fn at(pos: Vec3, tier: u8) -> Self {
        Insect {
            pos,
            target: (pos.x, pos.y),
            heading: 0.0,
            walk: 0.0,
            tier: tier.min(2),
        }
    }

    pub fn price(&self) -> i32 {
        TIER_PRICES[self.tier as usize]
    }

    /// Frame of the Bug animation, wrapping at `n` meshes.
    pub fn frame(&self, n: usize) -> usize {
        if n == 0 {
            return 0;
        }
        (self.walk / WALK_PER_FRAME).floor() as usize % n
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

    /// Ten times the original Fostral count of 30, scaled by map area.
    pub fn count_for_size(size: (i32, i32)) -> usize {
        let area = size.0.max(1) as i64 * size.1.max(1) as i64;
        let authored = MAX_INSECTS as i64 * 10;
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
            let step = MAX_SPEED * (1.0 - 0.5 * (dh.abs() / TURN_RATE));
            let fwd = glam::Quat::from_rotation_z(heading) * Vec3::Y;
            // Do not fold onto `[0, size)`: `rebase_torus` already shifts
            // the swarm with the camera, and a fold here is a seam jump.
            let nx = pos.x + fwd.x * step;
            let ny = pos.y + fwd.y * step;
            let z = level.get((nx as i32, ny as i32)).high() + MARK_LIFT;
            self.insects[i].walk += step;
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

    /// A vehicle at `pos` with `radius` has run over every insect inside
    /// that circle. Each one pays its tier, is moved away, and the old
    /// positions are returned so a burst can be spawned there.
    pub fn crush(&mut self, pos: Vec3, radius: f32) -> Crush {
        let mut awarded = 0;
        let mut at = Vec::new();
        let reach = radius * radius;
        let size = self.size;
        let mut hits = Vec::new();
        for (i, insect) in self.insects.iter().enumerate() {
            let dx = wrap_delta(insect.pos.x - pos.x, size.0 as f32);
            let dy = wrap_delta(insect.pos.y - pos.y, size.1 as f32);
            let dz = insect.pos.z - pos.z;
            if dx * dx + dy * dy + dz * dz <= reach {
                hits.push(i);
            }
        }
        for i in hits {
            awarded += self.insects[i].price();
            at.push(self.insects[i].pos);
            let nx = self.rng(size.0.max(1) as u32) as f32;
            let ny = self.rng(size.1.max(1) as u32) as f32;
            self.insects[i].pos.x = nx;
            self.insects[i].pos.y = ny;
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
    fn an_insect_walks_towards_its_target() {
        let level = test_level();
        let mut swarm = Swarm::new(level.size);
        let mut bug = Insect::at(Vec3::new(40.0, 40.0, 10.0), 0);
        bug.target = (140.0, 40.0);
        bug.heading = -std::f32::consts::FRAC_PI_2;
        swarm.push(bug);
        let start = swarm.insects()[0].pos;
        for _ in 0..30 {
            swarm.quant(&level, None);
        }
        let now = swarm.insects()[0].pos;
        assert!(
            now.x > start.x + 10.0,
            "did not walk along +x: {} -> {}",
            start.x,
            now.x
        );
        assert!(
            (now.y - start.y).abs() < 8.0,
            "drifted off the line: {}",
            now.y
        );
        let first = {
            let mut s = Swarm::new(level.size);
            let mut bug = Insect::at(Vec3::new(40.0, 40.0, 10.0), 0);
            bug.target = (140.0, 40.0);
            bug.heading = -std::f32::consts::FRAC_PI_2;
            s.push(bug);
            s.quant(&level, None);
            s.insects()[0].pos.x - 40.0
        };
        assert!(first <= MAX_SPEED + 0.1, "a quant still teleports: {first}");
    }

    #[test]
    fn they_walk_the_way_the_mesh_faces() {
        let level = test_level();
        let mut swarm = Swarm::new(level.size);
        let mut bug = Insect::at(Vec3::new(80.0, 80.0, 10.0), 0);
        bug.target = (80.0, 180.0);
        bug.heading = 0.0;
        swarm.push(bug);
        swarm.quant(&level, None);
        let now = swarm.insects()[0].pos;
        assert!(
            now.y > 80.0 + 0.5,
            "heading 0 is +Y on the mesh, but it went to {now:?}"
        );
        assert!((now.x - 80.0).abs() < 1.0, "slid sideways: {now:?}");
    }

    #[test]
    fn walking_advances_the_animation_frame() {
        let mut bug = Insect::at(Vec3::new(0.0, 0.0, 0.0), 0);
        bug.walk = 0.0;
        assert_eq!(bug.frame(4), 0);
        bug.walk = WALK_PER_FRAME * 2.5;
        assert_eq!(bug.frame(4), 2);
    }

    #[test]
    fn crushing_an_insect_pays_its_tier_and_moves_it() {
        let mut swarm = Swarm::new((256, 256));
        swarm.push(Insect::at(Vec3::new(50.0, 50.0, 10.0), 0));
        swarm.push(Insect::at(Vec3::new(200.0, 200.0, 10.0), 2));
        let crush = swarm.crush(Vec3::new(50.0, 50.0, 10.0), 20.0);
        assert_eq!(
            crush.awarded, TIER_PRICES[0],
            "wrong purse for a cheap beeb"
        );
        assert_eq!(crush.at.len(), 1);
        let moved = &swarm.insects()[0];
        assert!(
            (moved.pos.x - 50.0).abs() > 1.0 || (moved.pos.y - 50.0).abs() > 1.0,
            "the cheap beeb is still sitting on the impact"
        );
        let gold = &swarm.insects()[1];
        assert_eq!(gold.pos, Vec3::new(200.0, 200.0, 10.0), "the far one moved");
    }

    #[test]
    fn a_gold_beeb_pays_a_hundred() {
        let mut swarm = Swarm::new((256, 256));
        swarm.push(Insect::at(Vec3::new(10.0, 10.0, 5.0), 2));
        let crush = swarm.crush(Vec3::new(10.0, 10.0, 5.0), 8.0);
        assert_eq!(crush.awarded, 100);
    }

    #[test]
    fn populate_fills_the_world() {
        let level = test_level();
        let mut swarm = Swarm::new(level.size);
        swarm.populate(MAX_INSECTS, &level);
        assert_eq!(swarm.insects().len(), MAX_INSECTS);
        assert_eq!(swarm.insects()[0].tier, 2, "the first insect is gold");
    }

    #[test]
    fn density_is_ten_times_the_original_fostral_count() {
        assert_eq!(Swarm::count_for_size((2048, 16384)), MAX_INSECTS * 10);
        assert_eq!(Swarm::count_for_size((256, 256)), 1);
        assert_eq!(Swarm::count_for_size((4096, 16384)), MAX_INSECTS * 20);
    }

    #[test]
    fn a_full_map_quant_does_not_pair_every_insect() {
        // Fostral-scale population. Pairing each nearby beeb against the
        // whole list froze the game after the first frame.
        let level = test_level();
        let mut swarm = Swarm::new((8192, 8192));
        swarm.populate(12_000, &level);
        let t0 = std::time::Instant::now();
        swarm.quant(&level, Some(Vec3::new(80.0, 80.0, 10.0)));
        assert!(
            t0.elapsed() < std::time::Duration::from_millis(250),
            "quant scanned the whole population: {:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn far_insects_sleep() {
        let level = test_level();
        let mut swarm = Swarm::new((8192, 8192));
        swarm.push(Insect::at(Vec3::new(10.0, 10.0, 10.0), 0));
        let start = swarm.insects()[0].pos;
        swarm.quant(&level, Some(Vec3::new(7000.0, 7000.0, 10.0)));
        assert_eq!(
            swarm.insects()[0].pos,
            start,
            "a distant beeb still crawled"
        );
    }

    #[test]
    fn they_wander_instead_of_hunting_the_player() {
        let level = test_level();
        let mut swarm = Swarm::new((4096, 4096));
        let mut bug = Insect::at(Vec3::new(40.0, 40.0, 10.0), 0);
        bug.target = (40.0, 180.0);
        bug.heading = 0.0;
        swarm.push(bug);
        let player = Vec3::new(200.0, 40.0, 10.0);
        for _ in 0..20 {
            swarm.quant(&level, Some(player));
        }
        let now = swarm.insects()[0].pos;
        assert!(
            now.y > 50.0,
            "did not walk toward its wander target: {now:?}"
        );
        assert!(
            (now.x - 40.0).abs() < 20.0,
            "veered off toward the player: {now:?}"
        );
    }

    #[test]
    fn walking_changes_the_bug_frame() {
        let level = test_level();
        let mut swarm = Swarm::new(level.size);
        let mut bug = Insect::at(Vec3::new(40.0, 40.0, 10.0), 0);
        bug.target = (40.0, 180.0);
        swarm.push(bug);
        let start = swarm.insects()[0].frame(4);
        for _ in 0..40 {
            swarm.quant(&level, None);
        }
        assert_ne!(
            swarm.insects()[0].frame(4),
            start,
            "the walk cycle never left frame {start}"
        );
    }

    #[test]
    fn walking_west_does_not_jump_to_the_far_edge() {
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
        assert!(
            now.x < 0.0,
            "folding onto [0, size) is a seam teleport: {now:?}"
        );
        assert!(now.x > -40.0, "it ran off instead of walking west: {now:?}");
    }

    #[test]
    fn draw_skips_insects_outside_the_view() {
        let mut swarm = Swarm::new((2048, 2048));
        swarm.push(Insect::at(Vec3::new(10.0, 10.0, 10.0), 0));
        swarm.push(Insect::at(Vec3::new(1800.0, 1800.0, 10.0), 0));
        let mut near = LineBuffer::new();
        swarm.draw_near(&mut near, Vec3::new(10.0, 10.0, 10.0), 100.0);
        assert!(!near.is_empty(), "the nearby beeb was culled");
        let mut all = LineBuffer::new();
        swarm.draw_near(&mut all, Vec3::new(10.0, 10.0, 10.0), 4000.0);
        assert!(
            all.len() > near.len(),
            "the far beeb was drawn as if it were next to the camera"
        );
    }
}
