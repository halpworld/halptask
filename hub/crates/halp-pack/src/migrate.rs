//! Declarative item migrations (§3.7): pure functions over an item's
//! `fields` object. No code runs; the ops are data.

use halp_domain::{TaskType, limits};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{PackError, Result};
use crate::manifest::{Manifest, req, unique_map};

/// A migration of one of the pack's own types, for items written under a
/// pack version matching `from`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Migration {
    #[serde(rename = "type")]
    pub type_id: String,
    /// A semver requirement on the old (installed) pack version.
    pub from: String,
    pub ops: Vec<MigrationOp>,
}

/// One op. An unknown `op` fails to parse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum MigrationOp {
    /// Moves `from` to `to`. Absent `from` is a no-op; refused when both
    /// are present (it would overwrite a value).
    Rename { from: String, to: String },
    /// Sets `field` only when it is absent.
    Default { field: String, value: Value },
    /// Removes `field`.
    Drop { field: String },
    /// Replaces a string value of `field` found in `values` (`old: new`).
    /// Other values are left as they are.
    Map {
        field: String,
        #[serde(deserialize_with = "unique_map")]
        values: std::collections::BTreeMap<String, Value>,
    },
    /// Moves the item to another type of the same pack.
    Retype { to: String },
}

impl MigrationOp {
    /// Checks names and values (the pack-level checks, such as retype
    /// staying in the pack, are in [`Manifest::validate`]).
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Rename { from, to } => {
                limits::simple_id("migration field", from)?;
                limits::simple_id("migration field", to)?;
                if from == to {
                    return Err(PackError::invalid("rename needs two different fields"));
                }
            }
            Self::Default { field, value } => {
                limits::simple_id("migration field", field)?;
                limits::json_bounds(
                    "default value",
                    value,
                    limits::MAX_FIELDS_BYTES,
                    limits::MAX_FIELDS_DEPTH,
                )?;
            }
            Self::Drop { field } => limits::simple_id("migration field", field)?,
            Self::Map { field, values } => {
                limits::simple_id("migration field", field)?;
                if values.is_empty() || values.len() > 256 {
                    return Err(PackError::invalid("map needs 1 to 256 values"));
                }
                for v in values.values() {
                    limits::json_bounds(
                        "map value",
                        v,
                        limits::MAX_FIELDS_BYTES,
                        limits::MAX_FIELDS_DEPTH,
                    )?;
                }
            }
            Self::Retype { to } => limits::simple_id("retype target", to)?,
        }
        Ok(())
    }
}

/// The schema key a type's data class is declared under.
const DATA_CLASS_KEY: &str = "x-halp-data-class";

/// A type's declared data class (`None` is the default class, general).
fn class_of(t: &TaskType) -> Option<&str> {
    t.schema.get(DATA_CLASS_KEY).and_then(Value::as_str)
}

/// A retype may not move items into a type of another data class: a
/// private item retyped to a general type would become readable by every
/// agent (CONTRACT v1.10). Every `retype` target must declare exactly the
/// data class of the migration's type. Called by [`Manifest::validate`];
/// the Hub checks the same against the registry when it migrates.
pub fn check_retype_classes(m: &Manifest) -> Result<()> {
    let by_id = |id: &str| m.task_types.iter().find(|t| t.id == id);
    for mg in &m.migrations {
        let Some(from) = by_id(&mg.type_id) else {
            continue;
        };
        for op in &mg.ops {
            let MigrationOp::Retype { to } = op else {
                continue;
            };
            let Some(target) = by_id(to) else { continue };
            if class_of(from) != class_of(target) {
                return Err(PackError::invalid(format!(
                    "pack {}: retype from {} to {to} would change the items' data class from {} to {}",
                    m.id,
                    mg.type_id,
                    class_of(from).unwrap_or("general"),
                    class_of(target).unwrap_or("general")
                )));
            }
        }
    }
    Ok(())
}

/// An item's fields and type after migration.
#[derive(Debug, Clone, PartialEq)]
pub struct Migrated {
    pub type_id: String,
    pub fields: Map<String, Value>,
    /// Whether anything differs from the input.
    pub changed: bool,
}

/// Applies `ops` in order to an item of `type_id`. Pure: the input is not
/// touched.
pub fn apply_ops(
    type_id: &str,
    fields: &Map<String, Value>,
    ops: &[MigrationOp],
) -> Result<Migrated> {
    let mut out = fields.clone();
    let mut ty = type_id.to_string();
    for op in ops {
        op.validate()?;
        match op {
            MigrationOp::Rename { from, to } => {
                if let Some(v) = out.remove(from) {
                    if out.contains_key(to) {
                        return Err(PackError::invalid(format!(
                            "rename {from} -> {to}: the item already has {to}"
                        )));
                    }
                    out.insert(to.clone(), v);
                }
            }
            MigrationOp::Default { field, value } => {
                out.entry(field.clone()).or_insert_with(|| value.clone());
            }
            MigrationOp::Drop { field } => {
                out.remove(field);
            }
            MigrationOp::Map { field, values } => {
                if let Some(Value::String(s)) = out.get(field)
                    && let Some(new) = values.get(s)
                {
                    out.insert(field.clone(), new.clone());
                }
            }
            MigrationOp::Retype { to } => ty = to.clone(),
        }
    }
    let changed = out != *fields || ty != type_id;
    Ok(Migrated {
        type_id: ty,
        fields: out,
        changed,
    })
}

/// The migrations that apply to items of `type_id` when updating from
/// pack version `old`, in manifest order.
pub fn migrations_for<'a>(
    manifest: &'a Manifest,
    type_id: &str,
    old: &semver::Version,
) -> Result<Vec<&'a Migration>> {
    let mut out = Vec::new();
    for m in &manifest.migrations {
        if m.type_id == type_id && req(&m.from, "migration from")?.matches(old) {
            out.push(m);
        }
    }
    Ok(out)
}

/// Migrates one item of `type_id` written under pack version `old` to
/// the new manifest: every matching migration, in order. The caller
/// validates the result against the new schema and refuses the update if
/// any item fails (§3.7).
pub fn migrate_item(
    manifest: &Manifest,
    old: &semver::Version,
    type_id: &str,
    fields: &Map<String, Value>,
) -> Result<Migrated> {
    let mut cur = Migrated {
        type_id: type_id.to_string(),
        fields: fields.clone(),
        changed: false,
    };
    for m in migrations_for(manifest, type_id, old)? {
        let next = apply_ops(&cur.type_id, &cur.fields, &m.ops)?;
        cur = Migrated {
            changed: cur.changed || next.changed,
            ..next
        };
    }
    Ok(cur)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    fn ops(v: Value) -> Vec<MigrationOp> {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn rename() {
        let f = obj(json!({"a": 1, "b": 2}));
        let r = apply_ops(
            "acme.t",
            &f,
            &ops(json!([{"op": "rename", "from": "a", "to": "c"}])),
        )
        .unwrap();
        assert_eq!(r.fields, obj(json!({"c": 1, "b": 2})));
        assert!(r.changed);
        // The input is untouched (pure).
        assert_eq!(f, obj(json!({"a": 1, "b": 2})));
        // Absent source: no-op.
        let r = apply_ops(
            "acme.t",
            &f,
            &ops(json!([{"op": "rename", "from": "z", "to": "c"}])),
        )
        .unwrap();
        assert!(!r.changed);
        // Would overwrite.
        assert!(
            apply_ops(
                "acme.t",
                &f,
                &ops(json!([{"op": "rename", "from": "a", "to": "b"}]))
            )
            .is_err()
        );
        assert!(
            apply_ops(
                "acme.t",
                &f,
                &ops(json!([{"op": "rename", "from": "a", "to": "a"}]))
            )
            .is_err()
        );
    }

    #[test]
    fn default_drop_map_retype() {
        let f = obj(json!({"state": "wip", "old": true}));
        let r = apply_ops(
            "acme.t",
            &f,
            &ops(json!([
                {"op": "default", "field": "prio", "value": "normal"},
                {"op": "default", "field": "state", "value": "ignored"},
                {"op": "drop", "field": "old"},
                {"op": "drop", "field": "absent"},
                {"op": "map", "field": "state", "values": {"wip": "in_progress", "x": "y"}},
                {"op": "retype", "to": "acme.u"}
            ])),
        )
        .unwrap();
        assert_eq!(
            r.fields,
            obj(json!({"state": "in_progress", "prio": "normal"}))
        );
        assert_eq!(r.type_id, "acme.u");
        // map leaves non-matching and non-string values alone.
        let f = obj(json!({"state": 3}));
        let r = apply_ops(
            "acme.t",
            &f,
            &ops(json!([{"op": "map", "field": "state", "values": {"3": "x"}}])),
        )
        .unwrap();
        assert!(!r.changed);
    }

    fn class_manifest(target_class: Option<&str>) -> Manifest {
        let mut open = json!({"type": "object", "properties": {}, "additionalProperties": false});
        if let Some(c) = target_class {
            open["x-halp-data-class"] = json!(c);
        }
        serde_json::from_value(json!({
            "format": 1, "id": "acme.sre", "version": "1.1.0", "title": "A",
            "description": "", "license": "MIT", "publisher": {"name": "Acme"},
            "min_hub": "0.x", "role": {"id": "acme-sre", "title": "SRE", "accent": "#112233"},
            "task_types": [
                {"id": "acme.secret", "title": "S", "schema": {
                    "type": "object", "x-halp-data-class": "acme.private",
                    "properties": {}, "additionalProperties": false}},
                {"id": "acme.open", "title": "O", "schema": open}
            ],
            "migrations": [{"type": "acme.secret", "from": "*", "ops": [{"op": "retype", "to": "acme.open"}]}]
        }))
        .unwrap()
    }

    /// A retype into a type of another data class (general or a different
    /// private class) is refused; the same class is fine.
    #[test]
    fn retype_must_keep_the_data_class() {
        for bad in [None, Some("acme.other"), Some("personal")] {
            let e = check_retype_classes(&class_manifest(bad)).unwrap_err();
            assert!(e.message.contains("data class"), "{e}");
        }
        check_retype_classes(&class_manifest(Some("acme.private"))).unwrap();
    }

    #[test]
    fn unknown_or_malformed_ops_rejected() {
        for bad in [
            json!([{"op": "eval", "code": "rm -rf"}]),
            json!([{"op": "rename", "from": "a"}]),
            json!([{"op": "drop", "field": "a", "extra": 1}]),
            json!([{"field": "a"}]),
            json!([{"op": "map", "field": "a", "values": []}]),
        ] {
            assert!(
                serde_json::from_value::<Vec<MigrationOp>>(bad.clone()).is_err(),
                "{bad}"
            );
        }
        // Duplicate map keys.
        assert!(
            serde_json::from_str::<MigrationOp>(
                r#"{"op":"map","field":"a","values":{"x":1,"x":2}}"#
            )
            .is_err()
        );
        // Bad field names are caught when applied too.
        let f = Map::new();
        assert!(
            apply_ops(
                "acme.t",
                &f,
                &[MigrationOp::Drop {
                    field: "a b".into()
                }]
            )
            .is_err()
        );
        assert!(
            apply_ops(
                "acme.t",
                &f,
                &[MigrationOp::Map {
                    field: "a".into(),
                    values: Default::default()
                }]
            )
            .is_err()
        );
    }
}
