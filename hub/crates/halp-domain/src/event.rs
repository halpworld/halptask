use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The entity an event is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EntityRef {
    pub kind: String,
    pub id: String,
}

/// A change notification, as defined by the command contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Event {
    pub id: String,
    /// RFC 3339 UTC timestamp.
    pub ts: String,
    /// `user`, `agent:<client>`, `module:<id>`, `integration:<id>` or `system`.
    pub actor: String,
    pub command: String,
    pub entity: EntityRef,
    pub summary: String,
}
