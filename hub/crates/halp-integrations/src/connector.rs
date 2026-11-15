//! What a connector is: a kind of integration that turns a remote system
//! into batches of `ext.upsert` items. Connectors never write to the core
//! themselves; the runtime dispatches what they return as the integration's
//! own actor, so policy, validation and audit apply as for any other
//! caller.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::http::{Http, HttpError, HttpErrorKind};
use crate::oauth::OAuthProvider;
use crate::redact::redact;

pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// One `ext.upsert` call: items of one scope. `complete` means the batch is
/// the whole remote set for the scope (missing linked items get archived).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Batch {
    pub scope: String,
    pub complete: bool,
    pub items: Vec<Value>,
    /// Field changes to existing items this integration owns (its origin,
    /// linked to it) outside a full upsert: a calendar moving an
    /// `em.one_on_one`'s `scheduled_for`. Applied after the upsert through
    /// `ext.patch_fields`, which refuses items of another origin, unlinked
    /// or locked items, and keys the type does not mark
    /// `x-halp-ext-writable`.
    pub patches: Vec<FieldPatch>,
}

/// A change to some fields of an existing item.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FieldPatch {
    pub item_id: String,
    pub type_id: String,
    /// The version the change was computed from.
    pub if_version: Option<u64>,
    pub fields: Map<String, Value>,
}

/// Read access to the core's items, as the integration's own actor (so
/// policy applies: general-class T0 reads only; private notes are never
/// visible).
pub trait ItemSource: Send + Sync {
    /// `tasks.list` with `filter`; the `items` array.
    fn list<'a>(&'a self, filter: Value) -> BoxFuture<'a, Result<Vec<Value>, SyncError>>;
}

/// An [`ItemSource`] with nothing in it.
pub struct NoItems;

impl ItemSource for NoItems {
    fn list<'a>(&'a self, _filter: Value) -> BoxFuture<'a, Result<Vec<Value>, SyncError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SyncError {
    /// The credentials are missing, expired or rejected.
    Auth(String),
    /// Worth retrying later (network, 5xx, rate limits).
    Transient {
        message: String,
        retry_after: Option<Duration>,
    },
    /// The settings cannot work (a forbidden URL, a missing tool).
    Config(String),
    Cancelled,
}

impl SyncError {
    pub fn transient(m: impl Into<String>) -> Self {
        SyncError::Transient {
            message: m.into(),
            retry_after: None,
        }
    }

    pub fn message(&self) -> String {
        match self {
            SyncError::Auth(m) | SyncError::Config(m) => m.clone(),
            SyncError::Transient { message, .. } => message.clone(),
            SyncError::Cancelled => "cancelled".into(),
        }
    }

    /// The same error with secrets and URLs scrubbed from its text.
    pub fn redacted(self, known: &[&str]) -> Self {
        match self {
            SyncError::Auth(m) => SyncError::Auth(redact(&m, known)),
            SyncError::Config(m) => SyncError::Config(redact(&m, known)),
            SyncError::Transient {
                message,
                retry_after,
            } => SyncError::Transient {
                message: redact(&message, known),
                retry_after,
            },
            SyncError::Cancelled => SyncError::Cancelled,
        }
    }
}

impl From<HttpError> for SyncError {
    fn from(e: HttpError) -> Self {
        match e.kind {
            HttpErrorKind::Cancelled => SyncError::Cancelled,
            HttpErrorKind::Policy => SyncError::Config(e.message),
            HttpErrorKind::Network | HttpErrorKind::Timeout | HttpErrorKind::TooLarge => {
                SyncError::transient(e.message)
            }
        }
    }
}

/// What a sync gets to work with.
pub struct SyncCx<'a> {
    pub id: &'a str,
    pub settings: &'a Value,
    pub http: &'a Http,
    /// The access token, when the connector needs one.
    pub token: Option<&'a str>,
    pub cancel: &'a CancellationToken,
    /// This integration's private directory (MCP server working dir).
    pub data_dir: &'a Path,
    pub now: DateTime<Utc>,
    /// The core's items (calendar 1:1 linking reads people).
    pub items: &'a dyn ItemSource,
    /// The roles the user connected this integration for. Items may
    /// carry a narrower `roles` list; the core refuses wider ones.
    pub roles: &'a [String],
}

pub trait Connector: Send + Sync {
    /// The integration kind (`github`, `mcp`, `git`).
    fn kind(&self) -> &'static str;
    /// Checks and normalises settings; the result is what gets stored.
    fn validate(&self, settings: &Value, http: &Http) -> Result<Value, String>;
    /// Whether a sync needs a token from the keychain.
    fn needs_token(&self, settings: &Value) -> bool;
    /// The OAuth provider, when the settings ask for OAuth.
    fn oauth(&self, _settings: &Value) -> Option<OAuthProvider> {
        None
    }
    /// Checks (and normalises) a secret the user stores for these
    /// settings; the error is a code for the app (`invalid_token`,
    /// `invalid_url`).
    fn check_secret(
        &self,
        _settings: &Value,
        secret: &str,
        _http: &Http,
    ) -> Result<String, &'static str> {
        crate::secrets::check_user_token(secret)?;
        Ok(secret.to_string())
    }
    /// What the app may show of a stored secret (never the secret).
    fn secret_hint(&self, _settings: &Value, _secret: &str) -> Option<String> {
        None
    }
    /// Where a stored secret is sent: the source and the host (or program)
    /// that receives it. When a settings change moves this, the runtime
    /// deletes the stored secret, so a token meant for one service is never
    /// sent to another. Empty for kinds with one fixed destination.
    fn secret_audience(&self, _settings: &Value) -> String {
        String::new()
    }
    /// Whether a sync needs the integration's private directory.
    fn private_dir(&self, _settings: &Value) -> bool {
        false
    }
    /// Whether the runtime polls it (false: only on demand, like `git`).
    fn polls(&self) -> bool {
        true
    }
    /// One sync: the batches to upsert, in order.
    fn sync<'a>(&'a self, cx: SyncCx<'a>) -> BoxFuture<'a, Result<Vec<Batch>, SyncError>>;
    /// Forget per-integration caches.
    fn forget(&self, _id: &str) {}
}

/// Truncates to at most `n` characters.
pub(crate) fn cap(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// A string field of `v` at a `/`-separated path.
pub(crate) fn str_at<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    v.pointer(path)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// An `https` URL (as the item schemas require), or nothing.
pub(crate) fn https_url(s: Option<&str>) -> Option<String> {
    let s = s?;
    let u = url::Url::parse(s).ok()?;
    (u.scheme() == "https" && s.len() <= 2048 && u.username().is_empty() && u.password().is_none())
        .then(|| u.to_string())
}

/// A timestamp as RFC 3339 UTC seconds, from RFC 3339 text or epoch
/// seconds (Slack's `ts`).
pub(crate) fn timestamp(v: Option<&Value>) -> Option<String> {
    let v = v?;
    let dt = match v {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .map(|d| d.with_timezone(&Utc))
            .ok()
            .or_else(|| s.parse::<f64>().ok().and_then(epoch))
            .or_else(|| {
                // Jira: 2024-01-02T03:04:05.000+0000
                DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z")
                    .map(|d| d.with_timezone(&Utc))
                    .ok()
            }),
        Value::Number(n) => n.as_f64().and_then(epoch),
        _ => None,
    }?;
    Some(dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
}

fn epoch(f: f64) -> Option<DateTime<Utc>> {
    if !(0.0..1e11).contains(&f) {
        return None;
    }
    DateTime::from_timestamp(f as i64, 0)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn timestamps_and_urls() {
        assert_eq!(
            timestamp(Some(&json!("1730000000.000100"))).unwrap(),
            "2024-10-27T03:33:20Z"
        );
        assert_eq!(
            timestamp(Some(&json!("2024-01-02T03:04:05.000+0000"))).unwrap(),
            "2024-01-02T03:04:05Z"
        );
        assert!(timestamp(Some(&json!("yesterday"))).is_none());
        assert!(https_url(Some("http://example.com")).is_none());
        assert!(https_url(Some("https://u:p@example.com")).is_none());
        assert!(https_url(Some("javascript:alert(1)")).is_none());
        assert_eq!(
            https_url(Some("https://example.com/a")).unwrap(),
            "https://example.com/a"
        );
    }
}
