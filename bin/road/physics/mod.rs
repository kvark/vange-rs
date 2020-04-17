//! Physics port of the original game. Most closely described by the following documents:
//! - https://people.eecs.berkeley.edu/~jfc/mirtich/thesis/mirtichThesis.pdf

use vangers::{config, level, render::debug::LineBuffer, space};

use cgmath::prelude::*;

use std::f32::EPSILON;

mod rigid;
mod terrain;

pub use terrain::get_height;

const MAX_TRACTION: config::common::Traction = 4.0;

#[derive(Debug)]
struct AccelerationVectors {
    f: cgmath::Vector3<f32>, // linear
    k: cgmath::Vector3<f32>, // angular
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
    pub fn change_traction(&mut self, delta: config::common::Traction) {
        self.traction = (self.traction + delta).min(MAX_TRACTION).max(-MAX_TRACTION);
    }

    pub fn slow_down(&mut self, delta: config::common::Traction) {
        let old = self.traction;
        self.change_traction(delta * -old.signum());
        if old * self.traction < 0.0 {
            self.traction = 0.0;
        }
    }
}

pub fn jump_dir(power: f32) -> cgmath::Vector3<f32> {
    5.0 * power * cgmath::vec3(0.0, 3.0, 10.0).normalize()
}

pub fn step(
    dynamo: &mut Dynamo,
    transform: &mut space::Transform,
    dt: f32,
    car: &config::car::CarInfo,
    level: &level::Level,
    common: &config::common::Common,
    f_turbo: f32,
    f_brake: f32,
    jump: Option<f32>,
    roll: f32,
    mut line_buffer: Option<&mut LineBuffer>,
) {
    let speed_correction_factor = dt / common.nature.time_delta0;
    let acc_global = AccelerationVectors {
        f: cgmath::vec3(0.0, 0.0, -common.nature.gravity),
        k: cgmath::vec3(0.0, 0.0, 0.0),
    };
    let rot_inv = transform.rot.invert();
    log::debug!("dt {}, num {}", dt, common.nature.num_calls_analysis);
    let flood_level = level.flood_map[0] as f32;
    // Z axis in the local coordinate space
    let z_axis = rot_inv * cgmath::Vector3::unit_z();
    let device_modulation = 1.0;
    let dt_impulse = 1.0;

    let mut rigid = {
        let phys = &car.model.body.physics;
        let jacobian =
            cgmath::Matrix3::from(phys.jacobi) * (transform.scale * transform.scale / phys.volume);
        rigid::RigidBody::new(&jacobian, dynamo.linear_velocity, dynamo.angular_velocity)
    };

    if let Some(power) = jump {
        let mass = common.nature.density
            * car.model.body.physics.volume
            * transform.scale
            * transform.scale;
        let f = device_modulation * common.force.k_distance_to_force * dt_impulse / mass.powf(0.3);
        log::info!("jump mass {:?}, f {:?}", mass, f);
        //DBV dV = A_g2l*DBV(-Sin(Pi/10)*Sin(psi),-Sin(Pi/10)*Cos(psi),Cos(Pi/10));
        rigid.vel += f * jump_dir(power);
    }

    let mut wheels_touch = 0u32;
    let mut spring_touch = 0;
    //let mut in_water = false;

    let mut float_count = 0;
    let (mut terrain_immersion, mut water_immersion) = (0.0, 0.0);
    let stand_on_wheels =
        z_axis.z > 0.0 && (transform.rot * cgmath::Vector3::unit_x()).z.abs() < 0.7;
    let modulation = 1.0;
    let mut acc_cur = AccelerationVectors {
        f: rot_inv * acc_global.f,
        k: rot_inv * acc_global.k,
    };

    let mut down_minus_up = 0i32;
    let mut acc_springs = AccelerationVectors {
        f: cgmath::Vector3::zero(),
        k: cgmath::Vector3::zero(),
    };

    let mut sum_count = 0usize;
    let mut sum_rg0 = cgmath::Vector3::zero();
    let mut sum_df = 0.;

    for (bound_poly_id, poly) in car.model.shape.polygons.iter().enumerate() {
        let r = cgmath::Vector3::from(poly.middle) * (transform.scale * car.physics.scale_bound);
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
        match level.get((rglob.x as i32, rglob.y as i32)) {
            level::Texel::Single(level::Point(_, 0))
            | level::Texel::Dual {
                low: level::Point(_, 0),
                ..
            } => {
                let dz = flood_level - rglob.z;
                if dz > 0.0 {
                    float_count += 1;
                    water_immersion += dz;
                }
            }
            _ => {}
        };
        let poly_norm = cgmath::Vector3::from(poly.normal).normalize();
        if z_axis.dot(poly_norm) < 0.0 {
            let cdata = terrain::CollisionData::collide_low(
                poly,
                &car.model.shape.samples,
                car.physics.scale_bound,
                &transform,
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
            let mostly_horisontal = {
                let tmp = rigid.velocity_at(r);
                tmp.z * tmp.z < tmp.x * tmp.x + tmp.y * tmp.y
            };
            match cdata {
                terrain::CollisionData {
                    hard: Some(ref cp), ..
                } if mostly_horisontal => {
                    let r1 = rot_inv * cgmath::vec3(cp.pos.x - origin.x, cp.pos.y - origin.y, 0.0); // ignore vertical
                    let pv = rigid.velocity_at(r1);
                    let normal = {
                        let bm = car.model.body.bbox.max;
                        let n = cgmath::vec3(r1.x / bm[0], r1.y / bm[1], r1.z / bm[2]);
                        n.normalize()
                    };
                    let dot = pv.dot(normal);
                    if dot > 0.0 {
                        rigid.push(r, normal * (dot * -common.impulse.factors[0] * modulation));
                    }
                }
                terrain::CollisionData {
                    soft: Some(ref cp), ..
                } => {
                    //TODO: let r1 = rot_inv * (cp.pos - origin);
                    let r1 =
                        rot_inv * cgmath::vec3(cp.pos.x - origin.x, cp.pos.y - origin.y, rg0.z);
                    let pv = rigid.velocity_at(r1);
                    if pv.dot(z_axis) < 0.0 {
                        let vec = if stand_on_wheels {
                            // ignore XY
                            cgmath::vec3(0.0, 0.0, pv.z)
                        } else {
                            let projected = poly_norm * poly_norm.dot(pv);
                            common.impulse.k_friction * pv
                                + (1.0 - common.impulse.k_friction) * projected
                        };
                        rigid.push(r, vec * (-common.impulse.factors[1] * modulation));
                    }
                }
                _ => (),
            }
            if let Some(ref cp) = cdata.soft {
                let df0 = common.contact.k_elastic_spring * cp.depth * modulation;
                let df = df0.min(common.impulse.elastic_restriction);
                log::debug!("\t\tbound[{}] dF.z = {}, rg0={:?}", bound_poly_id, df, rg0);
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
        log::debug!("Avg df {} rg0 {:?}", sum_df * kf, sum_rg0 * kf);
    }

    if wheels_touch + spring_touch != 0 {
        log::debug!("\tsprings total {:?}", acc_springs);
        acc_cur.f += rot_inv * acc_springs.f;
        acc_cur.k += rot_inv * acc_springs.k;
    }

    // apply drag
    let mut v_drag = common.drag.free.v * common.drag.speed.v.powf(rigid.vel.magnitude());
    let mut w_drag = common.drag.free.w
        * common
            .drag
            .speed
            .w
            .powf(rigid.angular_velocity().magnitude2()); //why mag2?
    if wheels_touch > 0 {
        //TODO: why `ln()`?
        let speed =
            common.drag.wheel_speed.ln() * car.physics.mobility_factor * common.global.speed_factor
                / car.physics.speed_factor;
        rigid.vel.y *= (1.0 + speed).powf(speed_correction_factor);
    }

    let _ = (float_count, water_immersion, terrain_immersion); //TODO
    let is_after_collision = false;
    if wheels_touch != 0 && stand_on_wheels {
        let f_traction_per_wheel =
            car.physics.mobility_factor * common.global.mobility_factor * f_turbo * dynamo.traction
                / (car.model.wheels.len() as f32);
        let rudder_vec = {
            let (sin, cos) = dynamo.rudder.sin_cos();
            cgmath::vec3(cos, -sin, 0.0)
        };
        for wheel in car.model.wheels.iter() {
            let pw = transform.transform_point(cgmath::Point3::from(wheel.pos));
            let detect_wheel_hits = false;
            if detect_wheel_hits {
                let dist = terrain::get_distance_to_terrain(level, pw);
                if dist > 0.0 {
                    continue;
                }
            }

            let rx_max = if wheel.pos[0] > 0.0 {
                car.model.body.bbox.max[0]
            } else {
                car.model.body.bbox.min[0]
            };
            let pos = cgmath::vec3(rx_max, wheel.pos[1], wheel.pos[2]) * transform.scale;
            let pv = rigid.velocity_at(pos);

            acc_cur.f.y += f_traction_per_wheel;
            acc_cur.f -= pv * f_brake;

            if !is_after_collision {
                let dir = if wheel.steer != 0 {
                    rudder_vec
                } else {
                    cgmath::Vector3::unit_x()
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

    if spring_touch + wheels_touch != 0 {
        //|| in_water
        let tmp = cgmath::Vector3::new(
            0.0,
            0.0,
            car.physics.z_offset_of_mass_center * transform.scale,
        );
        acc_cur.k -= common.nature.gravity * tmp.cross(z_axis);
        let vz = z_axis.dot(rigid.vel);
        if vz < -10.0 {
            v_drag *= common.drag.z.powf(-vz);
        }
    }

    if roll != 0.0 && wheels_touch == 0 && spring_touch != 0 {
        let df = common.force.f_spring_impulse * speed_correction_factor;
        let x_edge = if roll > 0.0 {
            car.model.body.bbox.max[0]
        } else {
            car.model.body.bbox.min[0]
        };
        rigid.add_raw(
            cgmath::vec3(0.0, 0.0, df),
            cgmath::vec3(0.0, df * x_edge * transform.scale, 0.0),
        );
    }

    log::debug!("\tcur acc {:?}", acc_cur);
    rigid.add_raw(acc_cur.f * dt, acc_cur.k * dt);
    let (mut v_vel, mut w_vel) = rigid.finish();

    //log::debug!("J_inv {:?}, handedness {}", j_inv.transpose(), j_inv.x.cross(j_inv.y).dot(j_inv.z));
    log::debug!("\tresulting v={:?} w={:?}", v_vel, w_vel);
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
        let radius = car.model.body.bbox.radius; //approx?
        let local_z_scaled = z_axis * (radius * common.impulse.rolling_scale);
        let r_diff_sign = down_minus_up.signum() as f32;
        let vs = v_vel - r_diff_sign * local_z_scaled.cross(w_vel);

        let angle = cgmath::Rad(-dt * w_mag);
        let vel_rot_inv = cgmath::Quaternion::from_axis_angle(w_vel / (w_mag + EPSILON), angle);
        transform.disp += (transform.rot * vs) * dt;
        transform.rot = transform.rot * vel_rot_inv.invert();
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
    //log::debug!("\tdrag v={} w={}", v_drag, w_drag);
    v_vel *= v_drag.powf(speed_correction_factor);
    w_vel *= w_drag.powf(speed_correction_factor);

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

    dynamo.linear_velocity = v_vel;
    dynamo.angular_velocity = w_vel;
    // unsteer
    if dynamo.rudder.0 != 0.0 && wheels_touch != 0 {
        let change = dynamo.rudder.0 * v_vel.y * dt * common.car.rudder_k_decr;
        dynamo.rudder.0 -= dynamo.rudder.0.signum() * change.abs();
    }
    // slow down
    dynamo.slow_down(dt * common.car.traction_decr);
}
