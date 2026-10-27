//! A client's side of `ServerMessage::Died` and `Respawned`, shared by the
//! native and web clients so the two cannot drift: who to name in the HUD
//! note, where to draw the explosion, and how a car comes back.
//!
//! The server owns death. Our car's `Respawned` carries the pose and the
//! quantum the server numbers our inputs from, so the client snaps there
//! and restarts its prediction at that quantum; it sends nothing back.

use super::Dynamo;
use super::client::{ClientSync, RemoteCar};
use crate::{level, space};
use glam::Vec3;
use std::collections::HashMap;
use vangers_net::{NetTransform, PlayerId};

/// A client's copy of another player's car, as the death events need it.
pub trait DeathRemote {
    fn name(&self) -> &str;
    fn net_mut(&mut self) -> &mut RemoteCar;
    fn treads_mut(&mut self) -> &mut level::overlay::GhostTreads;
    /// A wreck waiting to respawn (it smokes).
    fn set_dead(&mut self, dead: bool);
}

/// Our own car, as the respawn snaps it.
pub struct LocalRespawn<'a> {
    pub transform: &'a mut space::Transform,
    pub dynamo: &'a mut Dynamo,
    pub armor: &'a mut u32,
    pub max_armor: u32,
}

/// What a client shows for a `Died`.
#[derive(Clone, Debug, PartialEq)]
pub struct Died {
    /// Ours: no control until the respawn.
    pub mine: bool,
    /// Where to draw the explosion.
    pub at: Vec3,
    /// The HUD note.
    pub note: String,
}

/// What a client did for a `Respawned`.
#[derive(Clone, Debug, PartialEq)]
pub enum Respawned {
    /// Ours, snapped to this pose (for the camera).
    Mine(space::Transform),
    /// Another player's car, by name.
    Remote(String),
    /// A player this client does not know.
    Unknown,
}

/// How the death notes name player `id`: "you" for us.
pub fn player_name<R: DeathRemote>(
    id: PlayerId,
    my_id: Option<PlayerId>,
    remotes: &HashMap<PlayerId, R>,
) -> String {
    if Some(id) == my_id {
        "you".to_string()
    } else {
        remotes
            .get(&id)
            .map_or_else(|| format!("player {id}"), |r| r.name().to_string())
    }
}

/// The HUD note for a death: `who` (named by [`player_name`]) died, ours
/// when `mine`, shot by `by` if the server knows.
pub fn death_note(mine: bool, who: &str, by: Option<&str>) -> String {
    match (mine, by) {
        (true, Some(by)) => format!("Destroyed by {by}! Respawning…"),
        (true, None) => "Destroyed! Respawning…".to_string(),
        (false, Some("you")) => format!("You destroyed {who}"),
        (false, Some(by)) => format!("{who} destroyed by {by}"),
        (false, None) => format!("{who} destroyed"),
    }
}

/// `ServerMessage::Died` for player `player_id`, shot by `killer`, at the
/// server's `position`. `eye`: our car's position, if we have one. A
/// remote car is marked a wreck and explodes where it is drawn; ours where
/// we have it.
pub fn on_died<R: DeathRemote>(
    remotes: &mut HashMap<PlayerId, R>,
    level: &level::Level,
    my_id: Option<PlayerId>,
    eye: Option<Vec3>,
    player_id: PlayerId,
    killer: Option<PlayerId>,
    position: [f32; 3],
) -> Died {
    let position = Vec3::from(position);
    let eye = eye.unwrap_or(position);
    let mine = Some(player_id) == my_id;
    let at = match remotes.get_mut(&player_id) {
        Some(remote) => {
            remote.set_dead(true);
            let net = remote.net_mut();
            if net.pose_ready {
                net.render_transform.disp
            } else {
                level.display_pos(position, eye)
            }
        }
        None if mine => eye,
        None => level.display_pos(position, eye),
    };
    let who = player_name(player_id, my_id, remotes);
    let by = killer.map(|k| player_name(k, my_id, remotes));
    Died {
        mine,
        at,
        note: death_note(mine, &who, by.as_deref()),
    }
}

/// `ServerMessage::Respawned`: player `player_id` is back at `transform`,
/// still and undamaged. Ours (`local`, if we have a car) is snapped there
/// and our prediction restarted at the server's `next_quant`; a remote
/// car is drawn there at once, its treads cut.
#[allow(clippy::too_many_arguments)]
pub fn on_respawned<R: DeathRemote>(
    remotes: &mut HashMap<PlayerId, R>,
    level: &level::Level,
    my_id: Option<PlayerId>,
    local: Option<LocalRespawn<'_>>,
    sync: &mut ClientSync,
    player_id: PlayerId,
    transform: &NetTransform,
    next_quant: u32,
) -> Respawned {
    let rot = glam::Quat::from_array(transform.rotation);
    let position = Vec3::from(transform.position);
    if Some(player_id) == my_id {
        sync.reset();
        sync.prediction.restart_at(next_quant);
        let Some(local) = local else {
            return Respawned::Unknown;
        };
        local.transform.disp = level.display_pos(position, local.transform.disp);
        local.transform.rot = rot;
        *local.dynamo = Dynamo::default();
        *local.armor = local.max_armor;
        Respawned::Mine(*local.transform)
    } else if let Some(remote) = remotes.get_mut(&player_id) {
        remote.set_dead(false);
        let net = remote.net_mut();
        let eye = net.render_transform.disp;
        net.respawn_at(space::Transform {
            disp: level.display_pos(position, eye),
            rot,
            scale: transform.scale,
        });
        remote.treads_mut().reset();
        Respawned::Remote(remote.name().to_string())
    } else {
        Respawned::Unknown
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::CarPhysicsData;

    struct Remote {
        name: String,
        net: RemoteCar,
        treads: level::overlay::GhostTreads,
        dead: bool,
    }

    impl DeathRemote for Remote {
        fn name(&self) -> &str {
            &self.name
        }
        fn net_mut(&mut self) -> &mut RemoteCar {
            &mut self.net
        }
        fn treads_mut(&mut self) -> &mut level::overlay::GhostTreads {
            &mut self.treads
        }
        fn set_dead(&mut self, dead: bool) {
            self.dead = dead;
        }
    }

    fn remotes() -> HashMap<PlayerId, Remote> {
        let mut net = RemoteCar::new(CarPhysicsData::test_default());
        net.respawn_at(space::Transform {
            disp: Vec3::new(10.0, 20.0, 5.0),
            ..space::Transform::IDENTITY
        });
        HashMap::from([(
            2,
            Remote {
                name: "Bob".to_string(),
                net,
                treads: Default::default(),
                dead: false,
            },
        )])
    }

    fn test_level() -> level::Level {
        level::load(&level::LevelConfig::new_test(), &Default::default())
    }

    #[test]
    fn notes_name_the_victim_and_the_killer() {
        assert_eq!(
            death_note(true, "you", Some("Bob")),
            "Destroyed by Bob! Respawning…"
        );
        assert_eq!(death_note(true, "you", None), "Destroyed! Respawning…");
        assert_eq!(death_note(false, "Bob", Some("you")), "You destroyed Bob");
        assert_eq!(
            death_note(false, "Bob", Some("Eve")),
            "Bob destroyed by Eve"
        );
        assert_eq!(death_note(false, "Bob", None), "Bob destroyed");
        let r = remotes();
        assert_eq!(player_name(1, Some(1), &r), "you");
        assert_eq!(player_name(2, Some(1), &r), "Bob");
        assert_eq!(player_name(7, Some(1), &r), "player 7");
    }

    #[test]
    fn a_remote_death_explodes_where_the_car_is_drawn() {
        let level = test_level();
        let mut r = remotes();
        let died = on_died(
            &mut r,
            &level,
            Some(1),
            Some(Vec3::ZERO),
            2,
            Some(1),
            [99.0, 99.0, 0.0],
        );
        assert_eq!(
            died,
            Died {
                mine: false,
                at: Vec3::new(10.0, 20.0, 5.0),
                note: "You destroyed Bob".to_string(),
            }
        );
        assert!(r[&2].dead);
        // Ours explodes at our car; an unknown player's at the server's spot.
        let eye = Vec3::new(1.0, 2.0, 3.0);
        let died = on_died(
            &mut r,
            &level,
            Some(1),
            Some(eye),
            1,
            Some(2),
            [9.0, 9.0, 9.0],
        );
        assert_eq!((died.mine, died.at), (true, eye));
        assert_eq!(died.note, "Destroyed by Bob! Respawning…");
        let died = on_died(&mut r, &level, Some(1), Some(eye), 5, None, [9.0, 9.0, 9.0]);
        assert_eq!((died.mine, died.at), (false, Vec3::new(9.0, 9.0, 9.0)));
    }

    #[test]
    fn our_respawn_restarts_the_prediction_at_the_servers_quantum() {
        let level = test_level();
        let mut r = remotes();
        let mut sync = ClientSync::default();
        sync.prediction.skip(40);
        let mut transform = space::Transform {
            disp: Vec3::new(50.0, 60.0, 7.0),
            ..space::Transform::IDENTITY
        };
        let mut dynamo = Dynamo {
            linear_velocity: Vec3::X * 30.0,
            ..Dynamo::default()
        };
        let mut armor = 0;
        let at = NetTransform {
            position: [70.0, 80.0, 9.0],
            rotation: [0.0, 0.0, 1.0, 0.0],
            scale: 1.0,
        };
        let done = on_respawned(
            &mut r,
            &level,
            Some(1),
            Some(LocalRespawn {
                transform: &mut transform,
                dynamo: &mut dynamo,
                armor: &mut armor,
                max_armor: 120,
            }),
            &mut sync,
            1,
            &at,
            70_000,
        );
        assert_eq!(sync.prediction.next(), 70_000);
        assert!(sync.prediction.is_empty());
        assert_eq!(transform.disp, Vec3::new(70.0, 80.0, 9.0));
        assert_eq!(transform.rot, glam::Quat::from_xyzw(0.0, 0.0, 1.0, 0.0));
        assert_eq!(dynamo.linear_velocity, Vec3::ZERO);
        assert_eq!(armor, 120);
        assert_eq!(done, Respawned::Mine(transform));
    }

    #[test]
    fn a_remote_respawn_snaps_the_car_and_leaves_our_prediction() {
        let level = test_level();
        let mut r = remotes();
        r.get_mut(&2).unwrap().dead = true;
        let mut sync = ClientSync::default();
        sync.prediction.skip(40);
        let at = NetTransform {
            position: [70.0, 80.0, 9.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: 1.0,
        };
        let done = on_respawned(&mut r, &level, Some(1), None, &mut sync, 2, &at, 5);
        assert_eq!(done, Respawned::Remote("Bob".to_string()));
        assert_eq!(sync.prediction.next(), 40);
        let bob = &r[&2];
        assert!(!bob.dead);
        assert_eq!(bob.net.render_transform.disp, Vec3::new(70.0, 80.0, 9.0));
        assert_eq!(bob.net.target_transform.disp, Vec3::new(70.0, 80.0, 9.0));
        assert_eq!(
            on_respawned(&mut r, &level, Some(1), None, &mut sync, 9, &at, 5),
            Respawned::Unknown
        );
    }
}
