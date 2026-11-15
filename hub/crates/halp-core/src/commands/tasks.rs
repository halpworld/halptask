use halp_domain::{Item, ItemKind, ItemView, Status, TaskFilter, limits};
use halp_store::EntityKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{clean_tags, nullable, parse_due};
use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::state::{Done, State, entity, short};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateArgs {
    pub title: String,
    /// `task` (default) or `bullet`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    /// Parent item; root when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Position among siblings; appended when absent or past the end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// `YYYY-MM-DD` or natural language (`friday`, `tomorrow`, `+3d`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    /// Typed fields, validated against the type's schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Value>>,
    /// Initial status (tasks only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    /// Roles the item belongs to (empty or absent = every role).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

pub fn create(st: &mut State, ctx: &Ctx, a: CreateArgs) -> Result<ItemView, CmdError> {
    let kind = a.kind.unwrap_or(ItemKind::Task);
    let mut item = Item::new(kind, limits::title(&a.title)?, st.now());
    item.origin = ctx.actor.to_string();
    if let Some(n) = &a.notes {
        item.notes = limits::notes(n)?;
    }
    if let Some(t) = &a.tags {
        item.tags = clean_tags(t)?;
    }
    if let Some(d) = &a.due {
        item.due = Some(parse_due(d, st.today())?);
    }
    if let Some(s) = a.status {
        if kind == ItemKind::Bullet {
            return Err(CmdError::invalid("bullets have no status"));
        }
        item.status = s;
    }
    if let Some(r) = &a.roles {
        item.roles = st.check_roles(r)?;
    }
    st.require_live_type(a.type_id.as_deref())?;
    item.type_id = a.type_id.clone();
    item.fields = a.fields.clone().unwrap_or_default();
    st.validate_item_fields(item.type_id.as_deref(), &item.fields)?;
    if let Some(p) = &a.parent_id {
        st.outline.require(p)?;
    }
    let id = item.id.clone();
    let summary = format!("Created {} \"{}\"", kind_word(kind), short(&item.title));
    st.write(ctx, "tasks.create", &a, true, |st, _| {
        st.outline
            .insert(item, a.parent_id.as_deref(), a.index.map(|i| i as usize))?;
        Ok(Done {
            result: st.view(&id)?,
            entity: entity("item", &id),
            summary,
        })
    })
}

fn kind_word(k: ItemKind) -> &'static str {
    match k {
        ItemKind::Task => "task",
        ItemKind::Bullet => "bullet",
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdArgs {
    pub id: String,
}

pub fn get(st: &mut State, _ctx: &Ctx, a: IdArgs) -> Result<ItemView, CmdError> {
    st.view(&a.id)
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ItemsResult {
    pub items: Vec<ItemView>,
}

pub fn list(st: &mut State, _ctx: &Ctx, f: TaskFilter) -> Result<ItemsResult, CmdError> {
    Ok(ItemsResult {
        items: f.apply(&st.outline)?,
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    pub id: String,
    pub patch: ItemPatch,
    /// Optimistic concurrency: fail with `conflict` unless the item is still at this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_version: Option<u64>,
}

/// Fields to change; absent fields stay as they are.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ItemPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// `YYYY-MM-DD` or natural language; `null` clears the due date.
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub due: Option<Option<String>>,
    /// `null` makes the item untyped (its fields are then cleared).
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub type_id: Option<Option<String>>,
    /// Replaces all fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Value>>,
    /// Own status of a leaf task. A parent task's status is derived from its subtasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    /// Roles the item belongs to (empty = every role).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

/// Item equality ignoring the bookkeeping fields.
fn same(a: &Item, b: &Item) -> bool {
    let mut b = b.clone();
    b.version = a.version;
    b.updated_at = a.updated_at;
    *a == b
}

pub fn update(st: &mut State, ctx: &Ctx, a: UpdateArgs) -> Result<ItemView, CmdError> {
    let cur = st.outline.require(&a.id)?.clone();
    if let Some(v) = a.if_version
        && v != cur.version
    {
        return Err(CmdError::conflict(format!(
            "item {} is at version {}, not {v}",
            a.id, cur.version
        )));
    }
    let p = &a.patch;
    let mut next = cur.clone();
    if let Some(r) = &p.roles {
        next.roles = st.check_roles(r)?;
    }
    if let Some(t) = &p.title {
        next.title = limits::title(t)?;
    }
    if let Some(n) = &p.notes {
        next.notes = limits::notes(n)?;
    }
    if let Some(t) = &p.tags {
        next.tags = clean_tags(t)?;
    }
    if let Some(d) = &p.due {
        next.due = match d {
            Some(s) => Some(parse_due(s, st.today())?),
            None => None,
        };
    }
    if let Some(t) = &p.type_id {
        // An item may keep a retired type (and be edited), never move to one.
        if *t != cur.type_id {
            st.require_live_type(t.as_deref())?;
        }
        next.type_id = t.clone();
        if t.is_none() && p.fields.is_none() {
            next.fields = Map::new();
        }
    }
    if let Some(f) = &p.fields {
        next.fields = f.clone();
    }
    if let Some(k) = p.kind {
        next.kind = k;
    }
    if let Some(s) = p.status {
        if next.kind == ItemKind::Bullet {
            return Err(CmdError::invalid("bullets have no status"));
        }
        if st.outline.has_subtasks(&a.id) {
            return Err(CmdError::invalid(
                "a parent task's status is derived from its subtasks; complete it with tasks.complete and cascade=true",
            ));
        }
        next.status = s;
    }
    if next.kind == ItemKind::Bullet {
        next.status = Status::Todo;
    }
    st.validate_item_fields(next.type_id.as_deref(), &next.fields)?;
    if same(&cur, &next) {
        return st.view(&a.id);
    }
    let id = a.id.clone();
    let summary = format!("Updated \"{}\"", short(&next.title));
    st.write(ctx, "tasks.update", &a, true, |st, _| {
        let now = st.now();
        st.outline.update(&id, now, |it| *it = next)?;
        Ok(Done {
            result: st.view(&id)?,
            entity: entity("item", &id),
            summary,
        })
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    pub id: String,
    /// New parent: an id, `null` for the root level, absent to stay under the current parent.
    #[serde(
        default,
        deserialize_with = "nullable",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_id: Option<Option<String>>,
    /// Position among the new siblings; appended when absent or past the end.
    #[serde(default, alias = "order", skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

pub fn moved(st: &mut State, ctx: &Ctx, a: MoveArgs) -> Result<ItemView, CmdError> {
    let cur = st.outline.require(&a.id)?.clone();
    let parent = match &a.parent_id {
        Some(p) => p.clone(),
        None => cur.parent_id.clone(),
    };
    let id = a.id.clone();
    let summary = format!("Moved \"{}\"", short(&cur.title));
    st.write(ctx, "tasks.move", &a, true, |st, _| {
        let now = st.now();
        st.outline
            .move_item(&id, parent.as_deref(), a.index.map(|i| i as usize), now)?;
        Ok(Done {
            result: st.view(&id)?,
            entity: entity("item", &id),
            summary,
        })
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteArgs {
    pub id: String,
    /// `false` reopens the task (back to `todo`). Default `true`.
    #[serde(default = "yes")]
    pub done: bool,
    /// Apply to every task in the subtree. Required for parents and bullets.
    #[serde(default)]
    pub cascade: bool,
}

fn yes() -> bool {
    true
}

pub fn complete(st: &mut State, ctx: &Ctx, a: CompleteArgs) -> Result<ItemView, CmdError> {
    let cur = st.outline.require(&a.id)?.clone();
    let group = cur.kind == ItemKind::Bullet || st.outline.has_subtasks(&a.id);
    if group && !a.cascade {
        return Err(CmdError::invalid(
            "this item has subtasks or is a bullet; pass cascade=true to change the whole group",
        ));
    }
    let target = if a.done { Status::Done } else { Status::Todo };
    let verb = if a.done { "Completed" } else { "Reopened" };
    let id = a.id.clone();
    let summary = if group {
        format!("{verb} \"{}\" and its subtasks", short(&cur.title))
    } else {
        format!("{verb} \"{}\"", short(&cur.title))
    };
    st.write(ctx, "tasks.complete", &a, true, |st, _| {
        let now = st.now();
        if a.cascade {
            st.outline.set_subtree_status(&id, target, now)?;
        } else if cur.status != target {
            st.outline.update(&id, now, |it| it.status = target)?;
        }
        Ok(Done {
            result: st.view(&id)?,
            entity: entity("item", &id),
            summary,
        })
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeleteResult {
    /// Every removed id, the item first, then its subtree in pre-order.
    pub deleted: Vec<String>,
}

pub fn delete(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<DeleteResult, CmdError> {
    let cur = st.outline.require(&a.id)?.clone();
    let n = st.outline.subtree_ids(&a.id).len();
    let id = a.id.clone();
    let summary = if n > 1 {
        format!(
            "Deleted \"{}\" and {} nested items",
            short(&cur.title),
            n - 1
        )
    } else {
        format!("Deleted \"{}\"", short(&cur.title))
    };
    st.write(ctx, "tasks.delete", &a, true, |st, tx| {
        let deleted = st.outline.remove_subtree(&id)?;
        let stale: Vec<String> = st
            .focus
            .iter()
            .filter(|(_, f)| deleted.contains(f))
            .map(|(r, _)| r.clone())
            .collect();
        for r in stale {
            st.set_focus(tx, &r, None);
        }
        for d in &deleted {
            if let Some(before) = st.store.get(EntityKind::Legacy, d)? {
                tx.record(EntityKind::Legacy, d, Some(before), None);
            }
        }
        Ok(Done {
            result: DeleteResult { deleted },
            entity: entity("item", &id),
            summary,
        })
    })
}
