//! Car-vs-car contact: a port of the original `Object::test_object_to_baseobject`
//! (`src/3d/dynamics.cpp`) for two mechos.
//!
//! The original runs a bounding-sphere precheck, an oriented-box overlap test
//! on the `scale_box`-shrunk model bounds, finds the contact point with
//! `box_to_box_intersection`, and exchanges `P = -2 K⁻¹ u0` along the line
//! between the centres, `K` being both bodies' collision matrices over their
//! masses. That is a fully elastic exchange with no positional correction;
//! the port adds a small push-apart so two cars pressed together by traction
//! do not sink into each other between impulses.

use super::{CarPhysicsData, Dynamo, rigid};
use crate::{config, space};

use glam::{Mat3, Vec3};

/// `1 + RESTITUTION` is the original's `-2` factor: fully elastic along the
/// normal. Lower it to make bumps less bouncy.
pub const RESTITUTION: f32 = 1.0;
/// Fraction of the box overlap pushed apart per call (split by inverse
/// mass). The original has none; this only keeps resting contact honest.
pub const SEPARATION: f32 = 0.5;
/// Overlap tolerated without a positional push, in world units.
const SLOP: f32 = 0.1;

/// One side of a car-car contact: the same state `physics::step` integrates.
pub struct CarBody<'a> {
    pub dynamo: &'a mut Dynamo,
    pub transform: &'a mut space::Transform,
    pub car: &'a CarPhysicsData,
}

#[derive(Clone, Copy, Debug)]
pub struct CarContact {
    /// World-space contact point, in `a`'s frame of the level wrap.
    pub point: Vec3,
    /// Unit normal from `a` towards `b`.
    pub normal: Vec3,
    /// Magnitude of the exchanged impulse (0 when the cars were already
    /// separating and only the overlap was resolved).
    pub impulse: f32,
    /// Box overlap depth along the least-penetration face axis.
    pub depth: f32,
}

/// Same mass `physics::step` uses for the jump impulse.
pub fn body_mass(car: &CarPhysicsData, scale: f32, common: &config::common::Common) -> f32 {
    common.nature.density * car.body_physics.volume * scale * scale
}

/// Inverse inertia per unit mass, as `physics::step` builds its rigid body.
fn inertia_inv(car: &CarPhysicsData, scale: f32) -> Mat3 {
    let phys = &car.body_physics;
    (Mat3::from_cols_array_2d(&phys.jacobi) * (scale * scale / phys.volume)).inverse()
}

struct Obb {
    center: Vec3,
    axes: [Vec3; 3],
    half: Vec3,
}

impl Obb {
    fn new(car: &CarPhysicsData, transform: &space::Transform, origin: Vec3) -> Self {
        let s = transform.scale * car.physics.scale_box;
        let min = Vec3::from(car.bbox.min) * s;
        let max = Vec3::from(car.bbox.max) * s;
        let rot = transform.rot;
        Obb {
            center: origin + rot * ((min + max) * 0.5),
            axes: [rot * Vec3::X, rot * Vec3::Y, rot * Vec3::Z],
            half: (max - min) * 0.5,
        }
    }

    fn project(&self, axis: Vec3) -> f32 {
        self.axes
            .iter()
            .zip(self.half.to_array())
            .map(|(a, h)| a.dot(axis).abs() * h)
            .sum()
    }
}

/// Separating-axis test over the 15 box axes. Returns the least overlap
/// among the 6 face axes and that axis, oriented from `a` to `b`.
fn overlap(a: &Obb, b: &Obb) -> Option<(f32, Vec3)> {
    let t = b.center - a.center;
    let mut best = (f32::MAX, Vec3::ZERO);
    let mut test = |axis: Vec3, face: bool| -> bool {
        if axis.length_squared() < 1e-6 {
            return true;
        }
        let axis = axis.normalize();
        let d = t.dot(axis);
        let depth = a.project(axis) + b.project(axis) - d.abs();
        if depth < 0.0 {
            return false;
        }
        if face && depth < best.0 {
            best = (depth, if d < 0.0 { -axis } else { axis });
        }
        true
    };
    for axis in a.axes.iter().chain(b.axes.iter()) {
        if !test(*axis, true) {
            return None;
        }
    }
    for ea in a.axes.iter() {
        for eb in b.axes.iter() {
            if !test(ea.cross(*eb), false) {
                return None;
            }
        }
    }
    Some(best)
}

/// Port of the original `box_to_box_intersection`, in `a`'s box frame:
/// `rr` is `b`'s box centre, `x`/`y`/`z` its half axes, `s` `a`'s half
/// extents. Bisects both boxes towards each other and returns the point
/// inside `a`'s box nearest to `b`.
fn box_to_box_intersection(
    mut rr: Vec3,
    mut x: Vec3,
    mut y: Vec3,
    mut z: Vec3,
    mut s: Vec3,
) -> Vec3 {
    let sign = |v: f32| if v > 0.0 { 1.0 } else { -1.0 };
    let mut r1 = Vec3::ZERO;
    let mut r2 = rr;
    for _ in 0..12 {
        let (px, py, pz) = (s.x * rr.x, s.y * rr.y, s.z * rr.z);
        if px.abs() > py.abs() && px.abs() > pz.abs() {
            s.x *= 0.5;
            r1.x += s.x * sign(px);
        } else if px.abs() <= py.abs() && py.abs() > pz.abs() {
            s.y *= 0.5;
            r1.y += s.y * sign(py);
        } else {
            s.z *= 0.5;
            r1.z += s.z * sign(pz);
        }

        let (qx, qy, qz) = (x.dot(rr), y.dot(rr), z.dot(rr));
        if qx.abs() > qy.abs() && qx.abs() > qz.abs() {
            x *= 0.5;
            r2 -= x * sign(qx);
        } else if qx.abs() <= qy.abs() && qy.abs() > qz.abs() {
            y *= 0.5;
            r2 -= y * sign(qy);
        } else {
            z *= 0.5;
            r2 -= z * sign(qz);
        }
        rr = r2 - r1;
    }
    r1
}

fn shortest_delta(d: f32, span: f32) -> f32 {
    if span <= 0.0 {
        return d;
    }
    let half = span * 0.5;
    (d + half).rem_euclid(span) - half
}

/// Resolve contact between two cars: exchange an impulse if they overlap
/// and are closing, and push them apart by part of the overlap.
///
/// `wrap` is the level size, so cars across the torus seam still touch;
/// `b` is moved in its own wrap, the caller re-wraps as usual.
pub fn collide_cars(
    a: CarBody<'_>,
    b: CarBody<'_>,
    wrap: Option<(f32, f32)>,
    common: &config::common::Common,
) -> Option<CarContact> {
    let mut delta = b.transform.disp - a.transform.disp;
    if let Some((w, h)) = wrap {
        delta.x = shortest_delta(delta.x, w);
        delta.y = shortest_delta(delta.y, h);
    }
    let reach = a.car.bbox.radius * a.transform.scale + b.car.bbox.radius * b.transform.scale;
    if delta.length_squared() >= reach * reach {
        return None;
    }
    let b_origin = a.transform.disp + delta;
    let box_a = Obb::new(a.car, a.transform, a.transform.disp);
    let box_b = Obb::new(b.car, b.transform, b_origin);
    let (depth, axis) = overlap(&box_a, &box_b)?;

    let a_inv = a.transform.rot.inverse();
    let b_inv = b.transform.rot.inverse();
    let centers = box_b.center - box_a.center;
    let r1 = box_to_box_intersection(
        a_inv * centers,
        a_inv * (box_b.axes[0] * box_b.half.x),
        a_inv * (box_b.axes[1] * box_b.half.y),
        a_inv * (box_b.axes[2] * box_b.half.z),
        box_a.half,
    );
    let point = box_a.center + a.transform.rot * r1;
    let lever_a = a_inv * (point - a.transform.disp);
    let lever_b = b_inv * (point - b_origin);

    // The original takes the normal along the line between the centres.
    let normal = centers.try_normalize().unwrap_or(axis);
    let vel_a =
        a.transform.rot * (a.dynamo.linear_velocity + a.dynamo.angular_velocity.cross(lever_a));
    let vel_b =
        b.transform.rot * (b.dynamo.linear_velocity + b.dynamo.angular_velocity.cross(lever_b));
    let closing = (vel_a - vel_b).dot(normal);

    let mass_a = body_mass(a.car, a.transform.scale, common);
    let mass_b = body_mass(b.car, b.transform.scale, common);
    let mut impulse = 0.0;
    if closing > 0.0 {
        let j_a = inertia_inv(a.car, a.transform.scale);
        let j_b = inertia_inv(b.car, b.transform.scale);
        let rot_a = Mat3::from_quat(a.transform.rot);
        let rot_b = Mat3::from_quat(b.transform.rot);
        let k = rot_a * rigid::collision_matrix(&j_a, &lever_a) * rot_a.transpose() / mass_a
            + rot_b * rigid::collision_matrix(&j_b, &lever_b) * rot_b.transpose() / mass_b;
        let p = k.inverse() * (normal * closing) * -(1.0 + RESTITUTION);
        let p_a = a_inv * p / mass_a;
        a.dynamo.linear_velocity += p_a;
        a.dynamo.angular_velocity += j_a * lever_a.cross(p_a);
        let p_b = b_inv * p / mass_b;
        b.dynamo.linear_velocity -= p_b;
        b.dynamo.angular_velocity -= j_b * lever_b.cross(p_b);
        impulse = p.length();
    }

    if depth > SLOP {
        let push = axis * ((depth - SLOP) * SEPARATION);
        let share_a = mass_b / (mass_a + mass_b);
        a.transform.disp -= push * share_a;
        b.transform.disp += push * (1.0 - share_a);
    }

    Some(CarContact {
        point,
        normal,
        impulse,
        depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;
    use std::f32::consts::FRAC_PI_2;

    fn car_at(x: f32, yaw: f32) -> (Dynamo, space::Transform) {
        (
            Dynamo::default(),
            space::Transform {
                disp: Vec3::new(x, 0.0, 0.0),
                rot: Quat::from_rotation_z(yaw),
                scale: 1.0,
            },
        )
    }

    fn hit(
        a: &mut (Dynamo, space::Transform),
        b: &mut (Dynamo, space::Transform),
        car: &CarPhysicsData,
        common: &config::common::Common,
    ) -> Option<CarContact> {
        collide_cars(
            CarBody {
                dynamo: &mut a.0,
                transform: &mut a.1,
                car,
            },
            CarBody {
                dynamo: &mut b.0,
                transform: &mut b.1,
                car,
            },
            None,
            common,
        )
    }

    #[test]
    fn head_on_ram_hands_over_momentum() {
        let car = CarPhysicsData::test_default();
        let common = config::common::Common::test_default();
        // Test car box is 20 units; centres 19 apart overlap by 1. `a`
        // faces +X (local +Y forward) and drives at 10, `b` faces -X parked.
        let mut a = car_at(0.0, -FRAC_PI_2);
        let mut b = car_at(19.0, FRAC_PI_2);
        a.0.linear_velocity = Vec3::new(0.0, 10.0, 0.0);
        let before_a = a.1.rot * a.0.linear_velocity;
        let contact = hit(&mut a, &mut b, &car, &common).expect("overlapping cars touch");
        let va = a.1.rot * a.0.linear_velocity;
        let vb = b.1.rot * b.0.linear_velocity;
        assert!(contact.impulse > 0.0);
        assert!(contact.normal.x > 0.99, "{:?}", contact.normal);
        // Equal masses, fully elastic, central: the original's Newton cradle.
        assert!(va.x.abs() < 0.5, "rammer keeps {va:?}");
        assert!((vb.x - before_a.x).abs() < 0.5, "parked car got {vb:?}");
        assert!(((va + vb) - before_a).length() < 1e-3, "momentum");
        assert!(a.1.disp.x < 0.0 && b.1.disp.x > 19.0, "pushed apart");
        // Already separating: no second impulse, so no double bump.
        let again = hit(&mut a, &mut b, &car, &common).expect("still overlapping");
        assert_eq!(again.impulse, 0.0);
    }

    #[test]
    fn apart_cars_do_not_touch() {
        let car = CarPhysicsData::test_default();
        let common = config::common::Common::test_default();
        let mut a = car_at(0.0, -FRAC_PI_2);
        let mut b = car_at(21.0, FRAC_PI_2);
        a.0.linear_velocity = Vec3::new(0.0, 10.0, 0.0);
        assert!(hit(&mut a, &mut b, &car, &common).is_none());
        assert_eq!(b.0.linear_velocity, Vec3::ZERO);
    }

    #[test]
    fn contact_across_the_level_seam() {
        let car = CarPhysicsData::test_default();
        let common = config::common::Common::test_default();
        let mut a = car_at(250.0, -FRAC_PI_2);
        let mut b = car_at(13.0, FRAC_PI_2);
        a.0.linear_velocity = Vec3::new(0.0, 10.0, 0.0);
        let c = collide_cars(
            CarBody {
                dynamo: &mut a.0,
                transform: &mut a.1,
                car: &car,
            },
            CarBody {
                dynamo: &mut b.0,
                transform: &mut b.1,
                car: &car,
            },
            Some((256.0, 256.0)),
            &common,
        );
        assert!(c.is_some_and(|c| c.impulse > 0.0));
        assert!((b.1.rot * b.0.linear_velocity).x > 5.0);
    }
}
