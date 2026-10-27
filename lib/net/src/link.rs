//! A client's upkeep of its server connection, apart from any transport:
//! when a keepalive is due, and when to dial again after the link drops.
//! Times are seconds on any monotonic-enough clock.

/// Seconds between keepalives (an empty `Input`) while a client sends
/// nothing else: well inside the server's 1.5 s before a same-name join
/// may take the session over, and its 8 s silent timeout.
pub const KEEPALIVE_EVERY: f64 = 0.5;

/// When a keepalive is due: [`KEEPALIVE_EVERY`] after the last message
/// of any kind went out.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keepalive {
    last_sent: Option<f64>,
}

impl Keepalive {
    /// A message went out at `now`.
    pub fn sent(&mut self, now: f64) {
        self.last_sent = Some(now);
    }

    /// Whether a keepalive is due at `now`; if so, it counts as sent.
    pub fn due(&mut self, now: f64) -> bool {
        match self.last_sent {
            Some(t) if now - t < KEEPALIVE_EVERY => false,
            _ => {
                self.last_sent = Some(now);
                true
            }
        }
    }
}

/// How a connection ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closed {
    /// It never opened (server down, network gone).
    NeverOpened,
    /// It opened but the server closed it without a `Welcome`: the join
    /// was refused (a live session of the same name, a protocol mismatch).
    Refused,
    /// It was in the game and dropped: the server timed us out (a page
    /// frozen in a background tab), restarted, or the network blinked.
    Dropped,
}

/// Seconds after a drop before the first redial: past the server's 1.5 s
/// before a same-name join may replace a silent session, should it still
/// hold ours.
pub const REDIAL_FIRST: f64 = 2.0;
/// Longest wait between redials while the server stays unreachable.
pub const REDIAL_MAX: f64 = 30.0;

/// When to dial the server again. Only a connection that was in the game
/// and dropped starts redialing; a refused join stops it, and attempts
/// that never open back off up to [`REDIAL_MAX`].
#[derive(Clone, Copy, Debug, Default)]
pub struct Redial {
    delay: f64,
    at: Option<f64>,
}

impl Redial {
    /// The connection ended at `now`, `how`.
    pub fn closed(&mut self, how: Closed, now: f64) {
        self.delay = match how {
            Closed::Dropped => REDIAL_FIRST,
            Closed::NeverOpened if self.delay > 0.0 => (self.delay * 2.0).min(REDIAL_MAX),
            Closed::NeverOpened | Closed::Refused => 0.0,
        };
        self.at = if self.delay > 0.0 {
            Some(now + self.delay)
        } else {
            None
        };
    }

    /// Whether to dial now; once per [`Self::closed`].
    pub fn due(&mut self, now: f64) -> bool {
        match self.at {
            Some(at) if now >= at => {
                self.at = None;
                true
            }
            _ => false,
        }
    }

    /// Seconds until the next redial, if one is planned.
    pub fn pending(&self, now: f64) -> Option<f64> {
        self.at.map(|at| (at - now).max(0.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_keepalive_is_due_only_after_a_quiet_spell() {
        let mut k = Keepalive::default();
        assert!(k.due(0.0), "nothing sent yet");
        assert!(!k.due(0.3));
        k.sent(0.25); // an input went out
        assert!(!k.due(0.7));
        assert!(k.due(0.75));
        assert!(!k.due(1.0), "the keepalive counts as sent");
        assert!(k.due(1.25));
    }

    #[test]
    fn a_dropped_link_redials_and_backs_off_while_the_server_is_away() {
        let mut r = Redial::default();
        assert!(!r.due(100.0), "nothing to redial before any drop");
        r.closed(Closed::Dropped, 10.0);
        assert_eq!(r.pending(10.0), Some(REDIAL_FIRST));
        assert!(!r.due(11.9));
        assert!(r.due(12.0));
        assert!(!r.due(12.1), "one dial per close");
        // The server is down: each attempt that never opens waits longer.
        let mut now = 12.5;
        let mut waits = Vec::new();
        for _ in 0..6 {
            r.closed(Closed::NeverOpened, now);
            let wait = r.pending(now).unwrap();
            waits.push(wait);
            now += wait;
            assert!(r.due(now));
        }
        assert_eq!(waits, [4.0, 8.0, 16.0, 30.0, 30.0, 30.0]);
        // Back in the game and dropped again: the first wait again.
        r.closed(Closed::Dropped, now);
        assert_eq!(r.pending(now), Some(REDIAL_FIRST));
    }

    #[test]
    fn a_refused_join_or_a_first_failure_does_not_redial() {
        let mut r = Redial::default();
        r.closed(Closed::NeverOpened, 0.0);
        assert_eq!(r.pending(0.0), None, "never in the game: no redial");
        r.closed(Closed::Dropped, 1.0);
        r.closed(Closed::Refused, 2.0);
        assert_eq!(r.pending(2.0), None, "the server said no");
        assert!(!r.due(100.0));
    }
}
