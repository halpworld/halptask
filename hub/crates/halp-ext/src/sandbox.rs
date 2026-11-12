//! The wasmtime sandbox: engine configuration, the ABI (§5.2) and one run
//! under the limits of §5.3.

use crate::error::{ExtError, clean_message};
use crate::limits::ExtLimits;
use wasmtime::{
    Caller, Config, Engine, Extern, ExternType, Instance, Linker, Memory, Module, ResourceLimiter,
    Store, StoreLimits, StoreLimitsBuilder, Trap, ValType,
};

/// The only import a module may have.
const IMPORT_MODULE: &str = "halp";
const IMPORT_LOG: &str = "log";

/// Builds the engine: fuel, epoch interruption, a bounded wasm stack, and
/// every proposal that could widen the surface switched off. The crate is
/// built without wasmtime's `cache`, `threads`, `component-model`, `gc`,
/// `async` and `wat` features, so those are absent, not just disabled.
pub(crate) fn engine(limits: &ExtLimits) -> Result<Engine, ExtError> {
    let mut config = Config::new();
    config
        .consume_fuel(true)
        .epoch_interruption(true)
        .max_wasm_stack(limits.max_wasm_stack)
        .wasm_multi_memory(false)
        .wasm_memory64(false)
        .wasm_shared_everything_threads(false)
        .wasm_stack_switching(false)
        .relaxed_simd_deterministic(true);
    Engine::new(&config).map_err(|e| ExtError::Internal(format!("engine setup failed: {e}")))
}

/// Compiles a binary module (text WAT is never accepted here).
pub(crate) fn compile(engine: &Engine, wasm: &[u8]) -> Result<Module, ExtError> {
    Module::from_binary(engine, wasm)
        .map_err(|e| ExtError::InvalidModule(clean_message(&e.to_string())))
}

/// Refuses any import other than `halp.log: (i32, i32) -> ()` before
/// instantiation, so there is no WASI and no imported memory or table.
pub(crate) fn check_imports(module: &Module) -> Result<(), ExtError> {
    for import in module.imports() {
        let ok = import.module() == IMPORT_MODULE
            && import.name() == IMPORT_LOG
            && match import.ty() {
                ExternType::Func(f) => {
                    let params: Vec<ValType> = f.params().collect();
                    params.len() == 2
                        && params.iter().all(|p| matches!(p, ValType::I32))
                        && f.results().len() == 0
                }
                _ => false,
            };
        if !ok {
            return Err(ExtError::Instantiate(format!(
                "module imports `{}.{}`; the only allowed import is `halp.log(i32, i32)`",
                clean_message(import.module()),
                clean_message(import.name())
            )));
        }
    }
    Ok(())
}

/// Wraps [`StoreLimits`] to remember that a memory or table limit was hit,
/// so the error can say so.
struct RunLimiter {
    inner: StoreLimits,
    hit: bool,
}

impl ResourceLimiter for RunLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let r = self.inner.memory_growing(current, desired, maximum);
        if !matches!(r, Ok(true)) {
            self.hit = true;
        }
        r
    }

    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let r = self.inner.table_growing(current, desired, maximum);
        if !matches!(r, Ok(true)) {
            self.hit = true;
        }
        r
    }

    fn instances(&self) -> usize {
        self.inner.instances()
    }

    fn tables(&self) -> usize {
        self.inner.tables()
    }

    fn memories(&self) -> usize {
        self.inner.memories()
    }
}

/// Per-run store data.
struct RunState {
    limiter: RunLimiter,
    log: String,
    log_cap: usize,
    log_truncated: bool,
}

/// What one run produced before output checks.
pub(crate) struct RawRun {
    pub output: Vec<u8>,
    pub log: String,
    pub log_truncated: bool,
}

/// Maps a wasmtime error from instantiation or a call to an [`ExtError`].
fn classify(err: &wasmtime::Error, limiter_hit: bool, other: fn(String) -> ExtError) -> ExtError {
    if limiter_hit {
        return ExtError::MemoryLimit;
    }
    match err.downcast_ref::<Trap>() {
        Some(Trap::OutOfFuel) => ExtError::FuelExhausted,
        Some(Trap::Interrupt) => ExtError::Timeout,
        Some(trap) => ExtError::Trap(trap.to_string()),
        None => other(clean_message(&err.to_string())),
    }
}

fn linker(engine: &Engine) -> Result<Linker<RunState>, ExtError> {
    let mut linker = Linker::new(engine);
    linker
        .func_wrap(
            IMPORT_MODULE,
            IMPORT_LOG,
            |mut caller: Caller<'_, RunState>, ptr: i32, len: i32| -> wasmtime::Result<()> {
                let Some(Extern::Memory(memory)) = caller.get_export("memory") else {
                    wasmtime::bail!("halp.log needs an exported memory");
                };
                // The cap applies to the cleaned text kept, so it never
                // grows past `log_cap` bytes (a replaced byte is 3 bytes).
                let st = caller.data();
                let remaining = st.log_cap.saturating_sub(st.log.len());
                let len = len as u32 as usize;
                let take = len.min(remaining);
                let mut buf = vec![0u8; take];
                if take > 0 {
                    memory
                        .read(&caller, ptr as u32 as usize, &mut buf)
                        .map_err(|_| wasmtime::format_err!("halp.log reads out of bounds"))?;
                }
                let st = caller.data_mut();
                if len > take {
                    st.log_truncated = true;
                }
                if take == 0 {
                    return Ok(());
                }
                if !st.log.is_empty() {
                    if st.log.len() >= st.log_cap {
                        st.log_truncated = true;
                        return Ok(());
                    }
                    st.log.push('\n');
                }
                for c in String::from_utf8_lossy(&buf).chars() {
                    let c = if (c.is_control() && c != '\t')
                        || halp_domain::limits::is_format_control(c)
                    {
                        '\u{FFFD}'
                    } else {
                        c
                    };
                    if st.log.len() + c.len_utf8() > st.log_cap {
                        st.log_truncated = true;
                        break;
                    }
                    st.log.push(c);
                }
                Ok(())
            },
        )
        .map_err(|e| ExtError::Internal(format!("linker setup failed: {e}")))?;
    Ok(linker)
}

/// Runs `halp_transform` once on `input` in a fresh store.
pub(crate) fn run(
    engine: &Engine,
    module: &Module,
    limits: &ExtLimits,
    input: &[u8],
) -> Result<RawRun, ExtError> {
    check_imports(module)?;
    let store_limits = StoreLimitsBuilder::new()
        .memory_size(limits.memory_bytes)
        .table_elements(limits.table_elements)
        .instances(1)
        .memories(1)
        .tables(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = Store::new(
        engine,
        RunState {
            limiter: RunLimiter {
                inner: store_limits,
                hit: false,
            },
            log: String::new(),
            log_cap: limits.log_bytes,
            log_truncated: false,
        },
    );
    store.limiter(|st| &mut st.limiter);
    store
        .set_fuel(limits.fuel)
        .map_err(|e| ExtError::Internal(format!("fuel setup failed: {e}")))?;
    store.set_epoch_deadline(limits.deadline_ticks());
    store.epoch_deadline_trap();

    let linker = linker(engine)?;
    let instance = linker
        .instantiate(&mut store, module)
        .map_err(|e| classify(&e, store.data().limiter.hit, ExtError::Instantiate))?;
    let output = call_transform(&mut store, &instance, input, limits.max_output_bytes)?;
    let st = store.into_data();
    Ok(RawRun {
        output,
        log: st.log,
        log_truncated: st.log_truncated,
    })
}

fn call_transform(
    store: &mut Store<RunState>,
    instance: &Instance,
    input: &[u8],
    max_output: usize,
) -> Result<Vec<u8>, ExtError> {
    let missing = |what: &str, e: wasmtime::Error| {
        ExtError::Instantiate(format!(
            "module must export {what}: {}",
            clean_message(&e.to_string())
        ))
    };
    let memory: Memory = instance
        .get_memory(&mut *store, "memory")
        .ok_or_else(|| ExtError::Instantiate("module must export its memory as `memory`".into()))?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&mut *store, "halp_alloc")
        .map_err(|e| missing("`halp_alloc(i32) -> i32`", e))?;
    let transform = instance
        .get_typed_func::<(i32, i32), i64>(&mut *store, "halp_transform")
        .map_err(|e| missing("`halp_transform(i32, i32) -> i64`", e))?;

    let len = i32::try_from(input.len())
        .map_err(|_| ExtError::Input("module input is too large".into()))?;
    let ptr = alloc
        .call(&mut *store, len)
        .map_err(|e| classify(&e, store.data().limiter.hit, ExtError::Trap))?;
    memory
        .write(&mut *store, ptr as u32 as usize, input)
        .map_err(|_| ExtError::Trap("halp_alloc returned memory out of bounds".into()))?;

    let packed = transform
        .call(&mut *store, (ptr, len))
        .map_err(|e| classify(&e, store.data().limiter.hit, ExtError::Trap))?;
    let packed = packed as u64;
    let out_ptr = (packed >> 32) as usize;
    let out_len = (packed & 0xffff_ffff) as usize;
    if out_len > max_output {
        return Err(ExtError::InvalidOutput(format!(
            "output is {out_len} bytes, more than the {max_output} allowed"
        )));
    }
    let mut out = vec![0u8; out_len];
    memory
        .read(&*store, out_ptr, &mut out)
        .map_err(|_| ExtError::InvalidOutput("output range is out of bounds".into()))?;
    Ok(out)
}
