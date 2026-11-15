//! What the core knows about integrations: their stored configuration, the
//! status the runtime reports, and the [`IntegrationHost`] hook through
//! which the core hands side effects (keychain, network, `git`) to the
//! integration runtime (`halp-integrations`), which runs in the same process.
//!
//! The core owns the configuration (in the encrypted store, changed only by
//! the user-only `integrations.connect` / `integrations.disconnect`) and the
//! synced items (written only through `ext.upsert`, by the integration's own
//! actor). The runtime owns everything that talks to the outside world.
//! Secrets never pass through the core at all: the shell hands tokens
//! straight to the runtime, which keeps them in the OS keychain.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Integration kinds the runtime implements.
pub const INTEGRATION_KINDS: &[&str] = &["github", "mcp", "git", "calendar", "energy"];
/// Shortest and longest polling interval.
pub const MIN_INTERVAL_SECS: u32 = 60;
pub const MAX_INTERVAL_SECS: u32 = 86_400;
pub const DEFAULT_INTERVAL_SECS: u32 = 300;
/// Largest `settings` document.
pub const MAX_SETTINGS_BYTES: usize = 16 * 1024;
/// Items per `ext.upsert` call.
pub const MAX_UPSERT_ITEMS: usize = 500;
/// Longest external id (`github:owner/repo#123`).
pub const MAX_EXTERNAL_ID_CHARS: usize = 300;
/// The tag an archived synced item carries.
pub const ARCHIVED_TAG: &str = "archived";

/// One configured integration, as stored (entity `integration`). Holds no
/// secrets.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IntegrationConfig {
    /// `[A-Za-z0-9_.-]{1,64}`; the integration acts as `integration:<id>`.
    pub id: String,
    /// One of [`INTEGRATION_KINDS`].
    pub kind: String,
    pub title: String,
    pub enabled: bool,
    pub interval_secs: u32,
    /// Roles the synced items belong to (empty = every role).
    pub roles: Vec<String>,
    /// Kind-specific settings, validated by the runtime.
    pub settings: Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl IntegrationConfig {
    /// The actor string the integration writes as.
    pub fn actor(&self) -> String {
        format!("integration:{}", self.id)
    }
}

/// Where an integration stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    /// Configured, not synced yet.
    #[default]
    Pending,
    /// Synchronising right now.
    Syncing,
    /// The last sync worked.
    Ok,
    /// The last sync failed; it is retried with backoff.
    Error,
    /// It needs the user to (re)connect: no token, or the token was refused.
    NeedsAuth,
    /// Turned off by the user.
    Disabled,
    /// The integration runtime is not running in this process (halpd).
    Stopped,
}

impl IntegrationState {
    pub fn as_str(self) -> &'static str {
        match self {
            IntegrationState::Pending => "pending",
            IntegrationState::Syncing => "syncing",
            IntegrationState::Ok => "ok",
            IntegrationState::Error => "error",
            IntegrationState::NeedsAuth => "needs_auth",
            IntegrationState::Disabled => "disabled",
            IntegrationState::Stopped => "stopped",
        }
    }
}

/// The runtime's view of one integration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
pub struct IntegrationStatus {
    pub state: IntegrationState,
    /// End of the last successful sync.
    pub last_sync: Option<DateTime<Utc>>,
    /// When the next sync is due (none while disabled or waiting for auth).
    pub next_sync: Option<DateTime<Utc>>,
    /// The last error, redacted (no tokens, no URLs with credentials).
    pub error: Option<String>,
    /// Items the last successful sync delivered.
    pub items: u32,
    /// A token (or other secret) is in the OS keychain for it.
    pub has_secret: bool,
    /// An authorization flow is waiting for the browser.
    pub authorizing: bool,
}

/// What `context.capture` handed to the runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CaptureTicket {
    /// The integration that will write the `dev.context` item.
    pub integration: String,
    /// The canonical repository path.
    pub repo_path: String,
    /// Always true: the snapshot is taken in the background and arrives
    /// as an `ext.upsert` event.
    pub queued: bool,
}

/// Side effects the core delegates to the integration runtime. Every
/// method is called while the core's state lock is held, so implementations
/// must be quick and must never call back into the core (queue the work).
pub trait IntegrationHost: Send + Sync {
    /// Checks and normalises kind-specific settings (for example
    /// canonicalising `git` roots). Returns the settings to store.
    fn validate(&self, kind: &str, settings: &Value) -> Result<Value, String>;
    /// The configuration was stored (new or changed). `reauthorize` asks for
    /// a fresh authorization (OAuth) even when a token is stored.
    fn configured(&self, cfg: &IntegrationConfig, reauthorize: bool);
    /// The integration was removed: stop it and delete its secrets.
    fn removed(&self, id: &str);
    /// Asks for a sync now (`None` = all). Returns the ids queued.
    fn sync_now(&self, id: Option<&str>) -> Result<Vec<String>, String>;
    /// The current status, if the runtime knows the integration.
    fn status(&self, id: &str) -> Option<IntegrationStatus>;
    /// Queues a local git context snapshot of `repo_path` for the `git`
    /// integration `cfg`. Path checks happen here, synchronously.
    fn capture_context(
        &self,
        cfg: &IntegrationConfig,
        repo_path: &str,
    ) -> Result<CaptureTicket, String>;
}

/// The sized handle the host is registered under (`halp_core::Core::set_extension`).
#[derive(Clone)]
pub struct HostHandle(pub std::sync::Arc<dyn IntegrationHost>);

/// Removes control and bidirectional-override characters from untrusted
/// text (applied to every string in synced fields).
pub fn strip_controls(s: &str) -> String {
    s.chars()
        .filter(|c| {
            !c.is_control() && !is_bidi_control(*c) && !crate::limits::is_format_control(*c)
        })
        .collect()
}

fn is_bidi_control(c: char) -> bool {
    matches!(
        c,
        '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{061C}'
    )
}

/// [`strip_controls`] on every string in a JSON value (keys included).
pub fn clean_json(v: &Value) -> Value {
    match v {
        Value::String(s) => Value::String(strip_controls(s)),
        Value::Array(a) => Value::Array(a.iter().map(clean_json).collect()),
        Value::Object(m) => Value::Object(
            m.iter()
                .map(|(k, x)| (strip_controls(k), clean_json(x)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn controls_and_bidi_overrides_are_removed() {
        assert_eq!(strip_controls("a\u{1b}[31mb\u{202E}c\nd"), "a[31mbcd");
        assert_eq!(
            clean_json(&json!({"k\u{7}": ["x\u{2066}y"]})),
            json!({"k": ["xy"]})
        );
    }
}
