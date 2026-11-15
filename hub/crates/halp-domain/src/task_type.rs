//! Typed tasks: a registry of task types whose `fields` documents are
//! validated against a JSON Schema (draft 2020-12).
//!
//! Schemas are untrusted (they come from packs, modules and agents), so they
//! are size and depth bounded, compiled without any remote or file `$ref`
//! resolution, and `pattern` keywords use the linear-time `regex` engine.

use std::collections::BTreeMap;
use std::sync::{Arc, LazyLock};

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::error::{DomainError, Result};
use crate::limits;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskTypeDisplay {
    #[serde(default)]
    pub primary_fields: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge_field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskType {
    /// Namespaced id such as `dev.pull_request`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub icon: String,
    /// JSON Schema (2020-12) for the item's `fields`.
    pub schema: Value,
    #[serde(default)]
    pub display: TaskTypeDisplay,
}

/// How far a pack is trusted (mirrors `halp_pack::PackTrust`; the domain
/// crate sits below halp-pack, so the core converts between the two).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PackTrust {
    Builtin,
    FirstParty,
    Community,
    Unsigned,
    Dev,
}

impl PackTrust {
    /// Whether types from a pack of this trust may carry the privileged
    /// annotations (`x-halp-immutable-when`, `x-halp-ext-writable`).
    pub fn may_annotate(self) -> bool {
        matches!(self, Self::Builtin | Self::FirstParty | Self::Community)
    }
}

/// Where a type definition came from. Only user types can be redefined.
///
/// Serialized internally tagged: `{"origin": "pack", "trust": "community",
/// "pack": "acme.sales"}`, so a view can flatten it next to the type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum TaskTypeOrigin {
    Builtin,
    /// Defined by a role pack (a built-in or an installed one).
    Pack {
        trust: PackTrust,
        /// The package id (`halp.developer`, `acme.sales`).
        pack: String,
    },
    User,
    /// The pack that defined it was uninstalled or disabled, or failed to
    /// load: existing items keep their protections, new items cannot use
    /// it.
    Retired {
        /// The trust of the pack that defined it.
        trust: PackTrust,
        pack: String,
    },
}

impl TaskTypeOrigin {
    /// Whether this type's privileged annotations are honoured. A retired
    /// type keeps what its pack was allowed (a retired dev or unsigned
    /// type gains nothing by being retired).
    pub fn may_annotate(&self) -> bool {
        match self {
            Self::Builtin => true,
            Self::Pack { trust, .. } | Self::Retired { trust, .. } => trust.may_annotate(),
            Self::User => false,
        }
    }

    pub fn is_user(&self) -> bool {
        matches!(self, Self::User)
    }

    pub fn is_retired(&self) -> bool {
        matches!(self, Self::Retired { .. })
    }

    /// `builtin`, `pack`, `user` or `retired`.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Pack { .. } => "pack",
            Self::User => "user",
            Self::Retired { .. } => "retired",
        }
    }
}

static TYPE_ID: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9_-]{0,31}(\.[a-z][a-z0-9_-]{0,31}){1,3}$").ok());

fn compile(schema: &Value) -> Result<jsonschema::Validator> {
    if !schema.is_object() {
        return Err(DomainError::invalid("schema must be a JSON object"));
    }
    limits::json_bounds(
        "schema",
        schema,
        limits::MAX_SCHEMA_BYTES,
        limits::MAX_SCHEMA_DEPTH,
    )?;
    if let Some(s) = schema.get("$schema")
        && s != "https://json-schema.org/draft/2020-12/schema"
    {
        return Err(DomainError::invalid(
            "schema must use JSON Schema draft 2020-12",
        ));
    }
    if schema.get("type").is_some_and(|t| t != "object") {
        return Err(DomainError::invalid(
            "schema must describe an object (type: object)",
        ));
    }
    jsonschema::draft202012::options()
        .should_validate_formats(true)
        .with_pattern_options(jsonschema::PatternOptions::regex())
        .build(schema)
        .map_err(|e| DomainError::invalid(format!("invalid schema: {e}")))
}

impl TaskType {
    /// Validates the definition itself (id, labels, schema).
    pub fn validate(&self) -> Result<()> {
        let id_ok = TYPE_ID.as_ref().is_some_and(|r| r.is_match(&self.id));
        if !id_ok {
            return Err(DomainError::invalid(format!(
                "task type id {:?} must look like `namespace.name` (lowercase, digits, _ or -)",
                self.id
            )));
        }
        let title = limits::label("type title", &self.title, 80)?;
        if title.is_empty() {
            return Err(DomainError::invalid("type title must not be empty"));
        }
        limits::label("type icon", &self.icon, 32)?;
        if self.display.primary_fields.len() > 8 {
            return Err(DomainError::invalid("at most 8 primary_fields"));
        }
        for f in self
            .display
            .primary_fields
            .iter()
            .chain(self.display.badge_field.iter())
        {
            limits::simple_id("display field", f)?;
        }
        compile(&self.schema).map(|_| ())
    }
}

#[derive(Clone)]
struct Entry {
    def: TaskType,
    origin: TaskTypeOrigin,
    validator: Arc<jsonschema::Validator>,
}

/// All known task types with compiled validators.
#[derive(Default, Clone)]
pub struct TaskTypeRegistry {
    types: BTreeMap<String, Entry>,
}

impl std::fmt::Debug for TaskTypeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.types.keys()).finish()
    }
}

/// Built-in base types available in every role.
pub fn builtin_types() -> Vec<TaskType> {
    vec![
        TaskType {
            id: "base.task".into(),
            title: "Task".into(),
            icon: "check".into(),
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "priority": { "enum": ["low", "normal", "high", "urgent"] },
                    "estimate_minutes": { "type": "integer", "minimum": 0, "maximum": 100000 },
                    "url": { "type": "string", "format": "uri", "maxLength": 2048 }
                },
                "additionalProperties": false
            }),
            display: TaskTypeDisplay {
                primary_fields: vec!["priority".into()],
                badge_field: Some("priority".into()),
            },
        },
        TaskType {
            id: "base.note".into(),
            title: "Note".into(),
            icon: "note".into(),
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "additionalProperties": false
            }),
            display: TaskTypeDisplay::default(),
        },
        TaskType {
            id: "base.link".into(),
            title: "Link".into(),
            icon: "link".into(),
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "url": { "type": "string", "format": "uri", "maxLength": 2048 }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
            display: TaskTypeDisplay {
                primary_fields: vec!["url".into()],
                badge_field: None,
            },
        },
        // An issue in an external tracker (Linear, Jira, GitHub issues),
        // synced by an integration. Pack-independent: any role can show it.
        TaskType {
            id: "ext.issue".into(),
            title: "Issue".into(),
            icon: "ticket".into(),
            schema: json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "source": { "enum": ["linear", "jira", "github", "other"] },
                    "key": { "type": "string", "minLength": 1, "maxLength": 100 },
                    "url": { "type": "string", "format": "uri", "maxLength": 2048 },
                    "state": { "type": "string", "maxLength": 100 },
                    "priority": { "type": "string", "maxLength": 50 },
                    "project": { "type": "string", "maxLength": 200 },
                    "assignee": { "type": "string", "maxLength": 200 },
                    "updated_at": { "type": "string", "format": "date-time", "maxLength": 40 }
                },
                "required": ["source", "key"],
                "additionalProperties": false
            }),
            display: TaskTypeDisplay {
                primary_fields: vec!["key".into(), "state".into()],
                badge_field: Some("state".into()),
            },
        },
    ]
}

impl TaskTypeRegistry {
    /// Registry preloaded with the built-in types.
    pub fn with_builtins() -> Result<Self> {
        let mut r = Self::default();
        for t in builtin_types() {
            r.insert(t, TaskTypeOrigin::Builtin)?;
        }
        Ok(r)
    }

    /// Adds or replaces a type. Built-in and pack types cannot be replaced
    /// by a different origin, and user types cannot shadow them.
    pub fn insert(&mut self, def: TaskType, origin: TaskTypeOrigin) -> Result<()> {
        def.validate()?;
        if let Some(existing) = self.types.get(&def.id)
            && !(existing.origin.is_user() && origin.is_user())
        {
            return Err(DomainError::conflict(format!(
                "task type {} is already defined by a {} and cannot be redefined",
                def.id,
                match existing.origin {
                    TaskTypeOrigin::Builtin => "built-in",
                    TaskTypeOrigin::Pack { .. } => "role pack",
                    TaskTypeOrigin::User => "user",
                    TaskTypeOrigin::Retired { .. } => "retired pack",
                }
            )));
        }
        let validator = Arc::new(compile(&def.schema)?);
        self.types.insert(
            def.id.clone(),
            Entry {
                def,
                origin,
                validator,
            },
        );
        Ok(())
    }

    /// Removes a user type (used when undoing a definition).
    pub fn remove_user(&mut self, id: &str) {
        if self.types.get(id).is_some_and(|e| e.origin.is_user()) {
            self.types.remove(id);
        }
    }

    pub fn get(&self, id: &str) -> Option<(&TaskType, &TaskTypeOrigin)> {
        self.types.get(id).map(|e| (&e.def, &e.origin))
    }

    pub fn list(&self) -> Vec<(&TaskType, &TaskTypeOrigin)> {
        self.types.values().map(|e| (&e.def, &e.origin)).collect()
    }

    /// Validates `fields` for an item of type `type_id` (`None` = untyped,
    /// which only allows an empty `fields` object).
    pub fn validate_fields(
        &self,
        type_id: Option<&str>,
        fields: &Map<String, Value>,
    ) -> Result<()> {
        let value = Value::Object(fields.clone());
        limits::json_bounds(
            "fields",
            &value,
            limits::MAX_FIELDS_BYTES,
            limits::MAX_FIELDS_DEPTH,
        )?;
        limits::check_json_text("fields", &value)?;
        let Some(type_id) = type_id else {
            if fields.is_empty() {
                return Ok(());
            }
            return Err(DomainError::invalid("fields require a type_id"));
        };
        let entry = self
            .types
            .get(type_id)
            .ok_or_else(|| DomainError::invalid(format!("unknown task type {type_id}")))?;
        let errors: Vec<String> = entry
            .validator
            .iter_errors(&value)
            .take(5)
            .map(|e| {
                let path = e.instance_path().to_string();
                if path.is_empty() {
                    e.to_string()
                } else {
                    format!("{path}: {e}")
                }
            })
            .collect();
        if errors.is_empty() {
            Ok(())
        } else {
            Err(DomainError::invalid(format!(
                "fields do not match {type_id}: {}",
                errors.join("; ")
            )))
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn builtins_validate_fields() {
        let r = TaskTypeRegistry::with_builtins().unwrap();
        assert!(
            r.validate_fields(Some("base.task"), &obj(json!({"priority": "high"})))
                .is_ok()
        );
        assert!(
            r.validate_fields(Some("base.task"), &obj(json!({"priority": "meh"})))
                .is_err()
        );
        assert!(
            r.validate_fields(Some("base.task"), &obj(json!({"extra": 1})))
                .is_err()
        );
        assert!(
            r.validate_fields(Some("base.task"), &obj(json!({"url": "not a uri"})))
                .is_err()
        );
        assert!(
            r.validate_fields(Some("base.link"), &obj(json!({})))
                .is_err()
        );
        assert!(
            r.validate_fields(Some("base.link"), &obj(json!({"url": "https://x.dev"})))
                .is_ok()
        );
        assert!(r.validate_fields(None, &Map::new()).is_ok());
        assert!(r.validate_fields(None, &obj(json!({"a": 1}))).is_err());
        assert!(r.validate_fields(Some("nope.type"), &Map::new()).is_err());
    }

    #[test]
    fn user_types_and_shadowing() {
        let mut r = TaskTypeRegistry::with_builtins().unwrap();
        let t = TaskType {
            id: "dev.pull_request".into(),
            title: "Pull request".into(),
            icon: "git-pull-request".into(),
            schema: json!({
                "type": "object",
                "properties": { "repo": {"type": "string", "pattern": "^[a-z0-9-]+/[a-z0-9-]+$"}, "number": {"type": "integer"} },
                "required": ["repo", "number"]
            }),
            display: TaskTypeDisplay::default(),
        };
        r.insert(t.clone(), TaskTypeOrigin::User).unwrap();
        r.insert(t.clone(), TaskTypeOrigin::User).unwrap(); // redefine OK
        assert!(
            r.validate_fields(
                Some("dev.pull_request"),
                &obj(json!({"repo": "acme/api", "number": 7}))
            )
            .is_ok()
        );
        let err = r
            .validate_fields(
                Some("dev.pull_request"),
                &obj(json!({"repo": "Bad Repo", "number": "x"})),
            )
            .unwrap_err();
        assert!(
            err.message.contains("/number") || err.message.contains("/repo"),
            "{err}"
        );
        // cannot shadow a built-in
        let mut b = t.clone();
        b.id = "base.task".into();
        assert!(r.insert(b, TaskTypeOrigin::User).is_err());
    }

    #[test]
    fn hostile_schemas_rejected() {
        let mk = |id: &str, schema: Value| TaskType {
            id: id.into(),
            title: "T".into(),
            icon: String::new(),
            schema,
            display: TaskTypeDisplay::default(),
        };
        // bad ids
        for id in ["Dev.x", "dev", "dev..x", "dev.x y", "1dev.x"] {
            assert!(
                mk(id, json!({"type": "object"})).validate().is_err(),
                "{id}"
            );
        }
        // remote refs cannot be resolved (no network/file retrieval)
        assert!(
            mk("a.b", json!({"$ref": "https://example.com/schema.json"}))
                .validate()
                .is_err()
        );
        assert!(
            mk("a.b", json!({"$ref": "file:///etc/passwd"}))
                .validate()
                .is_err()
        );
        // wrong draft, non-object, invalid keyword values
        assert!(
            mk(
                "a.b",
                json!({"$schema": "http://json-schema.org/draft-04/schema#"})
            )
            .validate()
            .is_err()
        );
        assert!(mk("a.b", json!({"type": "string"})).validate().is_err());
        assert!(
            mk("a.b", json!({"type": "object", "minProperties": -1}))
                .validate()
                .is_err()
        );
        assert!(mk("a.b", json!([1, 2])).validate().is_err());
        // backtracking-only regex features are refused by the linear engine
        assert!(
            mk(
                "a.b",
                json!({"type": "object", "properties": {"x": {"pattern": "(a+)+\\1"}}})
            )
            .validate()
            .is_err()
        );
        // too deep
        let mut deep = json!({"type": "object"});
        for _ in 0..40 {
            deep = json!({"type": "object", "properties": {"x": deep}});
        }
        assert!(mk("a.b", deep).validate().is_err());
    }

    #[test]
    fn oversized_fields_rejected() {
        let r = TaskTypeRegistry::with_builtins().unwrap();
        let big = obj(json!({"priority": "x".repeat(40_000)}));
        let err = r.validate_fields(Some("base.task"), &big).unwrap_err();
        assert!(err.message.contains("bytes"));
    }
}
