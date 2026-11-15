//! Run limits (§5.3).

use std::time::Duration;

/// How often the ticker thread advances the engine epoch while runs are
/// active.
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// Limits for every run of a host. [`ExtLimits::default`] is the §5.3
/// policy; tests lower or raise single values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtLimits {
    /// Fuel per run.
    pub fuel: u64,
    /// Wall-clock deadline per run (enforced by epoch interruption, so it
    /// is accurate to about one [`EPOCH_TICK`]).
    pub deadline: Duration,
    /// Linear memory cap.
    pub memory_bytes: usize,
    /// Table element cap.
    pub table_elements: usize,
    /// Wasm stack cap (an engine setting).
    pub max_wasm_stack: usize,
    /// Largest accepted wasm blob.
    pub max_module_bytes: usize,
    /// Largest serialized module input (`{inputs, config, now, today}`).
    pub max_input_bytes: usize,
    /// Largest accepted output.
    pub max_output_bytes: usize,
    /// Deepest accepted output.
    pub max_output_depth: usize,
    /// `halp.log` bytes kept per run; the rest is dropped.
    pub log_bytes: usize,
    /// Run pool threads.
    pub threads: usize,
    /// Runs waiting for a thread before new ones get [`crate::ExtError::Busy`].
    pub max_queued: usize,
    /// Compiled modules kept in the in-memory cache.
    pub max_cached_modules: usize,
}

impl Default for ExtLimits {
    fn default() -> Self {
        Self {
            fuel: 10_000_000,
            deadline: Duration::from_millis(200),
            memory_bytes: 16 * 1024 * 1024,
            table_elements: 10_000,
            max_wasm_stack: 512 * 1024,
            max_module_bytes: 2 * 1024 * 1024,
            max_input_bytes: 4 * 1024 * 1024,
            max_output_bytes: 256 * 1024,
            max_output_depth: 32,
            log_bytes: 4 * 1024,
            threads: 2,
            max_queued: 16,
            max_cached_modules: 32,
        }
    }
}

impl ExtLimits {
    /// The deadline in epoch ticks (at least one).
    pub(crate) fn deadline_ticks(&self) -> u64 {
        let tick = EPOCH_TICK.as_millis().max(1);
        let ticks = self.deadline.as_millis().div_ceil(tick);
        u64::try_from(ticks).unwrap_or(u64::MAX).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_the_plan() {
        let l = ExtLimits::default();
        assert_eq!(l.fuel, 10_000_000);
        assert_eq!(l.deadline_ticks(), 20);
        assert_eq!(l.memory_bytes, 16 << 20);
        assert_eq!(l.max_wasm_stack, 512 << 10);
        assert_eq!(l.max_module_bytes, 2 << 20);
        assert_eq!(l.max_output_bytes, 256 << 10);
        assert_eq!(l.log_bytes, 4096);
        assert_eq!(l.threads, 2);
    }

    #[test]
    fn deadline_rounds_up() {
        let l = ExtLimits {
            deadline: Duration::from_millis(1),
            ..ExtLimits::default()
        };
        assert_eq!(l.deadline_ticks(), 1);
    }
}
