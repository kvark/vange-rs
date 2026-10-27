//! Multiplayer: correcting the locally predicted car toward the server's
//! `WorldState` copy of it.

use super::Dynamo;
use crate::space;

use glam::Vec3;

/// The server's copy of the local car, as one `WorldState` entry carries it.
#[derive(Clone, Copy, Debug)]
pub struct ServerCar {
    pub transform: space::Transform,
    /// Car-local, like `Dynamo`.
    pub linear_velocity: Vec3,
    pub angular_velocity: Vec3,
    pub traction: crate::config::common::Traction,
    pub rudder: f32,
    /// The server's felt-bump counter for this car changed since the
    /// last snapshot the client reconciled against: another car hit it.
    pub bumped: bool,
}

/// What [`reconcile_local_car`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reconcile {
    /// Small error: pose blended, the local dynamics kept.
    Blend,
    /// Pose blended and the server's velocities adopted: a bump only the
    /// server could see (the rammer is a ghost locally, a tick behind).
    Bump,
    /// Large desync: pose and dynamics taken from the server.
    Snap,
}

/// Correct the predicted car toward `server`.
///
/// Normally only the pose is soft-blended (`Transform::reconcile_world_state`)
/// and the local velocity kept, so contact with the terrain is not slammed
/// every tick. That hides a bump: a rammed car's prediction does not know it
/// was hit, keeps its old velocity, and only creeps after the server pose at
/// the blend rate. On a server-reported bump the client adopts the server's
/// velocities as well, rotated into its own (blended) frame, so it moves off
/// with the hit within a tick. The pose still blends: no extra snap.
pub fn reconcile_local_car(
    transform: &mut space::Transform,
    dynamo: &mut Dynamo,
    server: &ServerCar,
) -> Reconcile {
    if transform.reconcile_world_state(&server.transform) {
        dynamo.linear_velocity = server.linear_velocity;
        dynamo.angular_velocity = server.angular_velocity;
        dynamo.traction = server.traction;
        dynamo.rudder = server.rudder;
        return Reconcile::Snap;
    }
    if !server.bumped {
        return Reconcile::Blend;
    }
    let to_local = transform.rot.inverse() * server.transform.rot;
    dynamo.linear_velocity = to_local * server.linear_velocity;
    dynamo.angular_velocity = to_local * server.angular_velocity;
    Reconcile::Bump
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn server_at(x: f32, vx: f32, bumped: bool) -> ServerCar {
        ServerCar {
            transform: space::Transform {
                disp: Vec3::new(x, 0.0, 0.0),
                rot: Quat::IDENTITY,
                scale: 1.0,
            },
            linear_velocity: Vec3::new(vx, 0.0, 0.0),
            angular_velocity: Vec3::new(0.0, 0.0, 0.3),
            traction: 1.0,
            rudder: 0.2,
            bumped,
        }
    }

    #[test]
    fn bump_adopts_velocity_but_blends_pose() {
        let mut transform = space::Transform::IDENTITY;
        let mut dynamo = Dynamo::default();
        let kind = reconcile_local_car(&mut transform, &mut dynamo, &server_at(2.0, 5.0, true));
        assert_eq!(kind, Reconcile::Bump);
        assert!((dynamo.linear_velocity.x - 5.0).abs() < 1e-5);
        assert!((dynamo.angular_velocity.z - 0.3).abs() < 1e-5);
        assert!(transform.disp.x > 0.0 && transform.disp.x < 2.0, "blended");
        // Controls stay the player's.
        assert_eq!(dynamo.traction, 0.0);
        assert_eq!(dynamo.rudder, 0.0);
    }

    #[test]
    fn no_bump_keeps_local_velocity() {
        let mut transform = space::Transform::IDENTITY;
        let mut dynamo = Dynamo::default();
        let kind = reconcile_local_car(&mut transform, &mut dynamo, &server_at(2.0, 5.0, false));
        assert_eq!(kind, Reconcile::Blend);
        assert_eq!(dynamo.linear_velocity, Vec3::ZERO);
    }

    #[test]
    fn bump_velocity_is_rotated_into_the_local_frame() {
        // Local car yawed from the server's: same world velocity.
        let mut transform = space::Transform {
            rot: Quat::from_rotation_z(0.3),
            ..space::Transform::IDENTITY
        };
        let mut dynamo = Dynamo::default();
        let server = server_at(0.0, 5.0, true);
        reconcile_local_car(&mut transform, &mut dynamo, &server);
        let world = transform.rot * dynamo.linear_velocity;
        let want = server.transform.rot * server.linear_velocity;
        assert!((world - want).length() < 1e-4, "{world:?} vs {want:?}");
    }
}
