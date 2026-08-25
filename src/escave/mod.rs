//! Escave interiors: counselor dialog and the shop counter.
//!
//! The original 2.5D iscreen chrome is not here. A visit is a dialog
//! session plus a shop, entered from the world by proximity or by hand.

pub mod dialog;
pub mod shop;

pub use dialog::{Room, Session, find_room};
pub use shop::{Good, Inventory, Shop, ShopError};

use std::path::Path;

/// A named pad the player can Use (Space): an escave, a spot, or a passage.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entrance {
    pub name: String,
    pub pos: (i32, i32),
    /// How close counts as standing on it, in level texels.
    pub reach: i32,
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
