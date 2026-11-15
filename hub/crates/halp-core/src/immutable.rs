//! Schema-driven immutability (`x-halp-immutable-when`, CONTRACT v1.9).
//!
//! A built-in or pack task type may declare
//!
//! ```json
//! "x-halp-immutable-when": { "status": ["approved"] }
//! ```
//!
//! An item of that type is *locked* while every named field holds one of
//! its listed values (all conditions must match). The rules, checked on
//! every item change a write or an undo/redo would commit:
//!
//! * **Agents, modules and integrations** may not change a locked item in
//!   any way (content, type, parent, delete), and may not put an item into
//!   the locked state (create it locked, or set the condition field).
//!   Only sibling renumbering (`order` moving because a neighbour was
//!   inserted or removed) passes; `tasks.move` of a locked item itself is
//!   refused before it runs.
//! * **The user** may put an item into the locked state, move it and delete
//!   it (T3, confirmed as always). While it is locked the only content
//!   change allowed is appending to its array fields (for `cso.evidence`,
//!   `approvals`): existing elements stay as they are. The user may **not**
//!   leave the locked state (no un-approve), so fields such as `sha256`
//!   never change once the item is approved. A mistaken approval is
//!   answered by appending a rejecting approval, or by deleting the item
//!   and creating a new one.
//!
//! Undo and redo run the same check on the changes they would apply, so
//! nobody can step an item out of (or, for non-users, into) the locked
//! state. The annotation counts only on built-in and pack types: a type
//! from `types.define` cannot lock items (nor unlock them by redefining).

use std::collections::BTreeMap;

use halp_domain::{DomainError, TaskType};
use halp_store::{Change, EntityKind};
use serde_json::{Map, Value};

use crate::envelope::{CmdError, ErrorCode};
use crate::policy::Actor;
use crate::state::State;

/// The schema key.
pub const IMMUTABLE_KEY: &str = "x-halp-immutable-when";
/// Most conditions per type, and values per condition.
const MAX_CONDITIONS: usize = 8;
const MAX_VALUES: usize = 16;

/// Item keys that may change on a locked item for anyone: bookkeeping and
/// the position among siblings (renumbered when a neighbour is inserted or
/// removed).
const BOOKKEEPING: &[&str] = &["order", "updated_at", "version"];

/// Rejects a malformed `x-halp-immutable-when` on a pack type: it must be a
/// non-empty object of at most 8 field names, each mapped to a non-empty
/// array of at most 16 scalar values, and each name must be a property of
/// the schema.
pub(crate) fn check_type(t: &TaskType) -> Result<(), DomainError> {
    let Some(v) = t.schema.get(IMMUTABLE_KEY) else {
        return Ok(());
    };
    let bad = |why: &str| DomainError::invalid(format!("type {}: {IMMUTABLE_KEY} {why}", t.id));
    let m = v
        .as_object()
        .ok_or_else(|| bad("must be an object of field: [values]"))?;
    if m.is_empty() || m.len() > MAX_CONDITIONS {
        return Err(bad("must name 1 to 8 fields"));
    }
    let props = t.schema.get("properties").and_then(Value::as_object);
    for (k, vals) in m {
        if !props.is_some_and(|p| p.contains_key(k)) {
            return Err(bad(&format!("names {k}, which is not a property")));
        }
        let vals = vals
            .as_array()
            .ok_or_else(|| bad("values must be arrays"))?;
        if vals.is_empty() || vals.len() > MAX_VALUES {
            return Err(bad("must list 1 to 16 values per field"));
        }
        if vals.iter().any(|x| x.is_array() || x.is_object()) {
            return Err(bad("values must be strings, numbers, booleans or null"));
        }
    }
    Ok(())
}

/// The lock conditions of every built-in and pack type that has them, as a
/// snapshot (so it can be used while the store is borrowed).
#[derive(Debug, Clone, Default)]
pub(crate) struct Locks {
    by_type: BTreeMap<String, Vec<(String, Vec<Value>)>>,
}

impl Locks {
    pub(crate) fn of(st: &State) -> Locks {
        let mut by_type = BTreeMap::new();
        for (t, origin) in st.types.list() {
            // Only types whose source may carry privileged annotations
            // (built-ins, trusted packs, and types retired from them).
            if !origin.may_annotate() {
                continue;
            }
            let Some(m) = t.schema.get(IMMUTABLE_KEY).and_then(Value::as_object) else {
                continue;
            };
            let conds: Vec<(String, Vec<Value>)> = m
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_array()?.clone())))
                .collect();
            if !conds.is_empty() {
                by_type.insert(t.id.clone(), conds);
            }
        }
        Locks { by_type }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.by_type.is_empty()
    }

    fn conds(&self, type_id: Option<&str>) -> Option<&Vec<(String, Vec<Value>)>> {
        self.by_type.get(type_id?)
    }

    /// Whether an item with this type and these fields is locked.
    pub(crate) fn fields_locked(&self, type_id: Option<&str>, fields: &Map<String, Value>) -> bool {
        self.conds(type_id).is_some_and(|conds| {
            conds
                .iter()
                .all(|(k, vals)| fields.get(k).is_some_and(|v| vals.contains(v)))
        })
    }

    /// Whether a stored item image (JSON) is locked.
    pub(crate) fn locked(&self, image: &Value) -> bool {
        let type_id = image.get("type_id").and_then(Value::as_str);
        let empty = Map::new();
        let fields = image
            .get("fields")
            .and_then(Value::as_object)
            .unwrap_or(&empty);
        self.fields_locked(type_id, fields)
    }

    /// "status is approved" for an item image's type.
    fn condition_text(&self, image: &Value) -> String {
        let Some(conds) = self.conds(image.get("type_id").and_then(Value::as_str)) else {
            return "locked".into();
        };
        conds
            .iter()
            .map(|(k, vals)| {
                let vs: Vec<String> = vals
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .collect();
                format!("{k} is {}", vs.join(" or "))
            })
            .collect::<Vec<_>>()
            .join(" and ")
    }

    /// Checks every item change against the rules for `actor`. The error is
    /// `denied` with a message that names the item id and type, never its
    /// text.
    pub(crate) fn check(&self, actor: &Actor, changes: &[Change]) -> Result<(), CmdError> {
        if self.is_empty() {
            return Ok(());
        }
        let user = matches!(actor, Actor::User | Actor::System);
        for c in changes.iter().filter(|c| c.kind == EntityKind::Item) {
            let was = c.before.as_ref().is_some_and(|v| self.locked(v));
            let now = c.after.as_ref().is_some_and(|v| self.locked(v));
            if !was && !now {
                continue;
            }
            let image = c
                .before
                .as_ref()
                .or(c.after.as_ref())
                .unwrap_or(&Value::Null);
            let what = describe(&c.id, image);
            let cond = self.condition_text(image);
            match (&c.before, &c.after, was, now) {
                // Entering the locked state (created locked, or the
                // condition field set): the user's decision only.
                (_, Some(_), false, true) => {
                    if !user {
                        return Err(denied(format!(
                            "{what} would become immutable ({cond}); only the user can do that"
                        )));
                    }
                }
                // Leaving it: nobody, the user included.
                (Some(_), Some(_), true, false) => {
                    return Err(denied(format!(
                        "{what} is immutable while {cond}; it cannot be changed back (append a new approval, or delete it and create a new one)"
                    )));
                }
                // Deleted while locked: the user only.
                (Some(_), None, true, _) if !user => {
                    return Err(denied(format!(
                        "{what} is immutable while {cond}; only the user can delete it"
                    )));
                }
                // Changed while locked.
                (Some(b), Some(a), true, true) if !allowed_while_locked(b, a, user) => {
                    return Err(denied(if user {
                        format!(
                            "{what} is immutable while {cond}; you can only add entries to its lists (such as approvals)"
                        )
                    } else {
                        format!(
                            "{what} is immutable while {cond}; agents and integrations cannot change, move or delete it"
                        )
                    }));
                }
                _ => {}
            }
        }
        Ok(())
    }
}

fn denied(m: String) -> CmdError {
    CmdError::new(ErrorCode::Denied, m)
}

fn describe(id: &str, image: &Value) -> String {
    match image.get("type_id").and_then(Value::as_str) {
        Some(t) => format!("item {id} ({t})"),
        None => format!("item {id}"),
    }
}

/// A change to a locked item that keeps it locked: only bookkeeping (and,
/// for the user, the parent) may differ, plus appends to array fields for
/// the user.
fn allowed_while_locked(before: &Value, after: &Value, user: bool) -> bool {
    let (Some(b), Some(a)) = (before.as_object(), after.as_object()) else {
        return false;
    };
    let skip = |k: &str| BOOKKEEPING.contains(&k) || (user && k == "parent_id");
    let keys: std::collections::BTreeSet<&String> = b.keys().chain(a.keys()).collect();
    for k in keys {
        if skip(k) {
            continue;
        }
        let (bv, av) = (b.get(k.as_str()), a.get(k.as_str()));
        if bv == av {
            continue;
        }
        if k != "fields" || !user {
            return false;
        }
        let empty = Map::new();
        let bf = bv.and_then(Value::as_object).unwrap_or(&empty);
        let af = av.and_then(Value::as_object).unwrap_or(&empty);
        if !fields_append_only(bf, af) {
            return false;
        }
    }
    true
}

/// Every field is unchanged, except arrays that only grew at the end (an
/// absent array counts as empty).
fn fields_append_only(before: &Map<String, Value>, after: &Map<String, Value>) -> bool {
    let keys: std::collections::BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    keys.into_iter().all(|k| {
        let (b, a) = (before.get(k), after.get(k));
        if b == a {
            return true;
        }
        match (b, a) {
            (None, Some(Value::Array(_))) => true,
            (Some(Value::Array(old)), Some(Value::Array(new))) => {
                new.len() > old.len() && new[..old.len()] == old[..]
            }
            _ => false,
        }
    })
}

/// Refuses, before a command runs, what would only fail later: an agent's
/// `tasks.move` of a locked item (a reorder among siblings changes nothing
/// but `order`, which the change check lets through for renumbering), and
/// a non-user `tasks.delete` of a subtree holding a locked item (so it is
/// not parked for an approval that could only fail).
pub(crate) fn precheck(
    st: &State,
    actor: &Actor,
    command: &str,
    args: &Value,
) -> Result<(), CmdError> {
    if matches!(actor, Actor::User | Actor::System) {
        return Ok(());
    }
    let locks = Locks::of(st);
    if locks.is_empty() {
        return Ok(());
    }
    if let "history.undo" | "history.redo" = command {
        // The arguments are pinned to one event by now.
        let event = args.get("event_id").and_then(Value::as_str);
        if event.is_some()
            && let Ok(Some(plan)) =
                st.store
                    .plan_step(command == "history.undo", event, &actor.to_string())
        {
            locks.check(actor, &plan.changes)?;
        }
        return Ok(());
    }
    let Some(id) = args.get("id").and_then(Value::as_str) else {
        return Ok(());
    };
    let ids = match command {
        "tasks.move" | "tasks.update" | "tasks.complete" => vec![id.to_string()],
        "tasks.delete" => st.outline.subtree_ids(id),
        _ => return Ok(()),
    };
    for i in ids {
        let Some(it) = st.outline.get(&i) else {
            continue;
        };
        if locks.fields_locked(it.type_id.as_deref(), &it.fields) {
            let image = serde_json::to_value(it)?;
            return Err(denied(format!(
                "{} is immutable while {}; agents and integrations cannot change, move or delete it",
                describe(&i, &image),
                locks.condition_text(&image)
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn append_only_fields() {
        let b = json!({"a": [1, 2], "s": "x"});
        let ok = json!({"a": [1, 2, 3], "s": "x"});
        let bad1 = json!({"a": [1, 3, 3], "s": "x"});
        let bad2 = json!({"a": [1, 2, 3], "s": "y"});
        let bad3 = json!({"a": [1], "s": "x"});
        let m = |v: &Value| v.as_object().unwrap().clone();
        assert!(fields_append_only(&m(&b), &m(&ok)));
        assert!(!fields_append_only(&m(&b), &m(&bad1)));
        assert!(!fields_append_only(&m(&b), &m(&bad2)));
        assert!(!fields_append_only(&m(&b), &m(&bad3)));
        assert!(fields_append_only(
            &m(&json!({"s": 1})),
            &m(&json!({"s": 1, "n": [1]}))
        ));
        assert!(!fields_append_only(
            &m(&json!({"s": 1})),
            &m(&json!({"s": 1, "n": 2}))
        ));
    }
}
