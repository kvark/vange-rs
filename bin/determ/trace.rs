//! A scripted solo tumble through the shared car physics, recorded bit
//! for bit: shared by the `determ` binary (built native and as wasm) and
//! the `determinism` test.

use glam::{Quat, Vec3};
use std::io::Cursor;
use vangers::{
    config::{self, settings},
    level,
    physics::{
        CarPhysicsData, Dynamo, QuantClock,
        prediction::{QuantInput, fold_home, step_input},
    },
    space,
    vfs::Vfs,
};

pub struct World {
    pub level: level::Level,
    pub common: config::common::Common,
    pub car: CarPhysicsData,
    pub max_quant: f32,
}

/// The settings the web build embeds (and the server's template).
pub fn settings() -> settings::Settings {
    ron::de::from_str(include_str!("../../config/settings.template.ron"))
        .expect("settings template")
}

impl World {
    /// The level, `common.prm` and the first main car out of the release
    /// archives, the way the web build loads them.
    pub fn from_zips(zips: &[&[u8]]) -> Self {
        let mut vfs = Vfs::new();
        for z in zips {
            vfs.mount_zip(z).expect("zip");
        }
        let settings = settings();
        let level_config = level::LevelConfig::load_from_vfs(&vfs, "world.ini");
        let level = level::load_from_vfs(&vfs, &level_config, &settings.game.geometry);
        let common = config::common::load_reader(Cursor::new(&*vfs.read("common.prm").unwrap()));
        let car = car_from_vfs(&vfs, settings.game.physics.shape_sampling);
        World {
            level,
            common,
            car,
            max_quant: settings.game.physics.max_quant,
        }
    }

    /// The procedural test level and the test car, no data needed.
    pub fn test() -> Self {
        let settings = settings();
        World {
            level: level::load(&level::LevelConfig::new_test(), &settings.game.geometry),
            common: config::common::Common::test_default(),
            car: CarPhysicsData::test_default(),
            max_quant: settings.game.physics.max_quant,
        }
    }

    /// Quanta per second of play.
    pub fn quanta_per_sec(&self) -> f32 {
        let n = &self.common.nature;
        self.common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
            / self.max_quant
    }

    /// One quantum, as the server and the predicting client run it.
    pub fn step(&self, transform: &mut space::Transform, dynamo: &mut Dynamo, input: &QuantInput) {
        step_input(
            dynamo,
            transform,
            input,
            self.max_quant,
            QuantClock::input_factor(self.max_quant, &self.common),
            &self.car,
            &self.level,
            &self.common,
            None,
        );
        fold_home(&mut transform.disp, self.level.period());
    }

    /// FNV-1a of the terrain the physics reads.
    pub fn level_hash(&self) -> u64 {
        let mut h = Fnv::default();
        h.bytes(&self.level.height);
        h.bytes(&self.level.meta);
        h.0
    }
}

/// The web build's `CarLib`: first main car of `car.prm`, its `.prm`
/// next to the model or `default.prm`.
fn car_from_vfs(vfs: &Vfs, shape_sampling: u8) -> CarPhysicsData {
    let registry =
        config::game::Registry::load_reader(Cursor::new(&*vfs.read("game.lst").unwrap()));
    let entries = config::car::entries(Cursor::new(&*vfs.read("car.prm").unwrap()));
    let (name, _, _) = entries
        .iter()
        .find(|(_, kind, _)| matches!(kind, config::car::Kind::Main))
        .expect("a main car");
    let info = &registry.model_infos[name];
    let m3d = vfs.read(&info.path).unwrap();
    let prm_path = std::path::Path::new(&info.path).with_extension("prm");
    let (prm, is_default) = match vfs.read(prm_path.to_str().unwrap()) {
        Some(b) => (b, false),
        None => {
            let mut p = std::path::Path::new(&info.path).to_path_buf();
            p.set_file_name("default.prm");
            (vfs.read(p.to_str().unwrap()).unwrap(), true)
        }
    };
    let physics = config::car::CarPhysics::load_reader(Cursor::new(&*prm));
    let scale = if is_default {
        info.scale
    } else {
        physics.scale_size
    };
    CarPhysicsData::from_bytes(&m3d, &prm, scale, shape_sampling)
}

#[derive(Clone, Copy)]
pub struct Fnv(pub u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    pub fn bytes(&mut self, b: &[u8]) {
        for &x in b {
            self.0 ^= x as u64;
            self.0 = self.0.wrapping_mul(0x100_0000_01b3);
        }
    }
    pub fn f32s(&mut self, v: &[f32]) {
        for x in v {
            self.bytes(&x.to_bits().to_le_bytes());
        }
    }
}

/// The scripted inputs: settle, then full motor with turbo, the wheel
/// swung every 15 quanta, a full jump every 60, a roll now and then and
/// a stretch of braking every 100.
pub fn script(q: u32) -> QuantInput {
    if q < 20 {
        return QuantInput::default();
    }
    const RUDDER: [f32; 6] = [0.0, 1.0, 0.0, -1.0, 1.0, -1.0];
    // Off the throttle and on the brake for the last 10 of every 100.
    let brake = q % 100 >= 90;
    QuantInput {
        motor: if brake { 0.0 } else { 1.0 },
        rudder: RUDDER[(q / 15) as usize % RUDDER.len()],
        brake,
        turbo: true,
        jump: (q % 60 == 59).then_some(1.0),
        roll: if q % 90 == 45 { 1.0 } else { 0.0 },
    }
}

/// One quantum of the record: position, rotation, velocities.
pub type Pose = [f32; 13];

pub fn pose(t: &space::Transform, d: &Dynamo) -> Pose {
    let r = t.rot;
    [
        t.disp.x,
        t.disp.y,
        t.disp.z,
        r.x,
        r.y,
        r.z,
        r.w,
        d.linear_velocity.x,
        d.linear_velocity.y,
        d.linear_velocity.z,
        d.angular_velocity.x,
        d.angular_velocity.y,
        d.angular_velocity.z,
    ]
}

/// Drop the car at `at` (XY), let it settle, and run the script for
/// `quanta`; the pose after every quantum.
pub fn tumble(world: &World, at: (i32, i32), quanta: u32) -> Vec<Pose> {
    let mut t = space::Transform {
        disp: Vec3::new(at.0 as f32, at.1 as f32, world.level.get(at).high() + 5.0),
        rot: Quat::IDENTITY,
        scale: world.car.scale,
    };
    let mut d = Dynamo::default();
    (0..quanta)
        .map(|q| {
            world.step(&mut t, &mut d, &script(q));
            pose(&t, &d)
        })
        .collect()
}

/// The record as text: a header and one line of hex bits per quantum.
pub fn render(world: &World, poses: &[Pose]) -> String {
    let mut out = format!("level {:016x}\n", world.level_hash());
    for (q, p) in poses.iter().enumerate() {
        out += &format!("{q}");
        for x in p {
            out += &format!(" {:08x}", x.to_bits());
        }
        out += "\n";
    }
    out
}
