//! Escave interiors: counselor dialog and the shop counter.
//!
//! Space opens the hatch. The visit starts after the car falls in, then
//! shutters cover the road and this module draws talk and trade. Leave
//! closes the door and kicks the car out.

pub mod cave;
pub mod dialog;
pub mod layout;
pub mod preview;
pub mod screen;
pub mod shop;

pub use dialog::{Room, Session, find_room};
pub use layout::{Catalog, Cell, Layout};
pub use preview::{SpinMesh, description_for, display_name, mesh_id};
pub use screen::{InteriorAction, Screen, draw_interior, draw_shutters};
pub use shop::{
    BAY_COUNT, DropTarget, GRID_HEIGHT, GRID_WIDTH, Good, Hand, Inventory, Kind, Placed, Preview,
    Shop, ShopError, drop_held, equipped_slot_ids, mounted_meshes, preview, preview_good,
};

use crate::level::vlc::sensor_kind;
use std::path::Path;

/// A named pad the player can Use (Space): an escave, a spot, or a passage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entrance {
    pub name: String,
    pub pos: (i32, i32),
    /// How close counts as standing on it, in level texels.
    pub reach: i32,
}

impl Entrance {
    pub const REACH: i32 = 128;

    pub fn named(name: impl Into<String>, pos: (i32, i32)) -> Self {
        Entrance {
            name: name.into(),
            pos,
            reach: Self::REACH,
        }
    }

    /// An ESCAVE or SPOT sensor becomes a pad. Passages and tunnels do not.
    pub fn from_sensor(kind: i32, name: &str, pos: (i32, i32), radius: i32) -> Option<Self> {
        if kind != sensor_kind::ESCAVE && kind != sensor_kind::SPOT {
            return None;
        }
        let name = if name.is_empty() {
            if kind == sensor_kind::SPOT {
                "Spot".to_string()
            } else {
                "Escave".to_string()
            }
        } else {
            name.to_string()
        };
        Some(Entrance {
            name,
            pos,
            reach: radius.max(48) + 48,
        })
    }
}

/// The closest entrance `at` is standing on, if any.
pub fn nearest_entrance(list: &[Entrance], at: (i32, i32)) -> Option<&Entrance> {
    list.iter()
        .filter(|e| {
            let dx = e.pos.0 - at.0;
            let dy = e.pos.1 - at.1;
            dx * dx + dy * dy <= e.reach * e.reach
        })
        .min_by_key(|e| {
            let dx = e.pos.0 - at.0;
            let dy = e.pos.1 - at.1;
            dx * dx + dy * dy
        })
}

/// An open visit: talking to the counselor and buying from the shop.
pub struct Visit {
    pub name: String,
    pub session: Option<Session>,
}

impl Visit {
    pub fn enter(name: &str, data_path: &Path) -> Self {
        let session = find_room(data_path, name).map(Session::start);
        Visit {
            name: name.to_string(),
            session,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(name: &str, x: i32, y: i32, reach: i32) -> Entrance {
        Entrance {
            name: name.to_string(),
            pos: (x, y),
            reach,
        }
    }

    #[test]
    fn a_spot_sensor_is_a_pad_and_a_passage_is_not() {
        let spot = Entrance::from_sensor(3, "Incubator", (10, 20), 40).unwrap();
        assert_eq!(spot.name, "Incubator");
        assert_eq!(spot.pos, (10, 20));
        assert!(Entrance::from_sensor(5, "Hole", (0, 0), 40).is_none());
    }

    #[test]
    fn space_opens_the_pad_you_are_standing_on() {
        let pads = [pad("Podish", 100, 100, 80), pad("Incubator", 800, 800, 80)];
        assert_eq!(
            nearest_entrance(&pads, (110, 90)).map(|e| e.name.as_str()),
            Some("Podish")
        );
        assert_eq!(
            nearest_entrance(&pads, (400, 400)).map(|e| e.name.as_str()),
            None
        );
    }
}
