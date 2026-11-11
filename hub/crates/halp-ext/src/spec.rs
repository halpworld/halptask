//! The module spec (one entry of a pack manifest's `modules` list, §5.1)
//! and its validation.

use crate::error::ExtError;
use halp_domain::limits;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// The read-only T0 commands a module may name as an input (§5.1).
pub const ALLOWED_INPUT_COMMANDS: &[&str] = &[
    "tasks.list",
    "tasks.get",
    "types.list",
    "types.get",
    "day.state",
    "focus.get",
    "layout.get",
];

/// Most inputs one module may declare.
pub const MAX_INPUTS: usize = 16;
/// Largest serialized `args` of one input.
pub const MAX_INPUT_ARGS_BYTES: usize = 16 * 1024;
/// Deepest `args` of one input.
pub const MAX_INPUT_ARGS_DEPTH: usize = 8;

/// One input of a module: a T0 command run as `module:<pack_id>` before
/// the module starts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSpec {
    pub command: String,
    #[serde(default = "empty_object")]
    pub args: Value,
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}

/// A pure-transform module as declared in a pack manifest:
/// `{ "name", "wasm", "inputs": {"<key>": {"command", "args"}}, "output_schema" }`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleSpec {
    pub name: String,
    /// The blob's path inside the pack (informational here: the caller
    /// passes the bytes).
    #[serde(default)]
    pub wasm: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, InputSpec>,
    pub output_schema: Value,
}

/// `[a-z][a-z0-9_-]{0,63}`.
fn is_simple_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some('a'..='z'))
        && s.len() <= 64
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_' | '-'))
}

/// A pack id as used in `module:<pack_id>`: 1..=128 chars of
/// `[a-z0-9._-]`, starting with a letter.
pub(crate) fn check_pack_id(pack_id: &str) -> Result<(), ExtError> {
    let mut chars = pack_id.chars();
    let ok = matches!(chars.next(), Some('a'..='z'))
        && pack_id.len() <= 128
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '.' | '_' | '-'));
    if ok {
        Ok(())
    } else {
        Err(ExtError::InvalidSpec(format!(
            "pack id {:?} is not a valid pack id",
            crate::error::clean_message(pack_id)
        )))
    }
}

impl ModuleSpec {
    /// Checks the name, the inputs and the output schema, and compiles the
    /// schema.
    pub(crate) fn validate(&self) -> Result<jsonschema::Validator, ExtError> {
        if !is_simple_name(&self.name) {
            return Err(ExtError::InvalidSpec(
                "module name must match [a-z][a-z0-9_-]{0,63}".into(),
            ));
        }
        if self.inputs.len() > MAX_INPUTS {
            return Err(ExtError::InvalidSpec(format!(
                "a module declares at most {MAX_INPUTS} inputs"
            )));
        }
        for (key, input) in &self.inputs {
            if !is_simple_name(key) {
                return Err(ExtError::InvalidSpec(
                    "input keys must match [a-z][a-z0-9_-]{0,63}".into(),
                ));
            }
            if !ALLOWED_INPUT_COMMANDS.contains(&input.command.as_str()) {
                return Err(ExtError::InvalidSpec(format!(
                    "input {key:?} uses command {:?}, which modules may not call (allowed: {})",
                    crate::error::clean_message(&input.command),
                    ALLOWED_INPUT_COMMANDS.join(", ")
                )));
            }
            if !input.args.is_object() {
                return Err(ExtError::InvalidSpec(format!(
                    "input {key:?} args must be a JSON object"
                )));
            }
            limits::json_bounds(
                "input args",
                &input.args,
                MAX_INPUT_ARGS_BYTES,
                MAX_INPUT_ARGS_DEPTH,
            )
            .map_err(|e| ExtError::InvalidSpec(e.to_string()))?;
            limits::check_json_text("input args", &input.args)
                .map_err(|e| ExtError::InvalidSpec(e.to_string()))?;
        }
        compile_output_schema(&self.output_schema)
    }
}

fn compile_output_schema(schema: &Value) -> Result<jsonschema::Validator, ExtError> {
    if !schema.is_object() {
        return Err(ExtError::InvalidSpec(
            "output_schema must be a JSON object".into(),
        ));
    }
    limits::json_bounds(
        "output_schema",
        schema,
        limits::MAX_SCHEMA_BYTES,
        limits::MAX_SCHEMA_DEPTH,
    )
    .map_err(|e| ExtError::InvalidSpec(e.to_string()))?;
    limits::check_json_text("output_schema", schema)
        .map_err(|e| ExtError::InvalidSpec(e.to_string()))?;
    if let Some(s) = schema.get("$schema")
        && s != "https://json-schema.org/draft/2020-12/schema"
    {
        return Err(ExtError::InvalidSpec(
            "output_schema must use JSON Schema draft 2020-12".into(),
        ));
    }
    jsonschema::draft202012::options()
        .should_validate_formats(true)
        .with_pattern_options(jsonschema::PatternOptions::regex())
        .build(schema)
        .map_err(|e| {
            ExtError::InvalidSpec(format!(
                "invalid output_schema: {}",
                crate::error::clean_message(&e.to_string())
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(v: Value) -> ModuleSpec {
        serde_json::from_value(v).expect("spec parses")
    }

    #[test]
    fn valid_spec_compiles() {
        let s = spec(json!({
            "name": "burn",
            "wasm": "wasm/burn.wasm",
            "inputs": {"open": {"command": "tasks.list", "args": {"status": "open"}}},
            "output_schema": {"type": "object"}
        }));
        assert!(s.validate().is_ok());
    }

    #[test]
    fn disallowed_input_command_is_rejected() {
        let s = spec(json!({
            "name": "x",
            "inputs": {"w": {"command": "tasks.create"}},
            "output_schema": {}
        }));
        assert!(matches!(s.validate(), Err(ExtError::InvalidSpec(_))));
    }

    #[test]
    fn bad_names_and_schemas_are_rejected() {
        let bad_name = spec(json!({"name": "Bad Name", "output_schema": {}}));
        assert!(bad_name.validate().is_err());
        let bad_schema = spec(json!({"name": "x", "output_schema": 3}));
        assert!(bad_schema.validate().is_err());
        let old_draft = spec(json!({
            "name": "x",
            "output_schema": {"$schema": "http://json-schema.org/draft-07/schema#"}
        }));
        assert!(old_draft.validate().is_err());
        let bad_args = spec(json!({
            "name": "x",
            "inputs": {"a": {"command": "tasks.list", "args": [1]}},
            "output_schema": {}
        }));
        assert!(bad_args.validate().is_err());
    }

    #[test]
    fn unknown_spec_fields_are_rejected() {
        let r: Result<ModuleSpec, _> =
            serde_json::from_value(json!({"name": "x", "output_schema": {}, "imports": []}));
        assert!(r.is_err());
    }

    #[test]
    fn pack_ids() {
        assert!(check_pack_id("acme.platform-sre").is_ok());
        assert!(check_pack_id("").is_err());
        assert!(check_pack_id("Acme").is_err());
        assert!(check_pack_id("a/b").is_err());
    }
}
