use cgmath;
use cgmath::prelude::*;
use gfx;
use std::f32::EPSILON;

use vangers::{config, level, model, space};
use vangers::render::LineBuffer;


const MAX_TRACTION: config::common::Traction = 4.0;

#[derive(Debug)]
struct AccelerationVectors {
    f: cgmath::Vector3<f32>, // linear
    k: cgmath::Vector3<f32>, // angular
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
    fn add(
        &mut self,
        pos: cgmath::Vector3<f32>,
        depth: f32,
    ) {
        self.pos += pos;
        self.depth += depth;
        self.count += 1.0;
    }
    fn finish(
        &self,
        min: f32,
    ) -> Option<CollisionPoint> {
        if self.count > min {
            Some(CollisionPoint {
                pos: self.pos / self.count,
                depth: self.depth / self.count,
            })
        } else {
            None
        }
    }
}


pub struct Dynamo {
    pub traction: config::common::Traction,
    pub rudder: cgmath::Rad<f32>,
    pub linear_velocity: cgmath::Vector3<f32>,
    pub angular_velocity: cgmath::Vector3<f32>,
}

impl Default for Dynamo {
    fn default() -> Self {
        Dynamo {
            traction: 0.,
            rudder: cgmath::Rad(0.),
            linear_velocity: cgmath::Vector3::zero(),
            angular_velocity: cgmath::Vector3::zero(),
        }
    }
}

impl Dynamo {
    pub fn change_traction(
        &mut self,
        delta: config::common::Traction,
    ) {
        let old = self.traction;
        self.traction = (old + delta).min(MAX_TRACTION).max(-MAX_TRACTION);
        if old * self.traction < 0.0 {
            self.traction = 0.0; // full stop
        }
    }
}


pub fn get_height(altitude: u8) -> f32 {
    altitude as f32 * (level::HEIGHT_SCALE as f32) / 256.0
}

fn collide_low(
    poly: &model::Polygon,
    samples: &[model::RawVertex],
    scale: f32,
    transform: &space::Transform,
    level: &level::Level,
    terraconf: &config::common::Terrain,
) -> CollisionData {
    let (mut soft, mut hard) = (Accumulator::new(), Accumulator::new());
    for s in samples[poly.samples.clone()].iter() {
        let sp = cgmath::Point3::from(*s).cast::<f32>();
        let pos = transform.transform_point(sp * scale).to_vec();
        let texel = level.get((pos.x as i32, pos.y as i32));
        let lo_alt = texel.low.0;
        let height = match texel.high {
            Some((delta, hi_alt, _)) => {
                let middle = get_height(lo_alt.saturating_add(delta));
                if pos.z > middle {
                    let high = get_height(hi_alt);
                    if pos.z - middle > high - pos.z {
                        high
                    } else {
                        continue;
                    }
                } else {
                    get_height(lo_alt)
                }
            }
            None => get_height(lo_alt),
        };
        let dz = height - pos.z;
        //debug!("\t\t\tSample h={:?} at {:?}, dz={}", height, pos, dz);
        if dz > terraconf.min_wall_delta {
            //debug!("\t\t\tHard touch of {} at {:?}", dz, pos);
            hard.add(pos, dz);
        } else if dz > 0.0 {
            //debug!("\t\t\tSoft touch of {} at {:?}", dz, pos);
            soft.add(pos, dz);
        }
    }
    CollisionData {
        soft: (if soft.count > 0.0 { &soft } else { &hard }).finish(0.0),
        hard: hard.finish(4.0),
    }
}

fn calc_collision_matrix_inv(
    r: cgmath::Vector3<f32>,
    ji: &cgmath::Matrix3<f32>,
) -> cgmath::Matrix3<f32> {
    let t3 = -r.z * ji[1][1] + r.y * ji[2][1];
    let t7 = -r.z * ji[1][2] + r.y * ji[2][2];
    let t12 = -r.z * ji[1][0] + r.y * ji[2][0];
    let t21 = r.z * ji[0][1] - r.x * ji[2][1];
    let t25 = r.z * ji[0][2] - r.x * ji[2][2];
    let t30 = r.z * ji[0][0] - r.x * ji[2][0];
    let t39 = -r.y * ji[0][1] + r.x * ji[1][1];
    let t43 = -r.y * ji[0][2] + r.x * ji[1][2];
    let t48 = -r.y * ji[0][0] + r.x * ji[1][0];
    let cm = cgmath::Matrix3::new(
        1.0 - t3 * r.z + t7 * r.y,
        t12 * r.z - t7 * r.x,
        -t12 * r.y + t3 * r.x,
        -t21 * r.z + t25 * r.y,
        1.0 + t30 * r.z - t25 * r.x,
        -t30 * r.y + t21 * r.x,
        -t39 * r.z + t43 * r.y,
        t48 * r.z - t43 * r.x,
        1.0 - t48 * r.y + t39 * r.x,
    );
    cm.invert().unwrap()
}


pub fn step<R: gfx::Resources>(
    dynamo: &mut Dynamo,
    transform: &mut space::Transform,
    dt: f32,
    car: &config::car::CarInfo<R>,
    level: &level::Level,
    common: &config::common::Common,
    f_turbo: f32,
    f_brake: f32,
    mut line_buffer: Option<&mut LineBuffer>,
) {
    let acc_global = AccelerationVectors {
        f: cgmath::vec3(0.0, 0.0, -common.nature.gravity),
        k: cgmath::vec3(0.0, 0.0, 0.0),
    };
    let rot_inv = transform.rot.invert();
    debug!("dt {}, num {}", dt, common.nature.num_calls_analysis);
    let flood_level = level.flood_map[0] as f32;
    // Z axis in the local coordinate space
    let z_axis = rot_inv * cgmath::Vector3::unit_z();
    let mut v_vel = dynamo.linear_velocity;
    let mut w_vel = dynamo.angular_velocity;
    let j_inv = {
        let phys = &car.model.body.physics;
        (cgmath::Matrix3::from(phys.jacobi)
            * (transform.scale * transform.scale / phys.volume))
            .invert()
            .unwrap()
    };

    let mut wheels_touch = 0u32;
    let mut spring_touch;
    //let mut in_water = false;

    /*for _ in 0 .. common.nature.num_calls_analysis*/
    {
        let mut float_count = 0;
        let (mut terrain_immersion, mut water_immersion) = (0.0, 0.0);
        let stand_on_wheels =
            z_axis.z > 0.0 && (transform.rot * cgmath::Vector3::unit_x()).z.abs() < 0.7;
        let modulation = 1.0;
        let mut acc_cur = AccelerationVectors {
            f: rot_inv * acc_global.f,
            k: rot_inv * acc_global.k,
        };

        // apply drag
        let mut v_drag = common.drag.free.v * common.drag.speed.v.powf(v_vel.magnitude());
        let mut w_drag = common.drag.free.w * common.drag.speed.w.powf(w_vel.magnitude2()); //why mag2?
        if wheels_touch > 0 {
            //TODO: why `ln()`?
            let speed = common.drag.wheel_speed.ln() * car.physics.mobility_factor
                * common.global.speed_factor
                / car.physics.speed_factor;
            v_vel.y *= (1.0 + speed).powf(config::common::SPEED_CORRECTION_FACTOR);
        }
        wheels_touch = 0;
        spring_touch = 0;
        let mut down_minus_up = 0i32;
        let mut acc_springs = AccelerationVectors {
            f: cgmath::Vector3::zero(),
            k: cgmath::Vector3::zero(),
        };

        let mut sum_count = 0usize;
        let mut sum_rg0 = cgmath::Vector3::zero();
        let mut sum_df = 0.;

        for (bound_poly_id, poly) in car.model.shape.polygons.iter().enumerate() {
            let r = cgmath::Vector3::from(poly.middle)
                * (transform.scale * car.physics.scale_bound);
            let rg0 = transform.rot * r;
            let rglob = rg0 + transform.disp;
            debug!(
                "\t\tpoly[{}]: normal={:?} scale={} mid={:?} r={:?}",
                bound_poly_id,
                poly.normal,
                transform.scale * car.physics.scale_bound,
                poly.middle,
                r
            );
            //let vr = v_vel + w_vel.cross(r);
            //let mostly_horisontal = vr.z*vr.z < vr.x*vr.x + vr.y*vr.y;
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
                let cdata = collide_low(
                    poly,
                    &car.model.shape.samples,
                    car.physics.scale_bound,
                    &transform,
                    level,
                    &common.terrain,
                );
                debug!("\t\tcollide_low = {:?}", cdata);
                terrain_immersion += match cdata.soft {
                    Some(ref cp) => cp.depth.abs(),
                    None => 0.0,
                };
                terrain_immersion += match cdata.hard {
                    Some(ref cp) => cp.depth.abs(),
                    None => 0.0,
                };
                /*
                let origin = self.transform.disp;
                match cdata {
                    CollisionData{ hard: Some(ref cp), ..} if mostly_horisontal => {
                        let r1 = rot_inv * cgmath::vec3(
                            cp.pos.x - origin.x, cp.pos.y - origin.y, 0.0); // ignore vertical
                        let normal = {
                            let bm = self.car.model.body.bbox.1;
                            let n = cgmath::vec3(r1.x / bm[0], r1.y / bm[1], r1.z / bm[2]);
                            n.normalize()
                        };
                        let u0 = v_vel + w_vel.cross(r1);
                        let dot = u0.dot(normal);
                        if dot > 0.0 {
                            let pulse = (calc_collision_matrix_inv(r1, &j_inv) * normal) *
                                (-common.impulse.factors[0] * modulation * dot);
                            debug!("\t\tCollision speed {:?} pulse {:?}", v_vel, pulse);
                            v_vel += pulse;
                            w_vel += j_inv * r1.cross(pulse);
                        }
                    },
                    CollisionData{ soft: Some(ref cp), ..} => {
                        let r1 = rot_inv * cgmath::vec3(cp.pos.x - origin.x, cp.pos.y - origin.y, rg0.z);
                        //TODO: let r1 = rot_inv * (cp.pos - origin);
                        let mut u0 = v_vel + w_vel.cross(r1);
                        debug!("\t\tContact {:?}\n\t\t\torigin={:?}\n\t\t\tu0 = {:?}", cp, origin, u0);
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
                            debug!("\t\tCollision momentum {:?}\n\t\t\tmatrix {:?}\n\t\t\tsample {:?}\n\t\t\tspeed {:?}\n\t\t\tpulse {:?}",
                                u0, cmi, r, v_vel, pulse);
                            v_vel += pulse;
                            w_vel += j_inv * r.cross(pulse);
                        }
                    }
                    _ => (),
                }*/
                if let Some(ref cp) = cdata.soft {
                    let df0 = common.contact.k_elastic_spring * cp.depth * modulation;
                    let df = df0.min(common.impulse.elastic_restriction);
                    debug!("\t\tbound[{}] dF.z = {}, rg0={:?}", bound_poly_id, df, rg0);
                    acc_springs.f.z += df;
                    acc_springs.k.x += rg0.y * df;
                    acc_springs.k.y -= rg0.x * df;
                    //let impulse = cgmath::vec3(0., 0., df);
                    //acc_springs.f += impulse;
                    //acc_springs.k += rg0.cross(impulse);
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
                        // Red: center -> collision point
                        lbuf.add(transform.disp.into(), rglob.into(), 0xFF000000);
                        // Yellow: collision point -> linear force
                        let up = rglob + cgmath::vec3(0.0, 0.0, df0);
                        lbuf.add(rglob.into(), up.into(), 0xFFFF0000);
                        // Purple: collision point -> angular force
                        let end = rglob + df * cgmath::vec3(rg0.y, -rg0.x, 0.0);
                        lbuf.add(rglob.into(), end.into(), 0xFF00FF00);
                    }
                }
            } else {
                //TODO: upper average
                // down_minus_up -= 1;
            }
        }

        if sum_count != 0 {
            let kf = 1.0 / sum_count as f32;
            debug!("Avg df {} rg0 {:?}", sum_df * kf, sum_rg0 * kf);
        }

        if wheels_touch + spring_touch != 0 {
            debug!("\tsprings total {:?}", acc_springs);
            acc_cur.f += rot_inv * acc_springs.f;
            acc_cur.k += rot_inv * acc_springs.k;
        }

        let _ = (float_count, water_immersion, terrain_immersion); //TODO
        let is_after_collision = false;
        if wheels_touch != 0 && stand_on_wheels {
            let f_traction_per_wheel = car.physics.mobility_factor
                * common.global.mobility_factor
                * f_turbo
                * dynamo.traction
                / (car.model.wheels.len() as f32);
            let rudder_vec = {
                let (sin, cos) = dynamo.rudder.sin_cos();
                cgmath::vec3(cos, -sin, 0.0)
            };
            for wheel in car.model.wheels.iter() {
                let rx_max = if wheel.pos[0] > 0.0 {
                    car.model.body.bbox.1[0]
                } else {
                    car.model.body.bbox.0[0]
                };
                let pos = cgmath::vec3(rx_max, wheel.pos[1], wheel.pos[2])
                    * transform.scale;
                acc_cur.f.y += f_traction_per_wheel;

                let vw = v_vel + w_vel.cross(pos);
                acc_cur.f -= vw * f_brake;

                if !is_after_collision {
                    let normal = if wheel.steer != 0 {
                        rudder_vec
                    } else {
                        cgmath::Vector3::unit_x()
                    };
                    let u0 = normal * vw.dot(normal);
                    let mx = calc_collision_matrix_inv(pos, &j_inv);
                    let pulse = -common.impulse.k_wheel * (mx * u0);
                    v_vel += pulse;
                    w_vel += j_inv * pos.cross(pulse);
                    if let Some(ref mut lbuf) = line_buffer {
                        let pw = transform.transform_point(cgmath::Point3::from(wheel.pos));
                        let dest = pw + transform.transform_vector(pulse) * 10.0;
                        lbuf.add(pw.into(), dest.into(), 0xFFFFFF00);
                    }
                }
            }
        }

        if spring_touch + wheels_touch != 0 { //|| in_water
            let tmp = cgmath::Vector3::new(
                0.0,
                0.0,
                car.physics.z_offset_of_mass_center * transform.scale,
            );
            acc_cur.k -= common.nature.gravity * tmp.cross(z_axis);
            let vz = z_axis.dot(v_vel);
            if vz < -10.0 {
                v_drag *= common.drag.z.powf(-vz);
            }
        }

        debug!("\tcur acc {:?}", acc_cur);
        v_vel += acc_cur.f * dt;
        w_vel += (j_inv * acc_cur.k) * dt;
        //debug!("J_inv {:?}, handedness {}", j_inv.transpose(), j_inv.x.cross(j_inv.y).dot(j_inv.z));
        debug!("\tresulting v={:?} w={:?}", v_vel, w_vel);
        if spring_touch != 0 {
            v_drag *= common.drag.spring.v;
            w_drag *= common.drag.spring.w;
        }
        let (v_mag, w_mag) = (v_vel.magnitude(), w_vel.magnitude());
        if stand_on_wheels && v_mag < common.drag.abs_min.v && w_mag < common.drag.abs_min.w {
            let v_pow = common.drag.abs_min.v / (v_mag + EPSILON);
            let w_pow = common.drag.abs_min.w / (w_mag + EPSILON);
            v_drag *= common.drag.coll.v.powf(v_pow);
            w_drag *= common.drag.coll.w.powf(w_pow);
        }

        if v_mag * v_drag > common.drag.abs_stop.v || w_mag * w_drag > common.drag.abs_stop.w {
            let radius = car.model.body.bbox.2; //approx?
            let local_z_scaled = z_axis * (radius * common.impulse.rolling_scale);
            let r_diff_sign = down_minus_up.signum() as f32;
            let vs = v_vel - r_diff_sign * local_z_scaled.cross(w_vel);

            let angle = cgmath::Rad(-dt * w_mag);
            let vel_rot_inv =
                cgmath::Quaternion::from_axis_angle(w_vel / (w_mag + EPSILON), angle);
            transform.disp += (transform.rot * vs) * dt;
            transform.rot = transform.rot * vel_rot_inv.invert();
            v_vel = vel_rot_inv * v_vel;
            w_vel = vel_rot_inv * w_vel;
            debug!(
                "\tvs={:?} {:?}\n\t\tdisp {:?} scale {}",
                vs, transform.rot, transform.disp, transform.scale
            );
        }
        //debug!("\tdrag v={} w={}", v_drag, w_drag);
        v_vel *= v_drag.powf(config::common::SPEED_CORRECTION_FACTOR);
        w_vel *= w_drag.powf(config::common::SPEED_CORRECTION_FACTOR);

        if let Some(ref mut lbuf) = line_buffer {
            // Note: velocity and acceleration are in local space
            let rot = transform.rot;
            let ba = transform.disp + cgmath::vec3(3.0, 0.0, 10.0);
            let xf = ba + rot * acc_cur.f;
            let xk = ba + rot * acc_cur.k;
            lbuf.add(ba.into(), xf.into(), 0x0000FF00);
            lbuf.add(ba.into(), xk.into(), 0xFF00FF00);
            // Yellow: center -> angular springs total
            lbuf.add(ba.into(), (ba + acc_springs.k).into(), 0xFFFF0000);
            let bv = transform.disp + cgmath::vec3(-3.0, 0.0, 10.0);
            let xv = bv + rot * v_vel;
            let xw = bv + rot * w_vel * 10.0; //TEMP
            lbuf.add(bv.into(), xv.into(), 0x00FF0000);
            lbuf.add(bv.into(), xw.into(), 0x00FFFF00);
        }
    }

    dynamo.linear_velocity = v_vel;
    dynamo.angular_velocity = w_vel;
    // unsteer
    if dynamo.rudder.0 != 0.0 && wheels_touch != 0 {
        let change = dynamo.rudder.0 * v_vel.y * dt * common.car.rudder_k_decr;
        dynamo.rudder.0 -= dynamo.rudder.0.signum() * change.abs();
    }
    // slow down
    let traction_step = -dynamo.traction.signum() * dt;
    dynamo
        .change_traction(traction_step * common.car.traction_decr);
}
