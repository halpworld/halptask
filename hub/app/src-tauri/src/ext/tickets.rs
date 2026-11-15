//! One-time tickets for widget entry documents.
//!
//! The scheme serves an HTML document only with a ticket that `ext_widget`
//! issued for that exact pack and widget, used once, within [`TTL`]. So a
//! frame cannot load another pack's (or its own) page by navigating itself
//! (§6.4 finding 11): the host never issued a ticket for that load. The
//! host tears such a frame down anyway (condition 5); this is the second
//! lock. Scripts, styles, PNG, fonts and JSON need no ticket: they are
//! inert as documents (`nosniff`, CSP `sandbox`). No other scriptable
//! document type (SVG, XHTML) is served at all.

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// How long an issued ticket stays usable.
pub const TTL: Duration = Duration::from_secs(60);
/// Most outstanding tickets; the oldest go first.
const MAX_OUTSTANDING: usize = 256;

#[derive(Default)]
pub struct Tickets {
    inner: Mutex<HashMap<String, (String, String, Instant)>>,
}

impl Tickets {
    /// A fresh 128-bit ticket for `pack_id`/`widget`.
    pub fn issue(&self, pack_id: &str, widget: &str) -> String {
        self.issue_at(pack_id, widget, Instant::now())
    }

    fn issue_at(&self, pack_id: &str, widget: &str, now: Instant) -> String {
        let t = random_hex16();
        let mut m = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        m.retain(|_, (_, _, at)| now.saturating_duration_since(*at) < TTL);
        while m.len() >= MAX_OUTSTANDING {
            let Some(oldest) = m
                .iter()
                .min_by_key(|(_, (_, _, at))| *at)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            m.remove(&oldest);
        }
        m.insert(t.clone(), (pack_id.to_string(), widget.to_string(), now));
        t
    }

    /// Uses up `ticket` if it was issued for `pack_id`/`widget` and has not
    /// expired. A ticket presented for the wrong widget is burnt too.
    pub fn redeem(&self, ticket: &str, pack_id: &str, widget: &str) -> bool {
        self.redeem_at(ticket, pack_id, widget, Instant::now())
    }

    fn redeem_at(&self, ticket: &str, pack_id: &str, widget: &str, now: Instant) -> bool {
        let mut m = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match m.remove(ticket) {
            Some((p, w, at)) => {
                p == pack_id && w == widget && now.saturating_duration_since(at) < TTL
            }
            None => false,
        }
    }
}

/// 16 bytes from the OS CSPRNG, as hex. Panics never: if the OS RNG
/// fails, the ticket is unusable (all `x`) and every redeem fails closed.
fn random_hex16() -> String {
    let mut b = [0u8; 16];
    match getrandom::fill(&mut b) {
        Ok(()) => hex::encode(b),
        Err(_) => "x".repeat(32),
    }
}

/// A ticket's shape: 32 lowercase hex characters.
pub fn is_ticket(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ticket_works_once_for_its_widget_only() {
        let t = Tickets::default();
        let a = t.issue("acme.sre", "oncall");
        assert!(is_ticket(&a));
        assert!(t.redeem(&a, "acme.sre", "oncall"));
        assert!(!t.redeem(&a, "acme.sre", "oncall"), "single use");
        let b = t.issue("acme.sre", "oncall");
        assert!(!t.redeem(&b, "other.pack", "oncall"));
        assert!(
            !t.redeem(&b, "acme.sre", "oncall"),
            "burnt by the wrong redeem"
        );
        assert!(!t.redeem("0".repeat(32).as_str(), "acme.sre", "oncall"));
    }

    #[test]
    fn tickets_expire_and_stay_bounded() {
        let t = Tickets::default();
        let t0 = Instant::now();
        let a = t.issue_at("acme.sre", "oncall", t0);
        assert!(!t.redeem_at(&a, "acme.sre", "oncall", t0 + TTL));
        for _ in 0..(MAX_OUTSTANDING + 10) {
            t.issue_at("acme.sre", "oncall", t0);
        }
        assert!(t.inner.lock().unwrap_or_else(PoisonError::into_inner).len() <= MAX_OUTSTANDING);
    }

    #[test]
    fn tickets_are_distinct() {
        let t = Tickets::default();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            assert!(seen.insert(t.issue("a.b", "c")));
        }
    }
}
