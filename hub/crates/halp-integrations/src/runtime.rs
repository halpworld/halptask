//! The integration runtime: one task per integration that syncs on an
//! interval (jittered), backs off on failure, reports state changes as
//! `ext.status` events and writes through `ext.upsert` as the
//! integration's own actor. Syncs run at most `max_concurrent_syncs` at a
//! time; HTTP has its own global and per-host limits.
//!
//! The core calls [`IntegrationHost`] methods while holding its state lock,
//! so they only touch the runtime's own state and spawn work; everything
//! that dispatches into the core runs on a blocking thread from a spawned
//! task.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use chrono::Utc;
use halp_core::{Actor, Core, Ctx, ErrorCode, Request, Response};
use halp_domain::integration::{
    CaptureTicket, HostHandle, IntegrationConfig, IntegrationHost, IntegrationState,
    IntegrationStatus,
};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::sync::{Notify, Semaphore};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::calendar::Calendar;
use crate::connector::{Batch, BoxFuture, Connector, ItemSource, SyncCx, SyncError};
use crate::git::{self, Git, GitSettings};
use crate::github::GitHub;
use crate::http::{Http, HttpOptions};
use crate::mcp::Mcp;
use crate::oauth::{self, OAuthProvider, Opener};
use crate::redact::redact;
use crate::secrets::{SecretStore, Token, check_user_token};

/// Longest wait between retries.
pub const MAX_BACKOFF: Duration = Duration::from_secs(3600);
/// Refresh an OAuth token this long before it expires.
pub const REFRESH_MARGIN_SECS: i64 = 60;

pub struct RuntimeOptions {
    /// Private working directories (`<data_dir>/<id>`), created 0700.
    pub data_dir: PathBuf,
    pub http: HttpOptions,
    pub secrets: Arc<dyn SecretStore>,
    /// Opens the authorization page; `None` disables OAuth.
    pub opener: Option<Opener>,
    pub auth_timeout: Duration,
    pub max_concurrent_syncs: usize,
    /// The `git` binary (default: [`git::find_git`]).
    pub git_binary: Option<PathBuf>,
    /// Empty: the built-in connectors.
    pub connectors: Vec<Arc<dyn Connector>>,
}

impl RuntimeOptions {
    pub fn new(data_dir: PathBuf, secrets: Arc<dyn SecretStore>) -> Self {
        RuntimeOptions {
            data_dir,
            http: HttpOptions::default(),
            secrets,
            opener: None,
            auth_timeout: Duration::from_secs(300),
            max_concurrent_syncs: 4,
            git_binary: None,
            connectors: Vec::new(),
        }
    }
}

struct Slot {
    cfg: IntegrationConfig,
    status: IntegrationStatus,
    wake: Arc<Notify>,
    cancel: CancellationToken,
    generation: u64,
    reported: Option<(IntegrationState, Option<String>)>,
    /// The settings moved to another destination and deleting the old
    /// secret failed; it must be deleted before any use.
    purge: bool,
    /// Bumped whenever the stored secret changes, so a sync whose keychain
    /// read raced with that change does not overwrite `has_secret`.
    secret_seq: u64,
}

struct Inner {
    core: Weak<Core>,
    rt: Handle,
    http: Http,
    secrets: Arc<dyn SecretStore>,
    opener: Option<Opener>,
    connectors: HashMap<&'static str, Arc<dyn Connector>>,
    sem: Arc<Semaphore>,
    slots: Mutex<HashMap<String, Slot>>,
    cancel: CancellationToken,
    tasks: TaskTracker,
    data_dir: PathBuf,
    auth_timeout: Duration,
    git: Option<PathBuf>,
}

/// The runtime. Cheap to clone.
#[derive(Clone)]
pub struct Runtime {
    inner: Arc<Inner>,
}

impl Runtime {
    /// Starts the runtime for `core`: attaches it as the core's integration
    /// host and starts every stored integration.
    pub fn start(core: &Arc<Core>, opts: RuntimeOptions, rt: Handle) -> Result<Runtime, String> {
        make_private_dir(&opts.data_dir)?;
        let http = Http::new(opts.http)?;
        let mut connectors: HashMap<&'static str, Arc<dyn Connector>> = HashMap::new();
        let list: Vec<Arc<dyn Connector>> = if opts.connectors.is_empty() {
            vec![
                Arc::new(GitHub::new()),
                Arc::new(Mcp),
                Arc::new(Git),
                Arc::new(Calendar),
                Arc::new(crate::energy::Energy),
            ]
        } else {
            opts.connectors
        };
        for c in list {
            connectors.insert(c.kind(), c);
        }
        let r = Runtime {
            inner: Arc::new(Inner {
                core: Arc::downgrade(core),
                rt,
                http,
                secrets: opts.secrets,
                opener: opts.opener,
                connectors,
                sem: Arc::new(Semaphore::new(opts.max_concurrent_syncs.max(1))),
                slots: Mutex::new(HashMap::new()),
                cancel: CancellationToken::new(),
                tasks: TaskTracker::new(),
                data_dir: opts.data_dir,
                auth_timeout: opts.auth_timeout,
                git: opts.git_binary.or_else(git::find_git),
            }),
        };
        core.set_extension(Arc::new(HostHandle(Arc::new(r.clone()))));
        let listed = core.dispatch(&Ctx::system(), Request::new("integrations.list", json!({})));
        let configs: Vec<IntegrationConfig> = match listed {
            Response::Ok(v) => v
                .get("integrations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|e| serde_json::from_value(e.get("config")?.clone()).ok())
                .collect(),
            Response::Err(e) => return Err(format!("could not list integrations: {}", e.message)),
        };
        for c in &configs {
            r.configured(c, false);
        }
        Ok(r)
    }

    /// Stores a personal access token (or API key) for an integration in
    /// the keychain. Returns only an error code; the token is never logged
    /// or echoed.
    ///
    /// The connector checks the value first: a `calendar` feed URL must be
    /// an `https` URL (`invalid_url`), `webcal://` is stored as `https://`.
    pub fn set_token(&self, id: &str, token: &str) -> Result<(), &'static str> {
        let (kind, settings) = {
            let slots = self.inner.lock();
            let s = slots.get(id).ok_or("unknown_integration")?;
            (s.cfg.kind.clone(), s.cfg.settings.clone())
        };
        let token = match self.inner.connector(&kind) {
            Some(c) => c.check_secret(&settings, token, &self.inner.http)?,
            None => {
                check_user_token(token)?;
                token.to_string()
            }
        };
        self.inner
            .secrets
            .set(id, &Token::personal(token))
            .map_err(|_| "keychain_error")?;
        let mut slots = self.inner.lock();
        if let Some(s) = slots.get_mut(id) {
            s.status.has_secret = true;
            s.purge = false;
            s.secret_seq += 1;
            s.wake.notify_one();
        }
        Ok(())
    }

    /// What the app may show of the stored secret, for kinds that have a
    /// hint (a calendar feed: host and last four characters). `None` when
    /// nothing is stored or the kind shows nothing.
    pub fn secret_hint(&self, id: &str) -> Option<String> {
        let (kind, settings) = {
            let slots = self.inner.lock();
            let s = slots.get(id).filter(|s| !s.purge)?;
            (s.cfg.kind.clone(), s.cfg.settings.clone())
        };
        let c = self.inner.connector(&kind)?;
        let t = self.inner.secrets.get(id).ok()??;
        c.secret_hint(&settings, &t.access_token)
    }

    /// Removes the stored token.
    pub fn clear_token(&self, id: &str) -> Result<(), &'static str> {
        if !self.inner.lock().contains_key(id) {
            return Err("unknown_integration");
        }
        self.inner
            .secrets
            .delete(id)
            .map_err(|_| "keychain_error")?;
        if let Some(s) = self.inner.lock().get_mut(id) {
            s.status.has_secret = false;
            s.purge = false;
            s.secret_seq += 1;
        }
        Ok(())
    }

    /// Stops every task (child processes are killed with them).
    pub fn stop(&self) {
        self.inner.cancel.cancel();
        self.inner.tasks.close();
    }

    /// Stops and waits up to `limit` for tasks to finish.
    pub async fn shutdown(&self, limit: Duration) {
        self.stop();
        let _ = tokio::time::timeout(limit, self.inner.tasks.wait()).await;
    }
}

fn make_private_dir(p: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(p).map_err(|e| format!("could not create {}: {e}", p.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn jitter(d: Duration, frac: f64) -> Duration {
    let mut b = [0u8; 4];
    let r = if getrandom::fill(&mut b).is_ok() {
        f64::from(u32::from_le_bytes(b)) / f64::from(u32::MAX)
    } else {
        0.5
    };
    d.mul_f64(1.0 + frac * (2.0 * r - 1.0))
}

/// `interval · 2^(failures-1)`, capped at an hour, ±20%.
pub fn backoff(interval: Duration, failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(16);
    let d = interval.saturating_mul(1u32 << exp).min(MAX_BACKOFF);
    jitter(d.max(Duration::from_secs(1)), 0.2)
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Slot>> {
        self.slots.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn current(&self, id: &str, generation: u64) -> Option<IntegrationConfig> {
        self.lock()
            .get(id)
            .filter(|s| s.generation == generation)
            .map(|s| s.cfg.clone())
    }

    fn update(&self, id: &str, generation: u64, f: impl FnOnce(&mut IntegrationStatus)) {
        if let Some(s) = self
            .lock()
            .get_mut(id)
            .filter(|s| s.generation == generation)
        {
            f(&mut s.status);
        }
    }

    fn connector(&self, kind: &str) -> Option<Arc<dyn Connector>> {
        self.connectors.get(kind).cloned()
    }

    async fn dispatch(
        &self,
        id: &str,
        command: &'static str,
        args: Value,
    ) -> Result<Value, SyncError> {
        let core = self.core.upgrade().ok_or(SyncError::Cancelled)?;
        let ctx = Ctx::new(Actor::Integration(id.to_string()));
        let r =
            tokio::task::spawn_blocking(move || core.dispatch(&ctx, Request::new(command, args)))
                .await
                .map_err(|_| SyncError::transient("dispatch task failed"))?;
        match r {
            Response::Ok(v) => Ok(v),
            Response::Err(e) if e.code == ErrorCode::Denied && e.message.contains("rate") => {
                Err(SyncError::Transient {
                    message: e.message,
                    retry_after: Some(Duration::from_secs(60)),
                })
            }
            Response::Err(e) => Err(SyncError::Config(format!("{command}: {}", e.message))),
        }
    }

    /// Records the state as an `ext.status` event when it changed.
    async fn report(
        &self,
        id: &str,
        generation: u64,
        state: IntegrationState,
        error: Option<String>,
    ) {
        let key = (state, error.clone());
        {
            let mut slots = self.lock();
            let Some(s) = slots.get_mut(id).filter(|s| s.generation == generation) else {
                return;
            };
            if s.reported.as_ref() == Some(&key) {
                return;
            }
            s.reported = Some(key);
        }
        let mut args = json!({ "state": state });
        if let Some(e) = error {
            args["error"] = json!(e);
        }
        let _ = self.dispatch(id, "ext.status", args).await;
    }

    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(Arc<dyn SecretStore>) -> T + Send + 'static,
    ) -> Result<T, SyncError> {
        let s = self.secrets.clone();
        tokio::task::spawn_blocking(move || f(s))
            .await
            .map_err(|_| SyncError::transient("keychain task failed"))
    }

    /// Deletes a secret left behind by a settings move (see
    /// [`Slot::purge`]); an error while it is still stored.
    async fn purge(&self, id: &str, generation: u64) -> Result<(), SyncError> {
        let needed = self
            .lock()
            .get(id)
            .is_some_and(|s| s.generation == generation && s.purge);
        if !needed {
            return Ok(());
        }
        let id2 = id.to_string();
        self.blocking(move |s| s.delete(&id2)).await?.map_err(|_| {
            SyncError::transient(
                "the settings moved to another service and the old token \
                     could not be deleted from the OS keychain",
            )
        })?;
        if let Some(s) = self
            .lock()
            .get_mut(id)
            .filter(|s| s.generation == generation)
        {
            s.purge = false;
        }
        Ok(())
    }

    async fn refresh(
        &self,
        id: &str,
        p: &OAuthProvider,
        refresh_token: &str,
        cancel: &CancellationToken,
    ) -> Result<Token, SyncError> {
        let t = oauth::refresh(&self.http, p, refresh_token, cancel)
            .await
            .map_err(|e| match e {
                oauth::AuthError::Cancelled => SyncError::Cancelled,
                oauth::AuthError::Failed(m) => {
                    SyncError::transient(format!("token refresh failed: {m}"))
                }
                other => SyncError::Auth(other.to_string()),
            })?;
        let (id2, t2) = (id.to_string(), t.clone());
        self.blocking(move |s| s.set(&id2, &t2))
            .await?
            .map_err(|_| SyncError::transient("could not store the refreshed token"))?;
        Ok(t)
    }

    /// One sync; the number of items the remote has.
    async fn sync_once(
        &self,
        cfg: &IntegrationConfig,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<u32, SyncError> {
        let conn = self
            .connector(&cfg.kind)
            .ok_or_else(|| SyncError::Config(format!("no connector for {}", cfg.kind)))?;
        let oauth = conn.oauth(&cfg.settings);
        let mut token: Option<Token> = None;
        self.purge(&cfg.id, generation).await?;
        if conn.needs_token(&cfg.settings) {
            let id = cfg.id.clone();
            let seq = self.lock().get(&cfg.id).map(|s| s.secret_seq);
            let got = self
                .blocking(move |s| s.get(&id))
                .await?
                .map_err(|_| SyncError::transient("could not read the OS keychain"))?;
            // A token set or cleared during the read wins over what it saw.
            if let Some(s) = self
                .lock()
                .get_mut(&cfg.id)
                .filter(|s| s.generation == generation && Some(s.secret_seq) == seq)
            {
                s.status.has_secret = got.is_some();
            }
            let Some(mut t) = got else {
                return Err(SyncError::Auth(
                    "no token stored; add one in Settings".into(),
                ));
            };
            if t.expiring(Utc::now(), REFRESH_MARGIN_SECS) {
                match (&oauth, t.refresh_token.clone()) {
                    (Some(p), Some(rt)) => t = self.refresh(&cfg.id, p, &rt, cancel).await?,
                    _ => return Err(SyncError::Auth("the token has expired".into())),
                }
            }
            token = Some(t);
        }
        let dir = self.data_dir.join(&cfg.id);
        if conn.private_dir(&cfg.settings) {
            make_private_dir(&dir).map_err(SyncError::Config)?;
        }
        let reader = CoreItems {
            inner: self,
            id: &cfg.id,
        };
        let mut retried = false;
        let batches = loop {
            let cx = SyncCx {
                id: &cfg.id,
                settings: &cfg.settings,
                http: &self.http,
                token: token.as_ref().map(|t| t.access_token.as_str()),
                cancel,
                data_dir: &dir,
                now: Utc::now(),
                items: &reader,
                roles: &cfg.roles,
            };
            let r = conn.sync(cx).await;
            let refreshable = token
                .as_ref()
                .and_then(|t| t.refresh_token.clone())
                .zip(oauth.as_ref());
            match (r, refreshable) {
                (Err(SyncError::Auth(_)), Some((rt, p))) if !retried => {
                    retried = true;
                    token = Some(self.refresh(&cfg.id, p, &rt, cancel).await?);
                }
                (r, _) => break r,
            }
        };
        let known: Vec<&str> = token.as_ref().map(|t| t.secrets()).unwrap_or_default();
        let batches = batches.map_err(|e| e.redacted(&known))?;
        let mut total = 0u32;
        for b in batches {
            total += self
                .upsert(&cfg.id, b)
                .await
                .map_err(|e| e.redacted(&known))?;
        }
        Ok(total)
    }

    async fn upsert(&self, id: &str, b: Batch) -> Result<u32, SyncError> {
        if b.items.is_empty() && !b.complete {
            self.patch(id, b.patches).await;
            return Ok(0);
        }
        let patches = b.patches;
        let complete = b.complete && b.items.len() <= halp_domain::integration::MAX_UPSERT_ITEMS;
        let items: Vec<Value> = b
            .items
            .into_iter()
            .take(halp_domain::integration::MAX_UPSERT_ITEMS)
            .collect();
        let r = self
            .dispatch(
                id,
                "ext.upsert",
                json!({ "scope": b.scope, "complete": complete, "items": items }),
            )
            .await?;
        let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0) as u32;
        self.patch(id, patches).await;
        Ok(n("created") + n("updated") + n("unchanged") + n("restored"))
    }

    /// Applies field patches through `ext.patch_fields`. Best effort: a
    /// conflict or a refusal (the item is not ours, or became private)
    /// leaves the item alone; an older core without the command stops it.
    async fn patch(&self, id: &str, patches: Vec<crate::connector::FieldPatch>) {
        for p in patches.into_iter().take(50) {
            let mut args = json!({
                "id": p.item_id,
                "type_id": p.type_id,
                "fields": p.fields,
            });
            if let Some(v) = p.if_version {
                args["if_version"] = json!(v);
            }
            if let Err(SyncError::Config(m)) = self.dispatch(id, "ext.patch_fields", args).await
                && m.contains("unknown command")
            {
                return;
            }
        }
    }

    async fn run(
        self: Arc<Self>,
        id: String,
        generation: u64,
        cancel: CancellationToken,
        wake: Arc<Notify>,
    ) {
        let mut failures = 0u32;
        loop {
            let Some(cfg) = self.current(&id, generation) else {
                return;
            };
            let Some(conn) = self.connector(&cfg.kind) else {
                self.update(&id, generation, |s| {
                    s.state = IntegrationState::Error;
                    s.error = Some(format!("unknown kind {}", cfg.kind));
                });
                return;
            };
            if !cfg.enabled {
                self.update(&id, generation, |s| {
                    s.state = IntegrationState::Disabled;
                    s.next_sync = None;
                });
                self.report(&id, generation, IntegrationState::Disabled, None)
                    .await;
                return;
            }
            if !conn.polls() {
                self.update(&id, generation, |s| {
                    s.state = IntegrationState::Ok;
                    s.error = None;
                });
                tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = wake.notified() => continue,
                }
            }
            let permit = tokio::select! {
                _ = cancel.cancelled() => return,
                p = self.sem.clone().acquire_owned() => p,
            };
            self.update(&id, generation, |s| s.state = IntegrationState::Syncing);
            let r = tokio::select! {
                _ = cancel.cancelled() => return,
                r = self.sync_once(&cfg, generation, &cancel) => r,
            };
            drop(permit);
            let interval = Duration::from_secs(u64::from(cfg.interval_secs.max(1)));
            let now = Utc::now();
            let (state, error, delay) = match r {
                Ok(n) => {
                    failures = 0;
                    self.update(&id, generation, |s| {
                        s.items = n;
                        s.last_sync = Some(now);
                    });
                    (IntegrationState::Ok, None, Some(jitter(interval, 0.1)))
                }
                Err(SyncError::Cancelled) => return,
                Err(SyncError::Auth(m)) => (IntegrationState::NeedsAuth, Some(m), None),
                Err(SyncError::Transient {
                    message,
                    retry_after,
                }) => {
                    failures += 1;
                    let d = backoff(interval, failures).max(retry_after.unwrap_or_default());
                    (
                        IntegrationState::Error,
                        Some(message),
                        Some(d.min(MAX_BACKOFF)),
                    )
                }
                Err(SyncError::Config(m)) => {
                    failures += 1;
                    (
                        IntegrationState::Error,
                        Some(m),
                        Some(backoff(interval, failures)),
                    )
                }
            };
            let error = error.map(|e| redact(&e, &[]));
            let next = delay
                .and_then(|d| chrono::Duration::from_std(d).ok())
                .map(|d| now + d);
            self.update(&id, generation, |s| {
                s.state = state;
                s.error = error.clone();
                s.next_sync = next;
            });
            self.report(&id, generation, state, error).await;
            match delay {
                Some(d) => tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = wake.notified() => {}
                    _ = tokio::time::sleep(d) => {}
                },
                None => tokio::select! {
                    _ = cancel.cancelled() => return,
                    _ = wake.notified() => {}
                },
            }
        }
    }

    async fn authorize(
        self: Arc<Self>,
        id: String,
        generation: u64,
        p: OAuthProvider,
        force: bool,
        cancel: CancellationToken,
    ) {
        if let Err(e) = self.purge(&id, generation).await {
            self.update(&id, generation, |s| {
                s.state = IntegrationState::Error;
                s.error = Some(e.message());
            });
            return;
        }
        if !force {
            let id2 = id.clone();
            if let Ok(Ok(Some(_))) = self.blocking(move |s| s.get(&id2)).await {
                return;
            }
        }
        let Some(open) = self.opener.clone() else {
            self.update(&id, generation, |s| {
                s.state = IntegrationState::NeedsAuth;
                s.error = Some("this app cannot open a browser for authorization".into());
            });
            return;
        };
        {
            let mut slots = self.lock();
            let Some(s) = slots.get_mut(&id).filter(|s| s.generation == generation) else {
                return;
            };
            if s.status.authorizing {
                return;
            }
            s.status.authorizing = true;
        }
        let r = oauth::authorize(&self.http, &p, &open, self.auth_timeout, &cancel).await;
        self.update(&id, generation, |s| s.authorizing = false);
        match r {
            Ok(t) => {
                let id2 = id.clone();
                let stored = self.blocking(move |s| s.set(&id2, &t)).await;
                if matches!(stored, Ok(Ok(()))) {
                    let wake = {
                        let mut slots = self.lock();
                        slots.get_mut(&id).map(|s| {
                            s.status.has_secret = true;
                            s.secret_seq += 1;
                            s.wake.clone()
                        })
                    };
                    if let Some(w) = wake {
                        w.notify_one();
                    }
                } else {
                    self.update(&id, generation, |s| {
                        s.error = Some("could not store the token in the OS keychain".into());
                    });
                }
            }
            Err(oauth::AuthError::Cancelled) => {}
            Err(e) => {
                let m = redact(&e.to_string(), &[]);
                self.update(&id, generation, |s| {
                    s.state = IntegrationState::NeedsAuth;
                    s.error = Some(m.clone());
                });
                self.report(&id, generation, IntegrationState::NeedsAuth, Some(m))
                    .await;
            }
        }
    }

    async fn capture(self: Arc<Self>, id: String, path: PathBuf, cancel: CancellationToken) {
        let Some(bin) = self.git.clone() else { return };
        let generation = self.lock().get(&id).map(|s| s.generation).unwrap_or(0);
        match git::snapshot(&bin, &path, &cancel).await {
            Ok(snap) => {
                let item = git::context_item(&snap, Utc::now());
                let b = Batch {
                    scope: "context".into(),
                    complete: false,
                    items: vec![item],
                    patches: Vec::new(),
                };
                if let Err(e) = self.upsert(&id, b).await {
                    let m = redact(&e.message(), &[]);
                    self.update(&id, generation, |s| {
                        s.state = IntegrationState::Error;
                        s.error = Some(m.clone());
                    });
                    self.report(&id, generation, IntegrationState::Error, Some(m))
                        .await;
                } else {
                    self.update(&id, generation, |s| {
                        s.state = IntegrationState::Ok;
                        s.error = None;
                        s.last_sync = Some(Utc::now());
                    });
                    self.report(&id, generation, IntegrationState::Ok, None)
                        .await;
                }
            }
            Err(e) => {
                let m = redact(&format!("capture failed: {e}"), &[]);
                self.update(&id, generation, |s| {
                    s.state = IntegrationState::Error;
                    s.error = Some(m.clone());
                });
                self.report(&id, generation, IntegrationState::Error, Some(m))
                    .await;
            }
        }
    }
}

/// The core's items as the integration's own actor.
struct CoreItems<'a> {
    inner: &'a Inner,
    id: &'a str,
}

impl ItemSource for CoreItems<'_> {
    fn list<'a>(&'a self, filter: Value) -> BoxFuture<'a, Result<Vec<Value>, SyncError>> {
        Box::pin(async move {
            let v = self.inner.dispatch(self.id, "tasks.list", filter).await?;
            Ok(v.get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default())
        })
    }
}

impl IntegrationHost for Runtime {
    fn validate(&self, kind: &str, settings: &Value) -> Result<Value, String> {
        let c = self
            .inner
            .connector(kind)
            .ok_or_else(|| format!("unknown integration kind {kind}"))?;
        c.validate(settings, &self.inner.http)
    }

    fn configured(&self, cfg: &IntegrationConfig, reauthorize: bool) {
        let inner = &self.inner;
        if inner.cancel.is_cancelled() {
            return;
        }
        let cancel = inner.cancel.child_token();
        let wake = Arc::new(Notify::new());
        let audience = |kind: &str, settings: &Value| {
            inner
                .connector(kind)
                .map(|c| c.secret_audience(settings))
                .unwrap_or_default()
        };
        let (generation, moved) = {
            let mut slots = inner.lock();
            let (generation, mut status, moved, secret_seq) = match slots.remove(&cfg.id) {
                Some(old) => {
                    old.cancel.cancel();
                    // A token stored for one service never goes to another:
                    // a new kind, source or host drops it (before the new
                    // run can read it).
                    let moved = old.cfg.kind != cfg.kind
                        || audience(&old.cfg.kind, &old.cfg.settings)
                            != audience(&cfg.kind, &cfg.settings);
                    let mut st = old.status;
                    if moved {
                        st.has_secret = false;
                    }
                    (
                        old.generation + 1,
                        st,
                        moved,
                        old.secret_seq + u64::from(moved),
                    )
                }
                None => (1, IntegrationStatus::default(), false, 0),
            };
            status = IntegrationStatus {
                state: if cfg.enabled {
                    IntegrationState::Pending
                } else {
                    IntegrationState::Disabled
                },
                error: None,
                authorizing: false,
                ..status
            };
            status.next_sync = None;
            slots.insert(
                cfg.id.clone(),
                Slot {
                    cfg: cfg.clone(),
                    status,
                    wake: wake.clone(),
                    cancel: cancel.clone(),
                    generation,
                    reported: None,
                    purge: false,
                    secret_seq,
                },
            );
            (generation, moved)
        };
        if moved && inner.secrets.delete(&cfg.id).is_err() {
            // Still stored: every sync and authorization tries again first
            // and does not use the old token meanwhile.
            if let Some(s) = inner.lock().get_mut(&cfg.id) {
                s.purge = true;
            }
        }
        if let Some(c) = inner.connector(&cfg.kind) {
            c.forget(&cfg.id);
            if cfg.enabled
                && let Some(p) = c.oauth(&cfg.settings)
            {
                let fut = inner.clone().authorize(
                    cfg.id.clone(),
                    generation,
                    p,
                    reauthorize,
                    cancel.clone(),
                );
                inner.tasks.spawn_on(fut, &inner.rt);
            }
        }
        let fut = inner.clone().run(cfg.id.clone(), generation, cancel, wake);
        inner.tasks.spawn_on(fut, &inner.rt);
    }

    fn removed(&self, id: &str) {
        let slot = self.inner.lock().remove(id);
        if let Some(s) = slot {
            s.cancel.cancel();
        }
        for c in self.inner.connectors.values() {
            c.forget(id);
        }
        let secrets = self.inner.secrets.clone();
        let id = id.to_string();
        self.inner.rt.spawn_blocking(move || {
            let _ = secrets.delete(&id);
        });
    }

    fn sync_now(&self, id: Option<&str>) -> Result<Vec<String>, String> {
        let slots = self.inner.lock();
        let mut queued = Vec::new();
        for (k, s) in slots.iter() {
            if id.is_some_and(|i| i != k) || !s.cfg.enabled {
                continue;
            }
            s.wake.notify_one();
            queued.push(k.clone());
        }
        if let Some(i) = id
            && queued.is_empty()
        {
            return Err(format!("integration {i} is not running"));
        }
        queued.sort();
        Ok(queued)
    }

    fn status(&self, id: &str) -> Option<IntegrationStatus> {
        self.inner.lock().get(id).map(|s| s.status.clone())
    }

    fn capture_context(
        &self,
        cfg: &IntegrationConfig,
        repo_path: &str,
    ) -> Result<CaptureTicket, String> {
        if self.inner.git.is_none() {
            return Err("git was not found on this computer".into());
        }
        let s = GitSettings::parse(&cfg.settings)?;
        let path = git::check_path(&s, repo_path)?;
        let cancel = self.inner.cancel.child_token();
        let fut = self
            .inner
            .clone()
            .capture(cfg.id.clone(), path.clone(), cancel);
        self.inner.tasks.spawn_on(fut, &self.inner.rt);
        Ok(CaptureTicket {
            integration: cfg.id.clone(),
            repo_path: path.to_string_lossy().into_owned(),
            queued: true,
        })
    }
}
