//! Physics port of the original game. Most closely described by the following documents:
//! - https://people.eecs.berkeley.edu/~jfc/mirtich/thesis/mirtichThesis.pdf

use crate::{config, level, model, render::debug::LineBuffer, space};

use glam::{Mat3, Quat, Vec3};

const EPSILON: f32 = f32::EPSILON;

mod rigid;
pub mod terrain;

const MAX_TRACTION: config::common::Traction = 4.0;

/// Original `traction` is an int; the port stores it already divided by 64
/// (`MAX_TRACTION` 4 = original 256). Water thrust in `analyse_dynamics`
/// uses the int, so swimming scales it back.
const ORIGINAL_TRACTION: f32 = 64.0;
/// Original rudder is a `PI/2048` integer angle; the port stores radians.
const ORIGINAL_ANGLE: f32 = std::f32::consts::PI / 2048.0;
/// `pow(15/16, XTCORE_FRAME_NORMAL)` while the hull is driving in water.
const WATER_RUDDER_DRAG: f32 = 15.0 / 16.0;

/// How deep the car has to be before the mole stops pushing it down, and
/// how shallow before it counts as having surfaced. Both are
/// `terrain_immersion` thresholds straight out of `analyse_dynamics`.
const MOLE_SUBMERGED: f32 = 900.0;
const MOLE_SURFACED: f32 = 50.0;

/// Original `analyse_dynamics` when `stand_on_wheels`: `u0.x = u0.y = 0`.
pub(crate) fn wheel_contact_cancel(pv: Vec3) -> Vec3 {
    Vec3::new(0.0, 0.0, pv.z)
}

/// Where a car is in a burrow.
///
/// The original spells this `mole_on`, an int that is `256` while the car
/// is under and counts down once the player lets go - though nothing ever
/// counts it, so it is really these three states.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mole {
    #[default]
    Off,
    /// Burrowing, and being pulled further down until it is deep enough.
    Under,
    /// On the way back up. Ends the moment the car is clear of the ground.
    Surfacing,
}

#[derive(Debug)]
struct AccelerationVectors {
    f: Vec3, // linear
    k: Vec3, // angular
}

pub struct Dynamo {
    pub traction: config::common::Traction,
    pub rudder: f32,
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    /// Whether the car is burrowing, and which way it is heading.
    pub mole: Mole,
}

impl Default for Dynamo {
    fn default() -> Self {
        Dynamo {
            traction: 0.,
            rudder: 0.0,
            linear_velocity: Vec3::ZERO,
            angular_velocity: Vec3::ZERO,
            mole: Mole::Off,
        }
    }
}

impl Dynamo {
    pub fn change_traction(&mut self, delta: config::common::Traction) {
        self.traction = (self.traction + delta).clamp(-MAX_TRACTION, MAX_TRACTION);
    }

    pub fn slow_down(&mut self, delta: config::common::Traction) {
        let old = self.traction;
        self.change_traction(delta * -old.signum());
        if old * self.traction < 0.0 {
            self.traction = 0.0;
        }
    }
}

/// GPU-free physics data extracted from a car model.
/// Contains only the CPU-side fields needed by the physics simulation,
/// without any wgpu buffers or GPU resources.
#[derive(Clone)]
pub struct CarPhysicsData {
    pub physics: config::car::CarPhysics,
    pub body_physics: m3d::Physics,
    pub bbox: model::BoundingBox,
    pub shape_polygons: Vec<model::Polygon>,
    pub shape_samples: Vec<model::RawVertex>,
    pub wheels: Vec<WheelPhysics>,
    pub scale: f32,
}

/// Wheel data needed for physics (no GPU mesh).
#[derive(Clone)]
pub struct WheelPhysics {
    pub steer: u32,
    pub pos: [f32; 3],
}

impl CarPhysicsData {
    /// Create a simple box-shaped car for testing without game data files.
    pub fn test_default() -> Self {
        let half = 10.0f32;
        // A simple box with 6 faces, each face is a polygon
        let faces: Vec<([f32; 3], [f32; 3])> = vec![
            ([0.0, 0.0, -half], [0.0, 0.0, -1.0]), // bottom
            ([0.0, 0.0, half], [0.0, 0.0, 1.0]),   // top
            ([half, 0.0, 0.0], [1.0, 0.0, 0.0]),   // right
            ([-half, 0.0, 0.0], [-1.0, 0.0, 0.0]), // left
            ([0.0, half, 0.0], [0.0, 1.0, 0.0]),   // front
            ([0.0, -half, 0.0], [0.0, -1.0, 0.0]), // back
        ];

        let mut polygons = Vec::new();
        let mut samples = Vec::new();
        for (middle, normal) in faces {
            let start = samples.len();
            samples.push([middle[0] as i8, middle[1] as i8, middle[2] as i8]);
            polygons.push(model::Polygon {
                middle,
                normal,
                samples: start..samples.len(),
            });
        }

        CarPhysicsData {
            physics: config::car::CarPhysics {
                name: "TestCar".into(),
                scale_size: 1.0,
                scale_bound: 1.0,
                scale_box: 1.0,
                z_offset_of_mass_center: 0.0,
                speed_factor: 1.0,
                mobility_factor: 1.0,
                water_speed_factor: 1.0,
                air_speed_factor: 1.0,
                underground_speed_factor: 1.0,
                k_archimedean: 0.5,
                k_water_traction: 0.5,
                k_water_rudder: 0.5,
                // The values every mechos `.prm` carries, so that a test
                // car has a grader blade like a real one does.
                terra_mover_sx: [1.0, 0.907029, 0.556837],
                defence: [100; config::car::NUM_SIDES],
                ram_power: [50; config::car::NUM_SIDES],
            },
            body_physics: m3d::Physics {
                volume: 8000.0, // 20^3
                rcm: [0.0, 0.0, 0.0],
                jacobi: [[1333.0, 0.0, 0.0], [0.0, 1333.0, 0.0], [0.0, 0.0, 1333.0]], // uniform box inertia: m*s^2/6
            },
            bbox: model::BoundingBox {
                min: [-half, -half, -half],
                max: [half, half, half],
                radius: half * 1.73, // sqrt(3) * half
            },
            shape_polygons: polygons,
            shape_samples: samples,
            wheels: vec![
                WheelPhysics {
                    steer: 1,
                    pos: [half, half, -half],
                },
                WheelPhysics {
                    steer: 1,
                    pos: [-half, half, -half],
                },
                WheelPhysics {
                    steer: 0,
                    pos: [half, -half, -half],
                },
                WheelPhysics {
                    steer: 0,
                    pos: [-half, -half, -half],
                },
            ],
            scale: 1.0,
        }
    }

    /// Load physics data from an M3D model file and a .prm parameters file.
    /// This does not require a GPU and is suitable for headless servers.
    pub fn load_from_files(
        m3d_file: std::fs::File,
        prm_file: std::fs::File,
        scale: f32,
        shape_sampling: u8,
    ) -> Self {
        Self::build(
            m3d::FullModel::load(m3d_file),
            config::car::CarPhysics::load(prm_file),
            scale,
            shape_sampling,
        )
    }

    /// Byte-slice variant of [`load_from_files`]. Used by the web build
    /// to construct physics data from zip-backed bytes.
    pub fn from_bytes(m3d_bytes: &[u8], prm_bytes: &[u8], scale: f32, shape_sampling: u8) -> Self {
        Self::build(
            m3d::FullModel::load(std::io::Cursor::new(m3d_bytes)),
            config::car::CarPhysics::load_reader(std::io::Cursor::new(prm_bytes)),
            scale,
            shape_sampling,
        )
    }

    fn build(
        raw: m3d::FullModel,
        physics: config::car::CarPhysics,
        scale: f32,
        shape_sampling: u8,
    ) -> Self {
        let shape = model::extract_shape_physics(raw.shape, shape_sampling);

        CarPhysicsData {
            body_physics: raw.body.physics,
            bbox: model::BoundingBox {
                min: [
                    raw.body.bounds.coord_min[0] as f32,
                    raw.body.bounds.coord_min[1] as f32,
                    raw.body.bounds.coord_min[2] as f32,
                ],
                max: [
                    raw.body.bounds.coord_max[0] as f32,
                    raw.body.bounds.coord_max[1] as f32,
                    raw.body.bounds.coord_max[2] as f32,
                ],
                radius: raw.body.max_radius as f32,
            },
            wheels: raw
                .wheels
                .iter()
                .map(|w| WheelPhysics {
                    steer: w.steer,
                    pos: w.pos,
                })
                .collect(),
            scale,
            shape_polygons: shape.polygons,
            shape_samples: shape.samples,
            physics,
        }
    }

    /// Extract physics data from a full CarInfo (which contains GPU resources).
    pub fn from_car_info(car: &config::car::CarInfo) -> Self {
        CarPhysicsData {
            physics: car.physics.clone(),
            body_physics: car.model.body.physics,
            bbox: car.model.body.bbox,
            shape_polygons: car.model.shape.polygons.clone(),
            shape_samples: car.model.shape.samples.clone(),
            wheels: car
                .model
                .wheels
                .iter()
                .map(|w| WheelPhysics {
                    steer: w.steer,
                    pos: w.pos,
                })
                .collect(),
            scale: car.scale,
        }
    }

    pub fn wheel_points(&self, transform: &space::Transform) -> Vec<Vec3> {
        let mut pts: Vec<Vec3> = self
            .wheels
            .iter()
            .map(|w| transform.transform_point(Vec3::from(w.pos)))
            .collect();
        if pts.is_empty() {
            pts.push(transform.disp);
        }
        pts
    }
}

pub fn jump_dir(power: f32) -> Vec3 {
    2.5 * power * Vec3::new(0.0, 3.0, 10.0).normalize()
}

pub fn step(
    dynamo: &mut Dynamo,
    transform: &mut space::Transform,
    dt: f32,
    car: &CarPhysicsData,
    level: &level::Level,
    common: &config::common::Common,
    f_turbo: f32,
    f_brake: f32,
    jump: Option<f32>,
    roll: f32,
    mut line_buffer: Option<&mut LineBuffer>,
    mut tracks: Option<&mut level::terraform::Tracks>,
) {
    let speed_correction_factor = dt / common.nature.time_delta0;
    let acc_global = AccelerationVectors {
        f: Vec3::new(0.0, 0.0, -common.nature.gravity),
        k: Vec3::new(0.0, 0.0, 0.0),
    };
    let rot_inv = transform.rot.inverse();
    log::debug!("dt {}, num {}", dt, common.nature.num_calls_analysis);
    // Z axis in the local coordinate space
    let z_axis = rot_inv * Vec3::Z;
    let num_bounds = car.shape_polygons.len().max(1);
    // Original `f_archimedean = k_archimedean * archimedean / 256 / num_bounds`.
    // Devices are not ported, so a wet hull swims at full flotation (256).
    let f_archimedean = if dynamo.mole == Mole::Off {
        car.physics.k_archimedean / num_bounds as f32
    } else {
        0.0
    };
    let device_modulation = 1.0;
    let dt_impulse = 1.0;

    let mut rigid = {
        let phys = &car.body_physics;
        let jacobian = Mat3::from_cols_array_2d(&phys.jacobi)
            * (transform.scale * transform.scale / phys.volume);
        rigid::RigidBody::new(&jacobian, dynamo.linear_velocity, dynamo.angular_velocity)
    };

    if let Some(power) = jump {
        let mass =
            common.nature.density * car.body_physics.volume * transform.scale * transform.scale;
        let f = device_modulation * common.force.k_distance_to_force * dt_impulse / mass.powf(0.3);
        log::info!("jump mass {:?}, f {:?}", mass, f);
        rigid.vel += f * jump_dir(power);
    }

    let mut wheels_touch = 0u32;
    let mut spring_touch = 0;

    let mut float_count = 0i32;
    let (mut terrain_immersion, mut water_immersion) = (0.0, 0.0);
    let stand_on_wheels = z_axis.z > 0.0 && (transform.rot * Vec3::X).z.abs() < 0.7;
    // `k_elastic_modulation` of the original: under the ground the car is
    // held far more softly, which is what lets it sink in at all.
    let modulation = match dynamo.mole {
        Mole::Under => common.mole.k_elastic_mole,
        _ => 1.0,
    };
    let mut acc_cur = AccelerationVectors {
        f: rot_inv * acc_global.f,
        k: rot_inv * acc_global.k,
    };

    let mut down_minus_up = 0i32;
    let mut acc_springs = AccelerationVectors {
        f: Vec3::ZERO,
        k: Vec3::ZERO,
    };

    let mut sum_count = 0usize;
    let mut sum_rg0 = Vec3::ZERO;
    let mut sum_df = 0.;

    for (bound_poly_id, poly) in car.shape_polygons.iter().enumerate() {
        let r = Vec3::from(poly.middle) * (transform.scale * car.physics.scale_bound);
        let rg0 = transform.rot * r;
        let rglob = rg0 + transform.disp;
        log::debug!(
            "\t\tpoly[{}]: normal={:?} scale={} mid={:?} r={:?}",
            bound_poly_id,
            poly.normal,
            transform.scale * car.physics.scale_bound,
            poly.middle,
            r
        );
        // Original: `GET_TERRAIN == WATER_TERRAIN` and `dZ = FloodLEVEL - rg.z`.
        // Terrain 0 is water on every shipped world. Mole skips this, same as
        // `if (!mole_on)` around the water test in `basic_mechous_analysis`.
        // `FloodLEVEL` is a height quant; the flood texture is per-Y, so
        // the plane the car feels is the same one the water draw uses.
        if dynamo.mole == Mole::Off {
            match level.get((rglob.x as i32, rglob.y as i32)) {
                level::Texel::Single(level::Point(_, 0))
                | level::Texel::Dual {
                    low: level::Point(_, 0),
                    ..
                } => {
                    let dz = level.flood_level_at(rglob.y as i32) - rglob.z;
                    if dz > 0.0 {
                        float_count += 1;
                        water_immersion += dz;
                        if f_archimedean != 0.0 {
                            let df = z_axis * (f_archimedean * dz);
                            acc_cur.f += df;
                            acc_cur.k += r.cross(df);
                        }
                    }
                }
                _ => {}
            }
        }
        let poly_norm = Vec3::from(poly.normal).normalize();
        if z_axis.dot(poly_norm) < 0.0 {
            let cdata = terrain::CollisionData::collide_low(
                poly,
                &car.shape_samples,
                car.physics.scale_bound,
                transform,
                level,
                &common.terrain,
            );

            log::debug!("\t\tcollide_low = {:?}", cdata);
            terrain_immersion += match cdata.soft {
                Some(ref cp) => cp.depth.abs(),
                None => 0.0,
            };
            terrain_immersion += match cdata.hard {
                Some(ref cp) => cp.depth.abs(),
                None => 0.0,
            };

            let origin = transform.disp;
            // Original `code & 2` wall bounce uses a bounding-box normal,
            // which punches a car sideways off a valley bank. Stay on the
            // vertical spring path for every ground contact.
            if let Some(cp) = cdata.soft.as_ref().or(cdata.hard.as_ref()) {
                let r1 = rot_inv * Vec3::new(cp.pos.x - origin.x, cp.pos.y - origin.y, rg0.z);
                let pv = rigid.velocity_at(r1);
                if pv.dot(z_axis) < 0.0 {
                    let vec = if stand_on_wheels {
                        wheel_contact_cancel(pv)
                    } else {
                        let projected = poly_norm * poly_norm.dot(pv);
                        common.impulse.k_friction * pv
                            + (1.0 - common.impulse.k_friction) * projected
                    };
                    rigid.push_capped(r, vec * (-common.impulse.factors[1] * modulation), pv);
                }
            }

            // Original `code & 5`: springs fire on soft contacts *or* a
            // wall-dominant poly (`N1 > N/2`). Skipping hard contacts left
            // a buried car with nothing to lift it back out.
            let spring = cdata.soft.as_ref().or(cdata.hard.as_ref());
            if let Some(cp) = spring {
                let depth = cp.depth.min(common.terrain.min_wall_delta);
                let df0 = common.contact.k_elastic_spring * modulation * depth;
                let df = df0.min(common.impulse.elastic_restriction);
                log::debug!("\t\tbound[{}] dF.z = {}, rg0={:?}", bound_poly_id, df, rg0);

                acc_springs.f.z += df;
                acc_springs.k.x += rg0.y * df;
                acc_springs.k.y -= rg0.x * df;

                if stand_on_wheels {
                    wheels_touch += 1;
                } else {
                    spring_touch += 1;
                }
                down_minus_up += 1;

                sum_count += 1;
                sum_rg0 += rg0;
                sum_df += df;

                if let Some(ref mut lbuf) = line_buffer {
                    lbuf.add(transform.disp.into(), rglob.into(), 0xFF000000);
                    let up = rglob + Vec3::new(0.0, 0.0, df0);
                    lbuf.add(rglob.into(), up.into(), 0xFFFF0000);
                    let end = rglob + df * Vec3::new(rg0.y, -rg0.x, 0.0);
                    lbuf.add(rglob.into(), end.into(), 0xFF00FF00);
                }
            }
        } else {
            let cdata = terrain::CollisionData::collide_high(
                poly,
                &car.shape_samples,
                car.physics.scale_bound,
                transform,
                level,
                &common.terrain,
            );
            if let Some(cp) = cdata.soft.as_ref().or(cdata.hard.as_ref()) {
                let origin = transform.disp;
                let r1 = rot_inv * Vec3::new(cp.pos.x - origin.x, cp.pos.y - origin.y, rg0.z);
                let pv = rigid.velocity_at(r1);
                if pv.dot(z_axis) > 0.0 {
                    let vec = poly_norm * poly_norm.dot(pv);
                    rigid.push_capped(r, vec * (-common.impulse.factors[1] * modulation), pv);
                }
                let df = (-common.contact.k_elastic_spring * modulation * cp.depth).max(-2.0);
                acc_springs.f.z += df;
                acc_springs.k.x += rg0.y * df;
                acc_springs.k.y -= rg0.x * df;
                spring_touch += 1;
                down_minus_up -= 1;
                let world_vz = (transform.rot * rigid.vel).z;
                if world_vz > 30.0
                    && let Some(ref mut tracks) = tracks
                {
                    tracks.smash_ceiling((cp.pos.x as i32, cp.pos.y as i32));
                }
            }
        }
    }

    if sum_count != 0 {
        let kf = 1.0 / sum_count as f32;
        log::debug!("Avg df {} rg0 {:?}", sum_df * kf, sum_rg0 * kf);
    }

    if wheels_touch + spring_touch != 0 {
        log::debug!("\tsprings total {:?}", acc_springs);
        acc_cur.f += rot_inv * acc_springs.f;
        acc_cur.k += rot_inv * acc_springs.k;
    }

    // apply drag
    let mut v_drag = common.drag.free.v * common.drag.speed.v.powf(rigid.vel.length());
    let mut w_drag = common.drag.free.w
        * common
            .drag
            .speed
            .w
            .powf(rigid.angular_velocity().length_squared());
    if wheels_touch > 0 {
        let speed =
            common.drag.wheel_speed.ln() * car.physics.mobility_factor * common.global.speed_factor
                / car.physics.speed_factor;
        rigid.vel.y *= (1.0 + speed).powf(speed_correction_factor);
    }

    // Original `in_water = (float_cnt << 8) / num_bounds` (0..=256).
    let in_water = (float_count << 8) / num_bounds as i32;
    log::debug!(
        "water in_water={} immersion={} terrain={}",
        in_water,
        water_immersion,
        terrain_immersion
    );
    // `archimedean && traction`: water thrust plus a bit of rudder, using
    // the original int traction/rudder units. `k_water_traction` is loaded
    // from the mechos `.prm` but never read in `analyse_dynamics`.
    if dynamo.mole == Mole::Off && in_water > 32 && dynamo.traction.abs() > EPSILON {
        let traction_int = dynamo.traction * ORIGINAL_TRACTION;
        let mut d_fy = traction_int;
        let rudder_int = dynamo.rudder / ORIGINAL_ANGLE;
        // Original `dFx = (traction > 0 ? -rudder : rudder) * dFy * k`.
        // Left (positive rudder, same as KeyA) yaws the nose toward -X.
        let d_fx = if dynamo.traction > 0.0 {
            -rudder_int
        } else {
            rudder_int
        } * d_fy
            * car.physics.k_water_rudder;
        d_fy *= car.physics.water_speed_factor * common.global.water_speed_factor;
        acc_cur.f.y += d_fy;
        acc_cur.f.x += d_fx;
        let ymax = car.bbox.max[1].abs().max(car.bbox.min[1].abs()) * transform.scale;
        let zmax = car.bbox.max[2].abs().max(car.bbox.min[2].abs()) * transform.scale;
        acc_cur.k.z -= if dynamo.traction > 0.0 {
            ymax * d_fx
        } else {
            -ymax * d_fx
        };
        acc_cur.k.x += zmax * d_fy * (1.0 / 16.0);
        dynamo.rudder *= WATER_RUDDER_DRAG.powf(speed_correction_factor);
    }
    let is_after_collision = false;
    if let Some(ref mut tracks) = tracks
        && (wheels_touch == 0 || !stand_on_wheels)
    {
        tracks.lift_all();
    }
    // `ground_pressing`: whatever is bearing on the ground pushes it down
    // to the hull, whether the car is rolling or has come to rest on its
    // belly. The blade and the tread both need it to be driving; this does
    // not.
    if let Some(ref mut tracks) = tracks
        && (wheels_touch != 0 || spring_touch != 0)
    {
        let half =
            |axis: usize| car.bbox.max[axis].abs().max(car.bbox.min[axis].abs()) * transform.scale;
        let (hx, hy) = (half(0), half(1));
        let floor = car.bbox.min[2] * transform.scale;
        let corner = |x: f32, y: f32| transform.rot * Vec3::new(x, y, floor) + transform.disp;
        tracks.press(level::terraform::Hull {
            corners: [
                corner(-hx, hy),
                corner(hx, hy),
                corner(hx, -hy),
                corner(-hx, -hy),
            ],
        });
    }
    if wheels_touch != 0 && stand_on_wheels && dynamo.mole == Mole::Off {
        let f_traction_per_wheel =
            car.physics.mobility_factor * common.global.mobility_factor * f_turbo * dynamo.traction
                / (car.wheels.len() as f32);
        let rudder_vec = {
            let (sin, cos) = dynamo.rudder.sin_cos();
            Vec3::new(cos, -sin, 0.0)
        };
        // The grader blade, from the `TerraMoverS*` the mechos `.prm` files
        // carry: a line across the leading edge of the car, at the bottom
        // of it. Reversing swings it round to the other end, so the blade
        // always faces the way the car is being driven.
        if let Some(ref mut tracks) = tracks
            && dynamo.traction == 0.0
        {
            // `WHEELS_TOUCH && traction` of the commented-out call site: a
            // car rolling to a stop is not grading anything.
            tracks.raise_blade();
        } else if let Some(ref mut tracks) = tracks {
            let s = car.physics.terra_mover_sx;
            let half = |axis: usize| {
                car.bbox.max[axis].abs().max(car.bbox.min[axis].abs()) * transform.scale
            };
            let (sx, sy, sz) = (
                s[0] * half(0),
                s[1] * half(1) * dynamo.traction.signum(),
                s[2] * half(2),
            );
            let corner = |x: f32| transform.rot * Vec3::new(x, sy, -sz) + transform.disp;
            tracks.blade(corner(-sx), corner(sx));
        }
        for wheel in car.wheels.iter() {
            let pw = transform.transform_point(Vec3::from(wheel.pos));
            let detect_wheel_hits = false;
            if detect_wheel_hits {
                let dist = terrain::get_distance_to_terrain(level, pw);
                if dist > 0.0 {
                    continue;
                }
            }

            let rx_max = if wheel.pos[0] > 0.0 {
                car.bbox.max[0]
            } else {
                car.bbox.min[0]
            };
            let pos = Vec3::new(rx_max, wheel.pos[1], wheel.pos[2]) * transform.scale;
            let pv = rigid.velocity_at(pos);

            acc_cur.f.y += f_traction_per_wheel;
            acc_cur.f -= pv * f_brake;

            if !is_after_collision {
                let dir = if wheel.steer != 0 {
                    rudder_vec
                } else {
                    Vec3::X
                };

                let dot = dir.dot(pv);
                let pulse = rigid.push(pos, dir * (dot * -common.impulse.k_wheel));
                if let Some(ref mut lbuf) = line_buffer {
                    let dest = pw + transform.transform_vector(pulse) * 10.0;
                    lbuf.add(pw.into(), dest.into(), 0xFFFFFF00);
                }
            }
        }
    }

    if spring_touch + wheels_touch != 0 || in_water != 0 {
        let tmp = Vec3::new(
            0.0,
            0.0,
            car.physics.z_offset_of_mass_center * transform.scale,
        );
        acc_cur.k -= common.nature.gravity * tmp.cross(z_axis);
        if spring_touch + wheels_touch != 0 {
            let vz = z_axis.dot(rigid.vel);
            if vz < -10.0 {
                v_drag *= common.drag.z.powf(-vz);
            }
        }
    }

    if roll != 0.0 && wheels_touch == 0 && spring_touch != 0 {
        let df = common.force.f_spring_impulse * speed_correction_factor;
        let x_edge = if roll > 0.0 {
            car.bbox.max[0]
        } else {
            car.bbox.min[0]
        };
        rigid.add_raw(
            Vec3::new(0.0, 0.0, df),
            Vec3::new(0.0, df * x_edge * transform.scale, 0.0),
        );
    }

    // The mounds a burrow throws up along the line across the car.
    if let Some(ref mut tracks) = tracks {
        if dynamo.mole == Mole::Off {
            tracks.surface();
        } else {
            let reach = car.bbox.max[0].abs().max(car.bbox.min[0].abs()) * transform.scale;
            let end = |x: f32| transform.rot * Vec3::new(x, 0.0, 0.0) + transform.disp;
            tracks.burrow(end(-reach), end(reach));
        }
    }

    // The burrow. `analyse_dynamics` runs this in place of the ordinary
    // ground handling: the car drives on `underground_speed_factor`
    // instead of its wheels, steers with its whole body, and is pulled
    // down until it is deep enough or pushed up until it is out.
    if dynamo.mole != Mole::Off {
        let mole = &common.mole;
        match dynamo.mole {
            Mole::Under => {
                if terrain_immersion < MOLE_SUBMERGED {
                    acc_cur.f -= rot_inv * Vec3::new(0.0, 0.0, mole.mole_submerging_fz);
                }
            }
            Mole::Surfacing => {
                if terrain_immersion < MOLE_SURFACED {
                    dynamo.mole = Mole::Off;
                    rigid.halt();
                } else {
                    acc_cur.f += rot_inv * Vec3::new(0.0, 0.0, mole.mole_emerging_fz);
                }
            }
            Mole::Off => unreachable!(),
        }
        if dynamo.mole != Mole::Off {
            v_drag *= common.drag.mole;
            w_drag *= common.drag.mole;
            acc_cur.f.y += car.physics.underground_speed_factor
                * common.global.k_traction_turbo.min(1.0)
                * dynamo.traction;
            // Steering underground turns the whole hull, and a torque keeps
            // it the right way up while it has no wheels to stand on.
            acc_cur.k.z += dynamo.rudder
                * car.bbox.radius
                * mole.k_mole_rudder
                * car.physics.underground_speed_factor;
            acc_cur.k.x -= z_axis.y * car.bbox.radius * mole.k_mole;
            acc_cur.k.y += z_axis.x * car.bbox.radius * mole.k_mole;
        }
    }

    log::debug!("\tcur acc {:?}", acc_cur);
    rigid.add_raw(acc_cur.f * dt, acc_cur.k * dt);
    let (mut v_vel, mut w_vel) = rigid.finish();

    log::debug!("\tresulting v={:?} w={:?}", v_vel, w_vel);
    if spring_touch != 0 {
        v_drag *= common.drag.spring.v;
        w_drag *= common.drag.spring.w;
    }
    if in_water > 64 {
        v_drag *= common.drag.float.v;
        w_drag *= common.drag.float.w;
    }
    let (v_mag, w_mag) = (v_vel.length(), w_vel.length());
    if stand_on_wheels
        && in_water < 32
        && v_mag < common.drag.abs_min.v
        && w_mag < common.drag.abs_min.w
    {
        let v_pow = common.drag.abs_min.v / (v_mag + EPSILON);
        let w_pow = common.drag.abs_min.w / (w_mag + EPSILON);
        v_drag *= common.drag.coll.v.powf(v_pow);
        w_drag *= common.drag.coll.w.powf(w_pow);
    }

    if v_mag * v_drag > common.drag.abs_stop.v || w_mag * w_drag > common.drag.abs_stop.w {
        let radius = car.bbox.radius;
        let local_z_scaled = z_axis * (radius * common.impulse.rolling_scale);
        let r_diff_sign = down_minus_up.signum() as f32;
        let vs = v_vel - r_diff_sign * local_z_scaled.cross(w_vel);

        let angle = -dt * w_mag;
        let vel_rot_inv = Quat::from_axis_angle(w_vel / (w_mag + EPSILON), angle);
        transform.disp += (transform.rot * vs) * dt;
        transform.rot *= vel_rot_inv.inverse();
        v_vel = vel_rot_inv * v_vel;
        w_vel = vel_rot_inv * w_vel;
        log::debug!(
            "\tvs={:?} {:?}\n\t\tdisp {:?} scale {}",
            vs,
            transform.rot,
            transform.disp,
            transform.scale
        );
    }
    v_vel *= v_drag.powf(speed_correction_factor);
    w_vel *= w_drag.powf(speed_correction_factor);

    // Record tread endpoints from the transform that will actually be
    // rendered. Doing this in the traction loop above sampled the pre-step
    // transform, leaving every trail one integration step behind the tyre.
    if wheels_touch != 0
        && stand_on_wheels
        && let Some(ref mut tracks) = tracks
    {
        let lateral = transform.rot * Vec3::X;
        let across = (lateral.x, lateral.y);
        for (index, wheel) in car.wheels.iter().enumerate() {
            let pw = transform.transform_point(Vec3::from(wheel.pos));
            let coord = (pw.x.round() as i32, pw.y.round() as i32);
            let gap = level::terraform::surface_height(level, coord) - pw.z;
            // The old one-sided test accepted a wheel arbitrarily high
            // above the surface whenever another wheel touched. Require
            // this wheel's own contact point to be close to the ground.
            if gap.abs() <= level::terraform::MAX_CONTACT_HEIGHT {
                tracks.touch(index, coord, across);
            } else {
                tracks.lift(index);
            }
        }
    }

    if let Some(ref mut lbuf) = line_buffer {
        let rot = transform.rot;
        let ba = transform.disp + Vec3::new(3.0, 0.0, 10.0);
        let xf = ba + rot * acc_cur.f;
        let xk = ba + rot * acc_cur.k;
        lbuf.add(ba.into(), xf.into(), 0x0000FF00);
        lbuf.add(ba.into(), xk.into(), 0xFF00FF00);
        let bv = transform.disp + Vec3::new(-3.0, 0.0, 10.0);
        let xv = bv + rot * v_vel;
        let xw = bv + rot * w_vel * 10.0;
        lbuf.add(bv.into(), xv.into(), 0x00FF0000);
        lbuf.add(bv.into(), xw.into(), 0x00FFFF00);
    }

    dynamo.linear_velocity = v_vel;
    dynamo.angular_velocity = w_vel;
    // unsteer
    if dynamo.rudder != 0.0 && wheels_touch != 0 {
        let change = dynamo.rudder * v_vel.y * dt * common.car.rudder_k_decr;
        dynamo.rudder -= dynamo.rudder.signum() * change.abs();
    }
    // slow down
    dynamo.slow_down(dt * common.car.traction_decr);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_contact_keeps_only_the_vertical() {
        let pv = Vec3::new(10.0, 4.0, 2.0);
        let out = wheel_contact_cancel(pv);
        assert_eq!(out, Vec3::new(0.0, 0.0, 2.0));
    }

    #[test]
    fn jump_is_half_the_original_impulse() {
        let dir = Vec3::new(0.0, 3.0, 10.0).normalize();
        let got = jump_dir(2.0);
        let want = 2.5 * 2.0 * dir;
        assert!((got - want).length() < 1e-5, "jump_dir {got:?} vs {want:?}");
        assert!(
            (got.length() * 2.0 - 5.0 * 2.0 * dir.length()).abs() < 1e-4,
            "max jump was not halved"
        );
    }

    fn water_level(flood: u8, ground: u8) -> level::Level {
        let mut level = level::load(
            &level::LevelConfig::new_test(),
            &crate::config::settings::Geometry::default(),
        );
        let bits = level.terrain_bits();
        let water = bits.write(0);
        for meta in level.meta.iter_mut() {
            *meta = water;
        }
        for h in level.height.iter_mut() {
            *h = ground;
        }
        for band in level.flood_map.iter_mut() {
            *band = flood;
        }
        level
    }

    fn step_in_water(
        dynamo: &mut Dynamo,
        transform: &mut space::Transform,
        car: &CarPhysicsData,
        level: &level::Level,
    ) {
        step(
            dynamo,
            transform,
            0.02,
            car,
            level,
            &config::common::Common::test_default(),
            1.0,
            0.0,
            None,
            0.0,
            None,
            None,
        );
    }

    #[test]
    fn a_car_in_deep_water_floats_instead_of_sinking() {
        let level = water_level(100, 20);
        let car = CarPhysicsData::test_default();
        let mut transform = space::Transform {
            scale: 1.0,
            rot: Quat::IDENTITY,
            disp: Vec3::new(40.0, 40.0, 50.0),
        };
        let mut dynamo = Dynamo::default();
        for _ in 0..200 {
            step_in_water(&mut dynamo, &mut transform, &car, &level);
        }
        assert!(
            transform.disp.z > 70.0,
            "should sit near the water line, got z={}",
            transform.disp.z
        );
        assert!(
            transform.disp.z < 120.0,
            "should not fly out of the water, got z={}",
            transform.disp.z
        );
    }

    #[test]
    fn a_car_in_deep_water_moves_on_traction() {
        let level = water_level(100, 20);
        let car = CarPhysicsData::test_default();
        let spawn = || {
            (
                Dynamo::default(),
                space::Transform {
                    scale: 1.0,
                    rot: Quat::IDENTITY,
                    disp: Vec3::new(40.0, 40.0, 80.0),
                },
            )
        };
        let (mut idle_dyn, mut idle_tf) = spawn();
        let (mut drive_dyn, mut drive_tf) = spawn();
        drive_dyn.traction = 1.0;
        for _ in 0..8 {
            step_in_water(&mut idle_dyn, &mut idle_tf, &car, &level);
            step_in_water(&mut drive_dyn, &mut drive_tf, &car, &level);
            drive_dyn.traction = 1.0;
        }
        assert!(
            drive_dyn.linear_velocity.y > idle_dyn.linear_velocity.y + 0.5,
            "water traction should add forward speed, idle={:?} drive={:?}",
            idle_dyn.linear_velocity,
            drive_dyn.linear_velocity
        );
    }

    #[test]
    fn water_steering_matches_the_left_key() {
        // KeyA / LEFT raises rudder (see bin/road and bin/web). Facing +Y,
        // a left turn points the nose toward -X.
        let level = water_level(100, 20);
        let car = CarPhysicsData::test_default();
        let mut transform = space::Transform {
            scale: 1.0,
            rot: Quat::IDENTITY,
            disp: Vec3::new(40.0, 40.0, 80.0),
        };
        let mut dynamo = Dynamo {
            traction: 1.0,
            rudder: 0.4,
            ..Dynamo::default()
        };
        for _ in 0..24 {
            step_in_water(&mut dynamo, &mut transform, &car, &level);
            dynamo.traction = 1.0;
            dynamo.rudder = 0.4;
        }
        let forward = transform.rot * Vec3::Y;
        assert!(
            forward.x < -0.02,
            "left rudder should yaw the nose left, forward={forward:?}"
        );
    }

    #[test]
    fn a_car_floats_at_the_flood_plane_not_the_raw_byte() {
        let mut level = water_level(200, 20);
        level.geometry.height = 0x80;
        let flood_z = level.flood_level_at(40);
        let want = 200.0 / 255.0 * 128.0;
        assert!(
            (flood_z - want).abs() < 1e-3,
            "flood texture scale {want}, got {flood_z}"
        );
        let car = CarPhysicsData::test_default();
        let mut transform = space::Transform {
            scale: 1.0,
            rot: Quat::IDENTITY,
            disp: Vec3::new(40.0, 40.0, 60.0),
        };
        let mut dynamo = Dynamo::default();
        for _ in 0..200 {
            step_in_water(&mut dynamo, &mut transform, &car, &level);
        }
        assert!(
            (transform.disp.z - flood_z).abs() < 25.0,
            "should sit near the flood plane {flood_z}, got z={}",
            transform.disp.z
        );
    }

    #[test]
    fn flood_height_follows_the_band_under_the_car() {
        let mut level = water_level(40, 20);
        level.flood_map = vec![40u8, 200].into_boxed_slice();
        let h = level.geometry.height as f32;
        assert!((level.flood_level_at(40) - 40.0 / 255.0 * h).abs() < 1e-3);
        assert!((level.flood_level_at(200) - 200.0 / 255.0 * h).abs() < 1e-3);
        let car = CarPhysicsData::test_default();
        let mut transform = space::Transform {
            scale: 1.0,
            rot: Quat::IDENTITY,
            disp: Vec3::new(40.0, 200.0, 160.0),
        };
        let mut dynamo = Dynamo::default();
        for _ in 0..200 {
            step_in_water(&mut dynamo, &mut transform, &car, &level);
        }
        let flood_z = level.flood_level_at(200);
        assert!(
            (transform.disp.z - flood_z).abs() < 25.0,
            "should sit on the far band at {flood_z}, got z={}",
            transform.disp.z
        );
    }
}
