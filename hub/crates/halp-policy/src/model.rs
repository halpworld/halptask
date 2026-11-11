//! What the engine decides over: paired clients, their grants and standing
//! rules. They are stored by the core (entities `client` and `rule`) and
//! loaded into the engine whenever they change.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Data classes a command's input or output may belong to. A client needs
/// a grant for a class to call commands of it. `general` is granted at
/// pairing; the others are off until the user grants them.
pub const DATA_CLASSES: &[&str] = &["general", "notes.private", "email.body", "secrets", "audit"];

/// The class every command belongs to unless it says otherwise.
pub const DEFAULT_DATA_CLASS: &str = "general";

pub fn is_data_class(s: &str) -> bool {
    DATA_CLASSES.contains(&s)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Grants {
    /// Reversible local writes (T1) run without asking. On at pairing.
    pub t1: bool,
    /// Data classes this client may touch (sorted, de-duplicated).
    pub data_classes: Vec<String>,
}

impl Default for Grants {
    fn default() -> Self {
        Grants {
            t1: true,
            data_classes: vec![DEFAULT_DATA_CLASS.to_string()],
        }
    }
}

impl Grants {
    pub fn allows_class(&self, class: &str) -> bool {
        self.data_classes.iter().any(|c| c == class)
    }
}

/// A paired MCP client. The token itself is never stored, only its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Client {
    /// `[A-Za-z0-9_.-]{1,64}`; the actor is `agent:<id>`.
    pub id: String,
    pub name: String,
    pub token_hash: String,
    pub created_at: String,
    #[serde(default)]
    pub revoked_at: Option<String>,
    #[serde(default)]
    pub grants: Grants,
}

impl Client {
    pub fn active(&self) -> bool {
        self.revoked_at.is_none()
    }
}

/// A standing rule: lets one client run one T2 command without asking when
/// every listed argument equals the given value. Only `user` creates them.
/// Rules never apply to T3, nor while the client's session is tainted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub id: String,
    pub client_id: String,
    pub command: String,
    /// Top-level argument name to the exact value it must have. At least one.
    pub args: BTreeMap<String, Value>,
    pub created_at: String,
    pub created_by: String,
}

impl Rule {
    pub fn matches(&self, client_id: &str, command: &str, args: &Value) -> bool {
        self.client_id == client_id
            && self.command == command
            && !self.args.is_empty()
            && self.args.iter().all(|(k, v)| args.get(k) == Some(v))
    }
}
