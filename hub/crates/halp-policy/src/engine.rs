//! The policy engine.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::actor::{Actor, Ctx, Tier};
use crate::model::{Client, Rule};
use crate::secret::{ct_eq, token_hash};

/// One call as the policy sees it. `tier` is the effective tier (the core
/// may raise a command's static tier for these particular arguments).
#[derive(Debug, Clone, Copy)]
pub struct Query<'a> {
    pub ctx: &'a Ctx,
    pub command: &'a str,
    pub tier: Tier,
    pub data_class: &'a str,
    pub args: &'a Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Allow {
        reason: String,
    },
    Deny {
        reason: String,
    },
    /// Park the call until a human approves. `tainted` names the source of
    /// untrusted content this session has read, if any.
    Confirm {
        reason: String,
        tainted: Option<String>,
    },
}

impl Verdict {
    pub fn reason(&self) -> &str {
        match self {
            Verdict::Allow { reason }
            | Verdict::Deny { reason }
            | Verdict::Confirm { reason, .. } => reason,
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Verdict::Allow { .. } => "allow",
            Verdict::Deny { .. } => "deny",
            Verdict::Confirm { .. } => "confirm",
        }
    }
}

/// What the core asks before every handler.
pub trait Policy: Send + Sync {
    fn check(&self, q: &Query<'_>, now: Instant) -> Verdict;
    /// Asked before a call is parked; `Err` refuses (park rate limit).
    fn may_park(&self, _ctx: &Ctx, _now: Instant) -> Result<(), String> {
        Ok(())
    }
    /// The call returned content created by `origin`.
    fn note_read(&self, _ctx: &Ctx, _origin: &str) {}
    /// The taint source of this caller's client, if tainted.
    fn taint(&self, _ctx: &Ctx) -> Option<String> {
        None
    }
    /// Forgets the caller's taint (the user's `policy.clear_taint`).
    fn clear_taint(&self, _ctx: &Ctx) {}
    /// Replaces the clients and rules the engine decides over.
    fn load(&self, _clients: Vec<Client>, _rules: Vec<Rule>) {}
    /// The active client a token belongs to.
    fn authenticate(&self, _token: &str) -> Option<String> {
        None
    }
    /// What the policy remembers of a paired client's recent calls (kept
    /// in memory only: empty again after a restart).
    fn activity(&self, _client: &str, _now: Instant) -> Activity {
        Activity::default()
    }
    /// Whether the caller may see and touch data of `class` (an item whose
    /// type declares `x-halp-data-class`). The user and the system see
    /// everything; anyone else only the default class unless granted.
    fn allows_class(&self, ctx: &Ctx, class: &str) -> bool {
        matches!(ctx.actor, Actor::User | Actor::System) || class == crate::DEFAULT_DATA_CLASS
    }
}

/// A client's recent activity, relative to the `now` it was asked at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Activity {
    /// How long ago the policy last decided a call of this client.
    pub idle: Option<Duration>,
    /// How long until its taint clears, when tainted.
    pub tainted_for: Option<Duration>,
}

/// Lets everything through without asking. Only for tests of code below
/// the policy; never installed by default.
#[cfg(feature = "allow-all")]
#[derive(Debug, Default, Clone, Copy)]
pub struct AllowAll;

#[cfg(feature = "allow-all")]
impl Policy for AllowAll {
    fn check(&self, _q: &Query<'_>, _now: Instant) -> Verdict {
        Verdict::Allow {
            reason: "allow-all test policy".into(),
        }
    }
}

/// A token bucket: `cap` tokens, refilled at `per_sec`.
#[derive(Debug, Clone)]
pub struct Bucket {
    cap: f64,
    per_sec: f64,
    tokens: f64,
    last: Instant,
}

impl Bucket {
    pub fn new(cap: u32, per_sec: f64, now: Instant) -> Self {
        Bucket {
            cap: f64::from(cap),
            per_sec,
            tokens: f64::from(cap),
            last: now,
        }
    }

    pub fn take(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = self.last.max(now);
        self.tokens = (self.tokens + dt * self.per_sec).min(self.cap);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

/// How long a client stays tainted after its last read of untrusted content.
pub const DEFAULT_TAINT_TTL: Duration = Duration::from_secs(30 * 60);

/// A client's taint: the first untrusted source it read, and when it last
/// read untrusted content.
#[derive(Debug, Clone)]
struct Taint {
    source: String,
    last: Instant,
}

/// Engine limits. The defaults are what halpd and the app install.
#[derive(Debug, Clone)]
pub struct Limits {
    /// All calls per client: burst and refill per second.
    pub calls: (u32, f64),
    /// Writes (T1 and up) per client.
    pub writes: (u32, f64),
    /// Requests parked for confirmation per client.
    pub parks: (u32, f64),
    /// Circuit breaker: this many T2/T3 requests within `breaker_window`
    /// open it for `breaker_cooldown`; while open, T2/T3 calls are denied.
    pub breaker_threshold: usize,
    pub breaker_window: Duration,
    pub breaker_cooldown: Duration,
    /// A client's taint clears this long after its last tainted read.
    pub taint_ttl: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            calls: (120, 4.0),
            writes: (60, 1.0),
            parks: (16, 0.1),
            breaker_threshold: 8,
            breaker_window: Duration::from_secs(60),
            breaker_cooldown: Duration::from_secs(300),
            taint_ttl: DEFAULT_TAINT_TTL,
        }
    }
}

#[derive(Debug)]
struct ClientState {
    calls: Bucket,
    writes: Bucket,
    parks: Bucket,
    /// When the policy last decided a call of this client.
    last_seen: Option<Instant>,
    destructive: VecDeque<Instant>,
    open_until: Option<Instant>,
}

#[derive(Default)]
struct Inner {
    clients: BTreeMap<String, Client>,
    rules: Vec<Rule>,
    rate: HashMap<String, ClientState>,
    /// Keyed by actor (`agent:<id>`), not by connection: a client that
    /// opens a fresh session is still the client that read the content.
    taint: HashMap<String, Taint>,
}

/// The only commands an `integration:<id>` actor may run without asking
/// besides T0 reads: writing the items it syncs, changing whitelisted
/// fields of items it created, and reporting its status. All are hidden
/// from MCP, refuse every other actor, and only ever touch the calling
/// integration's own items (see `docs/INTEGRATIONS.md`).
pub const INTEGRATION_WRITES: &[&str] = &["ext.upsert", "ext.patch_fields", "ext.status"];

/// The real policy (see `docs/MCP.md`, "Policy"):
///
/// * `user` and `system`: T0-T2 allowed, T3 always confirmed.
/// * `agent:<client>`: must be a paired, unrevoked client; rate limited;
///   needs a grant for the command's data class; then T0 allowed, T1
///   allowed with the T1 grant (else confirmed), T2 confirmed unless a
///   standing rule matches and the session is untainted, T3 always
///   confirmed. A circuit breaker denies T2/T3 after a burst of them.
/// * `integration:<id>`: T0 of class `general` and the [`INTEGRATION_WRITES`]
///   commands (rate limited) allowed, the rest confirmed.
/// * `module:<pack_id>`: T0 of class `general` allowed, everything else
///   denied at once, never confirmed (no grants for modules yet; a module
///   never raises a consent prompt).
pub struct Engine {
    inner: Mutex<Inner>,
    limits: Limits,
}

impl Default for Engine {
    fn default() -> Self {
        Engine::new(Limits::default())
    }
}

fn confirm(reason: impl Into<String>, tainted: Option<String>) -> Verdict {
    Verdict::Confirm {
        reason: reason.into(),
        tainted,
    }
}

fn deny(reason: impl Into<String>) -> Verdict {
    Verdict::Deny {
        reason: reason.into(),
    }
}

fn allow(reason: impl Into<String>) -> Verdict {
    Verdict::Allow {
        reason: reason.into(),
    }
}

impl Engine {
    pub fn new(limits: Limits) -> Self {
        Engine {
            inner: Mutex::new(Inner::default()),
            limits,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Plain data; a poisoned lock is still consistent enough to use.
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn state<'a>(&self, inner: &'a mut Inner, client: &str, now: Instant) -> &'a mut ClientState {
        let l = &self.limits;
        inner
            .rate
            .entry(client.to_string())
            .or_insert_with(|| ClientState {
                calls: Bucket::new(l.calls.0, l.calls.1, now),
                writes: Bucket::new(l.writes.0, l.writes.1, now),
                parks: Bucket::new(l.parks.0, l.parks.1, now),
                last_seen: None,
                destructive: VecDeque::new(),
                open_until: None,
            })
    }

    fn check_agent(&self, q: &Query<'_>, client_id: &str, now: Instant) -> Verdict {
        let mut inner = self.lock();
        let tainted = live_taint(&mut inner, &q.ctx.taint_key(), now, self.limits.taint_ttl);
        let Some(client) = inner.clients.get(client_id).cloned() else {
            return deny(format!("agent:{client_id} is not a paired client"));
        };
        if !client.active() {
            return deny(format!("client {client_id} was revoked"));
        }
        let rule_hit = q.tier == Tier::T2
            && inner
                .rules
                .iter()
                .any(|r| r.matches(client_id, q.command, q.args));
        let window = self.limits.breaker_window;
        let threshold = self.limits.breaker_threshold;
        let cooldown = self.limits.breaker_cooldown;
        let rs = self.state(&mut inner, client_id, now);
        rs.last_seen = Some(rs.last_seen.map_or(now, |t| t.max(now)));
        if !rs.calls.take(now) {
            return deny(format!(
                "rate limit: client {client_id} is calling too often"
            ));
        }
        if q.tier >= Tier::T1 && !rs.writes.take(now) {
            return deny(format!(
                "rate limit: client {client_id} is writing too often"
            ));
        }
        if q.tier >= Tier::T2 {
            if rs.open_until.is_some_and(|t| now < t) {
                return deny(format!(
                    "circuit breaker open: client {client_id} made too many destructive requests; try again later"
                ));
            }
            while rs
                .destructive
                .front()
                .is_some_and(|t| now.saturating_duration_since(*t) > window)
            {
                rs.destructive.pop_front();
            }
            rs.destructive.push_back(now);
            if rs.destructive.len() > threshold {
                rs.open_until = Some(now + cooldown);
                rs.destructive.clear();
                return deny(format!(
                    "circuit breaker opened: more than {threshold} destructive requests from {client_id} within {}s",
                    window.as_secs()
                ));
            }
        }
        if !client.grants.allows_class(q.data_class) {
            return deny(format!(
                "client {client_id} has no grant for data class {}",
                q.data_class
            ));
        }
        match q.tier {
            Tier::T0 => allow("read"),
            Tier::T1 if client.grants.t1 => allow("T1 grant"),
            Tier::T1 => confirm(
                format!("{} by {} (no T1 grant)", q.command, q.ctx.actor),
                tainted,
            ),
            Tier::T2 if tainted.is_some() => confirm(
                format!(
                    "{} by {} after reading untrusted content",
                    q.command, q.ctx.actor
                ),
                tainted,
            ),
            Tier::T2 if rule_hit => allow("standing rule"),
            Tier::T2 => confirm(format!("{} by {}", q.command, q.ctx.actor), tainted),
            Tier::T3 => confirm(format!("{} by {}", q.command, q.ctx.actor), tainted),
        }
    }

    /// Whether the breaker is open for a client (for status displays).
    pub fn breaker_open(&self, client_id: &str, now: Instant) -> bool {
        self.lock()
            .rate
            .get(client_id)
            .and_then(|s| s.open_until)
            .is_some_and(|t| now < t)
    }

    /// [`Policy::note_read`] at a given instant.
    pub fn note_read_at(&self, ctx: &Ctx, origin: &str, now: Instant) {
        if ctx.actor.is_user()
            || origin == "user"
            || origin == "system"
            || origin == ctx.actor.to_string()
        {
            return;
        }
        let mut inner = self.lock();
        let ttl = self.limits.taint_ttl;
        let key = ctx.taint_key();
        // An expired taint starts over with the new source.
        live_taint(&mut inner, &key, now, ttl);
        inner
            .taint
            .entry(key)
            .and_modify(|t| t.last = t.last.max(now))
            .or_insert_with(|| Taint {
                source: origin.to_string(),
                last: now,
            });
    }

    /// [`Policy::taint`] at a given instant.
    pub fn taint_at(&self, ctx: &Ctx, now: Instant) -> Option<String> {
        let mut inner = self.lock();
        live_taint(&mut inner, &ctx.taint_key(), now, self.limits.taint_ttl)
    }
}

/// The taint under `key`, dropping it once `ttl` has passed since the last
/// tainted read.
fn live_taint(inner: &mut Inner, key: &str, now: Instant, ttl: Duration) -> Option<String> {
    let t = inner.taint.get(key)?;
    if now.saturating_duration_since(t.last) >= ttl {
        inner.taint.remove(key);
        return None;
    }
    Some(t.source.clone())
}

impl Policy for Engine {
    fn check(&self, q: &Query<'_>, now: Instant) -> Verdict {
        match &q.ctx.actor {
            Actor::User | Actor::System if q.tier == Tier::T3 => {
                confirm(format!("{} is destructive", q.command), None)
            }
            Actor::User | Actor::System => allow("user"),
            Actor::Agent(c) => self.check_agent(q, c, now),
            Actor::Integration(id)
                if q.tier <= Tier::T1
                    && q.data_class == crate::DEFAULT_DATA_CLASS
                    && INTEGRATION_WRITES.contains(&q.command) =>
            {
                let mut inner = self.lock();
                let key = format!("integration:{id}");
                let rs = self.state(&mut inner, &key, now);
                if rs.writes.take(now) {
                    allow("integration sync")
                } else {
                    deny(format!("rate limit: integration {id} is writing too often"))
                }
            }
            // A module (Tier B transform, Tier C widget) never asks the
            // user: anything but a `general` read is refused at once, so a
            // pack can neither park a request nor raise a consent prompt
            // (P5-EXTENSIONS.md §5.4, CONTRACT v1.10 note 4).
            Actor::Module(_) => {
                if q.tier == Tier::T0 && q.data_class == crate::DEFAULT_DATA_CLASS {
                    allow("read")
                } else {
                    deny(format!(
                        "{} by {}: modules may only read general data",
                        q.command, q.ctx.actor
                    ))
                }
            }
            Actor::Integration(_) => {
                if q.tier == Tier::T0 && q.data_class == crate::DEFAULT_DATA_CLASS {
                    allow("read")
                } else {
                    let t = self.taint(q.ctx);
                    confirm(format!("{} by {}", q.command, q.ctx.actor), t)
                }
            }
        }
    }

    fn allows_class(&self, ctx: &Ctx, class: &str) -> bool {
        match &ctx.actor {
            Actor::User | Actor::System => true,
            Actor::Agent(c) => self
                .lock()
                .clients
                .get(c)
                .is_some_and(|cl| cl.active() && cl.grants.allows_class(class)),
            _ => class == crate::DEFAULT_DATA_CLASS,
        }
    }

    fn may_park(&self, ctx: &Ctx, now: Instant) -> Result<(), String> {
        let key = match &ctx.actor {
            Actor::User | Actor::System => return Ok(()),
            // Second lock behind `check`: a module's request is never parked.
            Actor::Module(_) => {
                return Err(format!("{} may not ask for confirmation", ctx.actor));
            }
            other => match other {
                Actor::Agent(c) => c.clone(),
                a => a.to_string(),
            },
        };
        let mut inner = self.lock();
        let rs = self.state(&mut inner, &key, now);
        if rs.parks.take(now) {
            Ok(())
        } else {
            Err(format!(
                "rate limit: {} has asked for too many confirmations",
                ctx.actor
            ))
        }
    }

    fn note_read(&self, ctx: &Ctx, origin: &str) {
        self.note_read_at(ctx, origin, Instant::now());
    }

    fn taint(&self, ctx: &Ctx) -> Option<String> {
        self.taint_at(ctx, Instant::now())
    }

    fn clear_taint(&self, ctx: &Ctx) {
        self.lock().taint.remove(&ctx.taint_key());
    }

    fn activity(&self, client: &str, now: Instant) -> Activity {
        let ttl = self.limits.taint_ttl;
        let mut inner = self.lock();
        let key = Actor::Agent(client.to_string()).to_string();
        live_taint(&mut inner, &key, now, ttl);
        let tainted_for = inner
            .taint
            .get(&key)
            .map(|t| ttl.saturating_sub(now.saturating_duration_since(t.last)));
        let idle = inner
            .rate
            .get(client)
            .and_then(|r| r.last_seen)
            .map(|t| now.saturating_duration_since(t));
        Activity { idle, tainted_for }
    }

    fn load(&self, clients: Vec<Client>, rules: Vec<Rule>) {
        let mut inner = self.lock();
        inner.clients = clients.into_iter().map(|c| (c.id.clone(), c)).collect();
        inner.rules = rules;
    }

    fn authenticate(&self, token: &str) -> Option<String> {
        let h = token_hash(token);
        let inner = self.lock();
        let mut found = None;
        // Compare against every client so timing does not reveal which one.
        for c in inner.clients.values() {
            if ct_eq(h.as_bytes(), c.token_hash.as_bytes()) && c.active() {
                found = Some(c.id.clone());
            }
        }
        found
    }
}
