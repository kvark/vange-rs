use std::collections::HashMap;
use cgmath;
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

struct CollisionPoint {
    pos: cgmath::Vector3<f32>,
    depth: f32,
}

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
    fn finish(self, min: f32, transform: &super::Transform) -> Option<CollisionPoint> {
        use cgmath::{EuclideanSpace, Transform};
        if self.count > min {
            let pos = cgmath::Point3::from_vec(self.pos / self.count);
            Some(CollisionPoint {
                pos: transform.transform_point(pos).to_vec(),
                depth: self.depth / self.count,
            })
        } else { None }
    }
}

fn get_height(altitude: u8) -> f32 {
    altitude as f32 * (level::HEIGHT_SCALE as f32) / 256.0
}

fn collide_low(poly: &model::Polygon, samples: &[[f32; 3]], transform: &super::Transform,
               level: &level::Level, terraconf: &config::common::Terrain) -> CollisionData
{
    use cgmath::{EuclideanSpace, Transform};
    let (mut soft, mut hard) = (Accumulator::new(), Accumulator::new());
    for &s in samples[poly.sample_range.0 as usize .. poly.sample_range.1 as usize].iter() {
        let pos = transform.transform_point(s.into()).to_vec();
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
    let tinv = transform.inverse_transform().unwrap();
    CollisionData {
        soft: soft.finish(0.0, &tinv),
        hard: hard.finish(4.0, &tinv),
    }
}

fn calc_collision_matrix(_point: cgmath::Vector3<f32>) -> cgmath::Matrix3<f32> {
    use cgmath::SquareMatrix;
    cgmath::Matrix3::identity()
}

impl<R: gfx::Resources> Agent<R> {
    fn step(&mut self, dt: f32, level: &level::Level, common: &config::common::Common) {
        use cgmath::{EuclideanSpace, InnerSpace, Rotation, Rotation3, SquareMatrix, Transform};

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
        let mut acc_cur = {
            let global2local = self.transform.inverse_transform().unwrap();
            AccelerationVectors {
                f: global2local.transform_vector(acc_global.f),
                k: global2local.transform_vector(acc_global.k),
            }
        };
        let flood_level = level.flood_map[0] as f32;
        let (mut v_vel, mut w_vel) = (self.dynamo.linear_velocity, self.dynamo.angular_velocity);
        let j_inv = {
            let phys = &self.car.model.body.physics;
            (cgmath::Matrix3::from(phys.jacobi) *
                (self.transform.scale * self.transform.scale / phys.volume))
                .invert().unwrap()
        };

        for _ in 0 .. common.nature.num_calls_analysis {
            let mut float_count = 0;
            let (mut terrain_immersion, mut water_immersion) = (0.0, 0.0);
            let stand_on_wheels = true; //TODO
            let modulation = 1.0;
            for poly in self.car.model.shape.polygons.iter() {
                let middle = self.transform.transform_point(poly.middle.into());
                let vr = v_vel + w_vel.cross(middle.to_vec());
                let mostly_horisontal = vr.z*vr.z < vr.x*vr.x + vr.y*vr.y;
                let texel = level.get((middle.x as i32, middle.y as i32));
                match texel.low.1 {
                    level::TerrainType::Water => {
                        let dz = flood_level - middle.z;
                        if dz > 0.0 {
                            float_count += 1;
                            water_immersion += dz;
                        }
                    },
                    level::TerrainType::Main => {
                        let normal = self.transform.transform_vector(poly.normal.into());
                        if normal.z < 0.0 {
                            let cdata = collide_low(poly, &self.car.model.shape.samples,
                                &self.transform, level, &common.terrain);
                            terrain_immersion += match cdata.soft {
                                Some(ref cp) => cp.depth,
                                None => 0.0,
                            };
                            terrain_immersion += match cdata.hard {
                                Some(ref cp) => cp.depth,
                                None => 0.0,
                            };
                            match cdata {
                                CollisionData{ hard: Some(ref cp), ..} if mostly_horisontal => {
                                    let mut pos = cp.pos;
                                    pos.z = 0.0; // ignore vertical
                                    let normal = pos.normalize();
                                    let u0 = v_vel + w_vel.cross(pos);
                                    let dot = u0.dot(normal);
                                    if dot > 0.0 {
                                        let pulse = (calc_collision_matrix(pos) * normal) *
                                            (-common.impulse.factors[0] * modulation * dot);
                                        v_vel += pulse;
                                        w_vel += j_inv * cp.pos.cross(pulse);
                                    }
                                },
                                CollisionData{ soft: Some(ref cp), ..} => {
                                    let mut u0 = v_vel + w_vel.cross(cp.pos);
                                    if u0.z < 0.0 {
                                        if stand_on_wheels { // ignore XY
                                            u0.x = 0.0;
                                            u0.y = 0.0;
                                        } else {
                                            //TODO
                                        }
                                        let pulse = (calc_collision_matrix(cp.pos) * u0) *
                                            (-common.impulse.factors[1] * modulation);
                                        v_vel += pulse;
                                        w_vel += j_inv * cp.pos.cross(pulse);
                                    }
                                }
                                _ => (),
                            }
                        }
                    },
                }
            }
            let _ = (float_count, water_immersion, terrain_immersion); //TODO
            let f_traction_per_wheel =
                self.car.physics.mobility_factor * common.global.mobility_factor *
                if self.control.turbo { common.global.k_traction_turbo } else { 1.0 } *
                self.dynamo.traction / (self.car.model.wheels.len() as f32);
            let v_drag = common.drag.free.v * common.drag.speed.v.powf(v_vel.magnitude());
            let w_drag = common.drag.free.w * common.drag.speed.w.powf(w_vel.magnitude2());
            for wheel in self.car.model.wheels.iter() {
                let vw = v_vel + w_vel.cross(wheel.pos.into());
                acc_cur.f.y += f_traction_per_wheel;
                if self.control.brake {
                    acc_cur.f -= vw * common.global.f_brake_max;
                }
            }
            v_vel += acc_cur.f * dt;
            w_vel += (j_inv * acc_cur.k) * dt;
            if v_vel.magnitude() * v_drag > common.drag.abs_stop.v || w_vel.magnitude() *w_drag > common.drag.abs_stop.w {
                self.transform.disp += self.dynamo.linear_velocity * dt;
                let ws = w_vel.magnitude();
                let rot_inverse = cgmath::Quaternion::from_axis_angle(w_vel / ws.max(0.01),
                    cgmath::Deg::new(ws * -dt).into());
                v_vel = rot_inverse.rotate_vector(v_vel);
                w_vel = rot_inverse.rotate_vector(v_vel);
            }
            v_vel *= v_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
            w_vel *= w_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
        }

        self.dynamo.linear_velocity = v_vel;
        self.dynamo.angular_velocity = w_vel;
        // slow down
        let sign = self.dynamo.traction.signum();
        self.dynamo.change_traction(-sign * dt * common.car.traction_decr);
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
        use cgmath::{InnerSpace, Rotation};
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