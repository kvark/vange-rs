//! Wedge stress test: tip the car onto its side or roof at trees, slopes
//! and cave walls on Fostral, step it the way the server does (20 Hz,
//! QuantClock), and check the player controls right it within seconds.
//! Needs game data; skipped without it.
use glam::{Quat, Vec3};
use vangers::{
    config, level,
    physics::{self, CarPhysicsData, Dynamo},
    space,
};

struct World {
    level: level::Level,
    common: config::common::Common,
    car: CarPhysicsData,
    max_quant: f32,
}

fn world() -> Option<World> {
    let settings = config::Settings::try_load("config/settings.ron")?;
    let ini = settings.data_path.join("thechain/fostral/world.ini");
    if !ini.is_file() {
        return None;
    }
    let reg = config::game::Registry::load(&settings);
    let level = level::load(&level::LevelConfig::load(&ini), &settings.game.geometry);
    let common = config::common::load(settings.open_relative("common.prm"));
    let mut cars = config::car::load_physics_registry(
        &settings.data_path,
        &reg,
        settings.game.physics.shape_sampling,
    );
    let car = cars.remove("OxidizeMonk")?;
    Some(World {
        level,
        common,
        car,
        max_quant: settings.game.physics.max_quant,
    })
}

#[derive(Clone, Copy, Default)]
struct Control {
    motor: f32,
    rudder: f32,
    /// Q/E held: -1 / +1.
    roll: f32,
    jump: Option<f32>,
}

/// Server-side car: 20 Hz ticks cut into whole `max_quant` quanta,
/// controls applied per quantum, jump/roll on the last one.
struct Sim<'a> {
    w: &'a World,
    t: space::Transform,
    d: Dynamo,
    clock: physics::QuantClock,
    roll_time: f32,
}

fn dup(d: &Dynamo) -> Dynamo {
    Dynamo {
        traction: d.traction,
        rudder: d.rudder,
        linear_velocity: d.linear_velocity,
        angular_velocity: d.angular_velocity,
        mole: d.mole,
        flotator: d.flotator,
    }
}

const TICK: f32 = 0.05;

impl<'a> Sim<'a> {
    fn tick(&mut self, c: Control) {
        let w = self.w;
        let common = &w.common;
        let physics_dt = TICK * {
            let n = &common.nature;
            common.speed.standard_frame_rate as f32 * n.time_delta0 * n.num_calls_analysis as f32
        };
        let qi = physics::QuantClock::input_factor(w.max_quant, common);
        // The client's Q/E pulse train.
        let roll = if c.roll != 0.0 {
            let count = (self.roll_time * common.speed.standard_frame_rate as f32).min(100.0) as u8;
            self.roll_time += TICK;
            if count > common.force.side_impulse_delay {
                self.roll_time = 0.0;
            }
            if count < common.force.side_impulse_duration {
                c.roll
            } else {
                0.0
            }
        } else {
            self.roll_time = 0.0;
            0.0
        };
        let quanta = self.clock.advance(physics_dt, w.max_quant);
        for i in 0..quanta {
            let last = i + 1 == quanta;
            if c.rudder != 0.0 {
                let a = self.d.rudder + common.car.rudder_step * 2.0 * qi * c.rudder;
                self.d.rudder = a.clamp(-common.car.rudder_max, common.car.rudder_max);
            }
            if c.motor != 0.0 {
                self.d
                    .change_traction(c.motor * qi * common.car.traction_incr);
            }
            physics::step(
                &mut self.d,
                &mut self.t,
                w.max_quant,
                &w.car,
                &w.level,
                common,
                1.0,
                0.0,
                if last { c.jump } else { None },
                if last { roll } else { 0.0 },
                None,
                None,
            );
        }
    }
    fn up(&self) -> f32 {
        (self.t.rot * Vec3::Z).z
    }
}

#[derive(Clone, Copy, Debug)]
enum Way {
    W,
    WA,
    S,
    Jump,
    AltW,
    Q,
    E,
    R,
}
const WAYS: [Way; 8] = [
    Way::W,
    Way::WA,
    Way::S,
    Way::Jump,
    Way::AltW,
    Way::Q,
    Way::E,
    Way::R,
];

/// Run `way` for up to 5 s; Ok(seconds) once the car is upright and can
/// drive away (W or S moves it 5u in 2 s).
fn recover(start: &Sim, way: Way) -> Result<f32, String> {
    let mut s = Sim {
        w: start.w,
        t: start.t,
        d: dup(&start.d),
        clock: start.clock,
        roll_time: 0.0,
    };
    let max_jump = s.w.common.force.max_jump_power;
    if let Way::R = way {
        s.t.rot = Quat::IDENTITY;
        s.d.linear_velocity = Vec3::ZERO;
        s.d.angular_velocity = Vec3::ZERO;
    }
    let ticks = (5.0 / TICK) as u32;
    let mut upright_at = None;
    for k in 0..ticks {
        let c = match way {
            Way::W => Control {
                motor: 1.0,
                ..Default::default()
            },
            Way::WA => Control {
                motor: 1.0,
                rudder: -1.0,
                ..Default::default()
            },
            Way::S => Control {
                motor: -1.0,
                ..Default::default()
            },
            // Hold Alt ~1 s, release (full charge), again after 2 s.
            Way::Jump => Control {
                jump: (k == 20 || k == 60).then_some(max_jump),
                ..Default::default()
            },
            Way::AltW => Control {
                motor: 1.0,
                jump: (k == 20 || k == 60).then_some(max_jump),
                ..Default::default()
            },
            Way::Q => Control {
                roll: -1.0,
                ..Default::default()
            },
            Way::E => Control {
                roll: 1.0,
                ..Default::default()
            },
            Way::R => Control::default(),
        };
        s.tick(c);
        if s.up() > 0.7 && s.d.angular_velocity.length() < 1.0 {
            upright_at.get_or_insert(k);
        } else {
            upright_at = None;
        }
    }
    let Some(at) = upright_at else {
        return Err(format!("up.z {:.2}", s.up()));
    };
    for motor in [1.0, -1.0] {
        let mut m = Sim {
            w: s.w,
            t: s.t,
            d: dup(&s.d),
            clock: s.clock,
            roll_time: 0.0,
        };
        m.d.traction = 0.0;
        let p0 = m.t.disp;
        for _ in 0..40 {
            m.tick(Control {
                motor,
                ..Default::default()
            });
        }
        if (m.t.disp - p0).truncate().length() > 5.0 && m.up() > 0.5 {
            return Ok(at as f32 * TICK);
        }
    }
    Err(format!("upright but stuck at {:?}", s.t.disp))
}

#[test]
fn every_tipped_pose_recovers() {
    let Some(w) = world() else {
        eprintln!("skipping: no game data");
        return;
    };
    // (x, y, z hint, label)
    let spots: &[(f32, f32, f32, &str)] = &[
        (432.9, 3926.0, 300.0, "freeze trees"),
        (440.0, 3932.0, 300.0, "trees +"),
        (426.0, 3920.0, 300.0, "trees -"),
        (436.0, 3916.0, 300.0, "trees s"),
        (707.0, 4060.0, 20.0, "slope 707,4060"),
        (712.0, 4024.0, 20.0, "slid 712,4024"),
        (682.0, 4097.0, 20.0, "slid 682,4097"),
        (739.5, 3948.0, 20.0, "south cliff"),
        (767.0, 3958.0, 30.0, "ramp by cliff"),
        (750.0, 3944.0, 20.0, "cliff foot"),
        (904.4, 3937.8, 300.0, "green columns"),
        (706.2, 4147.6, 300.0, "underpass mouth"),
        (555.0, 4151.0, 300.0, "spawn"),
        (900.0, 3900.0, 300.0, "hill"),
    ];
    let tilts: &[(f32, &str)] = &[
        (std::f32::consts::FRAC_PI_2, "left side"),
        (-std::f32::consts::FRAC_PI_2, "right side"),
        (std::f32::consts::PI, "roof"),
        (1.0, "57°"),
        (-2.3, "-132°"),
    ];
    let mut failures = vec![];
    let (mut poses, mut self_righted, mut needs_r) = (0, 0, 0);
    for &(x, y, zh, label) in spots {
        for k in 0..4 {
            let yaw = k as f32 * std::f32::consts::FRAC_PI_2 + 0.3;
            for &(tilt, tname) in tilts {
                let floor = w.level.floor_below(Vec3::new(x, y, zh));
                let mut s = Sim {
                    w: &w,
                    t: space::Transform {
                        disp: Vec3::new(x, y, floor + 8.0),
                        rot: Quat::from_rotation_z(yaw) * Quat::from_rotation_y(tilt),
                        scale: w.car.scale,
                    },
                    d: Dynamo::default(),
                    clock: physics::QuantClock::default(),
                    roll_time: 0.0,
                };
                // Drop it and let it come to rest (or fall wherever).
                for _ in 0..80 {
                    s.tick(Control::default());
                }
                poses += 1;
                if s.up() > 0.7 {
                    self_righted += 1;
                    continue;
                }
                let results: Vec<_> = WAYS.iter().map(|&way| (way, recover(&s, way))).collect();
                let any_no_r = results
                    .iter()
                    .any(|(w, r)| !matches!(w, Way::R) && r.is_ok());
                let any = results.iter().any(|(_, r)| r.is_ok());
                if !any_no_r {
                    needs_r += 1;
                }
                let line = format!(
                    "{label} yaw {yaw:.2} {tname}: rest {:?} up {:.2} | {}",
                    s.t.disp,
                    s.up(),
                    results
                        .iter()
                        .map(|(w, r)| match r {
                            Ok(t) => format!("{w:?} {t:.1}s"),
                            Err(e) => format!("{w:?} x({e})"),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                println!("{line}");
                if !any {
                    failures.push(line);
                }
            }
        }
    }
    // The live freeze pose (see `net_physics::wedged_car_escapes_at_client_max_quant`).
    let wedged = Sim {
        w: &w,
        t: space::Transform {
            disp: Vec3::new(432.90, 3925.99, 61.04),
            rot: Quat::from_xyzw(-0.227, -0.368, -0.620, 0.655).normalize(),
            scale: w.car.scale,
        },
        d: Dynamo::default(),
        clock: physics::QuantClock::default(),
        roll_time: 0.0,
    };
    let freeze: Vec<_> = WAYS
        .iter()
        .map(|&way| (way, recover(&wedged, way)))
        .collect();
    println!("live freeze pose: {freeze:?}");
    assert!(
        matches!(
            freeze.iter().find(|(w, _)| matches!(w, Way::AltW)),
            Some((_, Ok(_)))
        ),
        "Alt+W should free the live freeze pose"
    );
    println!(
        "{poses} poses, {self_righted} righted themselves, {needs_r} needed R, {} unrecoverable",
        failures.len()
    );
    assert!(
        failures.is_empty(),
        "unrecoverable:\n{}",
        failures.join("\n")
    );
}
