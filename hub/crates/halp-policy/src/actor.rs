//! Tiers, actors and the per-call context.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Consent tiers (see `docs/hub/PLAN.md`).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
pub enum Tier {
    /// Read.
    T0,
    /// Reversible local write.
    T1,
    /// External side effect.
    T2,
    /// Destructive or privileged.
    T3,
}

impl Tier {
    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T0 => "T0",
            Tier::T1 => "T1",
            Tier::T2 => "T2",
            Tier::T3 => "T3",
        }
    }
}

/// Who is calling. Serialized as the contract's actor string.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Actor {
    User,
    Agent(String),
    Module(String),
    Integration(String),
    System,
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Actor::User => f.write_str("user"),
            Actor::Agent(c) => write!(f, "agent:{c}"),
            Actor::Module(m) => write!(f, "module:{m}"),
            Actor::Integration(i) => write!(f, "integration:{i}"),
            Actor::System => f.write_str("system"),
        }
    }
}

impl Actor {
    /// Parses `user`, `system`, `agent:<id>`, `module:<id>` or
    /// `integration:<id>` (ids: `[A-Za-z0-9_.-]{1,64}`).
    pub fn parse(s: &str) -> Option<Actor> {
        match s {
            "user" => return Some(Actor::User),
            "system" => return Some(Actor::System),
            _ => {}
        }
        let (kind, id) = s.split_once(':')?;
        halp_domain::limits::simple_id("actor id", id).ok()?;
        let id = id.to_string();
        match kind {
            "agent" => Some(Actor::Agent(id)),
            "module" => Some(Actor::Module(id)),
            "integration" => Some(Actor::Integration(id)),
            _ => None,
        }
    }

    pub fn is_user(&self) -> bool {
        matches!(self, Actor::User)
    }
}

/// Per-call context. `session` identifies one connection (an MCP session);
/// taint is tracked per actor across all of its sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ctx {
    pub actor: Actor,
    pub session: Option<String>,
    /// The user, but over a channel anything running as the user can reach
    /// (the socket's `HALP-USER` channel for `halpd consent|clients`): it
    /// is not shown what private items hold (pending requests on them are
    /// redacted). The Hub window and `halpd serve --interactive` are not
    /// limited.
    pub limited: bool,
}

impl Ctx {
    pub fn new(actor: Actor) -> Self {
        Ctx {
            actor,
            session: None,
            limited: false,
        }
    }
    pub fn user() -> Self {
        Self::new(Actor::User)
    }
    /// The user over the socket's user channel (see [`Ctx::limited`]).
    pub fn user_cli() -> Self {
        Ctx {
            limited: true,
            ..Self::new(Actor::User)
        }
    }
    pub fn system() -> Self {
        Self::new(Actor::System)
    }
    pub fn agent(client: &str) -> Self {
        Self::new(Actor::Agent(client.to_string()))
    }
    pub fn with_session(mut self, session: impl Into<String>) -> Self {
        self.session = Some(session.into());
        self
    }
    /// The key taint is tracked under: the actor, whatever the session.
    /// (Per-session taint let a client shed it by reconnecting.)
    pub fn taint_key(&self) -> String {
        self.actor.to_string()
    }
}
