//! HalpTask Hub core: owns the store and the command registry and is the one
//! entry point for the desktop UI, MCP clients, modules and tests.
//!
//! Every call goes through [`Core::dispatch`] with the contract envelope
//! (`docs/CONTRACT.md`): look up the command, validate its arguments, work
//! out the effective tier, ask the [`Policy`], audit the decision, then run
//! the handler (or park the request for a human), record one event per
//! change and broadcast it. The state lock is held from the tier assessment
//! to the end of the handler, so what was assessed is what runs.

mod clock;
mod commands;
mod envelope;
mod immutable;
mod installed;
pub mod packs;
mod policy;
mod registry;
mod state;

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use halp_domain::Event;
use halp_policy::secret;
use halp_store::{AuditRecord, EntityKind, KeyProvider, Store};
use serde_json::{Value, json};
use tokio::sync::{broadcast, oneshot};

pub use clock::{Clock, FixedClock, SystemClock};
pub use commands::ImportReport;
pub use envelope::{CmdError, ErrorBody, ErrorCode, Request, Response};
pub use halp_pack;
pub use halp_pack::PackTrust;
pub use halp_policy;
pub use packs::{ExtFile, ExtModule, ExtUiWidget, PackPrompt};
pub use policy::{
    Actor, Client, Ctx, DATA_CLASSES, DEFAULT_DATA_CLASS, Engine, Grants, Limits, Policy, Query,
    Rule, Tier, Verdict,
};
pub use registry::CommandDescriptor;
pub use state::{ConsentDetails, ConsentTarget, PendingConsent};

use registry::{Command, Registry};
use state::{PendingEntry, State, entity, short};

/// Whether the store holds `client_id` as a paired, unrevoked client.
fn stored_client_active(st: &State, client_id: &str) -> bool {
    st.store
        .get(EntityKind::Client, client_id)
        .ok()
        .flatten()
        .and_then(|v| serde_json::from_value::<Client>(v).ok())
        .is_some_and(|c| c.active())
}

/// The handler of a command registered with [`Core::register_command`].
pub type ExternalHandler = Box<dyn Fn(&Ctx, Value) -> Result<Value, CmdError> + Send + Sync>;

/// Buffered change notifications per subscriber.
pub const EVENT_CAPACITY: usize = 256;
/// Largest request `dispatch_json` accepts.
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
/// How many consent outcomes are kept for callers waiting on them.
const MAX_OUTCOMES: usize = 256;
/// Pending confirmations per requesting actor.
pub const MAX_PENDING_PER_ACTOR: usize = 16;
/// Pending confirmations in all.
pub const MAX_PENDING_TOTAL: usize = 64;
/// How long a parked request waits for a decision before it is denied.
pub const DEFAULT_CONSENT_TIMEOUT: Duration = Duration::from_secs(300);
/// Largest argument document a consent prompt shows in full; a larger one
/// is shown in short form with `display_truncated` (and cannot be approved
/// from a client that honours the flag).
pub const MAX_DISPLAY_ARGS_BYTES: usize = 64 * 1024;
/// Budget for the full arguments in one `consent.list` answer (well under
/// the 2 MiB socket message cap).
pub const MAX_LIST_ARGS_BYTES: usize = 1024 * 1024;

/// What became of a parked request.
#[derive(Debug, Clone, PartialEq)]
pub enum ConsentOutcome {
    /// Approved; the command ran (as the original actor) with this response
    /// (which may be an error).
    Approved(Response),
    /// Denied by the user.
    Denied,
    /// Nobody decided in time.
    Expired,
    /// What the request would touch changed after it was shown, so the
    /// approval was refused and the request was asked again under this new
    /// confirmation id.
    Superseded(String),
}

impl ConsentOutcome {
    /// The response the original caller gets.
    pub fn into_response(self) -> Response {
        match self {
            ConsentOutcome::Approved(r) => r,
            ConsentOutcome::Denied => Response::Err(
                CmdError::new(ErrorCode::Denied, "the user denied this request").into(),
            ),
            ConsentOutcome::Expired => Response::Err(
                CmdError::new(
                    ErrorCode::Denied,
                    "nobody confirmed this request in time; it was denied",
                )
                .into(),
            ),
            ConsentOutcome::Superseded(new_id) => {
                let mut e = CmdError::new(
                    ErrorCode::ConfirmationRequired,
                    format!(
                        "what this request would change changed before it was approved; it was asked again as {new_id}"
                    ),
                );
                e.confirmation_id = Some(new_id);
                Response::Err(e.into())
            }
        }
    }
}

#[derive(Default)]
struct ConsentBook {
    /// id -> (requesting actor, outcome).
    outcomes: BTreeMap<String, (String, ConsentOutcome)>,
    order: VecDeque<String>,
    waiters: HashMap<String, oneshot::Sender<ConsentOutcome>>,
}

impl ConsentBook {
    /// Hands the outcome to a waiting caller, or keeps it for
    /// [`Core::take_consent_outcome`].
    fn resolve(&mut self, id: &str, actor: &str, o: ConsentOutcome) {
        if let Some(tx) = self.waiters.remove(id) {
            match tx.send(o) {
                Ok(()) => return,
                Err(o) => return self.put(id, actor, o),
            }
        }
        self.put(id, actor, o);
    }

    fn put(&mut self, id: &str, actor: &str, o: ConsentOutcome) {
        if self
            .outcomes
            .insert(id.to_string(), (actor.to_string(), o))
            .is_none()
        {
            self.order.push_back(id.to_string());
        }
        while self.order.len() > MAX_OUTCOMES {
            if let Some(old) = self.order.pop_front() {
                self.outcomes.remove(&old);
            }
        }
    }

    fn take(&mut self, id: &str, actor: &str) -> Option<ConsentOutcome> {
        if self.outcomes.get(id).is_none_or(|(a, _)| a != actor) {
            return None;
        }
        let (_, o) = self.outcomes.remove(id)?;
        self.order.retain(|x| x != id);
        Some(o)
    }
}

/// The result of assessing a call: its effective tier and the arguments
/// that will run (a plain undo/redo is pinned to the event it would step).
struct Assessed {
    tier: Tier,
    args: Value,
}

pub struct Core {
    state: Mutex<State>,
    registry: Registry,
    policy: Arc<dyn Policy>,
    events: broadcast::Sender<Event>,
    consent: Mutex<ConsentBook>,
    consent_timeout: Duration,
}

impl Core {
    /// Opens (or creates) the encrypted database at `path` with the real
    /// policy engine.
    pub fn open(path: &Path, keys: &dyn KeyProvider) -> Result<Core, CmdError> {
        let store = Store::open(path, keys)?;
        Core::with_store(store, Arc::new(SystemClock))
    }

    /// Builds a core over an already open store with the real policy
    /// ([`Engine`]). Seeds the built-in role packs on first use.
    pub fn with_store(store: Store, clock: Arc<dyn Clock>) -> Result<Core, CmdError> {
        Core::with_policy(store, clock, Arc::new(Engine::default()))
    }

    /// Like [`Core::with_store`] with another policy (tests, custom limits).
    pub fn with_policy(
        store: Store,
        clock: Arc<dyn Clock>,
        policy: Arc<dyn Policy>,
    ) -> Result<Core, CmdError> {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let packs = packs::builtin_packs()?;
        let mut st = State::open(store, packs, clock, events.clone(), policy.clone())?;
        let mut registry = Registry::default();
        commands::register_all(&mut registry);
        st.descriptors = registry.descriptors();
        apply_retention(&mut st)?;
        Ok(Core {
            state: Mutex::new(st),
            registry,
            policy,
            events,
            consent: Mutex::new(ConsentBook::default()),
            consent_timeout: DEFAULT_CONSENT_TIMEOUT,
        })
    }

    /// Attaches a process-local service of type `T` (an integrations host,
    /// say), replacing any earlier one. Handlers read it with
    /// `State::extension`. Only Rust code holding the `Core` can set it; no
    /// command or argument reaches it.
    pub fn set_extension<T: Any + Send + Sync>(&self, v: Arc<T>) {
        let mut st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        st.extensions.insert(TypeId::of::<T>(), v);
    }

    /// The service of type `T` attached with [`Core::set_extension`].
    pub fn extension<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let st = self.state.lock().unwrap_or_else(|p| p.into_inner());
        st.extension::<T>()
    }

    /// How long parked requests wait (default 5 minutes).
    pub fn set_consent_timeout(&mut self, t: Duration) {
        self.consent_timeout = t;
    }

    pub fn consent_timeout(&self) -> Duration {
        self.consent_timeout
    }

    /// Adds a command implemented outside the core (a module, a test). Its
    /// handler gets the validated JSON arguments; the policy applies as for
    /// built-in commands.
    pub fn register_command(
        &mut self,
        descriptor: CommandDescriptor,
        handler: ExternalHandler,
    ) -> Result<(), CmdError> {
        if self.registry.get(&descriptor.name).is_some() {
            return Err(CmdError::conflict(format!(
                "command {} already exists",
                descriptor.name
            )));
        }
        let validator = jsonschema_lite(&descriptor.input_schema);
        self.registry.add_raw(
            descriptor,
            Box::new(move |_st, ctx, args| handler(ctx, args)),
            validator,
        );
        let ds = self.registry.descriptors();
        self.lock()?.descriptors = ds;
        Ok(())
    }

    /// Change notifications: one [`Event`] per recorded change (including
    /// undo/redo, audit and consent events).
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        self.registry.descriptors()
    }

    /// The paired client a token belongs to (revoked clients never match).
    ///
    /// Clients, grants and rules are reloaded from the store first, so a
    /// change written to the database outside this Core's policy cache
    /// counts from the client's next connection. Only one Core opens a
    /// database at a time (`hub.lock`), so while the app runs `halpd
    /// clients pair` refuses to start: pair in the app instead. `halpd
    /// clients list|revoke` go through the running Hub's user channel.
    pub fn authenticate(&self, token: &str) -> Option<String> {
        if let Ok(mut st) = self.lock() {
            let _ = st.sync_policy();
        }
        self.policy.authenticate(token)
    }

    /// Whether `client_id` is (still) a paired, unrevoked client.
    pub fn client_active(&self, client_id: &str) -> bool {
        self.lock()
            .ok()
            .is_some_and(|st| stored_client_active(&st, client_id))
    }

    /// Serves a Tier C file. `None` if the pack is not installed and
    /// enabled, the widget is unknown, the path is not listed in the
    /// manifest's file index (or not under the widget's `root`, or of a
    /// content type outside the allow-list), or the stored bytes no longer
    /// match their hash.
    pub fn ext_ui_file(&self, pack_id: &str, widget: &str, path: &str) -> Option<ExtFile> {
        let st = self.lock().ok()?;
        packs::ext_ui_file(&st, pack_id, widget, path)
    }

    /// The UI widget's declared `calls` allow-list and entry, if the pack is
    /// installed and enabled and declares the widget.
    pub fn ext_ui_widget(&self, pack_id: &str, widget: &str) -> Option<ExtUiWidget> {
        let st = self.lock().ok()?;
        packs::ext_ui_widget(&st, pack_id, widget)
    }

    /// A Tier B module and its verified wasm, if its pack is installed (or
    /// in dev mode), enabled and trusted to run code (§2, §5). The bytes
    /// are hashed again against the installed file index on every call.
    pub fn ext_module(&self, pack_id: &str, name: &str) -> Option<ExtModule> {
        let st = self.lock().ok()?;
        packs::ext_module(&st, pack_id, name)
    }

    /// Prompts of enabled first-party and community packs, for MCP
    /// (never unsigned or dev packs; see [`PackPrompt`]).
    pub fn pack_prompts(&self) -> Vec<PackPrompt> {
        self.lock()
            .map(|st| packs::pack_prompts(&st))
            .unwrap_or_default()
    }

    /// The core clock's instant and the user's local day (the `now` and
    /// `today` a Tier B transform gets).
    pub fn now_and_today(&self) -> Option<(chrono::DateTime<chrono::Utc>, chrono::NaiveDate)> {
        let st = self.lock().ok()?;
        Some((st.now(), st.today()))
    }

    /// Problems repaired while opening (for example orphaned items that were
    /// re-rooted).
    pub fn startup_warnings(&self) -> Vec<String> {
        self.lock()
            .map(|st| st.warnings.clone())
            .unwrap_or_default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, State>, CmdError> {
        let st = self.state.lock().map_err(|_| {
            CmdError::internal("core state is unavailable after an earlier failure")
        })?;
        if let Some(why) = &st.failed {
            return Err(CmdError::internal(format!(
                "the core stopped after an error and must be restarted: {why}"
            )));
        }
        Ok(st)
    }

    fn book(&self) -> MutexGuard<'_, ConsentBook> {
        // Plain data; a poisoned lock is still usable.
        self.consent.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// The decision on a parked request, once made, for the actor that
    /// asked. Taking it removes it. Another actor gets `None`.
    pub fn take_consent_outcome(&self, ctx: &Ctx, confirmation_id: &str) -> Option<ConsentOutcome> {
        self.book().take(confirmation_id, &ctx.actor.to_string())
    }

    /// Runs one request. A call that needs confirmation returns
    /// `confirmation_required` at once; see [`Core::dispatch_wait`].
    pub fn dispatch(&self, ctx: &Ctx, req: Request) -> Response {
        match self.run(ctx, req) {
            Ok(v) => Response::Ok(v),
            Err(e) => Response::Err(e.into()),
        }
    }

    /// Parses and runs a JSON request; malformed input is `invalid_args`.
    pub fn dispatch_json(&self, ctx: &Ctx, raw: &str) -> Response {
        if raw.len() > MAX_REQUEST_BYTES {
            return Response::Err(CmdError::invalid("request is larger than 1 MiB").into());
        }
        match serde_json::from_str::<Request>(raw) {
            Ok(req) => self.dispatch(ctx, req),
            Err(e) => Response::Err(CmdError::invalid(format!("malformed request: {e}")).into()),
        }
    }

    /// Runs one request and, when it needs a human, waits until the user
    /// approves (the command's response), denies (`denied`) or the consent
    /// timeout passes (`denied`). The caller never handles a confirmation id.
    pub async fn dispatch_wait(&self, ctx: &Ctx, req: Request) -> Response {
        let first = self.dispatch(ctx, req);
        let id = match first.error() {
            Some(e) if e.code == ErrorCode::ConfirmationRequired => {
                match e.confirmation_id.clone() {
                    Some(id) => id,
                    None => return first,
                }
            }
            _ => return first,
        };
        self.wait_for(ctx, &id).await
    }

    /// Waits for the decision on a request `ctx` parked. A request that was
    /// asked again (its scope changed) is followed to its new id.
    pub async fn wait_for(&self, ctx: &Ctx, id: &str) -> Response {
        let mut id = id.to_string();
        // Each re-ask replaces one pending entry with one; bound the loop
        // anyway.
        for _ in 0..64 {
            match self.wait_once(ctx, &id).await {
                ConsentOutcome::Superseded(next) => id = next,
                o => return o.into_response(),
            }
        }
        ConsentOutcome::Expired.into_response()
    }

    async fn wait_once(&self, ctx: &Ctx, id: &str) -> ConsentOutcome {
        let actor = ctx.actor.to_string();
        // Lock order everywhere: state before book.
        if let Some(o) = self.book().take(id, &actor) {
            return o;
        }
        let owned = self
            .lock()
            .ok()
            .and_then(|st| st.pending.get(id).map(|p| p.public.actor == actor));
        let rx = {
            let mut book = self.book();
            if let Some(o) = book.take(id, &actor) {
                return o;
            }
            if owned != Some(true) {
                return ConsentOutcome::Approved(Response::Err(
                    CmdError::not_found(format!("no pending confirmation {id}")).into(),
                ));
            }
            let (tx, rx) = oneshot::channel();
            book.waiters.insert(id.to_string(), tx);
            rx
        };
        match tokio::time::timeout(self.consent_timeout, rx).await {
            Ok(Ok(o)) => o,
            _ => {
                self.expire(id);
                self.book()
                    .take(id, &actor)
                    .unwrap_or(ConsentOutcome::Expired)
            }
        }
    }

    /// Denies a parked request as expired (if it is still pending).
    fn expire(&self, id: &str) {
        let Ok(mut st) = self.lock() else { return };
        let Some(p) = st.pending.remove(id) else {
            return;
        };
        drop(st);
        self.book().waiters.remove(id);
        if let Ok(mut st) = self.lock()
            && let Err(e) = self.record_decision(&mut st, &p, "expired", "timed out", None)
        {
            eprintln!(
                "halp-core: could not record the expiry of {}: {}",
                p.public.id, e.message
            );
        }
        self.book()
            .resolve(id, &p.public.actor, ConsentOutcome::Expired);
    }

    /// Expires parked requests past their deadline, records them and wakes
    /// their callers; returns how many. Cheap when none is due. Hosts call
    /// it on a timer (about every 15 s) so a request nobody answers expires
    /// on time even when no other call comes in.
    pub fn sweep_expired(&self) -> usize {
        let Ok(mut st) = self.lock() else { return 0 };
        let now = Instant::now();
        if !st.pending.values().any(|p| p.deadline <= now) {
            return 0;
        }
        let before = st.pending.len();
        self.sweep(&mut st);
        before - st.pending.len()
    }

    /// Whether `ctx` may see item `id` now: it exists and the caller has a
    /// grant for its data class (its own type's or an ancestor's). MCP hosts
    /// check this before subscribing to or notifying about one item, so a
    /// subscription tells nothing about items the caller cannot read.
    pub fn may_see_item(&self, ctx: &Ctx, id: &str) -> bool {
        let Ok(st) = self.lock() else { return false };
        match st.item_class(id) {
            None => false,
            Some(class) => {
                matches!(ctx.actor, Actor::User | Actor::System)
                    || self.policy.allows_class(ctx, &class)
            }
        }
    }

    /// Expires every parked request past its deadline.
    fn sweep(&self, st: &mut State) {
        let now = Instant::now();
        let gone: Vec<String> = st
            .pending
            .iter()
            .filter(|(_, p)| p.deadline <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in gone {
            if let Some(p) = st.pending.remove(&id) {
                if let Err(e) = self.record_decision(st, &p, "expired", "timed out", None) {
                    eprintln!(
                        "halp-core: could not record the expiry of {id}: {}",
                        e.message
                    );
                }
                self.book()
                    .resolve(&id, &p.public.actor, ConsentOutcome::Expired);
            }
        }
    }

    fn run(&self, ctx: &Ctx, req: Request) -> Result<Value, CmdError> {
        let cmd = self
            .registry
            .get(&req.command)
            .ok_or_else(|| CmdError::not_found(format!("unknown command {}", req.command)))?;
        let d = &cmd.descriptor;
        if d.user_only && !ctx.actor.is_user() {
            return Err(CmdError::new(
                ErrorCode::Denied,
                format!("{} is for the user only", d.name),
            ));
        }
        match d.name.as_str() {
            "consent.approve" => return self.decide(ctx, req.args, true),
            "consent.deny" => return self.decide(ctx, req.args, false),
            _ => {}
        }
        // Malformed arguments never reach the policy or the consent queue.
        (cmd.validate)(&req.args)?;
        let mut st = self.lock()?;
        if matches!(ctx.actor, Actor::Agent(_)) {
            // A revoke or `grants.set` (from the app, or `halpd clients
            // revoke` over the user channel) may have changed the client
            // since it connected.
            st.sync_policy()?;
        }
        self.sweep(&mut st);
        let a = self.assess(&st, ctx, cmd, req.args)?;
        // Items of a type with its own data class (a private note) need a
        // grant for that class; without one they do not exist for the
        // caller.
        let class = request_class(&st, &ctx.actor.to_string(), &d.name, &a.args, &d.data_class);
        if class != d.data_class && !self.policy.allows_class(ctx, &class) {
            return Err(hidden_item(&a.args));
        }
        // An agent's change to an immutable item (an approved evidence
        // file) is refused now rather than parked or run (v1.9).
        immutable::precheck(&st, &ctx.actor, &d.name, &a.args)?;
        let verdict = self.policy.check(
            &Query {
                ctx,
                command: &d.name,
                tier: a.tier,
                data_class: &class,
                args: &a.args,
            },
            Instant::now(),
        );
        let quiet = matches!(ctx.actor, Actor::User | Actor::System)
            && a.tier == Tier::T0
            && matches!(verdict, Verdict::Allow { .. });
        let tainted = self.policy.taint(ctx);
        if !quiet && !matches!(verdict, Verdict::Confirm { .. }) {
            self.audit_log(
                &mut st,
                ctx,
                &d.name,
                a.tier,
                &a.args,
                verdict.label(),
                verdict.reason(),
                None,
                tainted.is_some(),
            )?;
        }
        match verdict {
            Verdict::Allow { .. } => {
                let out = (cmd.handler)(&mut st, ctx, a.args)
                    .and_then(|v| self.hide_private(&st, ctx, v));
                let writers = match &out {
                    Ok(v) => writer_origins(&st, ctx, v),
                    Err(_) => Vec::new(),
                };
                drop(st);
                if let Ok(v) = &out {
                    self.note_reads(ctx, v, writers);
                }
                out
            }
            Verdict::Deny { reason } => Err(CmdError::new(ErrorCode::Denied, reason)),
            Verdict::Confirm { reason, tainted } => {
                self.park(&mut st, ctx, cmd, a.tier, a.args, reason, tainted)
            }
        }
    }

    /// Marks the caller's session tainted by content from others: the
    /// origins and event actors in the result, and `writers`, everyone
    /// other than the user who edited what it returned (see
    /// [`writer_origins`]).
    fn note_reads(&self, ctx: &Ctx, v: &Value, writers: Vec<String>) {
        if ctx.actor.is_user() {
            return;
        }
        let mut origins = writers;
        collect_origins(v, &mut origins, 0);
        for o in origins {
            self.policy.note_read(ctx, &o);
        }
    }

    /// Takes items the caller may not see (a data class it has no grant
    /// for) out of a result: dropped from lists, `null` in place, and
    /// `not_found` when the result is such an item. The user sees all.
    fn hide_private(&self, st: &State, ctx: &Ctx, mut v: Value) -> Result<Value, CmdError> {
        if matches!(ctx.actor, Actor::User | Actor::System) {
            return Ok(v);
        }
        if let Value::Object(m) = &v
            && self.hidden(st, ctx, m)
        {
            return Err(hidden_item(&v));
        }
        self.scrub(st, ctx, &mut v, 0);
        Ok(v)
    }

    fn hidden(&self, st: &State, ctx: &Ctx, m: &serde_json::Map<String, Value>) -> bool {
        // An item view: `id`, `origin` and `type_id`.
        if !(m.contains_key("origin") && m.contains_key("type_id")) {
            return false;
        }
        let Some(id) = m.get("id").and_then(Value::as_str) else {
            return false;
        };
        let class = st
            .item_class(id)
            .unwrap_or_else(|| st.type_class(m.get("type_id").and_then(Value::as_str)));
        !self.policy.allows_class(ctx, &class)
    }

    fn scrub(&self, st: &State, ctx: &Ctx, v: &mut Value, depth: usize) {
        if depth > 70 {
            return;
        }
        match v {
            Value::Array(a) => {
                a.retain(|x| !matches!(x, Value::Object(m) if self.hidden(st, ctx, m)));
                for x in a {
                    self.scrub(st, ctx, x, depth + 1);
                }
            }
            Value::Object(m) => {
                for x in m.values_mut() {
                    if matches!(x, Value::Object(o) if self.hidden(st, ctx, o)) {
                        *x = Value::Null;
                    } else {
                        self.scrub(st, ctx, x, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }

    /// The effective tier for these arguments:
    ///
    /// * `history.undo` / `history.redo`: T3 when the step re-applies a T3
    ///   change (a redo of `tasks.delete`, say), and for any actor other
    ///   than the user also when it deletes any item.
    /// * `layout.remove_widget` of the `activity` widget by a non-user: T2.
    fn assess(
        &self,
        st: &State,
        ctx: &Ctx,
        cmd: &Command,
        args: Value,
    ) -> Result<Assessed, CmdError> {
        let d = &cmd.descriptor;
        let mut tier = d.tier;
        let mut args = args;
        match d.name.as_str() {
            "history.undo" | "history.redo" => {
                let undo = d.name == "history.undo";
                let a: commands::StepArgs = decode_value(&args)?;
                if let Some(id) = &a.event_id {
                    halp_domain::limits::simple_id("event id", id)?;
                }
                let actor = ctx.actor.to_string();
                if let Some(plan) = st.store.plan_step(undo, a.event_id.as_deref(), &actor)? {
                    let deleted = plan
                        .changes
                        .iter()
                        .filter(|c| {
                            c.kind == EntityKind::Item && c.before.is_some() && c.after.is_none()
                        })
                        .count();
                    let reapplies_t3 = !undo
                        && self
                            .registry
                            .get(&plan.original.command)
                            .is_some_and(|c| c.descriptor.tier == Tier::T3);
                    // The user's own undo stays one keystroke (it is itself
                    // redoable); only re-applying a T3 change asks again.
                    let destructive = reapplies_t3 || (!ctx.actor.is_user() && deleted > 0);
                    if destructive {
                        tier = Tier::T3;
                    }
                    // Pin the step to this event, so an approval later
                    // cannot step something else.
                    args = json!({ "event_id": plan.original.id });
                }
            }
            // A request about an item that does not exist is answered now,
            // not parked for a confirmation that could only fail.
            "tasks.delete" | "tasks.update" | "tasks.move" | "tasks.complete" | "focus.set" => {
                if let Some(id) = args.get("id").and_then(Value::as_str) {
                    st.view(id)?;
                }
            }
            "layout.remove_widget" if !ctx.actor.is_user() => {
                let role = args.get("role_id").and_then(Value::as_str);
                let wid = args.get("widget_id").and_then(Value::as_str).unwrap_or("");
                if let Ok(l) = st.layout_for(role)
                    && l.widget(wid).is_some_and(|w| w.kind == "activity")
                {
                    tier = tier.max(Tier::T2);
                }
            }
            _ => {}
        }
        Ok(Assessed { tier, args })
    }

    #[allow(clippy::too_many_arguments)]
    fn audit_log(
        &self,
        st: &mut State,
        ctx: &Ctx,
        command: &str,
        tier: Tier,
        args: &Value,
        decision: &str,
        reason: &str,
        confirmation_id: Option<&str>,
        tainted: bool,
    ) -> Result<(), CmdError> {
        let rec = AuditRecord {
            id: halp_domain::new_id(),
            ts: st
                .now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            actor: ctx.actor.to_string(),
            command: command.to_string(),
            tier: tier.as_str().to_string(),
            args_hash: secret::args_hash(command, args),
            decision: decision.to_string(),
            reason: short_reason(reason),
            approver: None,
            confirmation_id: confirmation_id.map(str::to_string),
            tainted,
        };
        // Fail closed: a decision that cannot be audited does not run.
        st.store.audit_append(&rec).map_err(CmdError::from)
    }

    fn record_decision(
        &self,
        st: &mut State,
        p: &PendingEntry,
        decision: &str,
        reason: &str,
        approver: Option<&str>,
    ) -> Result<(), CmdError> {
        let pc = &p.public;
        let rec = AuditRecord {
            id: halp_domain::new_id(),
            ts: st
                .now()
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            actor: pc.actor.clone(),
            command: pc.command.clone(),
            tier: pc.tier.as_str().to_string(),
            args_hash: pc.args_hash.clone(),
            decision: decision.to_string(),
            reason: short_reason(reason),
            approver: approver.map(str::to_string),
            confirmation_id: Some(pc.id.clone()),
            tainted: pc.tainted,
        };
        st.store.audit_append(&rec)?;
        let verb = match decision {
            "approved" => "Approved",
            "failed" => "Approved (failed)",
            "denied" => "Declined",
            "superseded" => "Changed since asked",
            _ => "Expired",
        };
        let command = match decision {
            "approved" | "failed" => "consent.approve",
            "denied" => "consent.deny",
            "superseded" => "consent.superseded",
            _ => "consent.expired",
        };
        let what = if p.private {
            format!("{} on a private item", pc.command)
        } else {
            short(&pc.summary)
        };
        st.audit(
            approver.unwrap_or("system"),
            command,
            entity("confirmation", &pc.id),
            format!("{verb}: {what}"),
            json!({ "confirmation_id": pc.id, "outcome": decision }),
        )?;
        Ok(())
    }

    /// Parks a request and returns `confirmation_required`.
    #[allow(clippy::too_many_arguments)]
    fn park(
        &self,
        st: &mut State,
        ctx: &Ctx,
        cmd: &Command,
        tier: Tier,
        args: Value,
        reason: String,
        tainted: Option<String>,
    ) -> Result<Value, CmdError> {
        let d = &cmd.descriptor;
        let actor = ctx.actor.to_string();
        let mine = st
            .pending
            .values()
            .filter(|p| p.public.actor == actor)
            .count();
        let refuse = |st: &mut State, why: String| -> Result<Value, CmdError> {
            self.audit_log(
                st,
                ctx,
                &d.name,
                tier,
                &args,
                "deny",
                &why,
                None,
                tainted.is_some(),
            )?;
            Err(CmdError::new(ErrorCode::Denied, why))
        };
        if mine >= MAX_PENDING_PER_ACTOR {
            return refuse(
                st,
                format!(
                    "{actor} already has {MAX_PENDING_PER_ACTOR} requests waiting for approval"
                ),
            );
        }
        // The user's own requests (a T3 delete from the window) always
        // park: agents filling the queue must not lock the user out.
        let own = matches!(ctx.actor, Actor::User | Actor::System);
        if !own && st.pending.len() >= MAX_PENDING_TOTAL {
            return refuse(st, "too many requests are waiting for approval".into());
        }
        if let Err(why) = self.policy.may_park(ctx, Instant::now()) {
            return refuse(st, why);
        }
        // Pin what the handler would work out at run time (the Inbox, a
        // natural-language due date), so what is approved is what runs.
        let args = pin_args(st, &d.name, args);
        let scope = scope_digest(st, &actor, &d.name, &args)?;
        let deadline = Instant::now() + self.consent_timeout;
        let (id, summary) = self.enqueue(
            st, ctx, &d.name, tier, args, reason, tainted, deadline, scope,
        )?;
        let mut err = CmdError::new(ErrorCode::ConfirmationRequired, summary);
        err.confirmation_id = Some(id);
        Err(err)
    }

    /// Stores a pending request, audits it and announces it with a
    /// `consent.requested` event. Returns its new id and summary.
    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &self,
        st: &mut State,
        ctx: &Ctx,
        command: &str,
        tier: Tier,
        args: Value,
        reason: String,
        tainted: Option<String>,
        deadline: Instant,
        scope: String,
    ) -> Result<(String, String), CmdError> {
        let actor = ctx.actor.to_string();
        let id = secret::random_hex().map_err(CmdError::internal)?;
        let (summary, details) = describe(st, ctx, command, tier, &args, &reason);
        let private =
            request_class(st, &actor, command, &args, DEFAULT_DATA_CLASS) != DEFAULT_DATA_CLASS;
        let created_at = st.now();
        let left = deadline.saturating_duration_since(Instant::now());
        let timeout =
            chrono::Duration::from_std(left).unwrap_or_else(|_| chrono::Duration::minutes(5));
        let shown = display_args(command, &args);
        let entry = PendingEntry {
            public: PendingConsent {
                id: id.clone(),
                command: command.to_string(),
                tier,
                actor: actor.clone(),
                args: shown.args,
                display_truncated: shown.truncated,
                args_hash: secret::args_hash(command, &args),
                summary: summary.clone(),
                details,
                tainted: tainted.is_some(),
                taint_source: tainted.clone(),
                created_at,
                expires_at: created_at + timeout,
                private,
            },
            args,
            short_args: shown.short.clone(),
            deadline,
            scope,
            reason: reason.clone(),
            private,
            truncated: shown.truncated,
        };
        self.audit_log(
            st,
            ctx,
            command,
            tier,
            &entry.args,
            "confirm",
            &reason,
            Some(&id),
            tainted.is_some(),
        )?;
        // The event log keeps the short form; clients read the whole
        // request with consent.get.
        let (said, payload) = if private {
            (
                format!("Asked: {command} on a private item"),
                json!({
                    "confirmation_id": id,
                    "command": command,
                    "tier": tier,
                    "private": true,
                }),
            )
        } else {
            (
                format!("Asked: {}", short(&summary)),
                json!({
                    "confirmation_id": id,
                    "command": command,
                    "tier": tier,
                    "args": shown.short,
                    "display_truncated": shown.truncated,
                }),
            )
        };
        st.audit(
            &actor,
            "consent.requested",
            entity("confirmation", &id),
            said,
            payload,
        )?;
        st.pending.insert(id.clone(), entry);
        Ok((id, summary))
    }

    /// The scope of `p` changed since it was shown: refuse this approval,
    /// and ask again (same requester, arguments and deadline) with a fresh
    /// summary under a new id. The original caller follows the new id.
    fn reask(
        &self,
        mut st: MutexGuard<'_, State>,
        p: PendingEntry,
        original: &Actor,
        scope: String,
    ) -> Result<Value, CmdError> {
        let why = "the request changed since you saw it";
        self.record_decision(&mut st, &p, "superseded", why, Some("user"))?;
        st.pending.remove(&p.public.id);
        let ctx = Ctx::new(original.clone());
        let tainted = p
            .public
            .taint_source
            .clone()
            .or_else(|| p.public.tainted.then(String::new));
        let asked = self.enqueue(
            &mut st,
            &ctx,
            &p.public.command,
            p.public.tier,
            p.args.clone(),
            p.reason.clone(),
            tainted,
            p.deadline,
            scope,
        );
        drop(st);
        let (new_id, _) = match asked {
            Ok(x) => x,
            Err(e) => {
                self.book()
                    .resolve(&p.public.id, &p.public.actor, ConsentOutcome::Denied);
                return Err(e);
            }
        };
        self.book().resolve(
            &p.public.id,
            &p.public.actor,
            ConsentOutcome::Superseded(new_id.clone()),
        );
        let mut err = CmdError::new(
            ErrorCode::Conflict,
            format!("{why}: review it again and decide (it is now {new_id})"),
        );
        err.confirmation_id = Some(new_id);
        Err(err)
    }

    /// `consent.approve` / `consent.deny`. The pending entry stays until the
    /// outcome is known; the approval is recorded together with it.
    fn decide(&self, ctx: &Ctx, args: Value, approve: bool) -> Result<Value, CmdError> {
        let verb = if approve { "approve" } else { "deny" };
        if !ctx.actor.is_user() {
            return Err(CmdError::new(
                ErrorCode::Denied,
                format!("only the user can {verb} a pending request"),
            ));
        }
        let a: commands::ConsentArgs = decode_value(&args)?;
        let mut st = self.lock()?;
        self.sweep(&mut st);
        let p = st
            .pending
            .get(&a.id)
            .cloned()
            .ok_or_else(|| CmdError::not_found(format!("no pending confirmation {}", a.id)))?;
        if !approve {
            self.record_decision(&mut st, &p, "denied", "denied by the user", Some("user"))?;
            st.pending.remove(&p.public.id);
            drop(st);
            self.book()
                .resolve(&p.public.id, &p.public.actor, ConsentOutcome::Denied);
            return Ok(json!({ "denied": p.public.id }));
        }
        // What the user could not see in full is never approved, whatever
        // the client shows (it stays pending until denied or expired).
        if p.truncated {
            return Err(CmdError::new(
                ErrorCode::Denied,
                "this request is too large to show in full, so it cannot be approved; deny it",
            ));
        }
        let original = Actor::parse(&p.public.actor)
            .ok_or_else(|| CmdError::internal(format!("bad stored actor {}", p.public.actor)))?;
        if let Actor::Agent(c) = &original
            && !stored_client_active(&st, c)
        {
            let why = format!("client {c} was revoked before the approval");
            self.record_decision(&mut st, &p, "denied", &why, Some("user"))?;
            st.pending.remove(&p.public.id);
            drop(st);
            self.book()
                .resolve(&p.public.id, &p.public.actor, ConsentOutcome::Denied);
            return Err(CmdError::new(ErrorCode::Denied, why));
        }
        let cmd = self
            .registry
            .get(&p.public.command)
            .ok_or_else(|| CmdError::not_found(format!("unknown command {}", p.public.command)))?;
        if secret::args_hash(&p.public.command, &p.args) != p.public.args_hash {
            return Err(CmdError::internal(
                "pending request does not match its hash",
            ));
        }
        // What the user saw is what runs: if the request would now touch
        // something else (an item moved under the one being deleted, a newer
        // step on top of the one being undone, more events to purge), the
        // approval does not count.
        let now_scope = match scope_digest(&st, &p.public.actor, &p.public.command, &p.args) {
            Ok(s) => s,
            // It can no longer run at all (an undo whose item was edited
            // since, say): settle it as failed rather than leave it pending.
            Err(e) => {
                let why = format!("approved; failed: {}", e.message);
                let recorded = self.record_decision(&mut st, &p, "failed", &why, Some("user"));
                st.pending.remove(&p.public.id);
                drop(st);
                self.book().resolve(
                    &p.public.id,
                    &p.public.actor,
                    ConsentOutcome::Approved(Response::Err(e.clone().into())),
                );
                recorded?;
                return Err(e);
            }
        };
        if now_scope != p.scope {
            return self.reask(st, p, &original, now_scope);
        }
        // Runs as the requester, without asking the policy again.
        let as_requester = Ctx::new(original);
        let result = (cmd.handler)(&mut st, &as_requester, p.args.clone())
            .and_then(|v| self.hide_private(&st, &as_requester, v));
        // What it returns to the agent taints it like any read would.
        let writers = match &result {
            Ok(v) => writer_origins(&st, &as_requester, v),
            Err(_) => Vec::new(),
        };
        let (decision, why) = match &result {
            Ok(_) => ("approved", "approved by the user".to_string()),
            Err(e) => ("failed", format!("approved; failed: {}", e.message)),
        };
        let recorded = self.record_decision(&mut st, &p, decision, &why, Some("user"));
        st.pending.remove(&p.public.id);
        drop(st);
        if let Ok(v) = &result {
            self.note_reads(&as_requester, v, writers);
        }
        let response = match &result {
            Ok(v) => Response::Ok(v.clone()),
            Err(e) => Response::Err(e.clone().into()),
        };
        self.book().resolve(
            &p.public.id,
            &p.public.actor,
            ConsentOutcome::Approved(response),
        );
        recorded?;
        result
    }
}

/// Applies a stored retention (`history.purge` with `remember`) at start.
fn apply_retention(st: &mut State) -> Result<(), CmdError> {
    let days = st
        .store
        .get(EntityKind::Setting, commands::SETTING_RETENTION)?
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok());
    if let Some(d) = days {
        let older = commands::cutoff(st, d)?;
        st.store.purge_events(Some(&older), false)?;
    }
    Ok(())
}

fn decode_value<A: serde::de::DeserializeOwned>(args: &Value) -> Result<A, CmdError> {
    let args = if args.is_null() {
        json!({})
    } else {
        args.clone()
    };
    serde_json::from_value(args).map_err(|e| CmdError::invalid(format!("invalid arguments: {e}")))
}

fn short_reason(s: &str) -> String {
    s.chars().take(300).collect()
}

/// Collects `origin` fields of items in a result (bounded walk).
fn collect_origins(v: &Value, out: &mut Vec<String>, depth: usize) {
    if depth > 70 || out.len() > 64 {
        return;
    }
    match v {
        Value::Object(m) => {
            // Only actor origins: a task type's `origin` (`builtin`,
            // `pack`) is not a writer.
            if let Some(Value::String(o)) = m.get("origin")
                && Actor::parse(o).is_some()
                && !out.contains(o)
            {
                out.push(o.clone());
            }
            // An event's summary quotes what its actor wrote.
            if m.contains_key("summary")
                && m.contains_key("command")
                && let Some(Value::String(a)) = m.get("actor")
                && !out.contains(a)
            {
                out.push(a.clone());
            }
            for x in m.values() {
                collect_origins(x, out, depth + 1);
            }
        }
        Value::Array(a) => {
            for x in a {
                collect_origins(x, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// Everyone other than the user, the system and the reader who wrote an
/// item, widget or task type in `v` (by the `id`s it holds): reading what
/// another agent edited taints like reading what it created, so an edit
/// cannot launder an injected title through a user-created item.
fn writer_origins(st: &State, ctx: &Ctx, v: &Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if ctx.actor.is_user() || st.writers.is_empty() {
        return out;
    }
    let mut stack: Vec<(&Value, usize)> = vec![(v, 0)];
    while let Some((v, d)) = stack.pop() {
        if d > 70 || out.len() > 64 {
            break;
        }
        match v {
            Value::Object(m) => {
                if let Some(Value::String(id)) = m.get("id") {
                    for kind in ["item", "widget", "type"] {
                        if let Some(w) = st.writers.get(&format!("{kind}:{id}")) {
                            for a in w {
                                if !out.contains(a) {
                                    out.push(a.clone());
                                }
                            }
                        }
                    }
                }
                stack.extend(m.values().map(|x| (x, d + 1)));
            }
            Value::Array(a) => stack.extend(a.iter().map(|x| (x, d + 1))),
            _ => {}
        }
    }
    out
}

/// The `not_found` a caller gets for an item it may not see.
fn hidden_item(v: &Value) -> CmdError {
    match v.get("id").and_then(Value::as_str) {
        Some(id) => CmdError::not_found(format!("item {id} not found")),
        None => CmdError::not_found("item not found"),
    }
}

/// The data class a request touches: the command's own class, or, for a
/// command on items, the first non-default class of the items it names
/// (their types or an ancestor's), the type it gives an item, the items a
/// step would change, or the type it redefines.
fn request_class(st: &State, actor: &str, command: &str, args: &Value, own: &str) -> String {
    if own != DEFAULT_DATA_CLASS {
        return own.to_string();
    }
    let s = |v: Option<&Value>| v.and_then(Value::as_str).map(str::to_string);
    let mut classes: Vec<String> = Vec::new();
    let mut items: Vec<String> = Vec::new();
    match command {
        "tasks.get" | "tasks.update" | "tasks.move" | "tasks.complete" | "focus.set"
        | "ext.patch_fields" => {
            items.extend(s(args.get("id")));
            items.extend(s(args.get("parent_id")));
            if let Some(t) = s(args.get("patch").and_then(|p| p.get("type_id")))
                // A type that does not exist (yet) has no class; the
                // command itself refuses an unknown type.
                && st.types.get(&t).is_some()
            {
                classes.push(st.type_class(Some(&t)));
            }
        }
        "tasks.delete" => {
            if let Some(id) = s(args.get("id")) {
                items.push(id.clone());
                for d in st.outline.subtree_ids(&id) {
                    if let Some(it) = st.outline.get(&d) {
                        classes.push(st.type_class(it.type_id.as_deref()));
                    }
                }
            }
        }
        "tasks.create" | "capture.quick" => {
            items.extend(s(args.get("parent_id")));
            // Without a parent, capture files into the Inbox (review S6b).
            if command == "capture.quick" && args.get("parent_id").is_none_or(Value::is_null) {
                items.extend(commands::capture::find_inbox(st));
            }
            if let Some(t) = s(args.get("type_id"))
                // A type that does not exist (yet) has no class; the
                // command itself refuses an unknown type.
                && st.types.get(&t).is_some()
            {
                classes.push(st.type_class(Some(&t)));
            }
        }
        "history.undo" | "history.redo" => {
            let undo = command == "history.undo";
            let ev = args.get("event_id").and_then(Value::as_str);
            if let Ok(Some(plan)) = st.store.plan_step(undo, ev, actor) {
                classes.extend(st.private_class(&plan.changes));
                // A step that redefines a type carries the class of the
                // definition it leaves and the one it restores, and of every
                // item of that type (review: undo must not reclassify or
                // probe items the caller may not see).
                for c in plan
                    .changes
                    .iter()
                    .filter(|c| c.kind == EntityKind::TaskType)
                {
                    type_classes(st, &c.id, &mut classes);
                    for img in [&c.before, &c.after].into_iter().flatten() {
                        classes.extend(s(img
                            .get("schema")
                            .and_then(|x| x.get(state::DATA_CLASS_KEY))));
                    }
                }
            }
        }
        "types.define" => {
            if let Some(id) = s(args.get("id")) {
                type_classes(st, &id, &mut classes);
            }
            classes.extend(s(args
                .get("schema")
                .and_then(|x| x.get(state::DATA_CLASS_KEY))));
        }
        _ => {}
    }
    for id in items {
        classes.extend(st.item_class(&id));
    }
    classes
        .into_iter()
        .find(|c| c != DEFAULT_DATA_CLASS)
        .unwrap_or_else(|| own.to_string())
}

/// The classes a change to task type `id` touches: the type's own class
/// and that of every item carrying it, ancestors included (an item under a
/// private note is private whatever its type says). A new type has no
/// class yet; one that exists, or that items still carry although it no
/// longer loads, keeps its items'.
fn type_classes(st: &State, id: &str, classes: &mut Vec<String>) {
    let carriers: Vec<String> = st
        .outline
        .preorder()
        .iter()
        .filter(|i| {
            st.outline
                .get(i)
                .is_some_and(|it| it.type_id.as_deref() == Some(id))
        })
        .cloned()
        .collect();
    if st.types.get(id).is_some() || !carriers.is_empty() {
        classes.push(st.type_class(Some(id)));
    }
    for i in carriers {
        classes.extend(st.item_class(&i));
    }
}

/// Arguments of a request about to be parked, with what the handler
/// would work out at run time fixed now: a natural-language `due`
/// (`friday`) becomes the date it means today, and `capture.quick`
/// without `parent_id` names the existing Inbox (its `due:` words are
/// pinned to dates in the text). Approving tomorrow runs what was shown
/// today.
fn pin_args(st: &State, command: &str, mut args: Value) -> Value {
    let today = st.today();
    let pin = |v: &mut Value| {
        if let Some(raw) = v.as_str()
            && let Ok(d) = commands::parse_due(raw, today)
        {
            *v = json!(d.format("%Y-%m-%d").to_string());
        }
    };
    match command {
        "tasks.create" => {
            if let Some(v) = args.get_mut("due") {
                pin(v);
            }
        }
        "tasks.update" => {
            if let Some(v) = args.get_mut("patch").and_then(|p| p.get_mut("due")) {
                pin(v);
            }
        }
        "capture.quick" => {
            if let Some(Value::String(text)) = args.get_mut("text") {
                *text = pin_capture_due(text, today);
            }
            if args.get("parent_id").is_none_or(Value::is_null)
                && let Some(inbox) = find_inbox(st)
                && let Value::Object(m) = &mut args
            {
                m.insert("parent_id".into(), json!(inbox));
            }
        }
        _ => {}
    }
    args
}

/// The Inbox capture.quick files into (a root bullet titled Inbox).
fn find_inbox(st: &State) -> Option<String> {
    st.outline.roots().iter().find_map(|id| {
        let it = st.outline.get(id)?;
        (it.kind == halp_domain::ItemKind::Bullet && it.title.eq_ignore_ascii_case("Inbox"))
            .then(|| id.clone())
    })
}

/// Capture text with every `due:<x>`, `due=<x>` and `@due(<x>)` word that
/// parses replaced by `due:<YYYY-MM-DD>`.
fn pin_capture_due(text: &str, today: chrono::NaiveDate) -> String {
    let mut changed = false;
    let words: Vec<String> = text
        .split(' ')
        .map(|w| {
            let lower = w.to_lowercase();
            let raw = if lower.starts_with("@due(") && w.ends_with(')') && w.len() > 6 {
                w.get(5..w.len() - 1)
            } else if (lower.starts_with("due:") || lower.starts_with("due=")) && w.len() > 4 {
                w.get(4..)
                    .map(|v| v.trim_matches(|c| c == '"' || c == '\''))
            } else {
                None
            };
            match raw.and_then(|r| commands::parse_due(r, today).ok()) {
                Some(d) => {
                    changed = true;
                    format!("due:{}", d.format("%Y-%m-%d"))
                }
                None => w.to_string(),
            }
        })
        .collect();
    if changed {
        words.join(" ")
    } else {
        text.to_string()
    }
}

/// The arguments of a parked request as clients see them.
struct Shown {
    /// In full (unless `truncated`).
    args: Value,
    truncated: bool,
    /// Long values cut, or size and hash (what the audit log keeps).
    short: Value,
}

/// Arguments as shown in a consent prompt: in full, with secrets and file
/// content replaced; past [`MAX_DISPLAY_ARGS_BYTES`] the short form.
fn display_args(command: &str, args: &Value) -> Shown {
    let mut a = args.clone();
    if command == "import.tui"
        && let Value::Object(m) = &mut a
    {
        if let Some(Value::String(c)) = m.get("content_base64") {
            let n = c.len() / 4 * 3;
            m.insert("content_base64".into(), json!(format!("<{n} bytes>")));
        }
        if m.get("passphrase").is_some_and(|p| !p.is_null()) {
            m.insert("passphrase".into(), json!("[redacted]"));
        }
    }
    let short = secret::audit_args(&a);
    if secret::canonical_json(&a).len() > MAX_DISPLAY_ARGS_BYTES {
        Shown {
            args: short.clone(),
            truncated: true,
            short,
        }
    } else {
        Shown {
            args: a,
            truncated: false,
            short,
        }
    }
}

/// A digest of everything a parked request would touch, resolved against
/// the current state: every item named in its arguments with its whole
/// subtree (ids, parents, versions), the exact changes an undo/redo would
/// apply, the events a purge would redact, the layout a widget command
/// edits. Equal digests at park and at approval mean the user approved
/// what runs.
fn scope_digest(st: &State, actor: &str, command: &str, args: &Value) -> Result<String, CmdError> {
    let mut scope = serde_json::Map::new();
    let item = |id: &str| -> Value {
        match st.outline.get(id) {
            Some(i) => json!([i.parent_id, i.version, i.updated_at]),
            None => Value::Null,
        }
    };
    let mut items = serde_json::Map::new();
    let mut names: Vec<&str> = Vec::new();
    for key in ["id", "item_id"] {
        if let Some(v) = args.get(key).and_then(Value::as_str) {
            names.push(v);
        }
    }
    if let Some(Value::Array(a)) = args.get("ids") {
        names.extend(a.iter().filter_map(Value::as_str));
    }
    for id in names {
        let sub = st.outline.subtree_ids(id);
        if sub.is_empty() {
            items.insert(id.to_string(), Value::Null);
        }
        for s in sub {
            let v = item(&s);
            items.insert(s, v);
        }
    }
    // The new parent itself (not its subtree: siblings may come and go).
    if let Some(p) = args.get("parent_id").and_then(Value::as_str) {
        items
            .entry(format!("parent:{p}"))
            .or_insert_with(|| item(p));
    }
    if !items.is_empty() {
        scope.insert("items".into(), Value::Object(items));
    }
    match command {
        "history.undo" | "history.redo" => {
            let undo = command == "history.undo";
            let a: commands::StepArgs = decode_value(args)?;
            let plan = st.store.plan_step(undo, a.event_id.as_deref(), actor)?;
            let step = plan.map(|p| {
                let changes: Vec<Value> = p
                    .changes
                    .iter()
                    .map(|c| json!([c.kind.as_str(), c.id, c.before, c.after]))
                    .collect();
                json!({ "event": p.original.id, "owner": p.owner, "changes": changes })
            });
            scope.insert("step".into(), step.unwrap_or(Value::Null));
        }
        "history.purge" => {
            let a: commands::PurgeArgs = decode_value(args)?;
            let older = match a.older_than_days {
                Some(0) | None => None,
                Some(d) => Some(commands::cutoff(st, d)?),
            };
            let ids = st
                .store
                .purge_candidates(older.as_deref(), a.deleted_items)?;
            scope.insert("events".into(), json!(ids));
        }
        c if c.starts_with("layout.") => {
            let role = args.get("role_id").and_then(Value::as_str);
            if let Ok(l) = st.layout_for(role) {
                scope.insert(
                    "layout".into(),
                    serde_json::to_value(l).unwrap_or(Value::Null),
                );
            }
        }
        _ => {}
    }
    Ok(secret::args_hash("scope", &Value::Object(scope)))
}

fn item_target(st: &State, id: &str) -> Option<ConsentTarget> {
    let item = st.outline.get(id)?;
    Some(ConsentTarget {
        kind: "item".into(),
        id: id.to_string(),
        title: short(&item.title),
        descendants: st.outline.subtree_ids(id).len().saturating_sub(1),
    })
}

fn below(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => " and 1 item below it".into(),
        n => format!(" and {n} items below it"),
    }
}

/// The consent summary and details, from the resolved targets.
fn describe(
    st: &State,
    ctx: &Ctx,
    command: &str,
    tier: Tier,
    args: &Value,
    reason: &str,
) -> (String, ConsentDetails) {
    let mut d = ConsentDetails::default();
    let id = args.get("id").and_then(Value::as_str);
    match command {
        "tasks.delete" | "tasks.update" | "tasks.move" | "tasks.complete" | "focus.set" => {
            if let Some(t) = id.and_then(|i| item_target(st, i)) {
                let verb = match command {
                    "tasks.delete" => "Deletes",
                    "tasks.update" => "Changes",
                    "tasks.move" => "Moves",
                    "tasks.complete" => "Completes",
                    _ => "Focuses",
                };
                d.effects
                    .push(format!("{verb} \"{}\"{}", t.title, below(t.descendants)));
                d.targets.push(t);
            }
        }
        "history.undo" | "history.redo" => {
            let undo = command == "history.undo";
            let ev = args.get("event_id").and_then(Value::as_str);
            if let Ok(Some(plan)) = st.store.plan_step(undo, ev, &ctx.actor.to_string()) {
                let verb = if undo { "Reverts" } else { "Re-applies" };
                d.effects.push(format!(
                    "{verb} \"{}\" ({} by {})",
                    short(&plan.original.summary),
                    plan.original.command,
                    plan.original.actor
                ));
                d.targets.push(ConsentTarget {
                    kind: "event".into(),
                    id: plan.original.id.clone(),
                    title: short(&plan.original.summary),
                    descendants: 0,
                });
                let items: Vec<_> = plan
                    .changes
                    .iter()
                    .filter(|c| c.kind == EntityKind::Item)
                    .collect();
                let title = |v: &Option<Value>| {
                    v.as_ref()
                        .and_then(|x| x.get("title"))
                        .and_then(Value::as_str)
                        .map(short)
                        .unwrap_or_default()
                };
                let deleted: Vec<_> = items
                    .iter()
                    .filter(|c| c.before.is_some() && c.after.is_none())
                    .collect();
                let restored = items
                    .iter()
                    .filter(|c| c.before.is_none() && c.after.is_some())
                    .count();
                if !deleted.is_empty() {
                    d.effects.push(format!("Deletes {} items", deleted.len()));
                }
                if restored > 0 {
                    d.effects.push(format!("Restores {restored} items"));
                }
                for c in deleted.iter().take(20) {
                    d.targets.push(ConsentTarget {
                        kind: "item".into(),
                        id: c.id.clone(),
                        title: title(&c.before),
                        descendants: 0,
                    });
                }
            }
        }
        "layout.remove_widget" => {
            let role = args.get("role_id").and_then(Value::as_str);
            let wid = args.get("widget_id").and_then(Value::as_str).unwrap_or("");
            if let Ok(l) = st.layout_for(role)
                && let Some(w) = l.widget(wid)
            {
                d.effects.push(format!(
                    "Removes the {} widget \"{}\" from layout {}",
                    w.kind,
                    short(&w.title),
                    l.id
                ));
                d.targets.push(ConsentTarget {
                    kind: "widget".into(),
                    id: w.id.clone(),
                    title: short(&w.title),
                    descendants: 0,
                });
            }
        }
        "import.tui" => {
            let file = args
                .get("file_name")
                .and_then(Value::as_str)
                .unwrap_or("a TUI file");
            let n = args
                .get("content_base64")
                .and_then(Value::as_str)
                .map_or(0, |c| c.len() / 4 * 3);
            d.effects.push(format!(
                "Imports {} ({n} bytes) into the outline",
                short(file)
            ));
            d.targets.push(ConsentTarget {
                kind: "file".into(),
                id: short(file),
                title: short(file),
                descendants: 0,
            });
        }
        "history.purge" => {
            d.effects.push(
                "Permanently redacts the selected events (their before/after state cannot be recovered)"
                    .into(),
            );
        }
        _ => {}
    }
    if d.effects.is_empty() {
        d.effects.push(format!("Runs {command}"));
    }
    if !ctx.actor.is_user() {
        d.effects.push(format!("Requested by {}", ctx.actor));
    }
    let summary = format!(
        "{} [{}; {}]",
        d.effects[0],
        tier.as_str(),
        short_reason(reason)
    );
    (short_text(&summary, 300), d)
}

fn short_text(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out
}

/// A validator for externally registered commands: the arguments must be an
/// object (or null) that has every `required` property of the schema and,
/// when `additionalProperties` is false, no others.
fn jsonschema_lite(schema: &Value) -> registry::Validator {
    let required: Vec<String> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let closed = schema.get("additionalProperties") == Some(&Value::Bool(false));
    let known: Vec<String> = schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    Box::new(move |args| {
        let empty = serde_json::Map::new();
        let m = match args {
            Value::Null => &empty,
            Value::Object(m) => m,
            _ => return Err(CmdError::invalid("arguments must be an object")),
        };
        for r in &required {
            if !m.contains_key(r) {
                return Err(CmdError::invalid(format!("missing argument {r}")));
            }
        }
        if closed && let Some(k) = m.keys().find(|k| !known.contains(k)) {
            return Err(CmdError::invalid(format!("unknown argument {k}")));
        }
        Ok(())
    })
}
