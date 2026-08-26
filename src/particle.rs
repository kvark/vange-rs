//! World-space particles: dust from wheels, smoke from a damaged hull,
//! and a burst when something is crushed or a crater is blown.
//!
//! Port of the spawn *rules* of `DrawMechosParticle` (`src/units/hobj.cpp`)
//! and the motion of `SimpleParticleType::Quant` (`src/units/effect.cpp`).
//! Positions live in world space and age in quants; the original's software
//! framebuffer plotter is not reproduced.

use crate::level::Level;
use crate::level::terraform::{MAIN_TERRAIN, Track};
use crate::render::debug::LineBuffer;

use glam::Vec3;

/// How long a dust puff lasts, in quants. Short, like the original wheel
/// dust that is gone almost as soon as the tyre has moved on.
const DUST_LIFE: i32 = 8;
/// Smoke hangs around longer.
const SMOKE_LIFE: i32 = 16;
/// A crush / crater burst.
const BURST_LIFE: i32 = 12;

const DUST_COUNT: usize = 4;
const SMOKE_COUNT: usize = 2;
const BURST_COUNT: usize = 16;

/// A hair above the terrain so a LessEqual depth test does not bury the
/// puff in the ground it just left. Larger values read as floating dust.
pub const SURFACE_LIFT: f32 = 0.6;
const DUST_TICK: f32 = 2.0;
const SMOKE_TICK: f32 = 6.0;

const DUST_COLOR: u32 = 0xFF88_AACC;
const SMOKE_COLOR: u32 = 0xFFAA_AAAA;
const BURST_COLOR: u32 = 0xFF44_88FF;

/// What kind of puff this is, which only changes how it is spawned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Dust,
    Smoke,
    Burst,
}

/// One particle. Position and colour are what a renderer needs; velocity
/// and age are what a quant uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Particle {
    pub pos: Vec3,
    pub vel: Vec3,
    pub color: u32,
    pub age: i32,
    pub life: i32,
    pub kind: Kind,
}

/// The pool the game steps each quant.
pub struct System {
    particles: Vec<Particle>,
    seed: u32,
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

impl System {
    pub fn new() -> Self {
        System {
            particles: Vec::new(),
            seed: 0x00C0_FFEE,
        }
    }

    pub fn particles(&self) -> &[Particle] {
        &self.particles
    }

    pub fn particles_mut(&mut self) -> &mut [Particle] {
        &mut self.particles
    }

    pub fn is_empty(&self) -> bool {
        self.particles.is_empty()
    }

    /// A wheel rolling on drivable ground. Terrain type 0 is water and
    /// never kicks up dust; everything else that is not the plain ground
    /// is left alone the same way a tread is.
    pub fn from_wheel(&mut self, pos: Vec3, terrain: u8, speed: f32) {
        if terrain != MAIN_TERRAIN || speed <= 0.0 {
            return;
        }
        self.emit_dust(pos, speed);
    }

    /// Smoke from a hull that is below its maximum armour, the original's
    /// `Armor < MaxArmor` gate. At full armour nothing comes out; at zero
    /// it always does, skipping the two `RND` rolls that would otherwise
    /// thin it out.
    pub fn from_hull(&mut self, pos: Vec3, armor: u16, max_armor: u16) {
        if max_armor == 0 || armor >= max_armor {
            return;
        }
        if armor > 0 {
            let roll = self.rng(max_armor as u32);
            if roll <= armor as u32 {
                return;
            }
            let roll = self.rng(max_armor as u32);
            if roll <= armor as u32 {
                return;
            }
        }
        self.emit_smoke(pos);
    }

    /// The puff a crush or a crater throws up.
    pub fn from_crush(&mut self, pos: Vec3) {
        self.emit_burst(pos);
    }

    pub fn from_crater(&mut self, pos: Vec3) {
        self.emit_burst(pos);
    }

    /// Dust along a wheel track, using the level to read the terrain and
    /// the altitude. This is what the road loop calls after physics.
    pub fn from_track(&mut self, track: &Track, level: &Level) {
        let (x, y) = track.to;
        let terrain = level.terrain_bits().read(level.meta[level.wrap((x, y))]);
        let dx = (track.to.0 - track.from.0) as f32;
        let dy = (track.to.1 - track.from.1) as f32;
        let speed = (dx * dx + dy * dy).sqrt();
        let z = level.get((x, y)).high() + SURFACE_LIFT;
        self.from_wheel(Vec3::new(x as f32, y as f32, z), terrain, speed);
    }

    pub fn emit_dust(&mut self, pos: Vec3, speed: f32) {
        let speed = speed.max(1.0);
        for _ in 0..DUST_COUNT {
            // Kick sideways along the ground, not up into the air.
            let vel = Vec3::new(
                self.signed(speed * 0.35),
                self.signed(speed * 0.35),
                0.05 + self.unit() * 0.15,
            );
            self.push(pos, vel, DUST_COLOR, DUST_LIFE, Kind::Dust);
        }
    }

    pub fn emit_smoke(&mut self, pos: Vec3) {
        for _ in 0..SMOKE_COUNT {
            let vel = Vec3::new(self.signed(0.4), self.signed(0.4), 1.2 + self.unit());
            self.push(pos, vel, SMOKE_COLOR, SMOKE_LIFE, Kind::Smoke);
        }
    }

    pub fn emit_burst(&mut self, pos: Vec3) {
        for _ in 0..BURST_COUNT {
            let vel = Vec3::new(self.signed(3.0), self.signed(3.0), 1.0 + self.unit() * 3.0);
            self.push(pos, vel, BURST_COLOR, BURST_LIFE, Kind::Burst);
        }
    }

    /// One quant: move, age, drop anything that has lived out its life.
    pub fn quant(&mut self) {
        for p in self.particles.iter_mut() {
            p.pos += p.vel;
            p.age += 1;
            // Dust and smoke settle; a burst keeps its kick.
            if p.kind != Kind::Burst {
                p.vel.z *= 0.7;
                p.vel.x *= 0.92;
                p.vel.y *= 0.92;
            }
        }
        self.particles.retain(|p| p.age < p.life);
    }

    pub fn draw(&self, lines: &mut LineBuffer) {
        for p in self.particles.iter() {
            let from = [p.pos.x, p.pos.y, p.pos.z];
            let to = if p.kind == Kind::Dust {
                let dir = Vec3::new(p.vel.x, p.vel.y, 0.0);
                let step = if dir.length_squared() > 1e-6 {
                    dir.normalize() * DUST_TICK
                } else {
                    Vec3::X * DUST_TICK
                };
                [from[0] + step.x, from[1] + step.y, from[2]]
            } else {
                [from[0], from[1], from[2] + SMOKE_TICK]
            };
            lines.add(from, to, p.color);
        }
    }

    fn push(&mut self, pos: Vec3, vel: Vec3, color: u32, life: i32, kind: Kind) {
        self.particles.push(Particle {
            pos,
            vel,
            color,
            age: 0,
            life,
            kind,
        });
    }

    fn rng(&mut self, modulus: u32) -> u32 {
        self.seed ^= self.seed >> 3;
        self.seed ^= self.seed << 28;
        self.seed &= 0x7FFF_FFFF;
        if modulus == 0 { 0 } else { self.seed % modulus }
    }

    fn unit(&mut self) -> f32 {
        self.rng(1000) as f32 / 1000.0
    }

    fn signed(&mut self, scale: f32) -> f32 {
        (self.unit() * 2.0 - 1.0) * scale
    }
}

/// Rebuild the debug ticks for the current particle and insect state.
///
/// Always starts from an empty buffer so a paused frame cannot append
/// another copy of the same ticks on top of the last one.
pub fn refresh_fx_lines(
    lines: &mut LineBuffer,
    particles: &System,
    swarm: &crate::creature::Swarm,
    eye: Vec3,
    insects_as_ticks: bool,
) {
    lines.clear();
    particles.draw(lines);
    if insects_as_ticks {
        swarm.draw_near(lines, eye, crate::creature::ACTIVE_RADIUS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::settings;
    use crate::level::{LevelConfig, TerrainBits};

    fn test_level() -> Level {
        let size = 64i32;
        let total = (size * size) as usize;
        let bits = TerrainBits::new(8);
        Level {
            size: (size, size),
            flood_map: vec![0; size as usize].into_boxed_slice(),
            height: vec![80u8; total].into_boxed_slice(),
            meta: vec![bits.write(MAIN_TERRAIN); total].into_boxed_slice(),
            palette: [[0; 4]; 0x100],
            terrains: LevelConfig::new_test().terrains,
            geometry: settings::Geometry::default(),
        }
    }

    #[test]
    fn a_moving_wheel_on_drivable_ground_kicks_up_dust() {
        let mut sys = System::new();
        sys.from_wheel(Vec3::new(10.0, 10.0, 80.0), MAIN_TERRAIN, 4.0);
        assert!(
            sys.particles().iter().any(|p| p.kind == Kind::Dust),
            "no dust from a rolling wheel"
        );
    }

    #[test]
    fn a_wheel_on_water_kicks_up_nothing() {
        let mut sys = System::new();
        sys.from_wheel(Vec3::new(10.0, 10.0, 80.0), 0, 4.0);
        assert!(sys.is_empty());
    }

    #[test]
    fn a_still_wheel_kicks_up_nothing() {
        let mut sys = System::new();
        sys.from_wheel(Vec3::new(10.0, 10.0, 80.0), MAIN_TERRAIN, 0.0);
        assert!(sys.is_empty());
    }

    #[test]
    fn a_damaged_hull_smokes_and_a_whole_one_does_not() {
        let mut sys = System::new();
        sys.from_hull(Vec3::new(0.0, 0.0, 10.0), 10, 10);
        assert!(sys.is_empty(), "full armour should not smoke");
        sys.from_hull(Vec3::new(0.0, 0.0, 10.0), 0, 10);
        assert!(
            sys.particles().iter().any(|p| p.kind == Kind::Smoke),
            "a wrecked hull should smoke"
        );
    }

    #[test]
    fn a_crush_or_crater_throws_a_burst() {
        let mut sys = System::new();
        sys.from_crush(Vec3::new(3.0, 4.0, 5.0));
        let n = sys.particles().len();
        assert!(n > 0, "crush made no burst");
        assert!(sys.particles().iter().all(|p| p.kind == Kind::Burst));
        sys.from_crater(Vec3::new(8.0, 8.0, 5.0));
        assert!(sys.particles().len() > n, "crater made no extra burst");
    }

    #[test]
    fn particles_move_and_then_expire() {
        let mut sys = System::new();
        sys.emit_dust(Vec3::new(0.0, 0.0, 10.0), 3.0);
        let start: Vec<Vec3> = sys.particles().iter().map(|p| p.pos).collect();
        assert!(!start.is_empty());
        sys.quant();
        let moved: Vec<Vec3> = sys.particles().iter().map(|p| p.pos).collect();
        assert_eq!(moved.len(), start.len());
        assert!(
            start.iter().zip(&moved).any(|(a, b)| a != b),
            "nothing moved after a quant"
        );
        for _ in 0..DUST_LIFE + 1 {
            sys.quant();
        }
        assert!(
            sys.particles().iter().all(|p| p.kind != Kind::Dust),
            "dust was still around after its lifetime"
        );
        assert!(sys.is_empty() || sys.particles().iter().all(|p| p.age < p.life));
    }

    #[test]
    fn a_track_on_the_level_uses_the_ground_under_the_wheel() {
        let level = test_level();
        let mut sys = System::new();
        let track = Track {
            from: (8, 8),
            to: (12, 8),
            across: (0.0, 1.0),
        };
        sys.from_track(&track, &level);
        assert!(
            sys.particles().iter().any(|p| p.kind == Kind::Dust),
            "a track on main terrain produced no dust"
        );
        let ground = level.get(track.to).high();
        assert!(
            sys.particles()
                .iter()
                .all(|p| p.pos.z >= ground + SURFACE_LIFT - 0.1),
            "dust spawned in the ground, where the depth test hides it"
        );
        assert!(
            sys.particles()
                .iter()
                .all(|p| p.pos.z <= ground + SURFACE_LIFT + 1.5),
            "dust spawned too high above the wheel"
        );

        let mut water = test_level();
        let bits = water.terrain_bits();
        for meta in water.meta.iter_mut() {
            *meta = bits.write(0);
        }
        let mut sys = System::new();
        sys.from_track(&track, &water);
        assert!(sys.is_empty(), "a track over water produced dust");
    }

    #[test]
    fn fx_lines_are_replaced_each_redraw() {
        let mut particles = System::new();
        particles.emit_dust(Vec3::new(0.0, 0.0, 10.0), 3.0);
        let mut swarm = crate::creature::Swarm::new((64, 64));
        swarm.push(crate::creature::Insect::at(Vec3::new(8.0, 8.0, 10.0), 0));
        let mut lines = LineBuffer::new();
        refresh_fx_lines(&mut lines, &particles, &swarm, Vec3::ZERO, true);
        let n = lines.len();
        assert!(n > 0, "nothing was drawn");
        assert_eq!(
            n % 2,
            0,
            "LineList draws pairs; odd vertex count would drop a tick"
        );
        refresh_fx_lines(&mut lines, &particles, &swarm, Vec3::ZERO, true);
        assert_eq!(
            lines.len(),
            n,
            "a second redraw appended instead of replacing"
        );
    }
}
