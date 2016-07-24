use std::collections::HashMap;
use cgmath;
use cgmath::prelude::*;
use glutin::Event;
use gfx;
use {config, level, model, render};


#[derive(Eq, PartialEq)]
enum Spirit {
    Player,
    //Computer,
}

use config::common::{Traction};
const MAX_TRACTION: Traction = 4.0;

#[derive(Debug)]
struct AccelerationVectors {
    f: cgmath::Vector3<f32>, // linear
    k: cgmath::Vector3<f32>, // angular
}

struct Dynamo {
    traction: Traction,
    _steer: cgmath::Rad<f32>,
    linear_velocity: cgmath::Vector3<f32>,
    angular_velocity: cgmath::Vector3<f32>,
}

impl Dynamo {
    fn change_traction(&mut self, delta: Traction) {
        let old = self.traction;
        self.traction = (old + delta).min(MAX_TRACTION).max(-MAX_TRACTION);
        if old * self.traction < 0.0 {
            self.traction = 0.0; // full stop
        }
    }
}

struct Control {
    motor: i8,
    brake: bool,
    turbo: bool,
}

pub struct Agent<R: gfx::Resources> {
    spirit: Spirit,
    pub transform: super::Transform,
    pub car: config::car::CarInfo<R>,
    dynamo: Dynamo,
    control: Control,
}

#[derive(Debug)]
struct CollisionPoint {
    pos: cgmath::Vector3<f32>,
    depth: f32,
}

#[derive(Debug)]
struct CollisionData {
    soft: Option<CollisionPoint>,
    hard: Option<CollisionPoint>,
}

struct Accumulator {
    pos: cgmath::Vector3<f32>,
    depth: f32,
    count: f32,
}

impl Accumulator {
    fn new() -> Accumulator {
        Accumulator {
            pos: cgmath::vec3(0.0, 0.0, 0.0),
            depth: 0.0,
            count: 0.0,
        }
    }
    fn add(&mut self, pos: cgmath::Vector3<f32>, depth: f32) {
        self.pos += pos;
        self.depth += depth;
        self.count += 1.0;
    }
    fn finish(&self, min: f32) -> Option<CollisionPoint> {
        if self.count > min {
            Some(CollisionPoint {
                pos: self.pos / self.count,
                depth: self.depth / self.count,
            })
        } else { None }
    }
}

fn get_height(altitude: u8) -> f32 {
    altitude as f32 * (level::HEIGHT_SCALE as f32) / 256.0
}

fn collide_low(poly: &model::Polygon, samples: &[[i8; 3]], scale: f32, transform: &super::Transform,
               level: &level::Level, terraconf: &config::common::Terrain) -> CollisionData
{
    let (mut soft, mut hard) = (Accumulator::new(), Accumulator::new());
    for &s in samples[poly.sample_range.0 as usize .. poly.sample_range.1 as usize].iter() {
        let sp = cgmath::Point3::from([s[0] as f32, s[1] as f32, s[2] as f32]);
        let pos = transform.transform_point(sp * scale).to_vec();
        let texel = level.get((pos.x as i32, pos.y as i32));
        let lo_alt = texel.low.0;
        let altitude = match texel.high {
            Some((delta, hi_alt, _))
                if pos.z - get_height(lo_alt + delta) > get_height(hi_alt) - pos.z
                => hi_alt,
            _ => lo_alt,
        };
        let dz = get_height(altitude) - pos.z;
        if dz > terraconf.min_wall_delta {
            hard.add(pos, dz);
        } else if dz > 0.0 {
            soft.add(pos, dz);
        }
    }
    CollisionData {
        soft: (if soft.count > 0.0 { &soft } else { &hard }).finish(0.0),
        hard: hard.finish(1.0), //originally: 4.0
    }
}

fn calc_collision_matrix_inv(r: cgmath::Vector3<f32>, ji: &cgmath::Matrix3<f32>) -> cgmath::Matrix3<f32> {
    let t3  = -r.z * ji[1][1] + r.y * ji[2][1];
    let t7  = -r.z * ji[1][2] + r.y * ji[2][2];
    let t12 = -r.z * ji[1][0] + r.y * ji[2][0];
    let t21 =  r.z * ji[0][1] - r.x * ji[2][1];
    let t25 =  r.z * ji[0][2] - r.x * ji[2][2];
    let t30 =  r.z * ji[0][0] - r.x * ji[2][0];
    let t39 = -r.y * ji[0][1] + r.x * ji[1][1];
    let t43 = -r.y * ji[0][2] + r.x * ji[1][2];
    let t48 = -r.y * ji[0][0] + r.x * ji[1][0];
    let cm = cgmath::Matrix3::new(
        1.0 - t3*r.z + t7*r.y, t12*r.z - t7*r.x, - t12*r.y + t3*r.x,
        - t21*r.z + t25*r.y, 1.0 + t30*r.z - t25*r.x, - t30*r.y + t21*r.x,
        - t39*r.z + t43*r.y, t48*r.z - t43*r.x, 1.0 - t48*r.y + t39*r.x
        );
    cm.invert().unwrap()
}

impl<R: gfx::Resources> Agent<R> {
    fn step(&mut self, dt: f32, level: &level::Level, common: &config::common::Common) {
        if self.control.motor != 0 {
            self.dynamo.change_traction(self.control.motor as f32 * dt * common.car.traction_incr);
        }
        if self.control.brake && self.dynamo.traction != 0.0 {
            self.dynamo.traction *= (config::common::ORIGINAL_FPS as f32 * -dt).exp2();
        }
        let acc_global = AccelerationVectors {
            f: cgmath::vec3(0.0, 0.0, -common.nature.gravity),
            k: cgmath::vec3(0.0, 0.0, 0.0),
        };
        let rot_inv = self.transform.rot.invert();
        let mut acc_cur = AccelerationVectors {
            f: rot_inv.rotate_vector(acc_global.f),
            k: rot_inv.rotate_vector(acc_global.k),
        };
        //println!("cur acc {:?}", acc_cur);
        let flood_level = level.flood_map[0] as f32;
        let z_axis = self.transform.rot.rotate_vector(cgmath::Vector3::unit_z());
        let mut v_vel = self.dynamo.linear_velocity;
        let mut w_vel = self.dynamo.angular_velocity;
        let j_inv = {
            let phys = &self.car.model.body.physics;
            (cgmath::Matrix3::from(phys.jacobi) *
                (self.transform.scale * self.transform.scale / phys.volume))
                .invert().unwrap()
        };

        for _ in 0 .. common.nature.num_calls_analysis {
            let mut float_count = 0;
            let (mut terrain_immersion, mut water_immersion) = (0.0, 0.0);
            let stand_on_wheels = z_axis.z > 0.0 && self.transform.rot.rotate_vector(
                cgmath::Vector3::unit_x()).z < 0.7;
            let modulation = 1.0;
            let mut hilow_diff = 0i32;

            // apply drag
            let mut v_drag = common.drag.free.v * common.drag.speed.v.powf(v_vel.magnitude());
            let mut w_drag = common.drag.free.w * common.drag.speed.w.powf(w_vel.magnitude2()); //why mag2?
            if stand_on_wheels { //TODO: why `ln()`? //TODO: wheels_touch
                let speed = common.drag.wheel_speed.ln() * self.car.physics.mobility_factor *
                    common.global.speed_factor / self.car.physics.speed_factor;
                v_vel.y *= (1.0 + speed).powf(config::common::SPEED_CORRECTION_FACTOR);
            }
            let (v_old, w_old) = (v_vel, w_vel);

            for poly in self.car.model.shape.polygons.iter() {
                let r = cgmath::Vector3::from(poly.middle) *
                    (self.transform.scale * self.car.physics.scale_bound);
                let rglob = self.transform.transform_point(
                    cgmath::Point3::from(poly.middle) * self.car.physics.scale_bound);
                let vr = v_old + w_old.cross(r);
                let mostly_horisontal = vr.z*vr.z < vr.x*vr.x + vr.y*vr.y;
                let texel = level.get((rglob.x as i32, rglob.y as i32));
                if texel.low.1 == level::TerrainType::Water {
                    let dz = flood_level - rglob.z;
                    if dz > 0.0 {
                        float_count += 1;
                        water_immersion += dz;
                    }
                }
                let poly_norm = cgmath::Vector3::from(poly.normal).normalize();
                if z_axis.dot(poly_norm) < 0.0 {
                    let cdata = collide_low(poly, &self.car.model.shape.samples,
                        self.car.physics.scale_bound, &self.transform, level, &common.terrain);
                    terrain_immersion += match cdata.soft {
                        Some(ref cp) => cp.depth.abs(),
                        None => 0.0,
                    };
                    terrain_immersion += match cdata.hard {
                        Some(ref cp) => cp.depth.abs(),
                        None => 0.0,
                    };
                    let origin = self.transform.disp;
                    match cdata {
                        CollisionData{ hard: Some(ref cp), ..} if mostly_horisontal => {
                            let r1 = rot_inv.rotate_vector(cgmath::vec3(
                                cp.pos.x - origin.x, cp.pos.y - origin.y, 0.0)); // ignore vertical
                            let normal = {
                                let bm = self.car.model.body.bbox.1;
                                let n = cgmath::vec3(r1.x / bm[0], r1.y / bm[1], r1.z / bm[2]);
                                n.normalize()
                            };
                            let u0 = v_old + w_old.cross(r1);
                            let dot = u0.dot(normal);
                            if dot > 0.0 {
                                let pulse = (calc_collision_matrix_inv(r1, &j_inv) * normal) *
                                    (-common.impulse.factors[0] * modulation * dot);
                                println!("Collision speed {:?} pulse {:?}", v_vel, pulse);
                                v_vel += pulse;
                                w_vel += j_inv * r1.cross(pulse);
                            }
                            hilow_diff += 1;
                        },
                        CollisionData{ soft: Some(ref cp), ..} => {
                            let r1 = rot_inv.rotate_vector(cgmath::vec3(
                                cp.pos.x - origin.x, cp.pos.y - origin.y,
                                self.transform.rot.rotate_vector(r).z));
                            let mut u0 = v_old + w_old.cross(r1);
                            if u0.dot(z_axis) < 0.0 {
                                if stand_on_wheels { // ignore XY
                                    u0.x = 0.0;
                                    u0.y = 0.0;
                                } else {
                                    let kn = u0.dot(poly_norm) * (1.0 - common.impulse.k_friction);
                                    u0 = u0 * common.impulse.k_friction + poly_norm * kn;
                                }
                                let cmi = calc_collision_matrix_inv(r, &j_inv);
                                let pulse = (cmi * u0) * (-common.impulse.factors[1] * modulation);
                                println!("Collision momentum {:?}\n\tmatrix {:?}\n\tsample {:?}\n\tspeed {:?}\n\tpulse {:?}", u0, cmi, r, v_vel, pulse);
                                v_vel += pulse;
                                w_vel += j_inv * r.cross(pulse);
                            }
                            hilow_diff += 1;
                        }
                        _ => (),
                    }
                }
            }

            let _ = (float_count, water_immersion, terrain_immersion); //TODO
            if stand_on_wheels {
                let f_traction_per_wheel =
                    self.car.physics.mobility_factor * common.global.mobility_factor *
                    if self.control.turbo { common.global.k_traction_turbo } else { 1.0 } *
                    self.dynamo.traction / (self.car.model.wheels.len() as f32);
                for wheel in self.car.model.wheels.iter() {
                    let mut pos = cgmath::Vector3::from(wheel.pos) * self.transform.scale;
                    pos.x = pos.x.signum() * self.car.model.body.bbox.1[0]; // why?
                    acc_cur.f.y += f_traction_per_wheel;
                    if self.control.brake {
                        let vw = v_vel + w_vel.cross(pos);
                        acc_cur.f -= vw * common.global.f_brake_max;
                    }
                }
            }
            if stand_on_wheels {
                acc_cur.k -= common.nature.gravity * z_axis.cross(cgmath::Vector3::new(0.0, 0.0, 
                    self.car.physics.z_offset_of_mass_center * self.transform.scale));
            }
            v_vel += acc_cur.f * dt;
            w_vel += (j_inv * acc_cur.k) * dt;
            let (v_mag, w_mag) = (v_vel.magnitude(), w_vel.magnitude());
            if stand_on_wheels && v_mag < common.drag.abs_min.v && w_mag < common.drag.abs_min.w {
                v_drag *= common.drag.coll.v.powf(common.drag.abs_min.v / v_mag.max(0.001));
                w_drag *= common.drag.coll.w.powf(common.drag.abs_min.w / w_mag.max(0.001));
            }
            if v_mag * v_drag > common.drag.abs_stop.v || w_mag * w_drag > common.drag.abs_stop.w {
                let vs = v_vel - (hilow_diff.signum() as f32) *
                    (z_axis * (self.car.model.body.bbox.2 * common.impulse.rolling_scale))
                    .cross(w_vel);
                let rot = cgmath::Quaternion::from_axis_angle(w_vel / w_mag.max(0.01),
                    cgmath::Deg::new(w_mag * dt).into());
                self.transform.disp += self.transform.rot.rotate_vector(vs) * dt;
                self.transform.rot = self.transform.rot * rot.invert();
                v_vel = rot.rotate_vector(v_vel);
                w_vel = rot.rotate_vector(v_vel);
            }
            v_vel *= v_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
            w_vel *= w_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
        }

        self.dynamo.linear_velocity  = v_vel;
        self.dynamo.angular_velocity = w_vel;
        // slow down
        let traction_step = -self.dynamo.traction.signum() * dt;
        self.dynamo.change_traction(traction_step * common.car.traction_decr);
    }
}

struct DataBase<R: gfx::Resources> {
    cars: HashMap<String, config::car::CarInfo<R>>,
    common: config::common::Common,
    _game: config::game::Registry,
}

pub struct Game<R: gfx::Resources> {
    db: DataBase<R>,
    render: render::Render<R>,
    level: level::Level,
    agents: Vec<Agent<R>>,
    cam: super::Camera,
}

impl<R: gfx::Resources> Game<R> {
    pub fn new<F: gfx::Factory<R>>(settings: &config::Settings,
           out_color: gfx::handle::RenderTargetView<R, render::ColorFormat>,
           out_depth: gfx::handle::DepthStencilView<R, render::DepthFormat>,
           factory: &mut F) -> Game<R>
    {
        info!("Loading world parameters");
        let db = {
            let game = config::game::Registry::load(settings);
            DataBase {
                cars: config::car::load_registry(settings, &game, factory),
                common: config::common::load(settings.open("common.prm")),
                _game: game,
            }
        };
        let lev_config = settings.get_level();
        let level = level::load(&lev_config);
        let pal_data = level::load_palette(&settings.get_object_palette_path());
        let car = db.cars[&settings.car.id].clone();

        let agent = Agent {
            spirit: Spirit::Player,
            transform: cgmath::Decomposed {
                scale: car.scale,
                disp: cgmath::vec3(0.0, 0.0, 40.0),
                rot: cgmath::One::one(),
            },
            car: car,
            dynamo: Dynamo {
                traction: 0.0,
                _steer: cgmath::Zero::zero(),
                linear_velocity: cgmath::vec3(0.0, 0.0, 0.0),
                angular_velocity: cgmath::vec3(0.0, 0.0, 0.0),
            },
            control: Control {
                motor: 0,
                brake: false,
                turbo: false,
            },
        };

        Game {
            db: db,
            render: render::init(factory, out_color, out_depth, &level, &pal_data),
            level: level,
            agents: vec![agent],
            cam: super::Camera {
                loc: cgmath::vec3(0.0, 0.0, 200.0),
                rot: cgmath::Quaternion::new(1.0, 0.0, 0.0, 0.0),
                proj: cgmath::PerspectiveFov {
                    fovy: cgmath::deg(45.0).into(),
                    aspect: settings.get_screen_aspect(),
                    near: 10.0,
                    far: 10000.0,
                },
            },
        }
    }

    fn _move_cam(&mut self, step: f32) {
        let mut back = self.cam.rot.rotate_vector(cgmath::Vector3::unit_z());
        back.z = 0.0;
        self.cam.loc -= back.normalize() * step;
    }
}

impl<R: gfx::Resources> super::App<R> for Game<R> {
    fn update<I: Iterator<Item=Event>, F: gfx::Factory<R>>(&mut self,
              events: I, delta: f32, factory: &mut F) -> bool {
        use glutin::VirtualKeyCode as Key;
        use glutin::ElementState::*;

        let pid = match self.agents.iter().position(|a| a.spirit == Spirit::Player) {
            Some(pos) => pos,
            None => return false,
        };
        for event in events {
            match event {
                Event::KeyboardInput(Pressed, _, Some(Key::Escape)) |
                Event::Closed => return false,
                Event::KeyboardInput(Pressed, _, Some(Key::L)) => self.render.reload(factory),
                /*
                Event::KeyboardInput(_, _, Some(Key::R)) =>
                    self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), angle),
                Event::KeyboardInput(_, _, Some(Key::F)) =>
                    self.cam.rot = self.cam.rot * cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_x(), -angle),
                */
                Event::KeyboardInput(Pressed, _, Some(Key::W)) => self.agents[pid].control.motor = 1,
                Event::KeyboardInput(Pressed, _, Some(Key::S)) => self.agents[pid].control.motor = -1,
                Event::KeyboardInput(Released, _, Some(Key::W)) | Event::KeyboardInput(Released, _, Some(Key::S)) =>
                    self.agents[pid].control.motor = 0,
                /*
                Event::KeyboardInput(Pressed, _, Some(Key::A)) => self.dyn_target.steer = cgmath::Rad::new(-0.2),
                Event::KeyboardInput(Pressed, _, Some(Key::D)) => self.dyn_target.steer = cgmath::Rad::new(0.2),
                Event::KeyboardInput(Released, _, Some(Key::A)) | Event::KeyboardInput(Released, _, Some(Key::D)) =>
                    self.dyn_target.steer = cgmath::Rad::new(0.0),
                */
                /*
                Event::KeyboardInput(_, _, Some(Key::W)) => self.move_cam(step),
                Event::KeyboardInput(_, _, Some(Key::S)) => self.move_cam(-step),
                Event::KeyboardInput(_, _, Some(Key::A)) =>
                    self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), angle) * self.cam.rot,
                Event::KeyboardInput(_, _, Some(Key::D)) =>
                    self.cam.rot = cgmath::Quaternion::from_axis_angle(cgmath::Vector3::unit_z(), -angle) * self.cam.rot,
                */
                _ => {},
            }
        }

        self.cam.follow(&self.agents[pid].transform, delta, &super::Follow {
            transform: cgmath::Decomposed {
                disp: cgmath::vec3(0.0, -200.0, 100.0),
                rot: cgmath::Rotation3::from_axis_angle(cgmath::Vector3::unit_x(), cgmath::Rad::new(1.1)),
                scale: 1.0,
            },
            speed: 4.0,
        });

        for a in self.agents.iter_mut() {
            a.step(delta * config::common::SPEED_CORRECTION_FACTOR, &self.level, &self.db.common);
        }

        true
    }

    fn draw<C: gfx::CommandBuffer<R>>(&mut self, enc: &mut gfx::Encoder<R, C>) {
        self.render.draw_world(enc, &self.agents, &self.cam);
    }
}