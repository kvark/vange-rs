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

/// Status line when the player is near a passage. `online`: connected to a
/// multiplayer server, which hosts a single world (see [`passage_open`]).
pub fn passage_prompt(passage: &Passage, spiral: &Spiral, online: bool) -> String {
    if online {
        format!(
            "Passage closed: {} is offline-only (the server hosts one world).",
            passage.to_world
        )
    } else if spiral.is_ready() {
        format!("Passage to {} in sight...", passage.to_world)
    } else {
        "Spiral discharged. Passage closed!".to_string()
    }
}

/// Whether a passage hop may start. Online it never may: a multiplayer
/// server simulates one world, so a hop would load the next world on this
/// client only while the server kept the car - and every other player's view
/// of it - in the old one, snapping it back across worlds from then on.
pub fn passage_open(spiral: &Spiral, online: bool) -> bool {
    !online && spiral.is_ready()
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

/// Online, the server stores our spiral charge (`SetSpiral`) and echoes it
/// in every `WorldState`. A snapshot sent before our latest `SetSpiral`
/// still carries the old charge; taken blindly it blips the spiral back
/// (say 2/2 -> 0/2 just after a station), and the station then charges and
/// reports again. `SpiralSync` makes the exchange idempotent and ordered:
/// a report is sent only when the server doesn't already have that charge,
/// and while a report is unconfirmed, echoes of other charges are older
/// than it and ignored. The server is the only other writer only before our
/// first report (the reclaim restore on join), so the first echo that
/// matches confirms the report.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SpiralSync {
    /// Charge we sent that the server hasn't echoed yet.
    pending: Option<u8>,
    /// Snapshots ignored while `pending`.
    stale: u32,
    /// The server's latest echo.
    server: Option<u8>,
}

/// What [`SpiralSync::on_server`] decided about an echo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpiralEcho {
    /// Take this charge (it is the server's, newer than any local change).
    Apply(u8),
    /// The echo confirms our pending report.
    Confirmed(u8),
    /// Older than our pending report: keep the local charge.
    Stale(u8),
}

impl SpiralSync {
    /// Snapshots to wait for an echo before trusting the server again, in
    /// case a report never landed (the server drops one before its join).
    /// Generous: the stale run also covers snapshots that queued up while a
    /// frame stalled (an escave load), 15 of them in the proof run, and at
    /// 20 Hz this is 20 s.
    pub const MAX_STALE: u32 = 400;

    /// Local charge changed to `charge`: `Some` to send `SetSpiral`, `None`
    /// when the server has, or is about to have, that charge already.
    pub fn report(&mut self, charge: u8) -> Option<u8> {
        let known = match self.pending {
            Some(p) => p == charge,
            None => self.server == Some(charge),
        };
        if known {
            return None;
        }
        self.pending = Some(charge);
        self.stale = 0;
        Some(charge)
    }

    /// The server's charge for us from a `WorldState`.
    pub fn on_server(&mut self, charge: u8) -> SpiralEcho {
        self.server = Some(charge);
        match self.pending {
            None => SpiralEcho::Apply(charge),
            Some(p) if p == charge => {
                self.pending = None;
                SpiralEcho::Confirmed(charge)
            }
            Some(_) => {
                self.stale += 1;
                if self.stale > Self::MAX_STALE {
                    self.pending = None;
                    SpiralEcho::Apply(charge)
                } else {
                    SpiralEcho::Stale(charge)
                }
            }
        }
    }

    /// A report is awaiting its echo.
    pub fn is_pending(&self) -> bool {
        self.pending.is_some()
    }

    /// New connection: nothing sent yet, the server's charge unknown.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
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
        assert!(passage_prompt(&p, &s, false).contains("discharged"));
        s.charge_full();
        assert!(passage_prompt(&p, &s, false).contains("Glorx"));
    }

    #[test]
    fn passages_stay_closed_online() {
        let p = passage("F2G", 0, 0);
        let mut s = Spiral::default();
        assert!(!passage_open(&s, false));
        assert!(!passage_open(&s, true));
        s.charge_full();
        assert!(passage_open(&s, false));
        // Charged, but connected to a one-world server: no hop, and the
        // prompt says why instead of "in sight".
        assert!(!passage_open(&s, true));
        let note = passage_prompt(&p, &s, true);
        assert!(note.starts_with("Passage closed"), "{note}");
        assert!(note.contains("Glorx"), "{note}");
        assert!(!note.contains("in sight"), "{note}");
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

    #[test]
    fn stale_echo_after_a_station_is_ignored() {
        // Joined with 0/2; the server echoes 0.
        let mut sync = SpiralSync::default();
        assert_eq!(sync.on_server(0), SpiralEcho::Apply(0));
        // Charged at a station: report 2.
        assert_eq!(sync.report(2), Some(2));
        // Snapshots sent before the server got it still say 0.
        for _ in 0..3 {
            assert_eq!(sync.on_server(0), SpiralEcho::Stale(0));
        }
        // Still on the station: the second charge isn't sent again.
        assert_eq!(sync.report(2), None);
        assert_eq!(sync.on_server(2), SpiralEcho::Confirmed(2));
        assert!(!sync.is_pending());
        // Steady echoes are applied (a no-op) and the station stays quiet.
        assert_eq!(sync.on_server(2), SpiralEcho::Apply(2));
        assert_eq!(sync.report(2), None);
    }

    #[test]
    fn reports_in_flight_are_ordered() {
        let mut sync = SpiralSync::default();
        sync.on_server(0);
        // Charge then discharge before any echo.
        assert_eq!(sync.report(2), Some(2));
        assert_eq!(sync.report(1), Some(1));
        // The echo of the first report is older than the second.
        assert_eq!(sync.on_server(0), SpiralEcho::Stale(0));
        assert_eq!(sync.on_server(2), SpiralEcho::Stale(2));
        assert_eq!(sync.on_server(1), SpiralEcho::Confirmed(1));
        // Back to the server's value: one report.
        assert_eq!(sync.report(0), Some(0));
        assert_eq!(sync.on_server(0), SpiralEcho::Confirmed(0));
    }

    #[test]
    fn server_wins_when_nothing_is_pending_or_a_report_is_lost() {
        let mut sync = SpiralSync::default();
        // Reclaim restore on join.
        assert_eq!(sync.on_server(3), SpiralEcho::Apply(3));
        assert_eq!(sync.report(4), Some(4));
        for _ in 0..SpiralSync::MAX_STALE {
            assert_eq!(sync.on_server(3), SpiralEcho::Stale(3));
        }
        assert_eq!(sync.on_server(3), SpiralEcho::Apply(3));
        assert!(!sync.is_pending());
        // A reconnect forgets everything.
        sync.report(1);
        sync.reset();
        assert_eq!(sync, SpiralSync::default());
        assert_eq!(sync.report(3), Some(3));
    }
}
