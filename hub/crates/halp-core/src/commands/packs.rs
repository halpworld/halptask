//! `packs.*` (P5, docs/hub/P5-EXTENSIONS.md §3.4-§3.8, §7): install,
//! update, roll back and uninstall extension packs, trusted publisher
//! keys, and dev mode.
//!
//! Every write is the user's (`user_only`, never over MCP) and is recorded
//! as one non-undoable event. A write first loads the would-be pack set
//! (verifying every bundle again) and refuses when it would stop another
//! pack from loading; the types a change retires are kept in
//! `retired_types` with their last definition.

use std::collections::{BTreeMap, BTreeSet};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use halp_domain::PackTrust as DomainTrust;
use halp_domain::task_type::TaskTypeRegistry;
use halp_domain::{TaskType, TaskTypeOrigin};
use halp_pack::{self as hp, Manifest, PackErrorKind, PackTrust, SignerKind, VerifiedBundle};
use halp_store::EntityKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::history::NoArgs;
use crate::envelope::{CmdError, ErrorCode};
use crate::installed::{
    self, DevState, KeyRecord, LoadedPack, MAX_PREVIEWS, MigratedItem, PREVIEW_TTL_MINUTES,
    PackCache, PackRecord, PackVersion, Preview, Status, domain_trust, fp_hex,
};
use crate::policy::Ctx;
use crate::state::{Done, State, entity};

fn denied(m: impl Into<String>) -> CmdError {
    CmdError::new(ErrorCode::Denied, m)
}

fn now_str(st: &State) -> String {
    st.now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

// ---- views ----------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PackStatus {
    Ok,
    Disabled,
    Error,
    Blocked,
}

impl From<Status> for PackStatus {
    fn from(s: Status) -> Self {
        match s {
            Status::Ok => PackStatus::Ok,
            Status::Disabled => PackStatus::Disabled,
            Status::Error => PackStatus::Error,
            Status::Blocked => PackStatus::Blocked,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PackCapabilities {
    pub modules: usize,
    pub ui_widgets: usize,
    pub prompts: usize,
    pub annotations: usize,
}

/// A pack as `packs.list` shows it. `description`, `publisher`,
/// `previous_version` and `error` are for the user only. Every string that
/// comes from a pack is untrusted text.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PackView {
    pub id: String,
    pub version: String,
    pub title: String,
    pub role_id: String,
    pub trust: DomainTrust,
    pub enabled: bool,
    /// Ids of the packs this one extends, nearest first.
    pub extends: Vec<String>,
    /// Full fingerprint of the pinned key, or null.
    pub key_fp: Option<String>,
    pub capabilities: PackCapabilities,
    pub status: PackStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub publisher: Option<String>,
    /// The version `packs.rollback` returns to (user only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_version: Option<Option<String>>,
    /// Why the pack is not loaded (user only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn view(m: &LoadedPack, user: bool) -> PackView {
    let man = m.manifest.as_deref();
    let mut extends: Vec<String> = m.chain.iter().rev().skip(1).cloned().collect();
    if extends.is_empty()
        && let Some(e) = man.and_then(|m| m.extends.as_ref())
    {
        extends.push(e.id.clone());
    }
    let prompts = if m.prompts.is_empty() {
        man.map_or(0, |m| m.prompts.add.len())
    } else {
        m.prompts.len()
    };
    let builtin = m.trust == PackTrust::Builtin;
    PackView {
        id: m.id.clone(),
        version: m.version.clone(),
        title: m.title.clone(),
        role_id: m.role_id.clone(),
        trust: domain_trust(m.trust),
        enabled: m.enabled,
        extends,
        key_fp: m.key_fp.clone(),
        capabilities: PackCapabilities {
            modules: man.map_or(0, |m| m.modules.len()),
            ui_widgets: man.map_or(0, |m| m.ui_widgets.len()),
            prompts,
            annotations: m.annotations,
        },
        status: m.status.into(),
        description: user.then(|| man.map(|m| m.description.clone()).unwrap_or_default()),
        publisher: user.then(|| {
            if builtin {
                "HalpTask".to_string()
            } else {
                man.map(|m| m.publisher.name.clone()).unwrap_or_default()
            }
        }),
        previous_version: user.then(|| m.previous_version.clone()),
        error: if user {
            m.error.as_ref().map(|e| e.message.clone())
        } else {
            None
        },
    }
}

fn loaded<'a>(st: &'a State, id: &str) -> Result<&'a LoadedPack, CmdError> {
    st.pack_cache
        .meta
        .get(id)
        .ok_or_else(|| CmdError::not_found(format!("pack {id} not found")))
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListResult {
    pub packs: Vec<PackView>,
}

/// `packs.list`: built-in, installed and dev packs.
pub fn list(st: &mut State, ctx: &Ctx, _a: NoArgs) -> Result<ListResult, CmdError> {
    let user = ctx.actor.is_user();
    let mut packs: Vec<&LoadedPack> = st.pack_cache.meta.values().collect();
    packs.sort_by_key(|m| (m.trust != PackTrust::Builtin, m.is_dev(), m.id.clone()));
    Ok(ListResult {
        packs: packs.into_iter().map(|m| view(m, user)).collect(),
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IdArgs {
    pub id: String,
}

pub fn get(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<PackView, CmdError> {
    Ok(view(loaded(st, &a.id)?, ctx.actor.is_user()))
}

// ---- the generic change ---------------------------------------------------

/// An item an install, update or rollback rewrites.
#[derive(Debug, Clone)]
struct ItemEdit {
    id: String,
    type_id: Option<String>,
    fields: Map<String, Value>,
    before_type: Option<String>,
    before_fields: Map<String, Value>,
}

/// A change to the pack set.
struct Change {
    cmd: &'static str,
    entity_kind: &'static str,
    entity_id: String,
    summary: String,
    records: Vec<PackRecord>,
    keys: Vec<KeyRecord>,
    dev: Option<DevState>,
    /// Bundles to store (by sha).
    extra: BTreeMap<String, Vec<u8>>,
    /// Packs this change may stop loading.
    targets: BTreeSet<String>,
    /// Any pack may stop loading (a key revocation).
    allow_breakage: bool,
    /// This pack must load.
    require_ok: Option<String>,
    remove_layouts: Vec<String>,
    items: Vec<ItemEdit>,
    /// The pack whose record lists `items` as migrated (an update).
    migrated_for: Option<String>,
}

impl Change {
    fn new(
        st: &mut State,
        cmd: &'static str,
        entity_kind: &'static str,
        entity_id: &str,
        summary: String,
    ) -> Self {
        let (records, keys, _) = st.pack_rows();
        Change {
            cmd,
            entity_kind,
            entity_id: entity_id.to_string(),
            summary,
            records,
            keys,
            dev: st.dev.clone(),
            extra: BTreeMap::new(),
            targets: BTreeSet::new(),
            allow_breakage: false,
            require_ok: None,
            remove_layouts: Vec::new(),
            items: Vec::new(),
            migrated_for: None,
        }
    }

    fn record_mut(&mut self, id: &str) -> Result<&mut PackRecord, CmdError> {
        self.records
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or_else(|| CmdError::not_found(format!("pack {id} not found")))
    }
}

/// What a candidate pack set would load as, against the stored retired
/// types plus the ones the change would retire (so its registry is the
/// one the items will be checked against after the change).
fn candidate(st: &mut State, c: &Change) -> PackCache {
    let (_, _, mut table) = st.pack_rows();
    let first = st.load_packs_with(&c.records, &c.keys, &table, c.dev.as_ref(), &c.extra);
    let (add, remove) = installed::retire_diff(&st.pack_cache, &first, &table, &now_str(st));
    if add.is_empty() {
        return first;
    }
    table.retain(|r| !remove.contains(&r.id) && !add.iter().any(|a| a.id == r.id));
    table.extend(add);
    st.load_packs_with(&c.records, &c.keys, &table, c.dev.as_ref(), &c.extra)
}

/// The refusals a candidate earns: the required pack does not load, or
/// another pack would stop loading.
fn check_candidate(st: &State, c: &Change, cand: &PackCache) -> Result<(), CmdError> {
    if let Some(id) = &c.require_ok {
        let m = cand
            .meta
            .get(id)
            .ok_or_else(|| CmdError::internal(format!("pack {id} vanished from its own change")))?;
        if m.status != Status::Ok && m.status != Status::Disabled {
            return Err(m
                .error
                .clone()
                .unwrap_or_else(|| CmdError::conflict(format!("pack {id} would not load"))));
        }
    }
    if c.allow_breakage {
        return Ok(());
    }
    for (id, m) in &st.pack_cache.meta {
        if !m.is_live() || c.targets.contains(id) {
            continue;
        }
        match cand.meta.get(id) {
            Some(n) if n.is_live() => {}
            Some(n) => {
                let why = n.error.as_ref().map(|e| e.message.as_str()).unwrap_or("");
                return Err(CmdError::conflict(format!(
                    "this would stop pack {id} from loading: {why}"
                )));
            }
            None if m.is_dev() && c.dev.is_none() => {}
            None => {
                return Err(CmdError::conflict(format!(
                    "this would stop pack {id} from loading"
                )));
            }
        }
    }
    Ok(())
}

fn to_value<T: Serialize>(v: &T) -> Result<Value, CmdError> {
    Ok(serde_json::to_value(v)?)
}

fn diff_rows<T: Serialize + PartialEq>(
    tx: &mut crate::state::Tx,
    kind: EntityKind,
    old: &[T],
    new: &[T],
    key: impl Fn(&T) -> &str,
) -> Result<(), CmdError> {
    let o: BTreeMap<&str, &T> = old.iter().map(|v| (key(v), v)).collect();
    let n: BTreeMap<&str, &T> = new.iter().map(|v| (key(v), v)).collect();
    for (k, v) in &n {
        match o.get(k) {
            Some(b) if *b == *v => {}
            b => tx.record(
                kind,
                k,
                b.map(|b| to_value(*b)).transpose()?,
                Some(to_value(*v)?),
            ),
        }
    }
    for (k, v) in &o {
        if !n.contains_key(k) {
            tx.record(kind, k, Some(to_value(*v)?), None);
        }
    }
    Ok(())
}

/// Commits a change as one non-undoable event, then reloads the packs.
fn apply<A: Serialize>(
    st: &mut State,
    ctx: &Ctx,
    args: &A,
    c: Change,
    cand: &PackCache,
) -> Result<(), CmdError> {
    let (old_records, old_keys, table) = st.pack_rows();
    let now = st.now();
    let (add, remove) = installed::retire_diff(&st.pack_cache, cand, &table, &now_str(st));
    for bytes in c.extra.values() {
        st.store.put_blob(bytes)?;
    }
    let Change {
        cmd,
        entity_kind,
        entity_id,
        summary,
        mut records,
        keys,
        dev,
        remove_layouts,
        items,
        migrated_for,
        ..
    } = c;
    let module = migrated_for.as_ref().map(|p| format!("module:{p}"));
    // Classify the event against the registry it leaves: a migration may
    // retype items into a type only this pack version brings, which the
    // old registry would count as unloaded (`secrets`) and so log the
    // install as a private write. A failed write reloads the old one.
    let cand_types = st.registry_for(cand)?;
    let old_types = std::mem::replace(&mut st.types, cand_types);
    let res = st.write(ctx, cmd, args, false, |st, tx| {
        for e in &items {
            st.outline.update(&e.id, now, |it| {
                it.type_id = e.type_id.clone();
                it.fields = e.fields.clone();
            })?;
        }
        if let Some(pid) = &migrated_for
            && let Some(r) = records.iter_mut().find(|r| r.id == *pid)
        {
            r.migrated = items
                .iter()
                .filter_map(|e| {
                    Some(MigratedItem {
                        id: e.id.clone(),
                        type_id: e.before_type.clone(),
                        fields: e.before_fields.clone(),
                        version: st.outline.get(&e.id)?.version,
                    })
                })
                .collect();
        }
        diff_rows(tx, EntityKind::Pack, &old_records, &records, |r| &r.id)?;
        diff_rows(tx, EntityKind::PackKey, &old_keys, &keys, |k| &k.key_id)?;
        for r in &add {
            let before = table
                .iter()
                .find(|t| t.id == r.id)
                .map(to_value)
                .transpose()?;
            tx.record(EntityKind::RetiredType, &r.id, before, Some(to_value(r)?));
        }
        for id in &remove {
            let before = table
                .iter()
                .find(|t| t.id == *id)
                .map(to_value)
                .transpose()?;
            tx.record(EntityKind::RetiredType, id, before, None);
        }
        for p in &cand.packs {
            if !st.layouts.contains_key(&p.layout.id) {
                tx.record(
                    EntityKind::Layout,
                    &p.layout.id,
                    None,
                    Some(to_value(&p.layout)?),
                );
            }
        }
        for lid in &remove_layouts {
            if let Some(l) = st.layouts.get(lid) {
                tx.record(EntityKind::Layout, lid, Some(to_value(l)?), None);
            }
        }
        if let Some(m) = &module {
            let targets = items.iter().map(|e| format!("item:{}", e.id)).collect();
            st.add_writers(m, targets, &mut tx.changes)?;
        }
        Ok(Done {
            result: (),
            entity: entity(entity_kind, &entity_id),
            summary,
        })
    });
    if let Err(e) = res {
        st.types = old_types;
        retain_blobs(st);
        return Err(e);
    }
    st.dev = dev;
    st.previews.clear();
    st.reload_packs();
    st.reload_or_fail()?;
    retain_blobs(st);
    Ok(())
}

/// Drops stored bundles no record refers to.
fn retain_blobs(st: &mut State) {
    let (records, _, _) = st.pack_rows();
    let keep: BTreeSet<String> = records
        .iter()
        .flat_map(|r| {
            std::iter::once(r.current.bundle_sha256.clone())
                .chain(r.previous.as_ref().map(|p| p.bundle_sha256.clone()))
        })
        .collect();
    if let Err(e) = st.store.retain_blobs(&keep) {
        eprintln!("halp-core: could not drop unused pack bundles: {e}");
    }
}

fn view_after(st: &State, id: &str) -> Result<PackView, CmdError> {
    Ok(view(loaded(st, id)?, true))
}

// ---- what a change does to existing items (CONTRACT v1.10) ---------------

/// A type's data class (`general` unless its schema says otherwise).
fn type_class(t: &TaskType) -> &str {
    t.schema
        .get(crate::state::DATA_CLASS_KEY)
        .and_then(Value::as_str)
        .unwrap_or(crate::policy::DEFAULT_DATA_CLASS)
}

/// A type's lock rule as it counts: `x-halp-immutable-when` from a source
/// that may carry it.
fn lock_rule<'a>(t: &'a TaskType, o: &TaskTypeOrigin) -> Option<&'a Value> {
    if o.may_annotate() {
        t.schema.get(crate::immutable::IMMUTABLE_KEY)
    } else {
        None
    }
}

/// The type rules a pack change must keep, against the registry `reg` it
/// would leave (refused with `conflict`):
///
/// * a type id that stays may not change its data class (an item would
///   move between classes, and a private item could become readable by
///   every agent);
/// * a type id with a locked item may not change or drop its lock rule
///   (it would unlock the item: locked is terminal, v1.9);
/// * a migration may not retype an item into a type of another class.
fn check_type_rules(
    st: &State,
    reg: &TaskTypeRegistry,
    edits: &[ItemEdit],
) -> Result<(), CmdError> {
    let locks = crate::immutable::Locks::of(st);
    let mut lock_changed = BTreeSet::new();
    for (old, origin) in st.types.list() {
        let Some((new, new_origin)) = reg.get(&old.id) else {
            continue;
        };
        if type_class(old) != type_class(new) {
            return Err(CmdError::conflict(format!(
                "task type {} would change its data class from {} to {}; a pack change may not move items between data classes",
                old.id,
                type_class(old),
                type_class(new)
            )));
        }
        if lock_rule(old, origin) != lock_rule(new, new_origin) {
            lock_changed.insert(old.id.clone());
        }
    }
    if !lock_changed.is_empty() {
        for id in st.outline.preorder() {
            let Some(it) = st.outline.get(&id) else {
                continue;
            };
            if let Some(t) = it.type_id.as_deref()
                && lock_changed.contains(t)
                && locks.fields_locked(Some(t), &it.fields)
            {
                return Err(CmdError::conflict(format!(
                    "task type {t} has locked items (such as {id}); this would change or remove its x-halp-immutable-when rule and unlock them"
                )));
            }
        }
    }
    for e in edits {
        // No type is general; a type that is not loaded fails closed
        // (`secrets`), as everywhere else (CONTRACT note 20).
        let class = |t: Option<&str>, r: &TaskTypeRegistry| match t {
            None => crate::policy::DEFAULT_DATA_CLASS.to_string(),
            Some(t) => r.get(t).map_or_else(
                || crate::state::UNKNOWN_TYPE_CLASS.to_string(),
                |(d, _)| type_class(d).to_string(),
            ),
        };
        let (from, to) = (
            class(e.before_type.as_deref(), &st.types),
            class(e.type_id.as_deref(), reg),
        );
        if e.type_id != e.before_type && from != to {
            return Err(CmdError::conflict(format!(
                "item {} would move from data class {from} to {to}; a migration may not retype items into another data class",
                e.id
            )));
        }
    }
    Ok(())
}

/// The items that would not validate against `reg` (with the first
/// error): every item whose type's definition changes, and every item
/// `edits` rewrites (checked as rewritten).
fn invalid_items(st: &State, reg: &TaskTypeRegistry, edits: &[ItemEdit]) -> Vec<(String, String)> {
    let changed: BTreeSet<&str> = st
        .types
        .list()
        .into_iter()
        .filter(|(old, _)| reg.get(&old.id).is_none_or(|(new, _)| new != *old))
        .map(|(old, _)| old.id.as_str())
        .collect();
    let edited: BTreeMap<&str, &ItemEdit> = edits.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut out = Vec::new();
    for id in st.outline.preorder() {
        let Some(it) = st.outline.get(&id) else {
            continue;
        };
        let (ty, fields) = match edited.get(id.as_str()) {
            Some(e) => (e.type_id.as_deref(), &e.fields),
            None => {
                if !it.type_id.as_deref().is_some_and(|t| changed.contains(t)) {
                    continue;
                }
                (it.type_id.as_deref(), &it.fields)
            }
        };
        if let Err(e) = reg.validate_fields(ty, fields) {
            out.push((id.clone(), e.message));
        }
    }
    out
}

/// Refuses a change that breaks [`check_type_rules`] or would leave items
/// that do not validate (named in the error, as an update does).
fn check_items(st: &State, reg: &TaskTypeRegistry, edits: &[ItemEdit]) -> Result<(), CmdError> {
    check_type_rules(st, reg, edits)?;
    let bad = invalid_items(st, reg, edits);
    if let Some((_, first)) = bad.first() {
        let ids: Vec<&str> = bad.iter().take(50).map(|(id, _)| id.as_str()).collect();
        return Err(CmdError::conflict(format!(
            "{} items would no longer validate ({}); nothing was changed. First: {first}",
            bad.len(),
            ids.join(", ")
        )));
    }
    Ok(())
}

// ---- install / update -----------------------------------------------------

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PermissionChange {
    /// `module`, `ui_widget_call`, `annotation`, `data_class` or `prompt`.
    pub kind: String,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PermissionDiff {
    pub added: Vec<PermissionChange>,
    pub removed: Vec<PermissionChange>,
}

fn class_of(schema: &Value) -> Option<String> {
    schema
        .get(crate::state::DATA_CLASS_KEY)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn permissions(m: &Manifest) -> Vec<PermissionChange> {
    let p = |kind: &str, id: String, detail: Option<String>| PermissionChange {
        kind: kind.into(),
        id,
        detail,
    };
    let mut out = Vec::new();
    for x in &m.modules {
        let inputs: Vec<&str> = x.inputs.values().map(|i| i.command.as_str()).collect();
        out.push(p("module", x.name.clone(), Some(inputs.join(", "))));
    }
    for w in &m.ui_widgets {
        for c in &w.calls {
            out.push(p("ui_widget_call", c.clone(), Some(w.kind(&m.id))));
        }
    }
    let mut classes = BTreeSet::new();
    for t in &m.task_types {
        for (_, a) in hp::trust::privileged_in(&t.schema) {
            out.push(p("annotation", format!("{}: {a}", t.id), None));
        }
        if let Some(c) = class_of(&t.schema) {
            classes.insert(c);
        }
    }
    for c in classes {
        out.push(p("data_class", c, None));
    }
    for path in &m.prompts.add {
        let name = hp::manifest::prompt_name(path).unwrap_or(path).to_string();
        out.push(p("prompt", name, None));
    }
    out
}

fn diff_permissions(before: &[PermissionChange], after: &[PermissionChange]) -> PermissionDiff {
    let key = |c: &PermissionChange| (c.kind.clone(), c.id.clone(), c.detail.clone());
    let b: BTreeSet<_> = before.iter().map(key).collect();
    let a: BTreeSet<_> = after.iter().map(key).collect();
    PermissionDiff {
        added: after
            .iter()
            .filter(|c| !b.contains(&key(c)))
            .cloned()
            .collect(),
        removed: before
            .iter()
            .filter(|c| !a.contains(&key(c)))
            .cloned()
            .collect(),
    }
}

/// A data class changed on a type that stays is an escalation, whichever
/// way it goes: it is listed as an added `data_class` change (`id` the
/// type, `detail` "old -> new"), and the old class is never shown as
/// removed. Such an update is refused (CONTRACT v1.10); the preview
/// still says what it would have done.
fn escalate_classes(d: &mut PermissionDiff, before: &Manifest, after: &Manifest) {
    let mut old_classes = BTreeSet::new();
    for t in &after.task_types {
        let Some(o) = before.task_types.iter().find(|o| o.id == t.id) else {
            continue;
        };
        let (oc, nc) = (type_class(o), type_class(t));
        if oc != nc {
            old_classes.insert(oc.to_string());
            d.added.push(PermissionChange {
                kind: "data_class".into(),
                id: t.id.clone(),
                detail: Some(format!("{oc} -> {nc}")),
            });
        }
    }
    d.removed
        .retain(|c| !(c.kind == "data_class" && old_classes.contains(&c.id)));
}

#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct MigrationReport {
    pub types: usize,
    pub items_checked: usize,
    pub items_migrated: usize,
    pub items_failed: usize,
    pub failed_ids: Vec<String>,
}

/// How the bundle's signature checked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SignatureStatus {
    Verified,
    /// A valid signature by a key the user has not trusted.
    UnknownKey,
    None,
}

struct Verification {
    vb: VerifiedBundle,
    signature: SignatureStatus,
    /// The key to trust when the bundle only verifies with it.
    new_key: Option<KeyRecord>,
}

/// Verifies an uploaded bundle against the trusted keys. With `want_key`,
/// a bundle signed by an unknown key is verified as if that key were
/// trusted, and the key is returned to be trusted with the install.
fn verify_upload(
    st: &State,
    bytes: &[u8],
    keys: &[KeyRecord],
    want_key: bool,
) -> Result<Verification, CmdError> {
    let community: Vec<hp::CommunityKey> = keys.iter().filter_map(KeyRecord::community).collect();
    let first = hp::verify(bytes, &community);
    let status = |vb: &VerifiedBundle| match vb.signer.as_ref().map(|s| s.kind) {
        None => SignatureStatus::None,
        Some(SignerKind::Unknown) => SignatureStatus::UnknownKey,
        Some(_) => SignatureStatus::Verified,
    };
    let unknown = match &first {
        Ok(vb) => status(vb) == SignatureStatus::UnknownKey,
        Err(e) => e.kind == PackErrorKind::Denied,
    };
    if !(want_key && unknown) {
        let vb = first?;
        return Ok(Verification {
            signature: status(&vb),
            vb,
            new_key: None,
        });
    }
    let parsed = hp::parse_bundle(bytes)?;
    let Some(sig) = parsed.signature else {
        return Err(first
            .err()
            .map(CmdError::from)
            .unwrap_or_else(|| CmdError::internal("an unsigned bundle reported an unknown key")));
    };
    let pubkey = hp::crypto::decode_pubkey(&sig.pubkey)?;
    let key_id = hp::key_id(&pubkey);
    if keys.iter().any(|k| k.key_id == key_id)
        || hp::FIRST_PARTY_KEYS.iter().any(|(_, k)| *k == pubkey)
    {
        // Known (and so revoked, or the error is about something else).
        return Err(first
            .err()
            .map(CmdError::from)
            .unwrap_or_else(|| CmdError::internal("a known key reported as unknown")));
    }
    let mut provisional = community;
    provisional.push(hp::CommunityKey {
        key_id: key_id.clone(),
        pubkey,
        revoked: false,
    });
    let vb = hp::verify(bytes, &provisional)?;
    let label: String = vb.manifest.publisher.name.chars().take(100).collect();
    Ok(Verification {
        vb,
        signature: SignatureStatus::UnknownKey,
        new_key: Some(KeyRecord {
            key_id,
            pubkey_b64: sig.pubkey.clone(),
            label: if label.trim().is_empty() {
                "publisher".into()
            } else {
                label
            },
            added_at: now_str(st),
            revoked_at: None,
        }),
    })
}

/// Everything an install of one bundle would do.
struct InstallPlan {
    verification: Verification,
    old: Option<PackRecord>,
    change: Change,
    cand: PackCache,
    report: MigrationReport,
    /// What the update would do to existing items ([`check_items`]).
    guard: Result<(), CmdError>,
}

fn semver_of(v: &str) -> Result<semver::Version, CmdError> {
    semver::Version::parse(v).map_err(|e| CmdError::invalid(format!("version {v}: {e}")))
}

/// Installed packs do not change while dev mode is on: their items are
/// checked against the registry, and with dev packs in it an item could
/// come to rely on a dev override that a restart (dev mode is never saved)
/// drops. A key revocation is never refused.
fn refuse_in_dev_mode(st: &State) -> Result<(), CmdError> {
    if st.dev.is_some() {
        return Err(CmdError::conflict(
            "turn dev mode off before changing installed packs or keys",
        ));
    }
    Ok(())
}

fn plan_install(st: &mut State, bytes: &[u8], want_key: bool) -> Result<InstallPlan, CmdError> {
    refuse_in_dev_mode(st)?;
    let (records, keys, _) = st.pack_rows();
    let verification = verify_upload(st, bytes, &keys, want_key)?;
    let vb = &verification.vb;
    let id = vb.manifest.id.clone();
    if hp::ids::BUILTIN_PACKAGE_IDS.contains(&id.as_str())
        || st
            .pack_cache
            .meta
            .get(&id)
            .is_some_and(|m| m.trust == PackTrust::Builtin)
    {
        return Err(CmdError::conflict(format!(
            "{id} is a built-in pack and cannot be replaced"
        )));
    }
    if vb.trust == PackTrust::Dev || id.starts_with(hp::ids::DEV_PREFIX) {
        return Err(CmdError::invalid("dev packs are loaded with packs.dev_set"));
    }
    let old = records.iter().find(|r| r.id == id).cloned();
    if let Some(o) = &old {
        let (new_v, old_v) = (
            semver_of(&vb.manifest.version)?,
            semver_of(&o.current.version)?,
        );
        if new_v <= old_v {
            return Err(CmdError::conflict(format!(
                "{} is not newer than the installed {}; a downgrade is refused (use packs.rollback)",
                vb.manifest.version, o.current.version
            )));
        }
        hp::check_pin(o.current.pin(), vb)?;
    }
    if !vb.manifest.supports_hub(&installed::hub_version())? {
        return Err(CmdError::conflict(format!(
            "pack {id} needs Hub {}, this is {}",
            vb.manifest.min_hub,
            installed::HUB_VERSION
        )));
    }
    let key_id = match &old {
        Some(o) if o.current.key_id.is_some() => o.current.key_id.clone(),
        _ => vb
            .pin_key_id()
            .map(str::to_string)
            .or_else(|| verification.new_key.as_ref().map(|k| k.key_id.clone())),
    };
    // The full fingerprint of the pinned key: kept from the old record,
    // or the signer's (on an update it passed `check_pin`).
    let key_fp = key_id.as_ref().and_then(|_| {
        old.as_ref()
            .and_then(|o| o.current.key_fp.clone())
            .or_else(|| vb.signer.as_ref().map(|s| fp_hex(&s.fingerprint)))
    });
    let version = PackVersion {
        version: vb.manifest.version.clone(),
        trust: vb.trust,
        key_id,
        key_fp,
        bundle_sha256: vb.bundle_sha256.clone(),
        manifest_sha256: vb.manifest_sha256.clone(),
        title: vb.manifest.title.clone(),
        role_id: vb.manifest.role.id.clone(),
        types: vb.manifest.task_types.clone(),
    };
    let summary = match &old {
        Some(o) => format!(
            "Updated pack {id} from {} to {}",
            o.current.version, vb.manifest.version
        ),
        None => format!("Installed pack {id} {}", vb.manifest.version),
    };
    let mut change = Change::new(st, "packs.install", "pack", &id, summary);
    let rec = PackRecord {
        id: id.clone(),
        enabled: old.as_ref().is_none_or(|o| o.enabled),
        installed_at: now_str(st),
        current: version,
        previous: old.as_ref().map(|o| o.current.clone()),
        migrated: Vec::new(),
    };
    change.records.retain(|r| r.id != id);
    change.records.push(rec);
    if let Some(k) = &verification.new_key {
        change.keys.push(k.clone());
    }
    change
        .extra
        .insert(vb.bundle_sha256.clone(), bytes.to_vec());
    change.targets.insert(id.clone());
    change.require_ok = Some(id.clone());
    change.migrated_for = Some(id.clone());
    let cand = candidate(st, &change);
    let reg = st.registry_for(&cand)?;
    let (report, edits) = dry_run(st, old.as_ref(), &verification.vb, &reg)?;
    let guard = check_items(st, &reg, &edits);
    change.items = edits;
    Ok(InstallPlan {
        verification,
        old,
        change,
        cand,
        report,
        guard,
    })
}

/// Migrates (on an update) and validates every item of the pack's types
/// against the would-be registry. Locked items may not change.
fn dry_run(
    st: &State,
    old: Option<&PackRecord>,
    vb: &VerifiedBundle,
    reg: &TaskTypeRegistry,
) -> Result<(MigrationReport, Vec<ItemEdit>), CmdError> {
    let mut affected: BTreeSet<String> = vb
        .manifest
        .task_types
        .iter()
        .map(|t| t.id.clone())
        .collect();
    let old_ver = match old {
        Some(o) => {
            affected.extend(o.current.types.iter().map(|t| t.id.clone()));
            affected.extend(vb.manifest.migrations.iter().map(|m| m.type_id.clone()));
            Some(semver_of(&o.current.version)?)
        }
        None => None,
    };
    let locks = crate::immutable::Locks::of(st);
    let mut report = MigrationReport::default();
    let mut migrated_types = BTreeSet::new();
    let mut edits = Vec::new();
    for id in st.outline.preorder() {
        let Some(it) = st.outline.get(&id) else {
            continue;
        };
        let Some(t) = it.type_id.as_deref() else {
            continue;
        };
        if !affected.contains(t) {
            continue;
        }
        report.items_checked += 1;
        let m = match &old_ver {
            Some(v) => hp::migrate_item(&vb.manifest, v, t, &it.fields).ok(),
            None => Some(hp::Migrated {
                type_id: t.to_string(),
                fields: it.fields.clone(),
                changed: false,
            }),
        };
        let ok = m.as_ref().is_some_and(|m| {
            reg.validate_fields(Some(&m.type_id), &m.fields).is_ok()
                && !(m.changed && locks.fields_locked(Some(t), &it.fields))
        });
        match m {
            Some(m) if ok => {
                if m.changed {
                    report.items_migrated += 1;
                    migrated_types.insert(t.to_string());
                    edits.push(ItemEdit {
                        id: id.clone(),
                        type_id: Some(m.type_id),
                        fields: m.fields,
                        before_type: it.type_id.clone(),
                        before_fields: it.fields.clone(),
                    });
                }
            }
            _ => {
                report.items_failed += 1;
                report.failed_ids.push(id.clone());
            }
        }
    }
    report.types = migrated_types.len();
    report.failed_ids.truncate(50);
    Ok((report, edits))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewArgs {
    /// The `.halppack` file, base64.
    pub bundle_base64: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewRole {
    pub id: String,
    pub title: String,
    pub accent: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewType {
    pub id: String,
    pub title: String,
    pub data_class: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewAnnotation {
    pub type_id: String,
    pub annotation: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewModule {
    pub id: String,
    pub title: String,
    pub inputs: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewWidget {
    pub kind: String,
    pub title: String,
    pub calls: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewPrompt {
    pub name: String,
    pub description: String,
}

/// A type override as the preview shows it. Every string is untrusted
/// text from the pack.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreviewOverride {
    pub type_id: String,
    pub title: Option<String>,
    pub icon: Option<String>,
    /// Property name to the description it sets.
    pub property_descriptions: BTreeMap<String, String>,
    pub added_properties: Vec<String>,
    /// Property name to the enum values it adds.
    pub added_enum_values: BTreeMap<String, Vec<Value>>,
}

/// What installing a bundle would do (`packs.preview`). Every string from
/// the pack is untrusted text.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PackPreview {
    pub preview_id: String,
    pub bundle_sha256: String,
    pub expires_at: String,
    pub id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub publisher: String,
    /// The trust it installs with (`community` for an unknown key that
    /// `trust_key` would trust).
    pub trust: DomainTrust,
    pub signature: SignatureStatus,
    pub key_fp: Option<String>,
    pub installed_version: Option<String>,
    pub files: Vec<PreviewFile>,
    /// Ids of the packs it extends, nearest first.
    pub chain: Vec<String>,
    pub role: PreviewRole,
    pub types: Vec<PreviewType>,
    pub annotations: Vec<PreviewAnnotation>,
    pub data_classes: Vec<String>,
    pub modules: Vec<PreviewModule>,
    pub ui_widgets: Vec<PreviewWidget>,
    pub prompts: Vec<PreviewPrompt>,
    /// Null on a first install.
    pub permission_diff: Option<PermissionDiff>,
    /// What the pack's type overrides change on other packs' types while
    /// it is enabled (titles and descriptions are shown to agents).
    pub overrides: Vec<PreviewOverride>,
    pub migrations: MigrationReport,
    pub warnings: Vec<String>,
}

fn decode_bundle(b64: &str) -> Result<Vec<u8>, CmdError> {
    if b64.len() > hp::bundle::MAX_BUNDLE_BYTES / 3 * 4 + 8 {
        return Err(CmdError::invalid(format!(
            "bundle exceeds {} bytes",
            hp::bundle::MAX_BUNDLE_BYTES
        )));
    }
    STANDARD
        .decode(b64.trim().as_bytes())
        .map_err(|_| CmdError::invalid("bundle_base64 is not base64"))
}

/// `packs.preview`: verifies a bundle and reports what installing it
/// would do. Installs nothing; the preview id is what `packs.install`
/// takes, bound to these exact bytes.
pub fn preview(st: &mut State, _ctx: &Ctx, a: PreviewArgs) -> Result<PackPreview, CmdError> {
    let bytes = decode_bundle(&a.bundle_base64)?;
    let plan = match plan_install(st, &bytes, false) {
        Ok(p) => p,
        // Signed by a key the user has not trusted, and it needs that
        // trust to load (annotations, modules, widgets).
        Err(e) if e.code == ErrorCode::Denied => {
            let p = plan_install(st, &bytes, true).map_err(|_| e.clone())?;
            if p.verification.new_key.is_none() {
                return Err(e);
            }
            p
        }
        Err(e) => return Err(e),
    };
    let vb = &plan.verification.vb;
    let m = &vb.manifest;
    let mut warnings = vb.warnings.clone();
    if plan.verification.new_key.is_some() {
        warnings.push(
            "It is signed by a key you have not trusted; installing it trusts that key (trust_key)."
                .into(),
        );
    }
    let probe = check_candidate(st, &plan.change, &plan.cand);
    if let Err(e) = probe {
        warnings.push(format!("It cannot be installed as it is: {}", e.message));
    }
    if plan.report.items_failed > 0 {
        warnings.push(format!(
            "{} items would fail the update; it is refused until they are fixed.",
            plan.report.items_failed
        ));
    } else if let Err(e) = &plan.guard {
        warnings.push(format!("It cannot be installed as it is: {}", e.message));
    }
    let chain = plan
        .cand
        .meta
        .get(&m.id)
        .map(|c| c.chain.iter().rev().skip(1).cloned().collect())
        .filter(|c: &Vec<String>| !c.is_empty())
        .unwrap_or_else(|| m.extends.iter().map(|e| e.id.clone()).collect());
    let before = plan.old.as_ref().map(|_| {
        st.pack_cache
            .meta
            .get(&m.id)
            .and_then(|l| l.manifest.as_deref())
            .map(permissions)
            .unwrap_or_default()
    });
    let before_manifest = plan
        .old
        .as_ref()
        .and_then(|_| st.pack_cache.meta.get(&m.id))
        .and_then(|l| l.manifest.clone());
    let after = permissions(m);
    let overrides = m
        .type_overrides
        .iter()
        .map(|o| PreviewOverride {
            type_id: o.type_id.clone(),
            title: o.title.clone(),
            icon: o.icon.clone(),
            property_descriptions: o.property_descriptions.clone(),
            added_properties: o.add_properties.keys().cloned().collect(),
            added_enum_values: o.add_enum.clone(),
        })
        .collect();
    let mut annotations = Vec::new();
    let mut classes = BTreeSet::new();
    for t in &m.task_types {
        for (_, an) in hp::trust::privileged_in(&t.schema) {
            annotations.push(PreviewAnnotation {
                type_id: t.id.clone(),
                annotation: an.to_string(),
            });
        }
        if let Some(c) = class_of(&t.schema) {
            classes.insert(c);
        }
    }
    let now = st.now();
    let expires = now + chrono::Duration::minutes(PREVIEW_TTL_MINUTES);
    st.previews.retain(|_, p| p.expires > now);
    while st.previews.len() >= MAX_PREVIEWS {
        let oldest = st
            .previews
            .iter()
            .min_by_key(|(_, p)| p.expires)
            .map(|(k, _)| k.clone());
        match oldest {
            Some(k) => st.previews.remove(&k),
            None => break,
        };
    }
    let preview_id = format!("pv_{}", halp_domain::new_id().to_lowercase());
    st.previews.insert(
        preview_id.clone(),
        Preview {
            bytes: std::sync::Arc::new(bytes.clone()),
            bundle_sha256: vb.bundle_sha256.clone(),
            expires,
        },
    );
    Ok(PackPreview {
        preview_id,
        bundle_sha256: vb.bundle_sha256.clone(),
        expires_at: expires.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        id: m.id.clone(),
        version: m.version.clone(),
        title: m.title.clone(),
        description: m.description.clone(),
        publisher: m.publisher.name.clone(),
        trust: domain_trust(vb.trust),
        signature: plan.verification.signature,
        key_fp: vb.signer.as_ref().map(|s| fp_hex(&s.fingerprint)),
        installed_version: plan.old.as_ref().map(|o| o.current.version.clone()),
        files: vb
            .file_index
            .iter()
            .map(|(p, e)| PreviewFile {
                path: p.clone(),
                size: e.size,
            })
            .collect(),
        chain,
        role: PreviewRole {
            id: m.role.id.clone(),
            title: m.role.title.clone(),
            accent: m.role.accent.clone(),
        },
        types: m
            .task_types
            .iter()
            .map(|t| PreviewType {
                id: t.id.clone(),
                title: t.title.clone(),
                data_class: class_of(&t.schema),
            })
            .collect(),
        annotations,
        data_classes: classes.into_iter().collect(),
        modules: m
            .modules
            .iter()
            .map(|x| PreviewModule {
                id: x.name.clone(),
                title: x.name.clone(),
                inputs: x.inputs.keys().cloned().collect(),
            })
            .collect(),
        ui_widgets: m
            .ui_widgets
            .iter()
            .map(|w| PreviewWidget {
                kind: w.kind(&m.id),
                title: w.title.clone(),
                calls: w.calls.clone(),
            })
            .collect(),
        prompts: m
            .prompts
            .add
            .iter()
            .map(|p| PreviewPrompt {
                name: hp::manifest::prompt_name(p).unwrap_or(p).to_string(),
                description: p.clone(),
            })
            .collect(),
        permission_diff: before.map(|b| {
            let mut d = diff_permissions(&b, &after);
            if let Some(bm) = before_manifest.as_deref() {
                escalate_classes(&mut d, bm, m);
            }
            d
        }),
        overrides,
        migrations: plan.report,
        warnings,
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstallArgs {
    pub preview_id: String,
    /// Trust the key that signed the bundle (when it is not trusted yet).
    #[serde(default)]
    pub trust_key: bool,
}

/// `packs.install`: installs or updates exactly the previewed bundle. An
/// update must be newer, signed by the pinned key, and every item of its
/// types must migrate and validate, or nothing changes.
pub fn install(st: &mut State, ctx: &Ctx, a: InstallArgs) -> Result<PackView, CmdError> {
    let now = st.now();
    let Some(p) = st.previews.get(&a.preview_id).cloned() else {
        return Err(CmdError::not_found(
            "unknown preview; choose the file again",
        ));
    };
    if p.expires <= now {
        st.previews.remove(&a.preview_id);
        return Err(CmdError::conflict(
            "the preview expired; choose the file again",
        ));
    }
    if hp::ids::sha256_hex(&p.bytes) != p.bundle_sha256 {
        return Err(CmdError::internal("the previewed bundle changed"));
    }
    let plan = match plan_install(st, &p.bytes, a.trust_key) {
        Ok(plan) => plan,
        Err(e) if e.code == ErrorCode::Denied && !a.trust_key => {
            if plan_install(st, &p.bytes, true).is_ok_and(|p| p.verification.new_key.is_some()) {
                return Err(denied(format!(
                    "{} (it is signed by a key you have not trusted: install with trust_key)",
                    e.message
                )));
            }
            return Err(e);
        }
        Err(e) => return Err(e),
    };
    if plan.report.items_failed > 0 {
        return Err(CmdError::conflict(format!(
            "the update would fail for {} items ({}); nothing was changed",
            plan.report.items_failed,
            plan.report.failed_ids.join(", ")
        )));
    }
    plan.guard.clone()?;
    check_candidate(st, &plan.change, &plan.cand)?;
    let id = plan.verification.vb.manifest.id.clone();
    apply(st, ctx, &a, plan.change, &plan.cand)?;
    view_after(st, &id)
}

// ---- enable / disable / rollback / uninstall ------------------------------

fn installed_record(st: &mut State, id: &str) -> Result<PackRecord, CmdError> {
    if st
        .pack_cache
        .meta
        .get(id)
        .is_some_and(|m| m.trust == PackTrust::Builtin)
    {
        return Err(CmdError::conflict(format!("{id} is a built-in pack")));
    }
    if st.pack_cache.meta.get(id).is_some_and(LoadedPack::is_dev) {
        return Err(CmdError::conflict(format!(
            "{id} is a dev pack; use packs.dev_set"
        )));
    }
    let (records, _, _) = st.pack_rows();
    records
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| CmdError::not_found(format!("pack {id} not found")))
}

pub fn enable(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<PackView, CmdError> {
    set_enabled(st, ctx, a, true)
}

pub fn disable(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<PackView, CmdError> {
    set_enabled(st, ctx, a, false)
}

fn set_enabled(st: &mut State, ctx: &Ctx, a: IdArgs, on: bool) -> Result<PackView, CmdError> {
    refuse_in_dev_mode(st)?;
    let rec = installed_record(st, &a.id)?;
    if rec.enabled == on {
        return view_after(st, &a.id);
    }
    let (cmd, verb) = if on {
        ("packs.enable", "Enabled")
    } else {
        ("packs.disable", "Disabled")
    };
    let mut c = Change::new(st, cmd, "pack", &a.id, format!("{verb} pack {}", a.id));
    c.record_mut(&a.id)?.enabled = on;
    c.targets.insert(a.id.clone());
    if on {
        c.require_ok = Some(a.id.clone());
    }
    let cand = candidate(st, &c);
    check_candidate(st, &c, &cand)?;
    check_items(st, &st.registry_for(&cand)?, &[])?;
    apply(st, ctx, &a, c, &cand)?;
    view_after(st, &a.id)
}

/// `packs.rollback`: back to the version before the last update, undoing
/// its migrations; refused when a migrated item changed since.
pub fn rollback(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<PackView, CmdError> {
    refuse_in_dev_mode(st)?;
    let rec = installed_record(st, &a.id)?;
    let Some(prev) = rec.previous.clone() else {
        return Err(CmdError::conflict(format!(
            "pack {} has no earlier version to roll back to",
            a.id
        )));
    };
    let mut edits = Vec::new();
    for m in &rec.migrated {
        let Some(it) = st.outline.get(&m.id) else {
            continue;
        };
        if it.version != m.version {
            return Err(CmdError::conflict(format!(
                "item {} changed after the update; rolling back would lose that change",
                m.id
            )));
        }
        edits.push(ItemEdit {
            id: m.id.clone(),
            type_id: m.type_id.clone(),
            fields: m.fields.clone(),
            before_type: it.type_id.clone(),
            before_fields: it.fields.clone(),
        });
    }
    let mut c = Change::new(
        st,
        "packs.rollback",
        "pack",
        &a.id,
        format!(
            "Rolled pack {} back from {} to {}",
            a.id, rec.current.version, prev.version
        ),
    );
    {
        let r = c.record_mut(&a.id)?;
        r.current = prev;
        r.previous = None;
        r.migrated = Vec::new();
    }
    c.targets.insert(a.id.clone());
    c.require_ok = Some(a.id.clone());
    let cand = candidate(st, &c);
    check_candidate(st, &c, &cand)?;
    // Every item of a type the rollback changes (the pack's own, and any
    // it overrides) must validate against the version it returns to.
    check_items(st, &st.registry_for(&cand)?, &edits)?;
    c.items = edits;
    apply(st, ctx, &a, c, &cand)?;
    view_after(st, &a.id)
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct IdResult {
    pub id: String,
}

/// `packs.uninstall`: removes the pack and its role layout; its types are
/// retired. Refused while another pack extends it.
pub fn uninstall(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<IdResult, CmdError> {
    refuse_in_dev_mode(st)?;
    installed_record(st, &a.id)?;
    let dependants: Vec<String> = st
        .pack_cache
        .meta
        .values()
        .filter(|m| m.id != a.id)
        .filter(|m| {
            m.manifest
                .as_ref()
                .and_then(|x| x.extends.as_ref())
                .is_some_and(|e| e.id == a.id)
                || m.chain.contains(&a.id)
        })
        .map(|m| m.id.clone())
        .collect();
    if !dependants.is_empty() {
        return Err(CmdError::conflict(format!(
            "pack {} is extended by {}; uninstall those first",
            a.id,
            dependants.join(", ")
        )));
    }
    let layout = st
        .pack_cache
        .meta
        .get(&a.id)
        .map(|m| m.role_id.clone())
        .and_then(|role| st.packs.iter().find(|p| p.id == role))
        .map(|p| p.layout.id.clone());
    let mut c = Change::new(
        st,
        "packs.uninstall",
        "pack",
        &a.id,
        format!("Uninstalled pack {}", a.id),
    );
    c.records.retain(|r| r.id != a.id);
    c.targets.insert(a.id.clone());
    c.remove_layouts.extend(layout);
    let cand = candidate(st, &c);
    check_candidate(st, &c, &cand)?;
    check_items(st, &st.registry_for(&cand)?, &[])?;
    apply(st, ctx, &a, c, &cand)?;
    Ok(IdResult { id: a.id })
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ResetLayoutResult {
    pub id: String,
    pub role_id: String,
}

/// `packs.reset_layout`: the pack's role layout back to the pack default
/// (undoable, like any layout change).
pub fn reset_layout(st: &mut State, ctx: &Ctx, a: IdArgs) -> Result<ResetLayoutResult, CmdError> {
    let m = loaded(st, &a.id)?;
    if !m.is_live() {
        return Err(CmdError::conflict(format!("pack {} is not loaded", a.id)));
    }
    let role_id = m.role_id.clone();
    let layout = st
        .packs
        .iter()
        .find(|p| p.id == role_id)
        .map(|p| p.layout.clone())
        .ok_or_else(|| CmdError::not_found(format!("role {role_id} not found")))?;
    let id = a.id.clone();
    st.write(ctx, "packs.reset_layout", &a, true, |st, tx| {
        let before = st.layouts.get(&layout.id).map(to_value).transpose()?;
        tx.record(
            EntityKind::Layout,
            &layout.id,
            before,
            Some(to_value(&layout)?),
        );
        st.layouts.insert(layout.id.clone(), layout.clone());
        Ok(Done {
            result: ResetLayoutResult {
                id: id.clone(),
                role_id: role_id.clone(),
            },
            entity: entity("layout", &layout.id),
            summary: format!("Reset the {role_id} layout to its pack default"),
        })
    })
}

// ---- keys -----------------------------------------------------------------

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PackKey {
    pub key_id: String,
    /// SHA-256 of the raw public key, 64 hex.
    pub fingerprint: String,
    pub label: String,
    pub first_party: bool,
    pub revoked: bool,
    pub added_at: Option<String>,
    /// Installed packs pinned to it.
    pub packs: usize,
}

fn key_view(k: &KeyRecord, records: &[PackRecord]) -> PackKey {
    let fingerprint = hp::crypto::decode_pubkey(&k.pubkey_b64)
        .map(|p| fp_hex(&hp::fingerprint(&p)))
        .unwrap_or_default();
    PackKey {
        key_id: k.key_id.clone(),
        fingerprint,
        label: k.label.clone(),
        first_party: false,
        revoked: k.revoked_at.is_some(),
        added_at: Some(k.added_at.clone()),
        packs: records.iter().filter(|r| r.current.pinned_to(k)).count(),
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KeysResult {
    pub keys: Vec<PackKey>,
}

pub fn keys_list(st: &mut State, _ctx: &Ctx, _a: NoArgs) -> Result<KeysResult, CmdError> {
    let (records, keys, _) = st.pack_rows();
    let mut out: Vec<PackKey> = hp::FIRST_PARTY_KEYS
        .iter()
        .map(|(label, pk)| {
            let key_id = hp::key_id(pk);
            let fp = fp_hex(&hp::fingerprint(pk));
            PackKey {
                packs: records
                    .iter()
                    .filter(|r| match &r.current.key_fp {
                        Some(f) => *f == fp,
                        None => r.current.key_id.as_deref() == Some(key_id.as_str()),
                    })
                    .count(),
                key_id,
                fingerprint: fp_hex(&hp::fingerprint(pk)),
                label: (*label).to_string(),
                first_party: true,
                revoked: false,
                added_at: None,
            }
        })
        .collect();
    out.extend(keys.iter().map(|k| key_view(k, &records)));
    Ok(KeysResult { keys: out })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeysAddArgs {
    /// The raw Ed25519 public key (32 bytes), base64.
    pub pubkey_b64: String,
    pub label: String,
}

pub fn keys_add(st: &mut State, ctx: &Ctx, a: KeysAddArgs) -> Result<PackKey, CmdError> {
    refuse_in_dev_mode(st)?;
    let label = a.label.trim().to_string();
    if label.is_empty() || label.chars().count() > 100 {
        return Err(CmdError::invalid("label must be 1 to 100 characters"));
    }
    halp_domain::limits::check_json_text("label", &Value::String(label.clone()))?;
    let pubkey = hp::crypto::decode_pubkey(a.pubkey_b64.trim())?;
    let key_id = hp::key_id(&pubkey);
    if hp::FIRST_PARTY_KEYS.iter().any(|(_, k)| *k == pubkey) {
        return Err(CmdError::conflict(
            "that is a HalpTask key; it is always trusted",
        ));
    }
    let mut c = Change::new(
        st,
        "packs.keys_add",
        "pack_key",
        &key_id,
        format!("Trusted key {label}"),
    );
    // Rows are keyed by the 16-hex key id. A different key with the same
    // id is refused, never merged (forging one for a given key takes a
    // 64-bit second preimage).
    if let Some(k) = c.keys.iter().find(|k| k.key_id == key_id) {
        return Err(CmdError::conflict(
            if k.pubkey_b64 != hp::crypto::encode_pubkey(&pubkey) {
                format!(
                    "a different key with the id {key_id} is already in the list; keys are never replaced, so this key cannot be added"
                )
            } else if k.revoked_at.is_some() {
                "that key was revoked; a revoked key stays revoked".to_string()
            } else {
                "that key is already in the list".to_string()
            },
        ));
    }
    let rec = KeyRecord {
        key_id: key_id.clone(),
        pubkey_b64: hp::crypto::encode_pubkey(&pubkey),
        label,
        added_at: now_str(st),
        revoked_at: None,
    };
    c.keys.push(rec.clone());
    let cand = candidate(st, &c);
    check_candidate(st, &c, &cand)?;
    check_items(st, &st.registry_for(&cand)?, &[])?;
    apply(st, ctx, &a, c, &cand)?;
    let (records, _, _) = st.pack_rows();
    Ok(key_view(&rec, &records))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeysRevokeArgs {
    pub key_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct KeysRevokeResult {
    pub key: PackKey,
    pub disabled_packs: Vec<String>,
    /// Items that no longer validate because a disabled pack's type
    /// override went away (a revocation is never refused). They keep
    /// their data; the user edits them back in line.
    pub invalid_items: Vec<String>,
}

/// `packs.keys_revoke`: the key stops verifying; every pack pinned to it is
/// disabled and its types retired.
pub fn keys_revoke(
    st: &mut State,
    ctx: &Ctx,
    a: KeysRevokeArgs,
) -> Result<KeysRevokeResult, CmdError> {
    if hp::FIRST_PARTY_KEYS
        .iter()
        .any(|(_, k)| hp::key_id(k) == a.key_id)
    {
        return Err(denied(
            "HalpTask keys are compiled in and cannot be revoked",
        ));
    }
    let mut c = Change::new(
        st,
        "packs.keys_revoke",
        "pack_key",
        &a.key_id,
        format!("Revoked key {}", a.key_id),
    );
    let now = now_str(st);
    let Some(k) = c.keys.iter_mut().find(|k| k.key_id == a.key_id) else {
        return Err(CmdError::not_found(format!("key {} not found", a.key_id)));
    };
    if k.revoked_at.is_none() {
        k.revoked_at = Some(now);
    }
    let key = k.clone();
    let mut disabled = Vec::new();
    for r in &mut c.records {
        if r.current.pinned_to(&key) && r.enabled {
            r.enabled = false;
            disabled.push(r.id.clone());
        }
    }
    c.allow_breakage = true;
    let cand = candidate(st, &c);
    let invalid: Vec<String> = invalid_items(st, &st.registry_for(&cand)?, &[])
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    apply(st, ctx, &a, c, &cand)?;
    let (records, _, _) = st.pack_rows();
    Ok(KeysRevokeResult {
        key: key_view(&key, &records),
        disabled_packs: disabled,
        invalid_items: invalid,
    })
}

// ---- dev mode -------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(transform = require_path)]
pub struct DevSetArgs {
    /// An absolute folder path, or null to turn dev mode off. Required, so
    /// that a missing key never turns dev mode off by accident.
    #[serde(deserialize_with = "Option::deserialize")]
    pub path: Option<String>,
}

/// `path` is required but nullable (schemars leaves an `Option` optional).
fn require_path(schema: &mut schemars::Schema) {
    if let Some(o) = schema.as_object_mut() {
        o.insert("required".into(), serde_json::json!(["path"]));
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DevSetResult {
    pub path: Option<String>,
    /// Dev packs that loaded.
    pub packs: usize,
}

fn dev_result(st: &State) -> DevSetResult {
    DevSetResult {
        path: st.dev.as_ref().map(|d| d.path.display().to_string()),
        packs: st
            .pack_cache
            .meta
            .values()
            .filter(|m| m.is_dev() && m.is_live())
            .count(),
    }
}

fn dev_change(
    st: &mut State,
    ctx: &Ctx,
    cmd: &'static str,
    args: &impl Serialize,
    dev: Option<DevState>,
) -> Result<DevSetResult, CmdError> {
    let summary = match &dev {
        Some(_) => "Dev mode on".to_string(),
        None => "Dev mode off".to_string(),
    };
    let mut c = Change::new(st, cmd, "pack_dev", "dev", summary);
    c.dev = dev;
    // Dev packs may come and go; installed packs must keep loading.
    c.targets.extend(
        st.pack_cache
            .meta
            .values()
            .filter(|m| m.is_dev())
            .map(|m| m.id.clone()),
    );
    let cand = candidate(st, &c);
    check_candidate(st, &c, &cand)?;
    // Dev packs' own types may change shape freely (their items are the
    // developer's test data), but never a data class or a lock rule. An
    // installed pack's items must stay valid: a dev override of an
    // installed type may not come or go while items need it.
    let reg = st.registry_for(&cand)?;
    check_type_rules(st, &reg, &[])?;
    let dev_owned = |o: &TaskTypeOrigin| {
        matches!(
            o,
            TaskTypeOrigin::Pack {
                trust: DomainTrust::Dev,
                ..
            } | TaskTypeOrigin::Retired {
                trust: DomainTrust::Dev,
                ..
            }
        )
    };
    let bad: Vec<(String, String)> = invalid_items(st, &reg, &[])
        .into_iter()
        .filter(|(id, _)| {
            let ty = st.outline.get(id).and_then(|it| it.type_id.clone());
            !ty.is_some_and(|t| {
                st.types.get(&t).is_some_and(|(_, o)| dev_owned(o))
                    && reg.get(&t).is_some_and(|(_, o)| dev_owned(o))
            })
        })
        .collect();
    if let Some((_, first)) = bad.first() {
        let ids: Vec<&str> = bad.iter().take(50).map(|(id, _)| id.as_str()).collect();
        return Err(CmdError::conflict(format!(
            "{} items of installed packs would no longer validate ({}); nothing was changed. First: {first}",
            bad.len(),
            ids.join(", ")
        )));
    }
    apply(st, ctx, args, c, &cand)?;
    Ok(dev_result(st))
}

/// `packs.dev_set`: loads every pack folder under `path` as a dev pack
/// (`dev.<id>`, never persisted), or turns dev mode off.
pub fn dev_set(st: &mut State, ctx: &Ctx, a: DevSetArgs) -> Result<DevSetResult, CmdError> {
    let dev = match &a.path {
        None => None,
        Some(p) => {
            let path = std::path::Path::new(p);
            if !path.is_absolute() {
                return Err(CmdError::invalid("path must be absolute"));
            }
            Some(installed::read_dev(path)?)
        }
    };
    dev_change(st, ctx, "packs.dev_set", &a, dev)
}

/// `packs.dev_reload`: reads the dev folder again.
pub fn dev_reload(st: &mut State, ctx: &Ctx, a: NoArgs) -> Result<DevSetResult, CmdError> {
    let Some(d) = &st.dev else {
        return Err(CmdError::conflict("dev mode is off"));
    };
    let dev = installed::read_dev(&d.path.clone())?;
    dev_change(st, ctx, "packs.dev_reload", &a, Some(dev))
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DevError {
    pub dir: String,
    pub message: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct DevGetResult {
    pub path: Option<String>,
    pub packs: Vec<PackView>,
    /// Folders that could not be read.
    pub errors: Vec<DevError>,
}

pub fn dev_get(st: &mut State, _ctx: &Ctx, _a: NoArgs) -> Result<DevGetResult, CmdError> {
    Ok(DevGetResult {
        path: st.dev.as_ref().map(|d| d.path.display().to_string()),
        packs: st
            .pack_cache
            .meta
            .values()
            .filter(|m| m.is_dev())
            .map(|m| view(m, true))
            .collect(),
        errors: st
            .dev
            .iter()
            .flat_map(|d| d.errors.iter())
            .map(|(dir, message)| DevError {
                dir: dir.clone(),
                message: message.clone(),
            })
            .collect(),
    })
}
