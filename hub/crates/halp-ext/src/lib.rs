//! HalpTask Hub extension host (P5 Tier B): runs pack modules that are pure
//! transforms in a locked-down wasmtime sandbox.
//!
//! - ABI (core wasm, no component model): the module exports `memory`,
//!   `halp_alloc(len: i32) -> i32` and `halp_transform(ptr: i32, len: i32)
//!   -> i64` (`ptr << 32 | len` of the output). Its only import may be
//!   `halp.log(ptr, len)`, capped at 4 KiB per run; any other import fails
//!   before instantiation, so there is no WASI, clock, randomness, file
//!   system or network.
//! - Input: `{inputs, config, now, today}` as JSON, where each input is a
//!   T0 command fetched through [`InputSource`] as `module:<pack_id>`
//!   before the module runs.
//! - Limits ([`ExtLimits`]): 10M fuel, a 200 ms epoch deadline, 16 MiB
//!   memory, one instance, memory and table, 10k table elements, a 512 KiB
//!   wasm stack, modules ≤ 2 MiB.
//! - Output: ≤ 256 KiB, depth ≤ 32, no control or invisible format
//!   characters (`check_json_text`), valid against the module's
//!   `output_schema`. It goes to the UI only.
//! - Compiled modules are cached in memory only, keyed by SHA-256; there is
//!   no on-disk cache. Runs execute on a 2-thread pool.
//! - Wiring ([`wiring`], P5-C2): [`wiring::CoreInputs`] runs inputs through
//!   `Core::dispatch` as `module:<pack_id>` (policy, class hiding, audit)
//!   and filters out any item the module may not see; [`wiring::attach`]
//!   hangs the host on the core; [`wiring::run_computed`] runs the
//!   transform behind a placed `computed` widget. Only the desktop shell
//!   links this crate, so `halpd` and the core never link wasmtime.

mod error;
mod host;
mod limits;
mod sandbox;
mod spec;
pub mod wiring;

pub use error::ExtError;
pub use host::{ExtHost, InputError, InputSource, RunOutput, TransformContext};
pub use limits::{EPOCH_TICK, ExtLimits};
pub use spec::{
    ALLOWED_INPUT_COMMANDS, InputSpec, MAX_INPUT_ARGS_BYTES, MAX_INPUT_ARGS_DEPTH, MAX_INPUTS,
    ModuleSpec,
};
