//! Spiral energy: charges at stations / escaves, spent to open passages.
//!
//! Outdoor refill matches KranX `SensorTypeList::KEY_UPDATE` pads
//! (`KeyUpdate*` in each world's `snstable.vlc`; compass label "Spiral
//! Station"). Escave enter also fills the spiral in this remake.
//! World hops are triggered from `PASSAGE` sensors (`snstable.vlc` +
//! `PassageEngine` in `location.lst`); this module still supplies prompts
//! and the prm-coordinate proximity helper used when engines are absent.
//!
//! Original UI strings live in `game.lst` ("Spiral charged…",
//! "Spiral discharged. Passage closed!"). Capacity matches
//! `ACI_MECHOS_SPIRAL_CAPACITY` (4) unless a car overrides via
//! `max_teleport`.

use crate::config::passages::Passage;

/// Default mechos spiral slots from actint.
pub const DEFAULT_CAPACITY: u8 = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spiral {
    pub charge: u8,
    pub capacity: u8,
}

impl Default for Spiral {
    fn default() -> Self {
        Spiral {
            charge: 0,
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl Spiral {
    pub fn with_capacity(capacity: u8) -> Self {
        Spiral {
            charge: 0,
            capacity: capacity.max(1),
        }
    }

    pub fn is_ready(&self) -> bool {
        self.charge > 0
    }

    pub fn is_full(&self) -> bool {
        self.charge >= self.capacity
    }

    /// Fill to capacity. Returns true if anything changed.
    pub fn charge_full(&mut self) -> bool {
        if self.is_full() {
            return false;
        }
        self.charge = self.capacity;
        true
    }

    /// Spend one charge to open a passage. False if empty.
    pub fn discharge(&mut self) -> bool {
        if self.charge == 0 {
            return false;
        }
        self.charge -= 1;
        true
    }
}

/// Status line when the player is near a passage.
pub fn passage_prompt(passage: &Passage, spiral: &Spiral) -> String {
    if spiral.is_ready() {
        format!("Passage to {} in sight...", passage.to_world)
    } else {
        "Spiral discharged. Passage closed!".to_string()
    }
}

/// Nearest passage within `reach`, using wrap-aware XY on a torus of `size`.
pub fn nearest_passage<'a>(
    list: &'a [Passage],
    at: (i32, i32),
    size: (i32, i32),
    reach: i32,
) -> Option<&'a Passage> {
    let reach2 = reach.saturating_mul(reach);
    list.iter()
        .filter_map(|p| {
            let dx = wrap_delta(p.coordinates.0 - at.0, size.0);
            let dy = wrap_delta(p.coordinates.1 - at.1, size.1);
            let d2 = dx * dx + dy * dy;
            (d2 <= reach2).then_some((d2, p))
        })
        .min_by_key(|&(d2, _)| d2)
        .map(|(_, p)| p)
}

fn wrap_delta(d: i32, span: i32) -> i32 {
    if span <= 0 {
        return d;
    }
    let half = span / 2;
    ((d + half).rem_euclid(span)) - half
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passage(id: &str, x: i32, y: i32) -> Passage {
        Passage {
            id: id.into(),
            from_world: "Fostral".into(),
            to_world: "Glorx".into(),
            coordinates: (x, y),
        }
    }

    #[test]
    fn charge_and_discharge() {
        let mut s = Spiral::with_capacity(4);
        assert!(!s.is_ready());
        assert!(s.charge_full());
        assert!(s.is_full());
        assert!(!s.charge_full());
        assert!(s.discharge());
        assert_eq!(s.charge, 3);
        s.charge = 0;
        assert!(!s.discharge());
    }

    #[test]
    fn prompts_depend_on_charge() {
        let p = passage("F2G", 0, 0);
        let mut s = Spiral::default();
        assert!(passage_prompt(&p, &s).contains("discharged"));
        s.charge_full();
        assert!(passage_prompt(&p, &s).contains("Glorx"));
    }

    #[test]
    fn finds_nearby_passage_across_the_seam() {
        let list = [passage("F2G", 10, 100)];
        let size = (256, 256);
        // Standing just west of 0, near x=10 via wrap.
        let hit = nearest_passage(&list, (250, 100), size, 40);
        assert_eq!(hit.map(|p| p.id.as_str()), Some("F2G"));
        assert!(nearest_passage(&list, (128, 128), size, 40).is_none());
    }
}
