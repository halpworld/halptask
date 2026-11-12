//! Client pairing, grants, standing rules and the audit log. Everything here
//! except `audit.list` is user only and hidden from MCP.

use std::collections::BTreeMap;

use halp_policy::secret;
use halp_store::{AuditRecord, EntityKind};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::envelope::CmdError;
use crate::policy::{Client, Ctx, Grants, Rule, Tier};
use crate::state::{Done, State, entity, short};

/// What the API shows of a client (never the token hash).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClientView {
    pub id: String,
    pub name: String,
    /// The actor its calls run as.
    pub actor: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
    pub grants: Grants,
    /// When the policy last decided a call of this client (`clients.list`
    /// only; kept in memory, so null again after the Hub restarts).
    #[serde(default)]
    pub last_seen: Option<String>,
    /// Until when the client is tainted (it read untrusted content), or
    /// null. Each new tainted read moves it later (`clients.list` only).
    #[serde(default)]
    pub tainted_until: Option<String>,
}

impl From<&Client> for ClientView {
    fn from(c: &Client) -> Self {
        ClientView {
            id: c.id.clone(),
            name: c.name.clone(),
            actor: format!("agent:{}", c.id),
            created_at: c.created_at.clone(),
            revoked_at: c.revoked_at.clone(),
            grants: c.grants.clone(),
            last_seen: None,
            tainted_until: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PairArgs {
    /// Display name, e.g. "Claude Code".
    pub name: String,
    /// Client id (`[A-Za-z0-9_.-]{1,64}`); derived from the name when absent.
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PairResult {
    pub client: ClientView,
    /// Shown once; only its SHA-256 is stored.
    pub token: String,
}

fn slug(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').chars().take(48).collect::<String>();
    if s.is_empty() { "client".into() } else { s }
}

fn now_str(st: &State) -> String {
    st.now()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn load_client(st: &State, id: &str) -> Result<Client, CmdError> {
    halp_domain::limits::simple_id("client id", id)?;
    let v = st
        .store
        .get(EntityKind::Client, id)?
        .ok_or_else(|| CmdError::not_found(format!("client {id} not found")))?;
    Ok(serde_json::from_value(v)?)
}

pub fn pair(st: &mut State, ctx: &Ctx, a: PairArgs) -> Result<PairResult, CmdError> {
    let name = halp_domain::limits::label("name", &a.name, 80)?;
    let id = match &a.id {
        Some(id) => {
            halp_domain::limits::simple_id("client id", id)?;
            if st.store.get(EntityKind::Client, id)?.is_some() {
                return Err(CmdError::conflict(format!("client {id} already exists")));
            }
            id.clone()
        }
        None => {
            let base = slug(&name);
            let mut id = base.clone();
            let mut n = 2;
            while st.store.get(EntityKind::Client, &id)?.is_some() {
                id = format!("{base}-{n}");
                n += 1;
            }
            id
        }
    };
    let token = secret::new_token().map_err(CmdError::internal)?;
    let client = Client {
        id: id.clone(),
        name: name.clone(),
        token_hash: secret::token_hash(&token),
        created_at: now_str(st),
        revoked_at: None,
        grants: Grants::default(),
    };
    let view = ClientView::from(&client);
    let args = serde_json::json!({"name": name, "id": id});
    st.write(ctx, "clients.pair", &args, false, |_, tx| {
        tx.record(
            EntityKind::Client,
            &id,
            None,
            Some(serde_json::to_value(&client)?),
        );
        Ok(Done {
            result: (),
            entity: entity("client", &id),
            summary: format!("Paired MCP client {}", short(&name)),
        })
    })?;
    st.sync_policy()?;
    Ok(PairResult {
        client: view,
        token,
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClientsResult {
    pub clients: Vec<ClientView>,
}

pub fn list(
    st: &mut State,
    _ctx: &Ctx,
    _a: super::history::NoArgs,
) -> Result<ClientsResult, CmdError> {
    let clients: Vec<Client> = st.store.load(EntityKind::Client)?;
    let (wall, mono) = (st.now(), std::time::Instant::now());
    let at = |d: std::time::Duration, later: bool| {
        let d = chrono::Duration::from_std(d).unwrap_or(chrono::Duration::zero());
        let t = if later { wall + d } else { wall - d };
        t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    Ok(ClientsResult {
        clients: clients
            .iter()
            .map(|c| {
                let a = st.policy.activity(&c.id, mono);
                let mut v = ClientView::from(c);
                v.last_seen = a.idle.map(|d| at(d, false));
                v.tainted_until = a.tainted_for.map(|d| at(d, true));
                v
            })
            .collect(),
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClientIdArgs {
    pub id: String,
}

/// Revokes a client: its token stops working at once (new connections and
/// calls on open ones) and its standing rules are deleted.
pub fn revoke(st: &mut State, ctx: &Ctx, a: ClientIdArgs) -> Result<ClientView, CmdError> {
    let before = load_client(st, &a.id)?;
    if !before.active() {
        return Ok(ClientView::from(&before));
    }
    let mut after = before.clone();
    after.revoked_at = Some(now_str(st));
    let rules: Vec<Rule> = st.store.load(EntityKind::Rule)?;
    st.write(ctx, "clients.revoke", &a, false, |_, tx| {
        tx.record(
            EntityKind::Client,
            &a.id,
            Some(serde_json::to_value(&before)?),
            Some(serde_json::to_value(&after)?),
        );
        for r in rules.iter().filter(|r| r.client_id == a.id) {
            tx.record(
                EntityKind::Rule,
                &r.id,
                Some(serde_json::to_value(r)?),
                None,
            );
        }
        Ok(Done {
            result: (),
            entity: entity("client", &a.id),
            summary: format!("Revoked MCP client {}", short(&before.name)),
        })
    })?;
    st.sync_policy()?;
    Ok(ClientView::from(&after))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ClearTaintArgs {
    pub client_id: String,
}

/// Result of `policy.clear_taint`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct ClearTaintResult {
    pub client_id: String,
    /// The source the client had read, if it was tainted.
    pub was_tainted_by: Option<String>,
}

/// `policy.clear_taint`: the user vouches for what a client has read, so
/// its next T2 calls may use standing rules again. User only, audited.
/// (Taint also clears by itself 30 minutes after the last tainted read.)
pub fn clear_taint(
    st: &mut State,
    ctx: &Ctx,
    a: ClearTaintArgs,
) -> Result<ClearTaintResult, CmdError> {
    let client = load_client(st, &a.client_id)?;
    let agent = Ctx::agent(&client.id);
    let was = st.policy.taint(&agent);
    st.policy.clear_taint(&agent);
    st.audit(
        &ctx.actor.to_string(),
        "policy.clear_taint",
        entity("client", &client.id),
        format!("Cleared the taint of MCP client {}", short(&client.name)),
        serde_json::json!({ "client_id": client.id, "was_tainted_by": was }),
    )?;
    Ok(ClearTaintResult {
        client_id: client.id,
        was_tainted_by: was,
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantsSetArgs {
    pub client_id: String,
    /// Run T1 (reversible local writes) without asking.
    #[serde(default)]
    pub t1: Option<bool>,
    /// The full list of data classes: the built-ins (`general`,
    /// `notes.private`, `email.body`, `secrets`, `audit`) or a class a loaded
    /// task type declares with `x-halp-data-class`.
    #[serde(default)]
    pub data_classes: Option<Vec<String>>,
}

pub fn grants_set(st: &mut State, ctx: &Ctx, a: GrantsSetArgs) -> Result<ClientView, CmdError> {
    let before = load_client(st, &a.client_id)?;
    let mut after = before.clone();
    if let Some(t1) = a.t1 {
        after.grants.t1 = t1;
    }
    if let Some(classes) = &a.data_classes {
        let mut out: Vec<String> = Vec::new();
        for c in classes {
            if !st.is_grantable_class(c) {
                return Err(CmdError::invalid(format!("unknown data class {c}")));
            }
            if !out.contains(c) {
                out.push(c.clone());
            }
        }
        out.sort();
        after.grants.data_classes = out;
    }
    st.write(ctx, "grants.set", &a, false, |_, tx| {
        tx.record(
            EntityKind::Client,
            &a.client_id,
            Some(serde_json::to_value(&before)?),
            Some(serde_json::to_value(&after)?),
        );
        Ok(Done {
            result: (),
            entity: entity("client", &a.client_id),
            summary: format!("Changed grants of {}", short(&before.name)),
        })
    })?;
    st.sync_policy()?;
    Ok(ClientView::from(&after))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrantsListArgs {
    #[serde(default)]
    pub client_id: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GrantsEntry {
    pub client_id: String,
    pub grants: Grants,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct GrantsListResult {
    pub grants: Vec<GrantsEntry>,
    /// The built-in data classes. Pack-declared classes can be granted too
    /// (see `types.list`, `x-halp-data-class`).
    pub data_classes: Vec<String>,
}

pub fn grants_list(
    st: &mut State,
    _ctx: &Ctx,
    a: GrantsListArgs,
) -> Result<GrantsListResult, CmdError> {
    let clients: Vec<Client> = match &a.client_id {
        Some(id) => vec![load_client(st, id)?],
        None => st.store.load(EntityKind::Client)?,
    };
    Ok(GrantsListResult {
        grants: clients
            .into_iter()
            .map(|c| GrantsEntry {
                client_id: c.id,
                grants: c.grants,
            })
            .collect(),
        data_classes: crate::policy::DATA_CLASSES
            .iter()
            .map(|s| s.to_string())
            .collect(),
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RuleCreateArgs {
    pub client_id: String,
    /// A T2 command (never T3, never user-only).
    pub command: String,
    /// Top-level argument name to the exact value it must have; at least one.
    pub args: BTreeMap<String, Value>,
}

pub fn rules_create(st: &mut State, ctx: &Ctx, a: RuleCreateArgs) -> Result<Rule, CmdError> {
    let client = load_client(st, &a.client_id)?;
    if !client.active() {
        return Err(CmdError::invalid(format!(
            "client {} is revoked",
            a.client_id
        )));
    }
    let d = st
        .descriptors
        .iter()
        .find(|d| d.name == a.command)
        .ok_or_else(|| CmdError::not_found(format!("unknown command {}", a.command)))?;
    if d.tier == Tier::T3 || d.user_only {
        return Err(CmdError::invalid(format!(
            "{} cannot have a standing rule (T3 and user-only commands always ask)",
            a.command
        )));
    }
    if a.args.is_empty() {
        return Err(CmdError::invalid(
            "a rule needs at least one argument predicate",
        ));
    }
    if a.args.len() > 16 {
        return Err(CmdError::invalid("at most 16 argument predicates"));
    }
    for (k, v) in &a.args {
        halp_domain::limits::label("argument name", k, 64)?;
        halp_domain::limits::json_bounds("argument value", v, 4096, 4)?;
    }
    let rule = Rule {
        id: halp_domain::new_id(),
        client_id: a.client_id.clone(),
        command: a.command.clone(),
        args: a.args.clone(),
        created_at: now_str(st),
        created_by: ctx.actor.to_string(),
    };
    let out = rule.clone();
    st.write(ctx, "rules.create", &a, false, |_, tx| {
        tx.record(
            EntityKind::Rule,
            &rule.id,
            None,
            Some(serde_json::to_value(&rule)?),
        );
        Ok(Done {
            result: (),
            entity: entity("rule", &rule.id),
            summary: format!("Standing rule: {} may run {}", rule.client_id, rule.command),
        })
    })?;
    st.sync_policy()?;
    Ok(out)
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RulesResult {
    pub rules: Vec<Rule>,
}

pub fn rules_list(st: &mut State, _ctx: &Ctx, a: GrantsListArgs) -> Result<RulesResult, CmdError> {
    let mut rules: Vec<Rule> = st.store.load(EntityKind::Rule)?;
    if let Some(c) = &a.client_id {
        rules.retain(|r| &r.client_id == c);
    }
    Ok(RulesResult { rules })
}

pub fn rules_delete(st: &mut State, ctx: &Ctx, a: ClientIdArgs) -> Result<RulesResult, CmdError> {
    halp_domain::limits::simple_id("rule id", &a.id)?;
    let before = st
        .store
        .get(EntityKind::Rule, &a.id)?
        .ok_or_else(|| CmdError::not_found(format!("rule {} not found", a.id)))?;
    st.write(ctx, "rules.delete", &a, false, |_, tx| {
        tx.record(EntityKind::Rule, &a.id, Some(before), None);
        Ok(Done {
            result: (),
            entity: entity("rule", &a.id),
            summary: "Deleted a standing rule".into(),
        })
    })?;
    st.sync_policy()?;
    rules_list(st, ctx, GrantsListArgs { client_id: None })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditListArgs {
    /// Default 100, max 1000.
    #[serde(default)]
    pub limit: Option<u32>,
    /// Page from this record id (exclusive).
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub actor_prefix: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AuditEntry {
    pub id: String,
    pub ts: String,
    pub actor: String,
    pub command: String,
    pub tier: String,
    pub args_hash: String,
    /// `allow`, `deny`, `confirm`, `approved`, `failed`, `denied`, `expired`,
    /// `superseded` (the request changed before approval and was asked again).
    pub decision: String,
    pub reason: String,
    pub approver: Option<String>,
    pub confirmation_id: Option<String>,
    pub tainted: bool,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct AuditListResult {
    /// Newest first.
    pub records: Vec<AuditEntry>,
}

/// `audit.list`: policy decisions, newest first. Data class `audit`: an
/// agent needs that grant.
pub fn audit_list(
    st: &mut State,
    _ctx: &Ctx,
    a: AuditListArgs,
) -> Result<AuditListResult, CmdError> {
    let limit = match a.limit {
        None => 100,
        Some(0) => return Err(CmdError::invalid("limit must be at least 1")),
        Some(n) => (n as usize).min(1000),
    };
    if let Some(b) = &a.before {
        halp_domain::limits::simple_id("audit record id", b)?;
    }
    if a.actor_prefix
        .as_ref()
        .is_some_and(|p| p.chars().count() > 80)
    {
        return Err(CmdError::invalid("actor_prefix is too long"));
    }
    let records = st
        .store
        .audit_list(limit, a.before.as_deref(), a.actor_prefix.as_deref())?
        .into_iter()
        .map(|r: AuditRecord| AuditEntry {
            id: r.id,
            ts: r.ts,
            actor: r.actor,
            command: r.command,
            tier: r.tier,
            args_hash: r.args_hash,
            decision: r.decision,
            reason: r.reason,
            approver: r.approver,
            confirmation_id: r.confirmation_id,
            tainted: r.tainted,
        })
        .collect();
    Ok(AuditListResult { records })
}
