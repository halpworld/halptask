//! In-memory state behind the core lock: the store plus caches (outline,
//! type registry, layouts, roles, settings), and the write path that turns a
//! handler's mutations into one recorded event.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use crate::installed::{
    DevState, Inputs, KeyRecord, PackCache, PackRecord, Preview, RetiredRecord, Status,
};
use chrono::{DateTime, NaiveDate, Utc};
use halp_domain::{
    EntityRef, Event, Item, ItemView, Layout, Outline, Role, TaskType, TaskTypeOrigin,
    TaskTypeRegistry,
};
use halp_store::{Change, EntityKind, NewEvent, Store};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::clock::Clock;
use crate::envelope::CmdError;
use crate::packs::{DEFAULT_ROLE, Pack};
use crate::policy::{Actor, Client, Ctx, Policy, Rule};
use crate::registry::CommandDescriptor;

pub(crate) const SETTING_ROLE: &str = "current_role";

/// The schema key a task type uses to put its items in a data class.
pub const DATA_CLASS_KEY: &str = "x-halp-data-class";

/// Longest data class name.
pub const MAX_DATA_CLASS_LEN: usize = 64;

/// Whether `s` is a well-formed data class name:
/// `[a-z0-9]+(\.[a-z0-9_]+)*`, at most 64 characters.
pub(crate) fn is_valid_class_name(s: &str) -> bool {
    if s.is_empty() || s.len() > MAX_DATA_CLASS_LEN {
        return false;
    }
    let mut parts = s.split('.');
    let head_ok = parts.next().is_some_and(|h| {
        !h.is_empty()
            && h.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    });
    head_ok
        && parts.all(|p| {
            !p.is_empty()
                && p.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

/// Rejects a task type whose `x-halp-data-class` is present but not a
/// well-formed class name. Run when a pack or a user type is loaded.
pub(crate) fn check_type_class(t: &halp_domain::TaskType) -> Result<(), halp_domain::DomainError> {
    match t.schema.get(DATA_CLASS_KEY) {
        None => Ok(()),
        Some(v) if v.as_str().is_some_and(is_valid_class_name) => Ok(()),
        Some(_) => Err(halp_domain::DomainError::invalid(format!(
            "type {}: {DATA_CLASS_KEY} must match [a-z0-9]+(.[a-z0-9_]+)*, at most {MAX_DATA_CLASS_LEN} characters",
            t.id
        ))),
    }
}

/// Most writers remembered per entity. Past that the content is
/// written by so many others that any reader is tainted anyway.
pub(crate) const MAX_WRITERS: usize = 8;

/// Who other than the user wrote an entity: setting `writers:<target>`,
/// target `item:<id>`, `widget:<id>` or `type:<id>`. Kept as settings so
/// they are recorded in the same event as the write and undo/redo moves
/// them with the content.
pub(crate) fn writers_key(target: &str) -> String {
    format!("writers:{target}")
}

/// The stored form of a writers setting.
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub(crate) struct WritersSetting {
    pub halp_writers: String,
    pub actors: BTreeSet<String>,
}

/// Focus is per role: setting `focus:<role id>`.
pub(crate) fn focus_key(role: &str) -> String {
    format!("focus:{role}")
}

/// A request parked until the person at the keyboard approves or denies it.
#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct PendingConsent {
    /// 256 random bits, hex. Single use; bound to actor, command and args.
    pub id: String,
    pub command: String,
    /// The effective tier (may be higher than the command's static tier).
    pub tier: crate::policy::Tier,
    /// The requesting actor; the approved command runs as this actor.
    pub actor: String,
    /// The arguments that will run, in full (canonical), with only secrets
    /// and imported file content replaced by placeholders. When they are
    /// larger than [`crate::MAX_DISPLAY_ARGS_BYTES`] this is a short form
    /// (long values cut, or size and hash) and `display_truncated` is set.
    pub args: Value,
    /// `args` is not the whole request. A client must not offer approval
    /// for a request the user cannot see in full; `consent.get` returns the
    /// whole form whenever it fits.
    #[serde(default)]
    pub display_truncated: bool,
    /// SHA-256 of the canonical `[command, args]` that will run.
    pub args_hash: String,
    /// One line built from the resolved targets.
    pub summary: String,
    pub details: ConsentDetails,
    /// The requesting session read content from someone other than the
    /// user or itself (an integration, another agent) before asking.
    pub tainted: bool,
    pub taint_source: Option<String>,
    pub created_at: DateTime<Utc>,
    /// Denied automatically after this.
    pub expires_at: DateTime<Utc>,
    /// It touches an item outside the default data class. On the socket's
    /// user channel (`halpd consent`) such a request is redacted: only its
    /// id, command, actor, tier and times are shown (v1.8).
    #[serde(default)]
    pub private: bool,
}

/// What an approval would do, resolved against the current state.
#[derive(Debug, Clone, Default, PartialEq, Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConsentDetails {
    pub targets: Vec<ConsentTarget>,
    /// Plain-language effects, e.g. "Deletes \"Plan\" and 3 items below it".
    pub effects: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConsentTarget {
    /// `item`, `widget`, `event`, `file`, ...
    pub kind: String,
    pub id: String,
    pub title: String,
    /// Items below it (for items), else 0.
    pub descendants: usize,
}

/// A parked request with the arguments that will actually run.
#[derive(Debug, Clone)]
pub(crate) struct PendingEntry {
    pub public: PendingConsent,
    pub args: Value,
    /// The short form of the display args (what the audit log keeps), used
    /// when a listing would otherwise grow past its size budget.
    pub short_args: Value,
    pub deadline: std::time::Instant,
    /// Digest of what the request would touch when it was shown
    /// (see `scope_digest`); recomputed at approval.
    pub scope: String,
    /// Why it was parked (for a re-park after the scope changed).
    pub reason: String,
    /// It touches an item outside the default data class: the event log
    /// records it without titles or arguments.
    pub private: bool,
    /// Its arguments were too large to show in full when it was parked:
    /// it can only be denied (or expire), never approved.
    pub truncated: bool,
}

pub struct State {
    pub(crate) store: Store,
    pub(crate) outline: Outline,
    pub(crate) types: TaskTypeRegistry,
    /// While dev mode is on: the registry without dev packs, which is what
    /// a restart gives (dev mode is never persisted). `None` otherwise.
    pub(crate) base_types: Option<TaskTypeRegistry>,
    pub(crate) layouts: BTreeMap<String, Layout>,
    /// Roles in pack order.
    pub(crate) roles: Vec<Role>,
    /// Loaded packs (built-in, installed, dev), in role order.
    pub(crate) packs: Vec<Pack>,
    /// The compiled-in packs.
    pub(crate) builtin: Vec<Pack>,
    /// What installed and dev packs contribute (rebuilt by `reload_packs`
    /// on open and after every pack change).
    pub(crate) pack_cache: PackCache,
    /// Dev mode (never persisted: a restart turns it off).
    pub(crate) dev: Option<DevState>,
    /// `packs.preview` results awaiting `packs.install`, by preview id.
    pub(crate) previews: BTreeMap<String, Preview>,
    pub(crate) current_role: String,
    /// Focused item per role id.
    pub(crate) focus: BTreeMap<String, String>,
    /// Pending consents (in memory; a restart drops them, which denies them).
    pub(crate) pending: BTreeMap<String, PendingEntry>,
    pub(crate) clock: Arc<dyn Clock>,
    pub(crate) events: broadcast::Sender<Event>,
    pub(crate) descriptors: Vec<CommandDescriptor>,
    pub(crate) policy: Arc<dyn Policy>,
    /// Set when the caches could not be rebuilt after a committed change:
    /// the core refuses further calls instead of serving stale state.
    pub(crate) failed: Option<String>,
    /// Problems repaired while opening (orphans re-rooted, ...).
    pub(crate) warnings: Vec<String>,
    /// Process-local services set with `Core::set_extension`, by type.
    pub(crate) extensions: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    /// Non-user writers per target (`item:<id>`, `widget:<id>`,
    /// `type:<id>`), from the `writers:*` settings. Reading content any of
    /// them wrote taints the reader (see `Core::note_reads`).
    pub(crate) writers: BTreeMap<String, BTreeSet<String>>,
}

/// What a write handler returns besides its result.
pub(crate) struct Done<R> {
    pub result: R,
    pub entity: EntityRef,
    pub summary: String,
}

/// Non-item changes collected during a write (items are journaled by the
/// outline itself).
#[derive(Default)]
pub(crate) struct Tx {
    pub changes: Vec<Change>,
}

impl Tx {
    pub fn record(
        &mut self,
        kind: EntityKind,
        id: &str,
        before: Option<Value>,
        after: Option<Value>,
    ) {
        self.changes.push(Change {
            kind,
            id: id.to_string(),
            before,
            after,
        });
    }
}

pub(crate) fn entity(kind: &str, id: &str) -> EntityRef {
    EntityRef {
        kind: kind.to_string(),
        id: id.to_string(),
    }
}

/// The summary of a write to a private item: its verb, never its text.
pub(crate) fn private_summary(summary: &str) -> String {
    let verb = summary
        .split_whitespace()
        .next()
        .filter(|w| w.chars().all(char::is_alphabetic))
        .unwrap_or("Changed");
    format!("{verb} a private item")
}

/// Shortens a title for event summaries.
pub(crate) fn short(s: &str) -> String {
    let mut out: String = s.chars().take(60).collect();
    if s.chars().count() > 60 {
        out.push('…');
    }
    out
}

impl State {
    pub(crate) fn open(
        store: Store,
        packs: Vec<Pack>,
        clock: Arc<dyn Clock>,
        events: broadcast::Sender<Event>,
        policy: Arc<dyn Policy>,
    ) -> Result<State, CmdError> {
        let roles = packs.iter().map(Pack::role).collect();
        let mut st = State {
            store,
            outline: Outline::new(),
            types: TaskTypeRegistry::default(),
            base_types: None,
            layouts: BTreeMap::new(),
            roles,
            packs: packs.clone(),
            builtin: packs,
            pack_cache: PackCache::default(),
            dev: None,
            previews: BTreeMap::new(),
            current_role: DEFAULT_ROLE.to_string(),
            focus: BTreeMap::new(),
            pending: BTreeMap::new(),
            clock,
            events,
            descriptors: Vec::new(),
            policy,
            failed: None,
            warnings: Vec::new(),
            extensions: HashMap::new(),
            writers: BTreeMap::new(),
        };
        st.repair()?;
        st.reload_packs();
        st.reload()?;
        st.seed()?;
        st.report_pack_failures();
        Ok(st)
    }

    /// Opening check: an outline that does not load (orphans, cycles, too
    /// deep) is repaired, its moved items re-rooted, and the fix recorded as
    /// one non-undoable `system.repair` event.
    fn repair(&mut self) -> Result<(), CmdError> {
        let items: Vec<Item> = self.store.load(EntityKind::Item)?;
        if Outline::from_items(items.clone()).is_ok() {
            return Ok(());
        }
        let before: BTreeMap<String, Item> =
            items.iter().map(|i| (i.id.clone(), i.clone())).collect();
        let (_, fixed, warnings) = Outline::repair(items);
        let mut changes = Vec::new();
        for f in fixed {
            changes.push(Change {
                kind: EntityKind::Item,
                id: f.id.clone(),
                before: before.get(&f.id).map(serde_json::to_value).transpose()?,
                after: Some(serde_json::to_value(&f)?),
            });
        }
        for w in &warnings {
            eprintln!("halp-core: repaired outline: {w}");
        }
        self.warnings.extend(warnings);
        if changes.is_empty() {
            return Ok(());
        }
        let n = changes.len();
        let ev = NewEvent {
            actor: Actor::System.to_string(),
            command: "system.repair".into(),
            entity: entity("outline", "items"),
            summary: format!("Repaired the outline ({n} items re-rooted)"),
            args: json!({}),
            changes,
            undoable: false,
        };
        let now = self.now();
        self.store.commit(ev, now)?;
        Ok(())
    }

    /// Rebuilds the caches after the store changed underneath them; when that
    /// fails the core is put into the failed state.
    pub(crate) fn reload_or_fail(&mut self) -> Result<(), CmdError> {
        self.reload().inspect_err(|e| {
            self.failed = Some(format!("could not reload after a change: {}", e.message));
        })
    }

    /// Pushes the stored clients and rules into the policy engine.
    pub(crate) fn sync_policy(&mut self) -> Result<(), CmdError> {
        let clients: Vec<Client> = self.store.load(EntityKind::Client)?;
        let rules: Vec<Rule> = self.store.load(EntityKind::Rule)?;
        self.policy.load(clients, rules);
        Ok(())
    }

    /// The service of type `T` attached with `Core::set_extension`.
    #[allow(dead_code)] // used by integration commands
    pub(crate) fn extension<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        self.extensions
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|a| a.downcast::<T>().ok())
    }

    pub(crate) fn now(&self) -> DateTime<Utc> {
        self.clock.now()
    }

    pub(crate) fn today(&self) -> NaiveDate {
        self.clock.today()
    }

    /// Reads stored rows of `kind` one by one, skipping (with a warning)
    /// any that no longer parse: pack data never blocks startup.
    pub(crate) fn load_rows<T: serde::de::DeserializeOwned>(&mut self, kind: EntityKind) -> Vec<T> {
        let rows: Vec<Value> = match self.store.load(kind) {
            Ok(r) => r,
            Err(e) => {
                self.warnings
                    .push(format!("could not read {}: {e}", kind.as_str()));
                return Vec::new();
            }
        };
        let mut out = Vec::new();
        for r in rows {
            match serde_json::from_value(r) {
                Ok(v) => out.push(v),
                Err(e) => self
                    .warnings
                    .push(format!("skipped a malformed {} row: {e}", kind.as_str())),
            }
        }
        out
    }

    /// The stored pack records, keys and retired types.
    pub(crate) fn pack_rows(&mut self) -> (Vec<PackRecord>, Vec<KeyRecord>, Vec<RetiredRecord>) {
        (
            self.load_rows(EntityKind::Pack),
            self.load_rows(EntityKind::PackKey),
            self.load_rows(EntityKind::RetiredType),
        )
    }

    /// Runs the pack loader over `records` and `keys` (and the stored
    /// retired types), with `extra` bundles not yet stored.
    pub(crate) fn load_packs_with(
        &self,
        records: &[PackRecord],
        keys: &[KeyRecord],
        retired: &[RetiredRecord],
        dev: Option<&DevState>,
        extra: &BTreeMap<String, Vec<u8>>,
    ) -> PackCache {
        let blob = |sha: &str| -> Option<Vec<u8>> {
            extra
                .get(sha)
                .cloned()
                .or_else(|| self.store.get_blob(sha).ok().flatten())
        };
        let user_types: BTreeSet<String> = self
            .store
            .load::<TaskType>(EntityKind::TaskType)
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.id)
            .collect();
        crate::installed::load(&Inputs {
            builtin: &self.builtin,
            records,
            blob: &blob,
            keys,
            retired,
            dev,
            user_types: &user_types,
        })
    }

    /// The type registry `cache` gives: built-in types, pack types (live
    /// and retired), then the user's own.
    pub(crate) fn registry_for(&self, cache: &PackCache) -> Result<TaskTypeRegistry, CmdError> {
        let mut types = TaskTypeRegistry::with_builtins()?;
        for t in cache.types.values() {
            if let Err(e) = types.insert(t.def.clone(), t.origin.clone()) {
                eprintln!("halp-core: pack type {} skipped: {}", t.def.id, e.message);
            }
        }
        let user: Vec<TaskType> = self.store.load(EntityKind::TaskType)?;
        for t in user {
            // A stored type that no longer validates (or now collides with a
            // pack type) is skipped rather than blocking startup.
            let _ = types.insert(t, TaskTypeOrigin::User);
        }
        Ok(types)
    }

    /// Checks an item's fields as every write does. While dev mode is on,
    /// an item of a type that is not a dev pack's must also validate
    /// without dev packs, so no installed item comes to rely on a dev
    /// override (turning dev mode off, or a restart, would leave it
    /// invalid).
    pub(crate) fn validate_item_fields(
        &self,
        type_id: Option<&str>,
        fields: &serde_json::Map<String, Value>,
    ) -> Result<(), CmdError> {
        validate_item_fields_in(&self.types, self.base_types.as_ref(), type_id, fields)
    }

    /// Re-verifies and re-resolves every installed and dev pack from the
    /// store (on open and after each pack change). Never fails.
    pub(crate) fn reload_packs(&mut self) {
        let (records, keys, retired) = self.pack_rows();
        let cache = self.load_packs_with(
            &records,
            &keys,
            &retired,
            self.dev.as_ref(),
            &BTreeMap::new(),
        );
        self.packs = cache.packs.clone();
        self.roles = self.packs.iter().map(Pack::role).collect();
        self.pack_cache = cache;
    }

    /// Startup: one warning and one `packs.status` event per pack that
    /// did not load.
    fn report_pack_failures(&mut self) {
        let bad: Vec<(String, String, &str)> = self
            .pack_cache
            .meta
            .values()
            .filter(|m| matches!(m.status, Status::Error | Status::Blocked))
            .map(|m| {
                let why = m
                    .error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_default();
                (m.id.clone(), why, m.status.as_str())
            })
            .collect();
        for (id, why, status) in bad {
            let msg = format!("pack {id} was not loaded: {why}");
            eprintln!("halp-core: {msg}");
            self.warnings.push(msg.clone());
            let _ = self.audit(
                &Actor::System.to_string(),
                "packs.status",
                entity("pack", &id),
                format!("Pack {id} was not loaded"),
                json!({"id": id, "status": status}),
            );
        }
    }

    /// Refuses a `type_id` whose type is retired (its pack was uninstalled,
    /// disabled or failed to load): no new items of it.
    pub(crate) fn require_live_type(&self, type_id: Option<&str>) -> Result<(), CmdError> {
        if let Some(t) = type_id
            && let Some((_, origin)) = self.types.get(t)
            && origin.is_retired()
        {
            return Err(CmdError::invalid(format!(
                "type {t} is retired (its pack is not installed or not enabled)"
            )));
        }
        Ok(())
    }

    /// Rebuilds every cache from the store.
    pub(crate) fn reload(&mut self) -> Result<(), CmdError> {
        let items: Vec<Item> = self.store.load(EntityKind::Item)?;
        self.outline = Outline::from_items(items)?;

        self.types = self.registry_for(&self.pack_cache)?;
        self.base_types = if self.dev.is_some() {
            let (records, keys, retired) = self.pack_rows();
            let cache = self.load_packs_with(&records, &keys, &retired, None, &BTreeMap::new());
            Some(self.registry_for(&cache)?)
        } else {
            None
        };

        let layouts: Vec<Layout> = self.store.load(EntityKind::Layout)?;
        self.layouts = layouts.into_iter().map(|l| (l.id.clone(), l)).collect();

        let role = self.store.get(EntityKind::Setting, SETTING_ROLE)?;
        self.current_role = role
            .as_ref()
            .and_then(Value::as_str)
            .filter(|r| self.roles.iter().any(|x| x.id == *r))
            .unwrap_or(DEFAULT_ROLE)
            .to_string();
        let mut focus = BTreeMap::new();
        for r in &self.roles {
            if let Some(Value::String(id)) =
                self.store.get(EntityKind::Setting, &focus_key(&r.id))?
            {
                focus.insert(r.id.clone(), id);
            }
        }
        self.focus = focus;
        let settings: Vec<Value> = self.store.load(EntityKind::Setting)?;
        self.writers = settings
            .into_iter()
            .filter_map(|v| serde_json::from_value::<WritersSetting>(v).ok())
            .map(|w| (w.halp_writers, w.actors))
            .collect();
        self.sync_policy()
    }

    /// The one decision for `grants.set`: a class can be granted if it is a
    /// built-in class, or a well-formed class declared by `x-halp-data-class`
    /// on a task type currently in the registry (built-in, pack or user).
    pub(crate) fn is_grantable_class(&self, class: &str) -> bool {
        if crate::policy::is_data_class(class) {
            return true;
        }
        is_valid_class_name(class)
            && self
                .types
                .list()
                .iter()
                .any(|(t, _)| t.schema.get(DATA_CLASS_KEY).and_then(Value::as_str) == Some(class))
    }

    /// The data class a task type puts its items in (`general` unless its
    /// schema says `x-halp-data-class`). An item whose type is not loaded
    /// (a stored definition that no longer validates, a pack type that was
    /// skipped) fails closed: its items count as `secrets`, since nothing
    /// says what they hold.
    pub(crate) fn type_class(&self, type_id: Option<&str>) -> String {
        let Some(t) = type_id else {
            return crate::policy::DEFAULT_DATA_CLASS.to_string();
        };
        let Some((def, _)) = self.types.get(t) else {
            return UNKNOWN_TYPE_CLASS.to_string();
        };
        def.schema
            .get(DATA_CLASS_KEY)
            .and_then(Value::as_str)
            .unwrap_or(crate::policy::DEFAULT_DATA_CLASS)
            .to_string()
    }

    /// The data class of an item: the first non-default class of its own
    /// type or an ancestor's (a bullet under a private note is private).
    /// `None` for an unknown item.
    pub(crate) fn item_class(&self, id: &str) -> Option<String> {
        let it = self.outline.get(id)?;
        let own = self.type_class(it.type_id.as_deref());
        if own != crate::policy::DEFAULT_DATA_CLASS {
            return Some(own);
        }
        for a in self.outline.ancestors(id) {
            let c = self.type_class(a.type_id.as_deref());
            if c != crate::policy::DEFAULT_DATA_CLASS {
                return Some(c);
            }
        }
        Some(own)
    }

    /// The class of an item image from a change (`before` or `after`).
    fn image_class(&self, v: &Value) -> String {
        let own = self.type_class(v.get("type_id").and_then(Value::as_str));
        if own != crate::policy::DEFAULT_DATA_CLASS {
            return own;
        }
        v.get("parent_id")
            .and_then(Value::as_str)
            .and_then(|p| self.item_class(p))
            .unwrap_or(own)
    }

    /// The first non-default data class among the items a change set
    /// touches (before or after), if any.
    pub(crate) fn private_class(&self, changes: &[Change]) -> Option<String> {
        changes
            .iter()
            .filter(|c| c.kind == EntityKind::Item)
            .flat_map(|c| [&c.before, &c.after])
            .flatten()
            .map(|v| self.image_class(v))
            .find(|c| c != crate::policy::DEFAULT_DATA_CLASS)
    }

    /// Records `actor` as a writer of what `changes` wrote, in the same
    /// event (user and system writes are not tracked).
    fn note_writers(&self, ctx: &Ctx, changes: &mut Vec<Change>) -> Result<(), CmdError> {
        if matches!(ctx.actor, Actor::User | Actor::System) {
            return Ok(());
        }
        let actor = ctx.actor.to_string();
        let mut targets: BTreeSet<String> = BTreeSet::new();
        for c in changes.iter() {
            let Some(after) = &c.after else { continue };
            match c.kind {
                EntityKind::Item => {
                    targets.insert(format!("item:{}", c.id));
                }
                EntityKind::TaskType => {
                    targets.insert(format!("type:{}", c.id));
                }
                EntityKind::Layout => {
                    // A widget whose title or config this write set.
                    let text = |l: Option<&Value>| -> BTreeMap<String, Value> {
                        l.and_then(|l| l.get("widgets"))
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|w| {
                                let id = w.get("id")?.as_str()?.to_string();
                                Some((id, json!([w.get("title"), w.get("config")])))
                            })
                            .collect()
                    };
                    let old = text(c.before.as_ref());
                    for (id, t) in text(Some(after)) {
                        if old.get(&id) != Some(&t) {
                            targets.insert(format!("widget:{id}"));
                        }
                    }
                }
                _ => {}
            }
        }
        self.add_writers(&actor, targets, changes)
    }

    /// Adds `actor` to the writers of each target, as setting changes.
    pub(crate) fn add_writers(
        &self,
        actor: &str,
        targets: BTreeSet<String>,
        changes: &mut Vec<Change>,
    ) -> Result<(), CmdError> {
        let actor = actor.to_string();
        for t in targets {
            let before = self.writers.get(&t);
            if before.is_some_and(|w| w.contains(&actor) || w.len() >= MAX_WRITERS) {
                continue;
            }
            let mut actors = before.cloned().unwrap_or_default();
            actors.insert(actor.clone());
            let old = before.map(|a| {
                serde_json::to_value(WritersSetting {
                    halp_writers: t.clone(),
                    actors: a.clone(),
                })
            });
            changes.push(Change {
                kind: EntityKind::Setting,
                id: writers_key(&t),
                before: old.transpose()?,
                after: Some(serde_json::to_value(WritersSetting {
                    halp_writers: t,
                    actors,
                })?),
            });
        }
        Ok(())
    }

    /// Installs missing pack layouts and the default role (not undoable).
    fn seed(&mut self) -> Result<(), CmdError> {
        let mut tx = Tx::default();
        for p in &self.packs {
            if !self.layouts.contains_key(&p.layout.id) {
                tx.record(
                    EntityKind::Layout,
                    &p.layout.id,
                    None,
                    Some(serde_json::to_value(&p.layout)?),
                );
            }
        }
        if self.store.get(EntityKind::Setting, SETTING_ROLE)?.is_none() {
            tx.record(
                EntityKind::Setting,
                SETTING_ROLE,
                None,
                Some(json!(DEFAULT_ROLE)),
            );
        }
        if tx.changes.is_empty() {
            return Ok(());
        }
        let n = tx.changes.len();
        let ev = NewEvent {
            actor: Actor::System.to_string(),
            command: "system.seed".into(),
            entity: entity("pack", "builtin"),
            summary: format!("Installed built-in role packs ({n} changes)"),
            args: json!({}),
            changes: tx.changes,
            undoable: false,
        };
        let now = self.now();
        let event = self.store.commit(ev, now)?;
        let _ = self.events.send(event);
        self.reload()
    }

    /// Runs a write: `f` mutates the outline (journaled) and records other
    /// changes in `Tx`; on success everything is committed with one event, on
    /// any error all caches are restored from the store.
    pub(crate) fn write<A, R, F>(
        &mut self,
        ctx: &Ctx,
        command: &str,
        args: &A,
        undoable: bool,
        f: F,
    ) -> Result<R, CmdError>
    where
        A: Serialize,
        F: FnOnce(&mut State, &mut Tx) -> Result<Done<R>, CmdError>,
    {
        let args = serde_json::to_value(args)?;
        self.outline.begin();
        let mut tx = Tx::default();
        let done = match f(self, &mut tx) {
            Ok(d) => d,
            Err(e) => {
                self.outline.rollback();
                self.reload_or_fail()?;
                return Err(e);
            }
        };
        let mut changes = Vec::new();
        for c in self.outline.commit() {
            changes.push(Change {
                kind: EntityKind::Item,
                id: c.id,
                before: c.before.map(serde_json::to_value).transpose()?,
                after: c.after.map(serde_json::to_value).transpose()?,
            });
        }
        changes.extend(tx.changes);
        changes.retain(|c| c.before != c.after);
        if changes.is_empty() {
            return Ok(done.result);
        }
        // Items of a type with `x-halp-immutable-when` (v1.9): checked on
        // the final change set, so every command and path is covered.
        if let Err(e) = crate::immutable::Locks::of(self).check(&ctx.actor, &changes) {
            self.reload_or_fail()?;
            return Err(e);
        }
        // Items of a non-default data class (a private note) keep their
        // text out of the event log and the activity feed.
        let (summary, args) = if self.private_class(&changes).is_some() {
            (private_summary(&done.summary), json!({ "private": true }))
        } else {
            (done.summary, args)
        };
        if let Err(e) = self.note_writers(ctx, &mut changes) {
            self.reload_or_fail()?;
            return Err(e);
        }
        let writers: Vec<(String, BTreeSet<String>)> = changes
            .iter()
            .filter(|c| c.kind == EntityKind::Setting)
            .filter_map(|c| c.after.clone())
            .filter_map(|v| serde_json::from_value::<WritersSetting>(v).ok())
            .map(|w| (w.halp_writers, w.actors))
            .collect();
        let ev = NewEvent {
            actor: ctx.actor.to_string(),
            command: command.to_string(),
            entity: done.entity,
            summary,
            args,
            changes,
            undoable,
        };
        let now = self.now();
        match self.store.commit(ev, now) {
            Ok(event) => {
                self.writers.extend(writers);
                let _ = self.events.send(event);
                Ok(done.result)
            }
            Err(e) => {
                self.reload_or_fail()?;
                Err(e.into())
            }
        }
    }

    pub(crate) fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    pub(crate) fn view(&self, id: &str) -> Result<ItemView, CmdError> {
        self.outline
            .view(id)
            .ok_or_else(|| CmdError::not_found(format!("item {id} not found")))
    }

    pub(crate) fn role(&self, id: Option<&str>) -> Result<&Role, CmdError> {
        let id = id.unwrap_or(&self.current_role);
        self.roles
            .iter()
            .find(|r| r.id == id)
            .ok_or_else(|| CmdError::not_found(format!("role {id} not found")))
    }

    /// Validates a `roles` list: known role ids, de-duplicated, at most 8.
    pub(crate) fn check_roles(&self, roles: &[String]) -> Result<Vec<String>, CmdError> {
        let mut out: Vec<String> = Vec::new();
        for r in roles {
            if !self.roles.iter().any(|x| x.id == *r) {
                return Err(CmdError::invalid(format!("unknown role {r}")));
            }
            if !out.contains(r) {
                out.push(r.clone());
            }
        }
        if out.len() > 8 {
            return Err(CmdError::invalid("at most 8 roles per item"));
        }
        Ok(out)
    }

    pub(crate) fn layout_for(&self, role_id: Option<&str>) -> Result<&Layout, CmdError> {
        let role = self.role(role_id)?;
        self.layouts
            .get(&role.layout_id)
            .ok_or_else(|| CmdError::not_found(format!("layout {} not found", role.layout_id)))
    }

    /// The current role's focused item, if it still exists.
    pub(crate) fn focus_view(&self) -> Option<ItemView> {
        self.focus
            .get(&self.current_role)
            .and_then(|id| self.outline.view(id))
    }

    /// Sets or clears `role`'s focus, recording the setting change.
    pub(crate) fn set_focus(&mut self, tx: &mut Tx, role: &str, id: Option<String>) {
        let before = self.focus.get(role).cloned().map(Value::String);
        let after = id.clone().map(Value::String);
        tx.record(EntityKind::Setting, &focus_key(role), before, after);
        match id {
            Some(i) => self.focus.insert(role.to_string(), i),
            None => self.focus.remove(role),
        };
    }

    /// Records an audit-only event (no entity changes) and broadcasts it.
    pub(crate) fn audit(
        &mut self,
        actor: &str,
        command: &str,
        entity: EntityRef,
        summary: String,
        args: Value,
    ) -> Result<Event, CmdError> {
        let ev = NewEvent {
            actor: actor.to_string(),
            command: command.to_string(),
            entity,
            summary,
            args,
            changes: Vec::new(),
            undoable: false,
        };
        let now = self.now();
        let event = self.store.commit(ev, now)?;
        self.emit(event.clone());
        Ok(event)
    }
}

/// The class of items whose task type is not loaded (fail closed).
pub(crate) const UNKNOWN_TYPE_CLASS: &str = "secrets";

/// [`State::validate_item_fields`] against given registries (`base`: the
/// registry without dev packs, while dev mode is on). Undo and redo use it
/// on the registry their step would leave.
pub(crate) fn validate_item_fields_in(
    types: &TaskTypeRegistry,
    base: Option<&TaskTypeRegistry>,
    type_id: Option<&str>,
    fields: &serde_json::Map<String, Value>,
) -> Result<(), CmdError> {
    types.validate_fields(type_id, fields)?;
    let (Some(base), Some(t)) = (base, type_id) else {
        return Ok(());
    };
    let dev_owned = |o: &TaskTypeOrigin| {
        matches!(
            o,
            TaskTypeOrigin::Pack {
                trust: halp_domain::PackTrust::Dev,
                ..
            } | TaskTypeOrigin::Retired {
                trust: halp_domain::PackTrust::Dev,
                ..
            }
        )
    };
    // User types are the same in both registries (and may change in
    // place); only pack and built-in types can carry a dev override.
    if let Some((_, o)) = base.get(t)
        && !dev_owned(o)
        && !matches!(o, TaskTypeOrigin::User)
    {
        base.validate_fields(type_id, fields).map_err(|e| {
            CmdError::invalid(format!(
                "{} (a dev pack's override cannot be relied on by this type's items: dev mode is not saved)",
                CmdError::from(e).message
            ))
        })?;
    }
    Ok(())
}
