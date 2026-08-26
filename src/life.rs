//! Beebs, other fauna, and particles. Both the native road build and the
//! web viewer step this once per quant.

use crate::config::common::MAIN_LOOP_TIME;
use crate::creature::Swarm;
use crate::fauna::Fauna;
use crate::level::Level;
use crate::particle::{self, System};
use crate::render::debug::LineBuffer;

use glam::Vec3;

use std::path::Path;

/// Player pose the world's life needs for a quant.
pub struct Contact<'a> {
    pub pos: Vec3,
    pub wheels: &'a [Vec3],
    pub radius: f32,
    pub armor: u16,
    pub max_armor: u16,
}

/// Everything that lives on the surface besides the cars.
pub struct World {
    pub swarm: Swarm,
    pub fauna: Fauna,
    pub particles: System,
    time: f32,
    pub beebs: i32,
}

impl World {
    pub fn spawn(world: &str, level: &Level, data_path: &Path) -> Self {
        let mut swarm = Swarm::new(level.size);
        swarm.populate(Swarm::count_for_size(level.size), level);
        World {
            swarm,
            fauna: Fauna::spawn(world, level, data_path),
            particles: System::new(),
            time: 0.0,
            beebs: 0,
        }
    }

    /// Advance by `delta` seconds, catching up at most four quants.
    pub fn step(&mut self, level: &Level, delta: f32, player: Contact<'_>, shots: &[Vec3]) -> u16 {
        self.time += delta;
        let mut nibble = 0u16;
        let mut catch_up = 0;
        while self.time >= MAIN_LOOP_TIME {
            if catch_up >= 4 {
                self.time = 0.0;
                break;
            }
            catch_up += 1;
            self.time -= MAIN_LOOP_TIME;
            nibble = nibble.saturating_add(self.one_quant(level, &player, shots));
        }
        nibble
    }

    fn one_quant(&mut self, level: &Level, player: &Contact<'_>, shots: &[Vec3]) -> u16 {
        self.swarm.quant(level, Some(player.pos));
        self.particles
            .from_hull(player.pos, player.armor, player.max_armor);
        let crush = self.swarm.crush(player.wheels, 8.0);
        if crush.awarded != 0 {
            self.beebs += crush.awarded;
            for at in crush.at {
                self.particles.from_crush(at);
            }
        }
        let ground = level.get((player.pos.x as i32, player.pos.y as i32)).high();
        let airborne = player.pos.z > ground + 14.0;
        let life = self
            .fauna
            .quant(level, player.pos, player.radius, airborne, shots);
        for at in life.bursts {
            self.particles.from_crush(at);
        }
        self.particles.quant();
        life.nibble
    }

    pub fn shift(&mut self, delta: Vec3) {
        for p in self.particles.particles_mut() {
            p.pos -= delta;
        }
        for insect in self.swarm.insects_mut() {
            insect.pos -= delta;
        }
        self.fauna.shift(delta);
    }

    pub fn draw_fx(&self, lines: &mut LineBuffer, eye: Vec3, insects_as_ticks: bool) {
        particle::refresh_fx_lines(lines, &self.particles, &self.swarm, eye, insects_as_ticks);
        self.fauna.draw_ticks(lines, eye);
    }
}
