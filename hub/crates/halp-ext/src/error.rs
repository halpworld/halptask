//! Errors of the extension host.

use thiserror::Error;

/// Longest message text taken from wasmtime or a module (it can carry
/// names from the module's own name section, so it is untrusted text).
const MAX_MESSAGE_CHARS: usize = 300;

/// Why a transform did not produce an output.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExtError {
    /// The module spec or the request is malformed (name, inputs, schema).
    #[error("invalid module spec: {0}")]
    InvalidSpec(String),
    /// The wasm blob is too large, not a binary module, or fails to compile.
    #[error("invalid module: {0}")]
    InvalidModule(String),
    /// The module imports something other than `halp.log`, lacks an ABI
    /// export, or fails to instantiate.
    #[error("module failed to instantiate: {0}")]
    Instantiate(String),
    /// An input command failed or the inputs are too large.
    #[error("module input failed: {0}")]
    Input(String),
    /// The run used up its fuel.
    #[error("module ran out of fuel")]
    FuelExhausted,
    /// The run passed its wall-clock deadline.
    #[error("module timed out")]
    Timeout,
    /// The module tried to use more memory or table space than allowed.
    #[error("module exceeded its memory limit")]
    MemoryLimit,
    /// The module trapped (unreachable, out-of-bounds access, stack
    /// overflow, ...).
    #[error("module trapped: {0}")]
    Trap(String),
    /// The output is too large, too deep, not JSON, or holds control or
    /// invisible format characters.
    #[error("invalid module output: {0}")]
    InvalidOutput(String),
    /// The output does not match the module's `output_schema`.
    #[error("module output does not match its output_schema: {0}")]
    SchemaMismatch(String),
    /// Too many runs are already queued.
    #[error("extension host is busy")]
    Busy,
    /// A host-side failure (engine setup, a run worker went away).
    #[error("extension host error: {0}")]
    Internal(String),
}

impl ExtError {
    /// A stable short code for the error kind (for the UI and logs).
    pub fn code(&self) -> &'static str {
        match self {
            ExtError::InvalidSpec(_) => "invalid_spec",
            ExtError::InvalidModule(_) => "invalid_module",
            ExtError::Instantiate(_) => "instantiate",
            ExtError::Input(_) => "input",
            ExtError::FuelExhausted => "fuel_exhausted",
            ExtError::Timeout => "timeout",
            ExtError::MemoryLimit => "memory_limit",
            ExtError::Trap(_) => "trap",
            ExtError::InvalidOutput(_) => "invalid_output",
            ExtError::SchemaMismatch(_) => "schema_mismatch",
            ExtError::Busy => "busy",
            ExtError::Internal(_) => "internal",
        }
    }
}

/// Makes untrusted text safe to show: control and invisible format
/// characters become U+FFFD and the text is cut to a bounded length.
pub(crate) fn clean_message(s: &str) -> String {
    let mut out: String = s
        .chars()
        .take(MAX_MESSAGE_CHARS)
        .map(|c| {
            if c.is_control() || halp_domain::limits::is_format_control(c) {
                '\u{FFFD}'
            } else {
                c
            }
        })
        .collect();
    if s.chars().nth(MAX_MESSAGE_CHARS).is_some() {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_message_strips_controls_and_truncates() {
        assert_eq!(clean_message("a\u{202E}b\nc"), "a\u{FFFD}b\u{FFFD}c");
        let long = "x".repeat(MAX_MESSAGE_CHARS + 10);
        let cleaned = clean_message(&long);
        assert_eq!(cleaned.chars().count(), MAX_MESSAGE_CHARS + 1);
        assert!(cleaned.ends_with('…'));
    }
}
