//! HalpTask Hub policy: who may run what, when a human has to confirm, and
//! the bookkeeping behind it (client grants, standing rules, session taint,
//! rate limits, circuit breaker, consent ids and argument hashes).
//!
//! The core asks [`Policy::check`] before every handler, for every actor,
//! and enforces the verdict; MCP, the desktop UI and halpd all go through
//! that one path.

mod actor;
mod engine;
mod model;
pub mod secret;

pub use actor::{Actor, Ctx, Tier};
#[cfg(feature = "allow-all")]
pub use engine::AllowAll;
pub use engine::{
    Activity, Bucket, DEFAULT_TAINT_TTL, Engine, INTEGRATION_WRITES, Limits, Policy, Query, Verdict,
};
pub use model::{Client, DATA_CLASSES, DEFAULT_DATA_CLASS, Grants, Rule, is_data_class};

#[cfg(test)]
mod tests;
