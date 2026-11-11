use chrono::{DateTime, NaiveDate, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// New ULID string identifier.
pub fn new_id() -> String {
    ulid::Ulid::generate().to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Bullet,
    Task,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Todo,
    InProgress,
    Done,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Todo => "todo",
            Status::InProgress => "in_progress",
            Status::Done => "done",
        }
    }
}

/// A node of the outline as stored. `status` is the item's *own* status; for
/// a task with subtasks the status shown to clients is derived (see
/// [`crate::Outline::effective_status`]). Bullets carry `todo` and ignore it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Item {
    pub id: String,
    pub parent_id: Option<String>,
    /// Position among siblings, 0-based and contiguous.
    pub order: u32,
    pub kind: ItemKind,
    pub status: Status,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Due date, `YYYY-MM-DD`.
    pub due: Option<NaiveDate>,
    pub type_id: Option<String>,
    #[serde(default)]
    pub fields: Map<String, Value>,
    /// Roles the item belongs to; empty = every role. Children without their
    /// own roles inherit the nearest ancestor's.
    #[serde(default)]
    pub roles: Vec<String>,
    /// Who wrote the content: `user`, `agent:<id>`, `module:<id>` or
    /// `integration:<id>`. Items stored before this field existed are `user`.
    /// Content whose origin is not the reader's own is untrusted (taint).
    #[serde(default = "user_origin")]
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
}

/// Default for [`Item::origin`].
pub fn user_origin() -> String {
    "user".to_string()
}

impl Item {
    /// A fresh, unattached item. Placement (`parent_id`, `order`) is set when
    /// it is inserted into an [`crate::Outline`].
    pub fn new(kind: ItemKind, title: String, now: DateTime<Utc>) -> Self {
        Item {
            id: new_id(),
            parent_id: None,
            order: 0,
            kind,
            status: Status::Todo,
            title,
            notes: String::new(),
            tags: Vec::new(),
            due: None,
            type_id: None,
            fields: Map::new(),
            roles: Vec::new(),
            origin: user_origin(),
            created_at: now,
            updated_at: now,
            version: 1,
        }
    }

    pub fn is_task(&self) -> bool {
        self.kind == ItemKind::Task
    }
}

/// The item as clients see it: contract fields, `status` derived for parent
/// tasks, plus `inherited_tags` from ancestors (an additive extension).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemView {
    pub id: String,
    pub parent_id: Option<String>,
    pub order: u32,
    pub kind: ItemKind,
    pub status: Status,
    pub title: String,
    pub notes: String,
    pub tags: Vec<String>,
    /// Tags inherited from ancestors, excluding the item's own tags.
    pub inherited_tags: Vec<String>,
    pub due: Option<NaiveDate>,
    pub type_id: Option<String>,
    pub fields: Map<String, Value>,
    pub roles: Vec<String>,
    /// See [`Item::origin`].
    #[serde(default = "user_origin")]
    pub origin: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub version: u64,
}
