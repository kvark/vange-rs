//! Hordes, sky-farmers (Glorx), fish (Weexow), and Hmok's clef.

use crate::level::Level;
use crate::level::vlc;
use crate::render::debug::LineBuffer;

use glam::Vec3;

use std::path::Path;

/// `MAX_HORDE_SOURCE_OBJECT`.
const MAX_HIVES: usize = 32;
/// `MAX_HORDE_OBJECT` free-flying clouds.
const MAX_CLOUDS: usize = 50;
/// Drawn motes per cloud. Original `HORDE_PARTICLE_NUM` is 200; each is
/// a pixel, so a few dozen ticks already read as a swarm.
const MOTES: usize = 64;
/// Original `HordeSource` radius and `HORDE_RADIUS_DELTA`.
const SWARM_RADIUS: f32 = 20.0;
/// `MAX_FISH_WARRIOR`.
const MAX_FISH: usize = 32;
/// `HordeSource` radius.
const HIVE_RADIUS: f32 = 20.0;
/// `AttackRadius`.
const ATTACK_RADIUS: f32 = 200.0;
const CLOUD_SPEED: f32 = 2.5;
const FARMER_SPEED: f32 = 3.5;
const FISH_SPEED: f32 = 2.0;
const FARMER_ALT: f32 = 48.0;
const HIVE_COLOR: u32 = 0xFF22_44CC;
const MOTE_COLOR: u32 = 0xFF10_1010;
const FARMER_COLOR: u32 = 0xFF66_CC44;
const FISH_COLOR: u32 = 0xFFCC_8844;
const CLEF_COLOR: u32 = 0xFF88_44AA;

/// What a crush or a nibble did this quant.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Life {
    /// Armour taken off the player.
    pub nibble: u16,
    /// Where a hive burst, for a particle puff.
    pub bursts: Vec<Vec3>,
}

/// All of one world's extra fauna.
pub struct Fauna {
    hives: Vec<Hive>,
    hordes: Vec<Horde>,
    farmers: Vec<Farmer>,
    fish: Vec<Fish>,
    clefs: Vec<Clef>,
    waypoints: Vec<(f32, f32)>,
    size: (i32, i32),
    seed: u32,
}

struct Hive {
    pos: Vec3,
}

struct Horde {
    pos: Vec3,
    home: Vec3,
    motes: Vec<Vec3>,
    vel: Vec3,
}

struct Farmer {
    pos: Vec3,
    heading: f32,
    waypoint: usize,
    dir: i32,
    kernoboo: bool,
}

struct Fish {
    pos: Vec3,
    heading: f32,
    target: Vec3,
}

struct Clef {
    pos: Vec3,
}

impl Fauna {
    pub fn spawn(world: &str, level: &Level, data_path: &Path) -> Self {
        let mut fauna = Fauna {
            hives: Vec::new(),
            hordes: Vec::new(),
            farmers: Vec::new(),
            fish: Vec::new(),
            clefs: Vec::new(),
            waypoints: Vec::new(),
            size: level.size,
            seed: 0x0F0A_FA00,
        };
        let world = world.to_ascii_lowercase();
        match world.as_str() {
            "fostral" | "glorx" | "necros" | "necross" | "xplo" => {
                fauna.scatter_hives(MAX_HIVES, level);
            }
            "weexow" | "arkonoy" => {
                fauna.scatter_clouds(MAX_CLOUDS, level);
            }
            _ => {}
        }
        if world == "weexow" {
            fauna.scatter_fish(MAX_FISH, level);
        }
        if world == "hmok" {
            fauna.place_clefs(level);
        }
        if world_has_sky_farmers(&world) {
            fauna.place_farmers(data_path, level);
        }
        fauna
    }

    pub fn hives(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.hives.iter().map(|h| h.pos)
    }

    pub fn farmers(&self) -> impl Iterator<Item = (Vec3, f32, bool)> + '_ {
        self.farmers.iter().map(|f| (f.pos, f.heading, f.kernoboo))
    }

    pub fn fish(&self) -> impl Iterator<Item = (Vec3, f32)> + '_ {
        self.fish.iter().map(|f| (f.pos, f.heading))
    }

    pub fn clefs(&self) -> impl Iterator<Item = Vec3> + '_ {
        self.clefs.iter().map(|c| c.pos)
    }

    pub fn shift(&mut self, delta: Vec3) {
        for h in &mut self.hives {
            h.pos -= delta;
        }
        for h in &mut self.hordes {
            h.pos -= delta;
            h.home -= delta;
            for m in &mut h.motes {
                *m -= delta;
            }
        }
        for f in &mut self.farmers {
            f.pos -= delta;
        }
        for f in &mut self.fish {
            f.pos -= delta;
            f.target -= delta;
        }
        for c in &mut self.clefs {
            c.pos -= delta;
        }
    }

    /// Step, crush hives under the car or a shot, and nibble airborne prey.
    pub fn quant(
        &mut self,
        level: &Level,
        player: Vec3,
        player_radius: f32,
        airborne: bool,
        shots: &[Vec3],
    ) -> Life {
        let mut life = Life::default();
        let size = self.size;
        self.crush_hives(player, player_radius, shots, &mut life);
        for horde in &mut self.hordes {
            horde.quant(level, player, airborne, size, &mut self.seed);
            if wrap_dist(horde.pos, player, size) < player_radius + 16.0 {
                life.nibble = life.nibble.saturating_add(1);
            }
        }
        for farmer in &mut self.farmers {
            farmer.quant(level, size, &self.waypoints);
        }
        for fish in &mut self.fish {
            fish.quant(level, player, size, &mut self.seed);
        }
        self.clefs.retain(|c| {
            !shots.iter().any(|s| {
                let d = wrap_dist(*s, c.pos, size);
                d < 24.0
            })
        });
        life
    }

    pub fn draw_ticks(&self, lines: &mut LineBuffer, eye: Vec3) {
        let size = self.size;
        let r2 = 900.0 * 900.0;
        for hive in &self.hives {
            if wrap_dist2(hive.pos, eye, size) > r2 {
                continue;
            }
            mark(lines, hive.pos, 10.0, HIVE_COLOR);
        }
        for horde in &self.hordes {
            if wrap_dist2(horde.pos, eye, size) > r2 {
                continue;
            }
            for mote in &horde.motes {
                fly(lines, *mote, MOTE_COLOR);
            }
        }
        for farmer in &self.farmers {
            if wrap_dist2(farmer.pos, eye, size) > r2 {
                continue;
            }
            mark(lines, farmer.pos, 14.0, FARMER_COLOR);
        }
        for fish in &self.fish {
            if wrap_dist2(fish.pos, eye, size) > r2 {
                continue;
            }
            mark(lines, fish.pos, 10.0, FISH_COLOR);
        }
        for clef in &self.clefs {
            mark(lines, clef.pos, 12.0, CLEF_COLOR);
        }
    }

    fn scatter_hives(&mut self, count: usize, level: &Level) {
        for _ in 0..count {
            let pos = self.rand_ground(level, 8.0);
            self.hives.push(Hive { pos });
        }
    }

    fn scatter_clouds(&mut self, count: usize, level: &Level) {
        for _ in 0..count {
            let home = self.rand_ground(level, 40.0);
            self.hordes.push(Horde::at(home));
        }
    }

    fn scatter_fish(&mut self, count: usize, level: &Level) {
        for _ in 0..count {
            let pos = self.rand_ground(level, 4.0);
            let heading = self.unit() * std::f32::consts::TAU;
            self.fish.push(Fish {
                heading,
                target: pos,
                pos,
            });
        }
    }

    fn place_clefs(&mut self, level: &Level) {
        // `ClefObject::CreateClef` on Hmok.
        let spots = [(808, 1630), (699, 1785), (1188, 1843)];
        let pick = spots[self.rng(spots.len() as u32) as usize];
        let z = level.get(pick).high() + 4.0;
        self.clefs.push(Clef {
            pos: Vec3::new(pick.0 as f32, pick.1 as f32, z),
        });
    }

    fn place_farmers(&mut self, data_path: &Path, level: &Level) {
        let path = data_path.join("resource/crypts/skyfarmer.vlc");
        let Ok(bytes) = std::fs::read(&path) else {
            return;
        };
        let waypoints = vlc::load_sensors_from_bytes(&bytes, "skyfarmer.vlc");
        if waypoints.len() < 3 {
            return;
        }
        self.waypoints = waypoints
            .iter()
            .map(|s| (s.pos.0 as f32, s.pos.1 as f32))
            .collect();
        let n = (self.waypoints.len() / 4).clamp(1, 6);
        let last = self.waypoints.len() - 1;
        for i in 0..n {
            let wp = (1 + i * last / n.max(1)).clamp(1, last.saturating_sub(1));
            let (x, y) = self.waypoints[wp];
            let z = level.get((x as i32, y as i32)).high() + FARMER_ALT;
            self.farmers.push(Farmer {
                pos: Vec3::new(x, y, z),
                heading: 0.0,
                waypoint: wp,
                dir: if i % 2 == 0 { 1 } else { -1 },
                kernoboo: i % 2 == 0,
            });
        }
    }

    fn crush_hives(&mut self, player: Vec3, player_radius: f32, shots: &[Vec3], life: &mut Life) {
        let size = self.size;
        let reach = player_radius + HIVE_RADIUS;
        let mut keep = Vec::new();
        for hive in self.hives.drain(..) {
            let hit = wrap_dist(hive.pos, player, size) < reach
                || shots
                    .iter()
                    .any(|s| wrap_dist(*s, hive.pos, size) < HIVE_RADIUS + 8.0);
            if hit {
                life.bursts.push(hive.pos);
                self.hordes.push(Horde::at(hive.pos));
            } else {
                keep.push(hive);
            }
        }
        self.hives = keep;
    }

    fn rand_ground(&mut self, level: &Level, lift: f32) -> Vec3 {
        let x = self.rng(level.size.0.max(1) as u32) as f32;
        let y = self.rng(level.size.1.max(1) as u32) as f32;
        let z = level.get((x as i32, y as i32)).high() + lift;
        Vec3::new(x, y, z)
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

impl Horde {
    fn at(home: Vec3) -> Self {
        let mut motes = Vec::with_capacity(MOTES);
        for i in 0..MOTES {
            motes.push(home + mote_offset(i, 0.0));
        }
        Horde {
            pos: home,
            home,
            motes,
            vel: Vec3::ZERO,
        }
    }

    fn quant(
        &mut self,
        level: &Level,
        player: Vec3,
        airborne: bool,
        size: (i32, i32),
        seed: &mut u32,
    ) {
        let goal = if airborne && wrap_dist(self.pos, player, size) < ATTACK_RADIUS {
            player
        } else {
            self.home
        };
        let to = Vec3::new(
            wrap_delta(goal.x - self.pos.x, size.0 as f32),
            wrap_delta(goal.y - self.pos.y, size.1 as f32),
            goal.z - self.pos.z,
        );
        let d = to.length();
        if d > 1.0 {
            self.vel = self.vel * 0.7 + to / d * CLOUD_SPEED * 0.3;
            let speed = self.vel.length();
            if speed > CLOUD_SPEED {
                self.vel *= CLOUD_SPEED / speed;
            }
        }
        self.pos += self.vel;
        let ground = level.get((self.pos.x as i32, self.pos.y as i32)).high() + 12.0;
        if self.pos.z < ground {
            self.pos.z = ground;
        }
        let phase = (*seed as f32) * 0.03;
        for (i, mote) in self.motes.iter_mut().enumerate() {
            *mote = self.pos + mote_offset(i, phase);
        }
        *seed = seed.wrapping_add(1);
    }
}

impl Farmer {
    fn quant(&mut self, level: &Level, size: (i32, i32), waypoints: &[(f32, f32)]) {
        if waypoints.len() < 2 {
            return;
        }
        let last = waypoints.len() - 1;
        let (tx, ty) = waypoints[self.waypoint.clamp(0, last)];
        let dx = wrap_delta(tx - self.pos.x, size.0 as f32);
        let dy = wrap_delta(ty - self.pos.y, size.1 as f32);
        if dx * dx + dy * dy < 80.0 * 80.0 {
            let next = self.waypoint as i32 + self.dir;
            if next <= 0 {
                self.dir = 1;
                self.waypoint = 1;
            } else if next as usize >= last {
                self.dir = -1;
                self.waypoint = last.saturating_sub(1);
            } else {
                self.waypoint = next as usize;
            }
        }
        self.heading = (-dx).atan2(dy);
        let fwd = glam::Quat::from_rotation_z(self.heading) * Vec3::Y;
        self.pos.x += fwd.x * FARMER_SPEED;
        self.pos.y += fwd.y * FARMER_SPEED;
        self.pos.z = level.get((self.pos.x as i32, self.pos.y as i32)).high() + FARMER_ALT;
    }
}

impl Fish {
    fn quant(&mut self, level: &Level, player: Vec3, size: (i32, i32), seed: &mut u32) {
        if wrap_dist(self.pos, self.target, size) < 24.0 {
            let span = 400.0;
            self.target = Vec3::new(
                self.pos.x + ((*seed % 1000) as f32 / 1000.0 - 0.5) * span,
                self.pos.y + (((*seed / 7) % 1000) as f32 / 1000.0 - 0.5) * span,
                self.pos.z,
            );
            *seed = seed.wrapping_add(13);
            let _ = player;
        }
        let dx = wrap_delta(self.target.x - self.pos.x, size.0 as f32);
        let dy = wrap_delta(self.target.y - self.pos.y, size.1 as f32);
        self.heading = (-dx).atan2(dy);
        let fwd = glam::Quat::from_rotation_z(self.heading) * Vec3::Y;
        self.pos.x += fwd.x * FISH_SPEED;
        self.pos.y += fwd.y * FISH_SPEED;
        self.pos.z = level.get((self.pos.x as i32, self.pos.y as i32)).high() + 6.0;
    }
}

fn world_has_sky_farmers(world: &str) -> bool {
    world == "glorx"
}

/// Original `HordeObject::DrawQuant` plots 200 pixels around the carrier.
/// A small 3-axis tick is the same idea in the debug line buffer.
fn fly(lines: &mut LineBuffer, p: Vec3, c: u32) {
    let s = 1.6;
    lines.add([p.x - s, p.y, p.z], [p.x + s, p.y, p.z], c);
    lines.add([p.x, p.y - s, p.z], [p.x, p.y + s, p.z], c);
    lines.add([p.x, p.y, p.z - s], [p.x, p.y, p.z + s], c);
}

fn mote_offset(i: usize, phase: f32) -> Vec3 {
    let t = i as f32 / MOTES as f32;
    let a = t * std::f32::consts::TAU * 7.0 + phase;
    let lift = (t * 2.0 - 1.0) * 0.85;
    let ring = (1.0 - lift * lift).max(0.05).sqrt();
    let r = SWARM_RADIUS * (0.35 + 0.65 * ((i * 3) % 7) as f32 / 6.0);
    Vec3::new(a.cos() * ring * r, a.sin() * ring * r, lift * r * 0.55)
}

fn mark(lines: &mut LineBuffer, p: Vec3, s: f32, c: u32) {
    let z = p.z + 0.5;
    lines.add([p.x - s, p.y, z], [p.x + s, p.y, z], c);
    lines.add([p.x, p.y - s, z], [p.x, p.y + s, z], c);
    lines.add([p.x, p.y, z], [p.x, p.y, z + s], c);
}

fn wrap_delta(d: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return d;
    }
    let half = span * 0.5;
    (d + half).rem_euclid(span) - half
}

fn wrap_dist(a: Vec3, b: Vec3, size: (i32, i32)) -> f32 {
    wrap_dist2(a, b, size).sqrt()
}

fn wrap_dist2(a: Vec3, b: Vec3, size: (i32, i32)) -> f32 {
    let dx = wrap_delta(a.x - b.x, size.0 as f32);
    let dy = wrap_delta(a.y - b.y, size.1 as f32);
    dx * dx + dy * dy
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
    fn worlds_get_the_right_fauna() {
        let level = test_level();
        let f = Fauna::spawn("Fostral", &level, Path::new("/no/such"));
        assert_eq!(f.hives.len(), MAX_HIVES);
        assert!(f.fish.is_empty() && f.farmers.is_empty());
        let w = Fauna::spawn("Weexow", &level, Path::new("/no/such"));
        assert_eq!(w.hordes.len(), MAX_CLOUDS);
        assert_eq!(w.fish.len(), MAX_FISH);
        assert_eq!(
            Fauna::spawn("hmok", &level, Path::new("/no/such"))
                .clefs
                .len(),
            1
        );
        assert!(world_has_sky_farmers("glorx"));
        assert!(!world_has_sky_farmers("fostral"));
    }

    #[test]
    fn crushing_a_hive_releases_a_horde() {
        let level = test_level();
        let mut fauna = Fauna::spawn("Fostral", &level, Path::new("/no/such"));
        fauna.hives.truncate(1);
        let at = fauna.hives[0].pos;
        let life = fauna.quant(&level, at, 8.0, false, &[]);
        assert_eq!(life.bursts.len(), 1);
        assert_eq!(fauna.hordes.len(), 1);
        assert_eq!(fauna.hordes[0].motes.len(), MOTES);
    }
}
