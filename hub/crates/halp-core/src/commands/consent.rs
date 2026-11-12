//! Pending confirmations. `consent.approve` / `consent.deny` are executed by
//! `Core::dispatch` itself (they run another command); the handlers here only
//! give them a descriptor and schema.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::history::NoArgs;
use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::state::{PendingConsent, State};
use halp_policy::secret;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct PendingResult {
    /// Oldest first.
    pub pending: Vec<PendingConsent>,
}

/// The user sees every pending request; anyone else only their own.
fn visible(ctx: &Ctx, p: &PendingConsent) -> bool {
    ctx.actor.is_user() || p.actor == ctx.actor.to_string()
}

/// What `ctx` is shown of a pending request: on the socket's user channel
/// ([`Ctx::limited`]) a request on a private item says only who asked for
/// which command (review S1); the Hub window sees it all.
fn shown(ctx: &Ctx, p: &PendingConsent) -> PendingConsent {
    if !(ctx.limited && p.private) {
        return p.clone();
    }
    PendingConsent {
        id: p.id.clone(),
        command: p.command.clone(),
        tier: p.tier,
        actor: p.actor.clone(),
        args: Value::Null,
        display_truncated: true,
        args_hash: String::new(),
        summary: format!("{} on a private item (open the Hub to see it)", p.command),
        details: Default::default(),
        tainted: p.tainted,
        taint_source: None,
        created_at: p.created_at,
        expires_at: p.expires_at,
        private: true,
    }
}

pub fn list(st: &mut State, ctx: &Ctx, _a: NoArgs) -> Result<PendingResult, CmdError> {
    let mut pending: Vec<PendingConsent> = st
        .pending
        .values()
        .filter(|e| visible(ctx, &e.public))
        .map(|e| shown(ctx, &e.public))
        .collect();
    pending.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    // Keep a listing well under the 2 MiB message cap: past the budget an
    // entry carries its short form (`display_truncated`); `consent.get`
    // still returns it in full.
    let mut used = 0usize;
    for p in &mut pending {
        if p.args.is_null() {
            continue;
        }
        let n = secret::canonical_json(&p.args).len();
        if used + n > crate::MAX_LIST_ARGS_BYTES {
            if let Some(e) = st.pending.get(&p.id) {
                p.args = e.short_args.clone();
            }
            p.display_truncated = true;
        } else {
            used += n;
        }
    }
    Ok(PendingResult { pending })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConsentArgs {
    /// The `confirmation_id` from the `confirmation_required` error.
    #[serde(alias = "confirmation_id")]
    pub id: String,
}

pub fn get(st: &mut State, ctx: &Ctx, a: ConsentArgs) -> Result<PendingConsent, CmdError> {
    st.pending
        .get(&a.id)
        .map(|e| &e.public)
        .filter(|p| visible(ctx, p))
        .map(|p| shown(ctx, p))
        .ok_or_else(|| CmdError::not_found(format!("no pending confirmation {}", a.id)))
}

/// Result of `consent.deny`.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DeniedResult {
    pub denied: String,
}

pub fn approve(_st: &mut State, _ctx: &Ctx, _a: ConsentArgs) -> Result<Value, CmdError> {
    Err(CmdError::internal(
        "consent.approve is handled by the dispatcher",
    ))
}

pub fn deny(_st: &mut State, _ctx: &Ctx, _a: ConsentArgs) -> Result<DeniedResult, CmdError> {
    Err(CmdError::internal(
        "consent.deny is handled by the dispatcher",
    ))
}
