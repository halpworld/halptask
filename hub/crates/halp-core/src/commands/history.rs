use std::collections::{BTreeMap, BTreeSet};

use halp_domain::{EntityRef, Event, Item, Outline, TaskType, TaskTypeOrigin, TaskTypeRegistry};
use halp_store::{EntityKind, StepPlan, StoreError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::envelope::CmdError;
use crate::policy::{Actor, Ctx};
use crate::state::State;

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NoArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepArgs {
    /// Undo (or redo) this specific event instead of the latest one. Fails
    /// with `conflict` if what it touched has changed since.
    #[serde(default)]
    pub event_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StepResult {
    /// The event that was undone or redone; `null` when there was nothing to do.
    pub event: Option<Event>,
    /// The `history.undo` / `history.redo` event just recorded.
    pub history_event: Option<Event>,
}

fn step(
    st: &mut State,
    ctx: &Ctx,
    undo: bool,
    target: Option<&str>,
) -> Result<StepResult, CmdError> {
    let actor = ctx.actor.to_string();
    let now = st.now();
    if let Some(id) = target {
        halp_domain::limits::simple_id("event id", id)?;
    }
    let current: Vec<Item> = st
        .outline
        .preorder()
        .iter()
        .filter_map(|id| st.outline.get(id).cloned())
        .collect();
    // Undo and redo respect immutable items like any write (v1.9).
    let locks = crate::immutable::Locks::of(st);
    let types = st.types.clone();
    let base = st.base_types.clone();
    let out = st.store.step(undo, target, &actor, now, &mut |plan| {
        check_post_state(&current, plan)?;
        check_post_fields(&current, &types, base.as_ref(), plan)?;
        locks
            .check(&ctx.actor, &plan.changes)
            .map_err(|e| StoreError::Denied(e.message))
    });
    let out = match out {
        Ok(o) => o,
        // Nothing on the stack is not an error for a plain undo/redo.
        Err(halp_store::StoreError::NotFound(_)) if target.is_none() => {
            return Ok(StepResult {
                event: None,
                history_event: None,
            });
        }
        Err(e) => {
            st.reload_or_fail()?;
            return Err(e.into());
        }
    };
    st.reload_or_fail()?;
    st.emit(out.event.clone());
    Ok(StepResult {
        event: Some(out.original),
        history_event: Some(out.event),
    })
}

/// The outline after `plan` must still load: every parent present, no
/// cycles, depth and item count in bounds. Otherwise the step is a
/// `conflict` and nothing is written (for example undoing the creation of
/// an item that has since gained children).
fn check_post_state(current: &[Item], plan: &StepPlan) -> Result<(), StoreError> {
    let mut items: BTreeMap<String, Item> =
        current.iter().map(|i| (i.id.clone(), i.clone())).collect();
    for c in plan.changes.iter().filter(|c| c.kind == EntityKind::Item) {
        match &c.after {
            Some(v) => {
                let item: Item = serde_json::from_value(v.clone())?;
                items.insert(c.id.clone(), item);
            }
            None => {
                items.remove(&c.id);
            }
        }
    }
    Outline::from_items(items.into_values().collect())
        .map(|_| ())
        .map_err(|e| {
            StoreError::Conflict(format!(
                "\"{}\" cannot be {} without breaking the outline ({}); change the items involved first",
                plan.original.summary,
                if plan.changes.is_empty() { "stepped" } else { "reverted" },
                e.message
            ))
        })
}

/// No item may be left with fields its type rejects: an item the step
/// writes, or any item of a task type the step redefines or removes, must
/// validate against the registry the step would leave (as every write
/// does, dev-mode rule included). Otherwise the step is a `conflict`.
fn check_post_fields(
    current: &[Item],
    types: &TaskTypeRegistry,
    base: Option<&TaskTypeRegistry>,
    plan: &StepPlan,
) -> Result<(), StoreError> {
    let mut types = types.clone();
    let mut changed_types: BTreeSet<String> = BTreeSet::new();
    for c in plan
        .changes
        .iter()
        .filter(|c| c.kind == EntityKind::TaskType)
    {
        changed_types.insert(c.id.clone());
        types.remove_user(&c.id);
        if let Some(v) = &c.after {
            let def: TaskType = serde_json::from_value(v.clone())?;
            types
                .insert(def, TaskTypeOrigin::User)
                .map_err(|e| StoreError::Conflict(e.to_string()))?;
        }
    }
    let mut items: BTreeMap<String, Item> =
        current.iter().map(|i| (i.id.clone(), i.clone())).collect();
    let mut touched: BTreeSet<String> = BTreeSet::new();
    for c in plan.changes.iter().filter(|c| c.kind == EntityKind::Item) {
        match &c.after {
            Some(v) => {
                items.insert(c.id.clone(), serde_json::from_value(v.clone())?);
                touched.insert(c.id.clone());
            }
            None => {
                items.remove(&c.id);
            }
        }
    }
    let bad: Vec<String> = items
        .values()
        .filter(|it| {
            touched.contains(&it.id)
                || it
                    .type_id
                    .as_ref()
                    .is_some_and(|t| changed_types.contains(t))
        })
        .filter_map(|it| {
            crate::state::validate_item_fields_in(&types, base, it.type_id.as_deref(), &it.fields)
                .err()
                // Ids only: the validator's text quotes field values, and
                // the caller may not be allowed to see this item.
                .map(|_| it.id.clone())
        })
        .take(5)
        .collect();
    if bad.is_empty() {
        return Ok(());
    }
    Err(StoreError::Conflict(format!(
        "\"{}\" cannot be {} because items would no longer match their task type ({}); change the items involved first",
        plan.original.summary,
        if plan.changes.is_empty() {
            "stepped"
        } else {
            "reverted"
        },
        bad.join(", ")
    )))
}

pub fn undo(st: &mut State, ctx: &Ctx, a: StepArgs) -> Result<StepResult, CmdError> {
    step(st, ctx, true, a.event_id.as_deref())
}

pub fn redo(st: &mut State, ctx: &Ctx, a: StepArgs) -> Result<StepResult, CmdError> {
    step(st, ctx, false, a.event_id.as_deref())
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {
    /// Default 50, max 500.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Only events older than this event id (for paging).
    #[serde(default)]
    pub before: Option<String>,
    /// Only events whose actor starts with this, e.g. `agent:`.
    #[serde(default)]
    pub actor_prefix: Option<String>,
    /// Only events newer than this event id, oldest first (a change feed).
    /// Cannot be combined with `before`.
    #[serde(default)]
    pub after: Option<String>,
}

/// An event as listed: the contract event plus log metadata.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct HistoryEntry {
    pub id: String,
    pub ts: String,
    pub actor: String,
    pub command: String,
    pub entity: EntityRef,
    pub summary: String,
    pub seq: i64,
    /// For `history.undo` / `history.redo` events: the event they reverted or re-applied.
    pub causation_id: Option<String>,
    pub undoable: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListResult {
    pub events: Vec<HistoryEntry>,
    pub can_undo: bool,
    pub can_redo: bool,
}

pub fn list(st: &mut State, ctx: &Ctx, a: ListArgs) -> Result<ListResult, CmdError> {
    let limit = match a.limit {
        None => 50,
        Some(0) => return Err(CmdError::invalid("limit must be at least 1")),
        Some(n) => (n as usize).min(500),
    };
    if let Some(b) = &a.before {
        halp_domain::limits::simple_id("event id", b)?;
    }
    if a.actor_prefix
        .as_ref()
        .is_some_and(|p| p.chars().count() > 80)
    {
        return Err(CmdError::invalid("actor_prefix is too long"));
    }
    let records = match &a.after {
        Some(after) => {
            if a.before.is_some() {
                return Err(CmdError::invalid("use either before or after, not both"));
            }
            halp_domain::limits::simple_id("event id", after)?;
            let mut r = st.store.events_since(Some(after), limit)?;
            if let Some(p) = &a.actor_prefix {
                r.retain(|e| e.event.actor.starts_with(p.as_str()));
            }
            r
        }
        None => st
            .store
            .list_events(limit, a.before.as_deref(), a.actor_prefix.as_deref())?,
    };
    // Summaries are redacted when written, but an item can become private
    // later (moved under a private note, or its type's class changed). For
    // anyone but the user, redact again with the item's class now
    // (review S6a). A deleted item has no class left to check, and it may
    // have been private when it went, so its events are redacted too.
    let trusted = matches!(ctx.actor, Actor::User | Actor::System);
    let now_private = |e: &EntityRef| {
        !trusted
            && e.kind == "item"
            && st
                .item_class(&e.id)
                .is_none_or(|c| c != crate::policy::DEFAULT_DATA_CLASS)
    };
    let events = records
        .into_iter()
        .map(|r| HistoryEntry {
            summary: if now_private(&r.event.entity) && !r.event.summary.ends_with("a private item")
            {
                crate::state::private_summary(&r.event.summary)
            } else if !trusted && r.event.command == "focus.set" {
                // Its entity is the role, so the item it names cannot be
                // checked here; agents read the focus with `focus.get`.
                "Set focus".to_string()
            } else if !trusted && r.event.command.starts_with("consent.") {
                // Consent summaries quote the request (an item title) as it
                // was when parked; the item may be private now. Agents read
                // requests with `consent.get`/`consent.list`, which check.
                let verb = r.event.summary.split(':').next().unwrap_or("Asked");
                format!("{verb}: a request")
            } else {
                r.event.summary
            },
            id: r.event.id,
            ts: r.event.ts,
            actor: r.event.actor,
            command: r.event.command,
            entity: r.event.entity,
            seq: r.seq,
            causation_id: r.causation_id,
            undoable: r.undoable,
        })
        .collect();
    let (u, r) = st.store.history_depth()?;
    Ok(ListResult {
        events,
        can_undo: u > 0,
        can_redo: r > 0,
    })
}

/// Setting holding the event retention in days (applied when the core opens).
pub(crate) const SETTING_RETENTION: &str = "history.retention_days";

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PurgeArgs {
    /// Redact events older than this many days (1-36500).
    #[serde(default)]
    pub older_than_days: Option<u32>,
    /// Redact every event about an item that no longer exists.
    #[serde(default)]
    pub deleted_items: bool,
    /// Also keep `older_than_days` as the retention applied at every start
    /// (`0` clears a stored retention).
    #[serde(default)]
    pub remember: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PurgeResult {
    /// Events whose payload, inverse and summary were redacted.
    pub redacted: usize,
    pub retention_days: Option<u32>,
}

pub(crate) fn cutoff(st: &State, days: u32) -> Result<String, CmdError> {
    if !(1..=36_500).contains(&days) {
        return Err(CmdError::invalid("older_than_days must be 1-36500"));
    }
    let t = st.now() - chrono::Duration::days(i64::from(days));
    Ok(t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
}

/// `history.purge`: the event log keeps every change (with before/after
/// state) until purged; this redacts old or deleted-item events in place.
/// Redacted events leave the undo/redo stacks. User only, T3.
pub fn purge(st: &mut State, ctx: &Ctx, a: PurgeArgs) -> Result<PurgeResult, CmdError> {
    let days = match a.older_than_days {
        Some(0) if a.remember => None,
        Some(d) => Some(d),
        None => None,
    };
    let older = days.map(|d| cutoff(st, d)).transpose()?;
    let n = st.store.purge_events(older.as_deref(), a.deleted_items)?;
    let mut retention = st
        .store
        .get(EntityKind::Setting, SETTING_RETENTION)?
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok());
    if a.remember {
        let before = retention.map(serde_json::Value::from);
        retention = days;
        let after = days.map(serde_json::Value::from);
        st.write(ctx, "history.purge", &a, false, |_, tx| {
            tx.record(EntityKind::Setting, SETTING_RETENTION, before, after);
            Ok(crate::state::Done {
                result: (),
                entity: crate::state::entity("setting", SETTING_RETENTION),
                summary: match days {
                    Some(d) => format!("Event retention set to {d} days"),
                    None => "Event retention cleared".into(),
                },
            })
        })?;
    }
    st.audit(
        &ctx.actor.to_string(),
        "history.purge",
        crate::state::entity("history", "events"),
        format!("Redacted {n} events"),
        serde_json::json!({"older_than_days": a.older_than_days, "deleted_items": a.deleted_items}),
    )?;
    Ok(PurgeResult {
        redacted: n,
        retention_days: retention,
    })
}
