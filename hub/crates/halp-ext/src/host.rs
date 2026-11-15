//! [`ExtHost`]: the in-memory module cache, the epoch ticker, the run pool
//! and the run API.

use crate::error::{ExtError, clean_message};
use crate::limits::{EPOCH_TICK, ExtLimits};
use crate::sandbox;
use crate::spec::{ModuleSpec, check_pack_id};
use chrono::{DateTime, NaiveDate, Utc};
use halp_domain::limits as dlimits;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, Thread};
use std::time::Duration;
use wasmtime::{Engine, Module};

/// Native stack of a run thread: well above the 512 KiB wasm stack cap plus
/// host frames.
const RUN_THREAD_STACK: usize = 4 * 1024 * 1024;

/// Why an input command failed (as reported by the [`InputSource`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    /// A short error code (for example the core's `denied` or `not_found`).
    pub code: String,
    pub message: String,
}

/// Runs a module's declared input commands. P5-C2 implements this over
/// `Core::dispatch` with the actor `module:<pack_id>`, so the policy, class
/// hiding and audit apply. The host calls it on a run thread, before the
/// module starts and while holding none of its own locks.
pub trait InputSource: Send + Sync {
    /// Runs one read-only T0 `command` (one of
    /// [`crate::ALLOWED_INPUT_COMMANDS`]) with `args` as `module:<pack_id>`
    /// and returns its result.
    fn fetch(&self, pack_id: &str, command: &str, args: &Value) -> Result<Value, InputError>;
}

/// Run-time context passed to the module next to its inputs.
#[derive(Debug, Clone, PartialEq)]
pub struct TransformContext {
    /// The widget's config for this transform (passed through as `config`).
    pub config: Value,
    /// Passed as `now` (RFC 3339).
    pub now: DateTime<Utc>,
    /// Passed as `today` (`YYYY-MM-DD`, the user's local day).
    pub today: NaiveDate,
}

/// A checked transform result. The output goes to the UI only.
#[derive(Debug, Clone, PartialEq)]
pub struct RunOutput {
    /// The module's JSON output, validated against its `output_schema`.
    pub output: Value,
    /// Text the module wrote with `halp.log` (for the dev console only),
    /// cleaned of control and invisible format characters.
    pub log: String,
    /// Whether `halp.log` output was cut at the per-run cap.
    pub log_truncated: bool,
}

/// State shared by the host and its run jobs.
struct Shared {
    engine: Engine,
    limits: ExtLimits,
    source: Arc<dyn InputSource>,
    cache: Mutex<HashMap<[u8; 32], Module>>,
    /// Runs submitted and not finished (running or queued).
    pending: AtomicUsize,
    /// Runs executing wasm right now (the ticker only ticks while > 0).
    active: Arc<AtomicUsize>,
    ticker: Thread,
}

struct Request {
    pack_id: String,
    spec: ModuleSpec,
    validator: jsonschema::Validator,
    wasm: Vec<u8>,
    ctx: TransformContext,
}

type Job = Box<dyn FnOnce() + Send + 'static>;

/// The Tier B extension host. Cheap to share behind an `Arc`.
pub struct ExtHost {
    shared: Arc<Shared>,
    jobs: Mutex<Option<mpsc::Sender<Job>>>,
    stop: Arc<AtomicBool>,
}

impl std::fmt::Debug for ExtHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtHost")
            .field("limits", &self.shared.limits)
            .finish_non_exhaustive()
    }
}

impl ExtHost {
    /// A host with the §5.3 limits.
    pub fn new(source: Arc<dyn InputSource>) -> Result<Self, ExtError> {
        Self::with_limits(source, ExtLimits::default())
    }

    /// A host with custom limits (tests).
    pub fn with_limits(source: Arc<dyn InputSource>, limits: ExtLimits) -> Result<Self, ExtError> {
        let engine = sandbox::engine(&limits)?;
        let stop = Arc::new(AtomicBool::new(false));
        let active = Arc::new(AtomicUsize::new(0));
        let ticker = spawn_ticker(engine.clone(), active.clone(), stop.clone())?;

        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..limits.threads.max(1) {
            let rx = rx.clone();
            thread::Builder::new()
                .name(format!("halp-ext-run-{i}"))
                .stack_size(RUN_THREAD_STACK)
                .spawn(move || {
                    loop {
                        let job = match rx.lock() {
                            Ok(guard) => guard.recv(),
                            Err(_) => break,
                        };
                        match job {
                            // A panicking job drops its reply sender; the
                            // caller then gets `Internal`. The thread lives on.
                            Ok(job) => {
                                let _ = catch_unwind(AssertUnwindSafe(job));
                            }
                            Err(_) => break,
                        }
                    }
                })
                .map_err(|e| ExtError::Internal(format!("cannot start run thread: {e}")))?;
        }

        Ok(Self {
            shared: Arc::new(Shared {
                engine,
                limits,
                source,
                cache: Mutex::new(HashMap::new()),
                pending: AtomicUsize::new(0),
                active,
                ticker,
            }),
            jobs: Mutex::new(Some(tx)),
            stop,
        })
    }

    /// The limits this host enforces.
    pub fn limits(&self) -> &ExtLimits {
        &self.shared.limits
    }

    /// How many compiled modules are cached (in memory only).
    pub fn cached_module_count(&self) -> usize {
        lock(&self.shared.cache).len()
    }

    /// Runs a pure transform: fetches the declared inputs through the
    /// [`InputSource`] as `module:<pack_id>`, runs `halp_transform` on
    /// `{inputs, config, now, today}` on the run pool, and checks the output
    /// (size, depth, text, `output_schema`).
    pub async fn run_transform(
        &self,
        pack_id: &str,
        spec: &ModuleSpec,
        wasm: &[u8],
        ctx: TransformContext,
    ) -> Result<RunOutput, ExtError> {
        let req = self.prepare(pack_id, spec, wasm, ctx)?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.submit(req, move |r| {
            let _ = tx.send(r);
        })?;
        rx.await
            .map_err(|_| ExtError::Internal("the run was aborted".into()))?
    }

    /// [`ExtHost::run_transform`] for synchronous callers: blocks the
    /// calling thread until the run pool finishes.
    pub fn run_transform_blocking(
        &self,
        pack_id: &str,
        spec: &ModuleSpec,
        wasm: &[u8],
        ctx: TransformContext,
    ) -> Result<RunOutput, ExtError> {
        let req = self.prepare(pack_id, spec, wasm, ctx)?;
        let (tx, rx) = mpsc::channel();
        self.submit(req, move |r| {
            let _ = tx.send(r);
        })?;
        rx.recv()
            .map_err(|_| ExtError::Internal("the run was aborted".into()))?
    }

    /// Cheap checks done on the caller's thread before queueing.
    fn prepare(
        &self,
        pack_id: &str,
        spec: &ModuleSpec,
        wasm: &[u8],
        ctx: TransformContext,
    ) -> Result<Request, ExtError> {
        check_pack_id(pack_id)?;
        let max = self.shared.limits.max_module_bytes;
        if wasm.len() > max {
            return Err(ExtError::InvalidModule(format!(
                "module is {} bytes, more than the {max} allowed",
                wasm.len()
            )));
        }
        let validator = spec.validate()?;
        Ok(Request {
            pack_id: pack_id.to_string(),
            spec: spec.clone(),
            validator,
            wasm: wasm.to_vec(),
            ctx,
        })
    }

    fn submit(
        &self,
        req: Request,
        reply: impl FnOnce(Result<RunOutput, ExtError>) + Send + 'static,
    ) -> Result<(), ExtError> {
        let limits = &self.shared.limits;
        let cap = limits.threads.max(1) + limits.max_queued;
        let prev = self.shared.pending.fetch_add(1, Ordering::SeqCst);
        if prev >= cap {
            self.shared.pending.fetch_sub(1, Ordering::SeqCst);
            return Err(ExtError::Busy);
        }
        let shared = self.shared.clone();
        let job: Job = Box::new(move || {
            let _pending = PendingGuard(&shared.pending);
            reply(execute(&shared, &req));
        });
        let sent = lock(&self.jobs).as_ref().map(|tx| tx.send(job));
        match sent {
            Some(Ok(())) => Ok(()),
            _ => {
                self.shared.pending.fetch_sub(1, Ordering::SeqCst);
                Err(ExtError::Internal("the run pool has stopped".into()))
            }
        }
    }
}

impl Drop for ExtHost {
    fn drop(&mut self) {
        // Workers exit once the queue drains; the ticker on its next wake.
        lock(&self.jobs).take();
        self.stop.store(true, Ordering::SeqCst);
        self.shared.ticker.unpark();
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

struct PendingGuard<'a>(&'a AtomicUsize);

impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);

impl<'a> ActiveGuard<'a> {
    fn new(active: &'a AtomicUsize, ticker: &Thread) -> Self {
        active.fetch_add(1, Ordering::SeqCst);
        ticker.unpark();
        Self(active)
    }
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Advances the engine epoch every [`EPOCH_TICK`] while a run is active,
/// and sleeps otherwise.
fn spawn_ticker(
    engine: Engine,
    active: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
) -> Result<Thread, ExtError> {
    let handle = thread::Builder::new()
        .name("halp-ext-epoch".into())
        .spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                if active.load(Ordering::SeqCst) == 0 {
                    thread::park_timeout(Duration::from_secs(1));
                    continue;
                }
                thread::sleep(EPOCH_TICK);
                engine.increment_epoch();
            }
        })
        .map_err(|e| ExtError::Internal(format!("cannot start epoch ticker: {e}")))?;
    Ok(handle.thread().clone())
}

/// Returns the compiled module for `wasm`, compiling it outside the cache
/// lock on a miss.
fn module_for(shared: &Shared, wasm: &[u8]) -> Result<Module, ExtError> {
    let key: [u8; 32] = Sha256::digest(wasm).into();
    if let Some(m) = lock(&shared.cache).get(&key) {
        return Ok(m.clone());
    }
    let module = sandbox::compile(&shared.engine, wasm)?;
    let mut cache = lock(&shared.cache);
    if cache.len() >= shared.limits.max_cached_modules.max(1)
        && let Some(evict) = cache.keys().next().copied()
    {
        cache.remove(&evict);
    }
    cache.insert(key, module.clone());
    Ok(module)
}

fn execute(shared: &Shared, req: &Request) -> Result<RunOutput, ExtError> {
    let limits = &shared.limits;
    let module = module_for(shared, &req.wasm)?;
    sandbox::check_imports(&module)?;

    // Inputs first, with no lock held: the source dispatches into the core.
    let mut inputs = Map::new();
    for (key, input) in &req.spec.inputs {
        let value = shared
            .source
            .fetch(&req.pack_id, &input.command, &input.args)
            .map_err(|e| {
                ExtError::Input(format!(
                    "input {key:?} ({}) failed: {}: {}",
                    input.command,
                    clean_message(&e.code),
                    clean_message(&e.message)
                ))
            })?;
        inputs.insert(key.clone(), value);
    }
    let envelope = json!({
        "inputs": inputs,
        "config": req.ctx.config,
        "now": req.ctx.now.to_rfc3339(),
        "today": req.ctx.today.format("%Y-%m-%d").to_string(),
    });
    let input = serde_json::to_vec(&envelope)
        .map_err(|e| ExtError::Internal(format!("cannot encode module input: {e}")))?;
    if input.len() > limits.max_input_bytes {
        return Err(ExtError::Input(format!(
            "module input is {} bytes, more than the {} allowed",
            input.len(),
            limits.max_input_bytes
        )));
    }

    let raw = {
        let _active = ActiveGuard::new(&shared.active, &shared.ticker);
        sandbox::run(&shared.engine, &module, limits, &input)?
    };
    let output = check_output(&raw.output, limits, &req.validator)?;
    Ok(RunOutput {
        output,
        log: raw.log,
        log_truncated: raw.log_truncated,
    })
}

/// Output checks (§5.1): size, JSON, depth, text, then `output_schema`.
fn check_output(
    bytes: &[u8],
    limits: &ExtLimits,
    validator: &jsonschema::Validator,
) -> Result<Value, ExtError> {
    if bytes.len() > limits.max_output_bytes {
        return Err(ExtError::InvalidOutput(format!(
            "output is {} bytes, more than the {} allowed",
            bytes.len(),
            limits.max_output_bytes
        )));
    }
    let value: Value = serde_json::from_slice(bytes).map_err(|e| {
        ExtError::InvalidOutput(format!(
            "output is not valid JSON: {}",
            clean_message(&e.to_string())
        ))
    })?;
    if dlimits::json_depth(&value) > limits.max_output_depth {
        return Err(ExtError::InvalidOutput(format!(
            "output nests deeper than {} levels",
            limits.max_output_depth
        )));
    }
    dlimits::check_json_text("module output", &value)
        .map_err(|e| ExtError::InvalidOutput(e.to_string()))?;
    if let Err(e) = validator.validate(&value) {
        return Err(ExtError::SchemaMismatch(clean_message(&format!(
            "at {:?}: {}",
            e.instance_path().as_str(),
            e
        ))));
    }
    Ok(value)
}
