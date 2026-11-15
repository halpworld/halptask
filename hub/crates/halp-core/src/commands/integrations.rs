//! Integrations: configuration (user only), status, manual syncs, local git
//! context capture, and the write path synced items take (`ext.upsert`).
//!
//! The runtime that talks to the outside world (`halp-integrations`) is
//! attached as a [`HostHandle`] extension; without it (halpd, tests)
//! `integrations.list` reports `stopped` and the commands that need the
//! runtime fail with `invalid_args`.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use chrono::{DateTime, NaiveDate, Utc};
use halp_domain::integration::{
    ARCHIVED_TAG, CaptureTicket, DEFAULT_INTERVAL_SECS, HostHandle, INTEGRATION_KINDS,
    IntegrationConfig, IntegrationHost, IntegrationState, IntegrationStatus, MAX_EXTERNAL_ID_CHARS,
    MAX_INTERVAL_SECS, MAX_SETTINGS_BYTES, MAX_UPSERT_ITEMS, MIN_INTERVAL_SECS, clean_json,
    strip_controls,
};
use halp_domain::{Item, ItemKind, Status, limits};
use halp_policy::secret;
use halp_store::EntityKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::envelope::{CmdError, ErrorCode};
use crate::policy::{Actor, Ctx, Tier};
use crate::registry::Registry;
use crate::state::{Done, State, Tx, entity, short};

/// The role synced items get when `connect` names none.
const DEFAULT_ROLES: &[&str] = &["developer"];
/// Longest status error text kept.
const MAX_STATUS_ERROR_CHARS: usize = 300;
/// The external id of an integration's root bullet.
const ROOT_EXTERNAL_ID: &str = "__root__";

/// The link between an external object and the item it was synced to
/// (entity `external_link`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct ExternalLink {
    integration: String,
    external_id: String,
    item_id: String,
    scope: String,
    /// SHA-256 of the normalised remote content last applied.
    hash: String,
    archived: bool,
    updated_at: DateTime<Utc>,
}

/// The store id of a link: a hash, so any external id is a safe key.
fn link_id(integration: &str, external_id: &str) -> String {
    secret::token_hash(&format!("{integration}\u{0}{external_id}"))[..40].to_string()
}

pub(crate) fn register(r: &mut Registry) {
    use Tier::*;
    r.add(
        "integrations.list",
        "List configured integrations (GitHub, MCP servers, local git) with their sync status.",
        T0,
        list,
    );
    r.add(
        "integrations.sync_now",
        "Ask integrations to sync now (one by id, or all enabled ones).",
        T1,
        sync_now,
    );
    r.add(
        "integrations.connect",
        "Add or change an integration (kind, settings, roles, interval). User only; tokens are entered in the app, never passed here.",
        T2,
        connect,
    );
    r.add(
        "integrations.disconnect",
        "Remove an integration and its stored token; optionally delete the items it synced. User only.",
        T3,
        disconnect,
    );
    r.add(
        "context.capture",
        "Snapshot a local git repository (branch, recently changed files, last commit subject) as a dev.context item. The path must be under a configured root.",
        T1,
        capture,
    );
    r.add(
        "ext.upsert",
        "Integration write path: upsert synced items by external id. Integrations only.",
        T1,
        upsert,
    );
    r.add(
        "ext.status",
        "Integration write path: report a status change. Integrations only.",
        T1,
        status,
    );
    r.add(
        "ext.patch_fields",
        "Integration write path: change fields a pack type marks `x-halp-ext-writable` on an item this integration created. Integrations only.",
        T1,
        patch_fields,
    );
    r.user_only("integrations.connect");
    r.user_only("integrations.disconnect");
    r.hidden("ext.upsert");
    r.hidden("ext.status");
    r.hidden("ext.patch_fields");
}

fn host(st: &State) -> Option<Arc<dyn IntegrationHost>> {
    st.extension::<HostHandle>().map(|h| h.0.clone())
}

fn need_host(st: &State) -> Result<Arc<dyn IntegrationHost>, CmdError> {
    host(st).ok_or_else(|| {
        CmdError::invalid("the integration runtime is not running in this process (use the app)")
    })
}

fn load_config(st: &State, id: &str) -> Result<Option<IntegrationConfig>, CmdError> {
    match st.store.get(EntityKind::Integration, id)? {
        Some(v) => Ok(Some(serde_json::from_value(v)?)),
        None => Ok(None),
    }
}

fn require_config(st: &State, id: &str) -> Result<IntegrationConfig, CmdError> {
    load_config(st, id)?.ok_or_else(|| CmdError::not_found(format!("integration {id} not found")))
}

fn links_of(st: &State, integration: &str) -> Result<BTreeMap<String, ExternalLink>, CmdError> {
    let all: Vec<ExternalLink> = st.store.load(EntityKind::ExternalLink)?;
    Ok(all
        .into_iter()
        .filter(|l| l.integration == integration)
        .map(|l| (link_id(&l.integration, &l.external_id), l))
        .collect())
}

fn denied(m: impl Into<String>) -> CmdError {
    CmdError::new(ErrorCode::Denied, m)
}

// ---------------------------------------------------------------- list

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListArgs {}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct IntegrationEntry {
    pub config: IntegrationConfig,
    pub status: IntegrationStatus,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ListResult {
    /// Whether the integration runtime runs in this process.
    pub running: bool,
    pub integrations: Vec<IntegrationEntry>,
}

pub fn list(st: &mut State, _ctx: &Ctx, _a: ListArgs) -> Result<ListResult, CmdError> {
    let h = host(st);
    let configs: Vec<IntegrationConfig> = st.store.load(EntityKind::Integration)?;
    let integrations = configs
        .into_iter()
        .map(|config| {
            let status = match &h {
                None => IntegrationStatus {
                    state: IntegrationState::Stopped,
                    ..Default::default()
                },
                Some(h) => h.status(&config.id).unwrap_or_else(|| IntegrationStatus {
                    state: if config.enabled {
                        IntegrationState::Pending
                    } else {
                        IntegrationState::Disabled
                    },
                    ..Default::default()
                }),
            };
            IntegrationEntry { config, status }
        })
        .collect();
    Ok(ListResult {
        running: h.is_some(),
        integrations,
    })
}

// ---------------------------------------------------------------- connect

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConnectArgs {
    /// `[A-Za-z0-9_.-]{1,64}`. Connecting an existing id changes it.
    pub id: String,
    /// `github`, `mcp` or `git`. Cannot change for an existing id.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Polling interval, 60..=86400 seconds (default 300).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u32>,
    /// Roles synced items belong to (default `["developer"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
    /// Kind-specific settings (see docs/INTEGRATIONS.md). Never secrets:
    /// tokens go to the OS keychain through the app.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<Value>,
    /// Start a fresh authorization (OAuth) even if a token is stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reauthorize: Option<bool>,
}

pub fn connect(st: &mut State, ctx: &Ctx, a: ConnectArgs) -> Result<IntegrationEntry, CmdError> {
    limits::simple_id("integration id", &a.id)?;
    if !INTEGRATION_KINDS.contains(&a.kind.as_str()) {
        return Err(CmdError::invalid(format!(
            "unknown integration kind {} (one of {})",
            a.kind,
            INTEGRATION_KINDS.join(", ")
        )));
    }
    let h = need_host(st)?;
    let existing = load_config(st, &a.id)?;
    if let Some(e) = &existing
        && e.kind != a.kind
    {
        return Err(CmdError::conflict(format!(
            "integration {} is a {} integration; disconnect it first",
            a.id, e.kind
        )));
    }
    let now = st.now();
    let title = match &a.title {
        Some(t) => limits::label("title", t, 100)?,
        None => existing
            .as_ref()
            .map(|e| e.title.clone())
            .unwrap_or_else(|| a.id.clone()),
    };
    let interval_secs = a
        .interval_secs
        .or(existing.as_ref().map(|e| e.interval_secs))
        .unwrap_or(DEFAULT_INTERVAL_SECS);
    if !(MIN_INTERVAL_SECS..=MAX_INTERVAL_SECS).contains(&interval_secs) {
        return Err(CmdError::invalid(format!(
            "interval_secs must be {MIN_INTERVAL_SECS}..={MAX_INTERVAL_SECS}"
        )));
    }
    let roles = match &a.roles {
        Some(r) => st.check_roles(r)?,
        None => match &existing {
            Some(e) => e.roles.clone(),
            None => st.check_roles(
                &DEFAULT_ROLES
                    .iter()
                    .map(|s| s.to_string())
                    .filter(|r| st.roles.iter().any(|x| x.id == *r))
                    .collect::<Vec<_>>(),
            )?,
        },
    };
    let raw_settings = match &a.settings {
        Some(s) => s.clone(),
        None => existing
            .as_ref()
            .map(|e| e.settings.clone())
            .unwrap_or_else(|| json!({})),
    };
    if !raw_settings.is_object() {
        return Err(CmdError::invalid("settings must be an object"));
    }
    limits::json_bounds("settings", &raw_settings, MAX_SETTINGS_BYTES, 8)?;
    let settings = h
        .validate(&a.kind, &raw_settings)
        .map_err(|e| CmdError::invalid(format!("settings: {e}")))?;
    let cfg = IntegrationConfig {
        id: a.id.clone(),
        kind: a.kind.clone(),
        title,
        enabled: a
            .enabled
            .or(existing.as_ref().map(|e| e.enabled))
            .unwrap_or(true),
        interval_secs,
        roles,
        settings,
        created_at: existing.as_ref().map(|e| e.created_at).unwrap_or(now),
        updated_at: now,
    };
    let before = existing.as_ref().map(serde_json::to_value).transpose()?;
    let after = serde_json::to_value(&cfg)?;
    let verb = if existing.is_some() {
        "Changed"
    } else {
        "Connected"
    };
    let summary = format!(
        "{verb} integration \"{}\" ({})",
        short(&cfg.title),
        cfg.kind
    );
    let unchanged = before.as_ref().is_some_and(|b| {
        // `updated_at` alone does not count as a change.
        let mut b = b.clone();
        let mut x = after.clone();
        b.as_object_mut().map(|m| m.remove("updated_at"));
        x.as_object_mut().map(|m| m.remove("updated_at"));
        b == x
    });
    if !unchanged {
        let id = cfg.id.clone();
        st.write(ctx, "integrations.connect", &a, false, |_st, tx| {
            tx.record(EntityKind::Integration, &id, before, Some(after));
            Ok(Done {
                result: (),
                entity: entity("integration", &id),
                summary,
            })
        })?;
    }
    let cfg = require_config(st, &a.id)?;
    h.configured(&cfg, a.reauthorize.unwrap_or(false));
    let status = h.status(&cfg.id).unwrap_or_default();
    Ok(IntegrationEntry {
        config: cfg,
        status,
    })
}

// ---------------------------------------------------------------- disconnect

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisconnectArgs {
    pub id: String,
    /// Also delete every item the integration synced (default false: they
    /// stay as ordinary items).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remove_items: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DisconnectResult {
    pub id: String,
    pub removed_items: usize,
}

pub fn disconnect(
    st: &mut State,
    ctx: &Ctx,
    a: DisconnectArgs,
) -> Result<DisconnectResult, CmdError> {
    let cfg = require_config(st, &a.id)?;
    let links = links_of(st, &cfg.id)?;
    let remove = a.remove_items.unwrap_or(false);
    let summary = format!("Disconnected integration \"{}\"", short(&cfg.title));
    let before = serde_json::to_value(&cfg)?;
    let id = cfg.id.clone();
    let removed = st.write(ctx, "integrations.disconnect", &a, false, |st, tx| {
        tx.record(EntityKind::Integration, &id, Some(before), None);
        let mut removed = 0;
        for (lid, l) in &links {
            tx.record(
                EntityKind::ExternalLink,
                lid,
                Some(serde_json::to_value(l)?),
                None,
            );
        }
        if remove {
            // Children first would be redundant: removing a subtree removes
            // what is below it; skip ids already gone.
            for l in links.values() {
                if l.external_id == ROOT_EXTERNAL_ID {
                    continue;
                }
                if st.outline.get(&l.item_id).is_some() {
                    removed += st.outline.remove_subtree(&l.item_id)?.len();
                }
            }
            if let Some(root) = links.values().find(|l| l.external_id == ROOT_EXTERNAL_ID)
                && st.outline.get(&root.item_id).is_some()
                && st.outline.children(&root.item_id).is_empty()
            {
                removed += st.outline.remove_subtree(&root.item_id)?.len();
            }
        }
        Ok(Done {
            result: removed,
            entity: entity("integration", &id),
            summary,
        })
    })?;
    if let Some(h) = host(st) {
        h.removed(&cfg.id);
    }
    Ok(DisconnectResult {
        id: cfg.id,
        removed_items: removed,
    })
}

// ---------------------------------------------------------------- sync_now

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncNowArgs {
    /// One integration; all enabled ones when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct SyncNowResult {
    /// Integrations a sync was queued for.
    pub queued: Vec<String>,
}

pub fn sync_now(st: &mut State, _ctx: &Ctx, a: SyncNowArgs) -> Result<SyncNowResult, CmdError> {
    let h = need_host(st)?;
    if let Some(id) = &a.id {
        let cfg = require_config(st, id)?;
        if !cfg.enabled {
            return Err(CmdError::invalid(format!("integration {id} is disabled")));
        }
    }
    let queued = h
        .sync_now(a.id.as_deref())
        .map_err(|e| CmdError::invalid(strip_controls(&e)))?;
    Ok(SyncNowResult { queued })
}

// ---------------------------------------------------------------- context.capture

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureArgs {
    /// A git work tree under one of the `git` integration's roots.
    pub repo_path: String,
    /// The `git` integration to use (default: the first enabled one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration: Option<String>,
}

pub fn capture(st: &mut State, _ctx: &Ctx, a: CaptureArgs) -> Result<CaptureTicket, CmdError> {
    if a.repo_path.is_empty() || a.repo_path.len() > 4096 || a.repo_path.contains('\0') {
        return Err(CmdError::invalid("repo_path must be 1..4096 bytes"));
    }
    let h = need_host(st)?;
    let configs: Vec<IntegrationConfig> = st.store.load(EntityKind::Integration)?;
    let cfg = configs
        .into_iter()
        .filter(|c| c.kind == "git" && c.enabled)
        .find(|c| a.integration.as_ref().is_none_or(|id| *id == c.id))
        .ok_or_else(|| {
            CmdError::not_found("no enabled git integration (connect one with repository roots)")
        })?;
    h.capture_context(&cfg, &a.repo_path)
        .map_err(|e| CmdError::invalid(strip_controls(&e)))
}

// ---------------------------------------------------------------- ext.status

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {
    pub state: IntegrationState,
    /// Redacted error text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct StatusResult {
    pub recorded: bool,
}

/// The integration reports a state change; recorded as an audit event
/// (`integrations.status`) so the UI and the activity feed see it.
pub fn status(st: &mut State, ctx: &Ctx, a: StatusArgs) -> Result<StatusResult, CmdError> {
    let Actor::Integration(id) = &ctx.actor else {
        return Err(denied("ext.status is for integrations only"));
    };
    let cfg = require_config(st, id)?;
    let error = a.error.as_deref().map(|e| {
        let e = strip_controls(e);
        let mut t: String = e.chars().take(MAX_STATUS_ERROR_CHARS).collect();
        if e.chars().count() > MAX_STATUS_ERROR_CHARS {
            t.push('…');
        }
        t
    });
    let summary = match &error {
        Some(e) => format!("{}: {} ({})", short(&cfg.title), a.state.as_str(), short(e)),
        None => format!("{}: {}", short(&cfg.title), a.state.as_str()),
    };
    st.audit(
        &ctx.actor.to_string(),
        "integrations.status",
        entity("integration", &cfg.id),
        summary,
        json!({ "id": cfg.id, "state": a.state, "error": error }),
    )?;
    Ok(StatusResult { recorded: true })
}

// ---------------------------------------------------------------- ext.upsert

/// One external object as an item.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtItem {
    /// Stable remote id, e.g. `github:owner/repo#123` (at most 300 chars).
    pub external_id: String,
    pub title: String,
    /// `task` (default) or `bullet`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Applied to leaf tasks only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Status>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Map<String, Value>>,
    /// Roles this item belongs to: a non-empty subset of the integration's
    /// configured `roles` (default: all of them). Lets one connector put
    /// each item type in its own pack's view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpsertArgs {
    /// Groups items for `complete` (e.g. `prs`, `ci`, `issues`).
    pub scope: String,
    /// The batch is the whole remote set for this scope: linked items
    /// missing from it are archived.
    #[serde(default)]
    pub complete: bool,
    pub items: Vec<ExtItem>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Rejected {
    pub external_id: String,
    pub reason: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct Synced {
    pub external_id: String,
    pub item_id: String,
}

#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct UpsertResult {
    pub created: u32,
    pub updated: u32,
    pub unchanged: u32,
    pub archived: u32,
    pub restored: u32,
    /// Deleted by the user and unchanged remotely: left deleted.
    pub skipped: u32,
    pub rejected: Vec<Rejected>,
    pub items: Vec<Synced>,
    /// Set when a `complete: true` batch was treated as incomplete (nothing
    /// was archived) because an item had no usable external_id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// What an [`ExtItem`] normalises to; its hash decides whether to write.
#[derive(Debug, Clone, Serialize)]
struct Norm {
    title: String,
    kind: ItemKind,
    type_id: Option<String>,
    notes: String,
    status: Option<Status>,
    tags: Vec<String>,
    due: Option<NaiveDate>,
    fields: Map<String, Value>,
    /// Only when the item names its own roles, so the hash of items that
    /// do not is what it was before per-item roles existed.
    #[serde(skip_serializing_if = "Option::is_none")]
    roles: Option<Vec<String>>,
}

/// An item's own `roles`: de-duplicated, non-empty, and every one of them
/// among the integration's configured roles. The user decides where an
/// integration's items show up (`integrations.connect` is user only); an
/// item can narrow that, never widen it.
fn item_roles(
    raw: &Option<Vec<String>>,
    allowed: &[String],
) -> Result<Option<Vec<String>>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    if raw.is_empty() {
        return Err("roles must not be empty (leave it out for the integration's roles)".into());
    }
    let mut out: Vec<String> = Vec::new();
    for r in raw {
        if !allowed.contains(r) {
            return Err(format!(
                "role {} is not one of the integration's roles",
                strip_controls(r).chars().take(64).collect::<String>()
            ));
        }
        if !out.contains(r) {
            out.push(r.clone());
        }
    }
    Ok(Some(out))
}

fn normalise(st: &State, it: &ExtItem, allowed_roles: &[String]) -> Result<Norm, String> {
    let title = limits::sanitize_title(&strip_controls(&it.title));
    if title.is_empty() {
        return Err("empty title".into());
    }
    let notes = it
        .notes
        .as_deref()
        .map(|n| {
            // Keep line breaks; drop other controls and bidi overrides.
            let kept: String = n.lines().map(strip_controls).collect::<Vec<_>>().join("\n");
            limits::sanitize_notes(&kept)
        })
        .unwrap_or_default();
    let mut tags = Vec::new();
    for t in it.tags.iter().flatten() {
        if let Some(t) = limits::sanitize_tag(&strip_controls(t))
            && !tags.contains(&t)
            && t != ARCHIVED_TAG
        {
            tags.push(t);
        }
    }
    tags.truncate(limits::MAX_TAGS_PER_ITEM - 1);
    let due = match &it.due {
        Some(d) => {
            Some(NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| "due must be YYYY-MM-DD")?)
        }
        None => None,
    };
    let fields = match &it.fields {
        Some(f) => match clean_json(&Value::Object(f.clone())) {
            Value::Object(m) => m,
            _ => Map::new(),
        },
        None => Map::new(),
    };
    let type_id = it.type_id.clone();
    // Integrations write only `general` items: a type with its own data
    // class (a private note) would put remote content into private data.
    if st.type_class(type_id.as_deref()) != crate::policy::DEFAULT_DATA_CLASS {
        return Err("this type cannot be written by integrations".into());
    }
    st.validate_item_fields(type_id.as_deref(), &fields)
        .map_err(|e| e.message)?;
    let kind = it.kind.unwrap_or(ItemKind::Task);
    let status = if kind == ItemKind::Bullet {
        None
    } else {
        it.status
    };
    let roles = item_roles(&it.roles, allowed_roles)?;
    Ok(Norm {
        title,
        kind,
        type_id,
        notes,
        status,
        tags,
        due,
        fields,
        roles,
    })
}

fn norm_hash(n: &Norm) -> Result<String, CmdError> {
    Ok(secret::token_hash(&secret::canonical_json(
        &serde_json::to_value(n)?,
    )))
}

/// What the event records: counts only, never remote content.
#[derive(Serialize)]
struct UpsertEventArgs<'a> {
    scope: &'a str,
    complete: bool,
    received: usize,
}

pub fn upsert(st: &mut State, ctx: &Ctx, a: UpsertArgs) -> Result<UpsertResult, CmdError> {
    let Actor::Integration(iid) = &ctx.actor else {
        return Err(denied("ext.upsert is for integrations only"));
    };
    let iid = iid.clone();
    let cfg = require_config(st, &iid)?;
    if !cfg.enabled {
        return Err(denied(format!("integration {iid} is disabled")));
    }
    limits::simple_id("scope", &a.scope)?;
    if a.items.len() > MAX_UPSERT_ITEMS {
        return Err(CmdError::invalid(format!(
            "at most {MAX_UPSERT_ITEMS} items per call"
        )));
    }

    let mut res = UpsertResult::default();
    let mut batch: Vec<(String, Norm, String)> = Vec::new();
    let mut seen = BTreeSet::new();
    // Links of every item that has a usable external_id, valid or not: an
    // item the remote still lists must never be archived as "gone".
    let mut named: BTreeSet<String> = BTreeSet::new();
    let mut unnamed = false;
    for it in &a.items {
        let eid = &it.external_id;
        let bad = |r: &str| Rejected {
            external_id: strip_controls(eid)
                .chars()
                .take(MAX_EXTERNAL_ID_CHARS)
                .collect(),
            reason: r.to_string(),
        };
        if eid.is_empty()
            || eid.chars().count() > MAX_EXTERNAL_ID_CHARS
            || eid.chars().any(char::is_control)
            || eid == ROOT_EXTERNAL_ID
        {
            res.rejected.push(bad("invalid external_id"));
            unnamed = true;
            continue;
        }
        named.insert(link_id(&iid, eid));
        if !seen.insert(eid.clone()) {
            res.rejected.push(bad("duplicate external_id in batch"));
            continue;
        }
        match normalise(st, it, &cfg.roles) {
            Ok(n) => {
                let h = norm_hash(&n)?;
                batch.push((eid.clone(), n, h));
            }
            Err(e) => res.rejected.push(bad(&strip_controls(&e))),
        }
    }

    let links = links_of(st, &iid)?;
    let origin = ctx.actor.to_string();
    let roles = cfg.roles.clone();
    let title = cfg.title.clone();
    let ev_args = UpsertEventArgs {
        scope: &a.scope,
        complete: a.complete,
        received: a.items.len(),
    };
    let scope = a.scope.clone();
    let mut complete = a.complete;
    if complete && unnamed {
        complete = false;
        res.note = Some(
            "an item had no usable external_id, so this batch was treated as incomplete and nothing was archived"
                .to_string(),
        );
    }
    // Immutable items (an approved evidence file, v1.9) are reported per
    // item instead of failing the whole batch.
    let locks = crate::immutable::Locks::of(st);
    let immutable = |eid: &str, why: &str| Rejected {
        external_id: strip_controls(eid)
            .chars()
            .take(MAX_EXTERNAL_ID_CHARS)
            .collect(),
        reason: why.to_string(),
    };
    st.write(ctx, "ext.upsert", &ev_args, false, |st, tx| {
        let now = st.now();
        let root = ensure_root(st, tx, &iid, &title, &roles, &origin, &links, now)?;
        // Same rule as ext.patch_fields: an item that is outside the
        // `general` class now (the user moved it, or the root, under a
        // private parent) is not written. Counted as skipped, no reason.
        let general = |st: &State, id: &str| {
            st.item_class(id).as_deref() == Some(crate::policy::DEFAULT_DATA_CLASS)
        };
        let root_general = general(st, &root);
        let mut touched = named;
        for (eid, n, h) in batch {
            let lid = link_id(&iid, &eid);
            touched.insert(lid.clone());
            let prev = links.get(&lid);
            let live = prev.filter(|l| st.outline.get(&l.item_id).is_some());
            if live.is_some_and(|l| !general(st, &l.item_id)) || (live.is_none() && !root_general) {
                res.skipped += 1;
                continue;
            }
            let locked_now = prev
                .and_then(|l| st.outline.get(&l.item_id))
                .is_some_and(|it| locks.fields_locked(it.type_id.as_deref(), &it.fields));
            if locked_now && prev.is_some_and(|l| l.hash != h || l.archived) {
                res.rejected.push(immutable(
                    &eid,
                    "the item is immutable in the Hub (for example approved); it was not changed",
                ));
                continue;
            }
            if !locked_now && locks.fields_locked(n.type_id.as_deref(), &n.fields) {
                res.rejected.push(immutable(
                    &eid,
                    "only the user can put an item into its immutable state (for example approved)",
                ));
                continue;
            }
            let item_id = match prev {
                Some(l) if st.outline.get(&l.item_id).is_some() => {
                    let was_archived = l.archived;
                    if l.hash != h {
                        apply(st, &l.item_id, &n, now)?;
                        res.updated += 1;
                    } else {
                        res.unchanged += 1;
                    }
                    if was_archived {
                        restore(st, &l.item_id, &n, now)?;
                        res.restored += 1;
                    }
                    l.item_id.clone()
                }
                Some(l) if l.hash == h => {
                    // The user deleted it and nothing changed remotely.
                    res.skipped += 1;
                    if !l.archived && l.scope == scope {
                        continue;
                    }
                    let mut nl = l.clone();
                    nl.archived = false;
                    nl.scope = scope.clone();
                    nl.updated_at = now;
                    record_link(tx, &lid, Some(l), &nl)?;
                    continue;
                }
                _ => {
                    let id = create(st, &n, &root, &roles, &origin, now)?;
                    res.created += 1;
                    id
                }
            };
            let nl = ExternalLink {
                integration: iid.clone(),
                external_id: eid.clone(),
                item_id: item_id.clone(),
                scope: scope.clone(),
                hash: h,
                archived: false,
                updated_at: prev.map(|p| p.updated_at).unwrap_or(now),
            };
            let nl = if prev.is_some_and(|p| {
                p.item_id == nl.item_id && p.hash == nl.hash && !p.archived && p.scope == nl.scope
            }) {
                nl
            } else {
                ExternalLink {
                    updated_at: now,
                    ..nl
                }
            };
            record_link(tx, &lid, prev, &nl)?;
            res.items.push(Synced {
                external_id: eid,
                item_id,
            });
        }
        if complete {
            for (lid, l) in &links {
                if l.scope != scope
                    || l.archived
                    || l.external_id == ROOT_EXTERNAL_ID
                    || touched.contains(lid)
                {
                    continue;
                }
                if st
                    .outline
                    .get(&l.item_id)
                    .is_some_and(|it| locks.fields_locked(it.type_id.as_deref(), &it.fields))
                    || (st.outline.get(&l.item_id).is_some() && !general(st, &l.item_id))
                {
                    // Gone remotely, but immutable here: left as it is.
                    continue;
                }
                if st.outline.get(&l.item_id).is_some() {
                    archive(st, &l.item_id, now)?;
                    res.archived += 1;
                }
                let mut nl = l.clone();
                nl.archived = true;
                nl.updated_at = now;
                record_link(tx, lid, Some(l), &nl)?;
            }
        }
        let summary = format!(
            "Synced {}: {} new, {} updated, {} archived",
            short(&title),
            res.created,
            res.updated,
            res.archived
        );
        Ok(Done {
            result: (),
            entity: entity("integration", &iid),
            summary,
        })
    })?;
    Ok(res)
}

fn record_link(
    tx: &mut Tx,
    lid: &str,
    prev: Option<&ExternalLink>,
    next: &ExternalLink,
) -> Result<(), CmdError> {
    let before = prev.map(serde_json::to_value).transpose()?;
    let after = serde_json::to_value(next)?;
    if before.as_ref() != Some(&after) {
        tx.record(EntityKind::ExternalLink, lid, before, Some(after));
    }
    Ok(())
}

/// The bullet synced items go under; recreated if the user deleted it.
#[allow(clippy::too_many_arguments)]
fn ensure_root(
    st: &mut State,
    tx: &mut Tx,
    iid: &str,
    title: &str,
    roles: &[String],
    origin: &str,
    links: &BTreeMap<String, ExternalLink>,
    now: DateTime<Utc>,
) -> Result<String, CmdError> {
    let lid = link_id(iid, ROOT_EXTERNAL_ID);
    let prev = links.get(&lid);
    if let Some(l) = prev
        && st.outline.get(&l.item_id).is_some()
    {
        return Ok(l.item_id.clone());
    }
    let mut item = Item::new(ItemKind::Bullet, limits::sanitize_title(title), now);
    if item.title.is_empty() {
        item.title = iid.to_string();
    }
    item.origin = origin.to_string();
    item.roles = roles.to_vec();
    let id = item.id.clone();
    st.outline.insert(item, None, None)?;
    let l = ExternalLink {
        integration: iid.to_string(),
        external_id: ROOT_EXTERNAL_ID.to_string(),
        item_id: id.clone(),
        scope: ROOT_EXTERNAL_ID.to_string(),
        hash: String::new(),
        archived: false,
        updated_at: now,
    };
    record_link(tx, &lid, prev, &l)?;
    Ok(id)
}

fn create(
    st: &mut State,
    n: &Norm,
    root: &str,
    roles: &[String],
    origin: &str,
    now: DateTime<Utc>,
) -> Result<String, CmdError> {
    let mut item = Item::new(n.kind, n.title.clone(), now);
    item.origin = origin.to_string();
    item.notes = n.notes.clone();
    item.tags = n.tags.clone();
    item.due = n.due;
    item.type_id = n.type_id.clone();
    item.fields = n.fields.clone();
    item.roles = n.roles.clone().unwrap_or_else(|| roles.to_vec());
    if let Some(s) = n.status {
        item.status = s;
    }
    let id = item.id.clone();
    st.outline.insert(item, Some(root), None)?;
    Ok(id)
}

/// Overwrites the remote-owned parts of an item (title, notes, due, type,
/// fields), unions tags and sets the status of leaf tasks.
fn apply(st: &mut State, id: &str, n: &Norm, now: DateTime<Utc>) -> Result<(), CmdError> {
    let leaf_task = st.outline.get(id).is_some_and(Item::is_task) && !st.outline.has_subtasks(id);
    st.outline.update(id, now, |it| {
        it.title = n.title.clone();
        it.notes = n.notes.clone();
        it.due = n.due;
        it.type_id = n.type_id.clone();
        it.fields = n.fields.clone();
        if let Some(r) = &n.roles {
            it.roles = r.clone();
        }
        for t in &n.tags {
            if !it.tags.contains(t) && it.tags.len() < limits::MAX_TAGS_PER_ITEM {
                it.tags.push(t.clone());
            }
        }
        if leaf_task && let Some(s) = n.status {
            it.status = s;
        }
    })?;
    Ok(())
}

/// Removed remotely: tagged `archived` and (leaf tasks) done. Never deleted.
fn archive(st: &mut State, id: &str, now: DateTime<Utc>) -> Result<(), CmdError> {
    let leaf_task = st.outline.get(id).is_some_and(Item::is_task) && !st.outline.has_subtasks(id);
    st.outline.update(id, now, |it| {
        if !it.tags.iter().any(|t| t == ARCHIVED_TAG) {
            if it.tags.len() >= limits::MAX_TAGS_PER_ITEM {
                it.tags.pop();
            }
            it.tags.push(ARCHIVED_TAG.to_string());
        }
        if leaf_task {
            it.status = Status::Done;
        }
    })?;
    Ok(())
}

/// Back remotely: the `archived` tag goes, the status is the remote one
/// (or todo).
fn restore(st: &mut State, id: &str, n: &Norm, now: DateTime<Utc>) -> Result<(), CmdError> {
    let leaf_task = st.outline.get(id).is_some_and(Item::is_task) && !st.outline.has_subtasks(id);
    st.outline.update(id, now, |it| {
        it.tags.retain(|t| t != ARCHIVED_TAG);
        if leaf_task {
            it.status = n.status.unwrap_or(Status::Todo);
        }
    })?;
    Ok(())
}

// ---------------------------------------------------------------- ext.patch_fields

/// The schema key a type uses to let integrations change some of its fields
/// on items they created (`"x-halp-ext-writable": ["scheduled_for"]`).
pub(crate) const EXT_WRITABLE_KEY: &str = "x-halp-ext-writable";
/// Most fields one patch may name.
const MAX_PATCH_FIELDS: usize = 16;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PatchFieldsArgs {
    /// The item to change.
    pub id: String,
    /// The item's type, which must be its current type.
    pub type_id: String,
    /// Fail with `conflict` unless the item is still at this version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub if_version: Option<u64>,
    /// Fields to set (merged into the item's fields; `null` removes one).
    /// Every name must be listed in the type's `x-halp-ext-writable`.
    pub fields: Map<String, Value>,
}

/// The fields a type lets integrations write. Only built-in types and
/// types of trusted packs count (`may_annotate`): a user or agent defined
/// type, or one from an unsigned or dev pack, cannot open fields to
/// integrations.
fn ext_writable(st: &State, type_id: &str) -> Option<Vec<String>> {
    let (t, origin) = st.types.get(type_id)?;
    if !origin.may_annotate() {
        return None;
    }
    let list = t.schema.get(EXT_WRITABLE_KEY)?.as_array()?;
    Some(
        list.iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect(),
    )
}

/// `ext.patch_fields`: an integration changes whitelisted fields of an item
/// it created itself. The item must have the integration as its `origin`
/// **and** be linked to it (an `external_link` of this integration points
/// at it), so user- and agent-created items are never touched, nor items of
/// another integration. Only `general`-class items. The merged fields are
/// validated against the type schema, and the change is a normal, undoable
/// history event as `integration:<id>`.
pub fn patch_fields(
    st: &mut State,
    ctx: &Ctx,
    a: PatchFieldsArgs,
) -> Result<halp_domain::ItemView, CmdError> {
    let Actor::Integration(iid) = &ctx.actor else {
        return Err(denied("ext.patch_fields is for integrations only"));
    };
    let iid = iid.clone();
    let cfg = require_config(st, &iid)?;
    if !cfg.enabled {
        return Err(denied(format!("integration {iid} is disabled")));
    }
    let not_found = || CmdError::not_found(format!("item {} not found", a.id));
    let cur = st.outline.get(&a.id).cloned().ok_or_else(not_found)?;
    if cur.type_id.as_deref() != Some(a.type_id.as_str()) {
        return Err(not_found());
    }
    // A private item does not exist for an integration.
    if st.item_class(&a.id).as_deref() != Some(crate::policy::DEFAULT_DATA_CLASS) {
        return Err(not_found());
    }
    let origin = ctx.actor.to_string();
    let linked = links_of(st, &iid)?
        .values()
        .any(|l| l.item_id == a.id && l.external_id != ROOT_EXTERNAL_ID);
    if cur.origin != origin || !linked {
        return Err(denied(format!(
            "integration {iid} may only change items it created"
        )));
    }
    if crate::immutable::Locks::of(st).fields_locked(cur.type_id.as_deref(), &cur.fields) {
        return Err(denied(format!(
            "item {} ({}) is immutable; integrations cannot change it",
            a.id, a.type_id
        )));
    }
    if a.fields.is_empty() {
        return Err(CmdError::invalid("fields must name at least one field"));
    }
    if a.fields.len() > MAX_PATCH_FIELDS {
        return Err(CmdError::invalid(format!(
            "at most {MAX_PATCH_FIELDS} fields per patch"
        )));
    }
    let writable = ext_writable(st, &a.type_id).unwrap_or_default();
    if let Some(k) = a.fields.keys().find(|k| !writable.contains(k)) {
        return Err(denied(format!(
            "field {} of {} is not writable by integrations",
            strip_controls(k).chars().take(64).collect::<String>(),
            a.type_id
        )));
    }
    if let Some(v) = a.if_version
        && v != cur.version
    {
        return Err(CmdError::conflict(format!(
            "item {} is at version {}, not {v}",
            a.id, cur.version
        )));
    }
    let mut fields = cur.fields.clone();
    for (k, v) in &a.fields {
        if v.is_null() {
            fields.remove(k);
        } else {
            fields.insert(k.clone(), v.clone());
        }
    }
    // Same text rules as every other write: reject, never silently strip.
    limits::check_json_text(
        "fields",
        &serde_json::Value::Object(fields.clone().into_iter().collect()),
    )?;
    st.validate_item_fields(Some(&a.type_id), &fields)?;
    if fields == cur.fields {
        return st.view(&a.id);
    }
    let names: Vec<&str> = a.fields.keys().map(String::as_str).collect();
    let summary = format!(
        "{} changed {} of \"{}\"",
        short(&cfg.title),
        names.join(", "),
        short(&cur.title)
    );
    let id = a.id.clone();
    st.write(ctx, "ext.patch_fields", &a, true, |st, _| {
        let now = st.now();
        st.outline.update(&id, now, |it| it.fields = fields)?;
        Ok(Done {
            result: st.view(&id)?,
            entity: entity("item", &id),
            summary,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_ids_are_stable_and_distinct() {
        assert_eq!(link_id("gh", "a"), link_id("gh", "a"));
        assert_ne!(link_id("gh", "a"), link_id("gh2", "a"));
        assert_eq!(link_id("gh", "a").len(), 40);
    }
}
