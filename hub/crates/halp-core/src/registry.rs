//! The command registry: name, description, tier, JSON Schemas and handler.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::envelope::CmdError;
use crate::policy::{Ctx, Tier};
use crate::state::State;

/// What `commands.list` returns per command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandDescriptor {
    pub name: String,
    pub description: String,
    pub tier: Tier,
    pub input_schema: Value,
    pub output_schema: Value,
    /// What the command touches, for client grants (`general`, `audit`, ...).
    #[serde(default = "default_class")]
    pub data_class: String,
    /// Whether MCP clients see it as a tool. Off for consent decisions and
    /// for client, grant, rule and other user-only administration.
    #[serde(default = "yes")]
    pub mcp: bool,
    /// Only `user` may run it; any other actor gets `denied` before the
    /// policy is asked.
    #[serde(default)]
    pub user_only: bool,
}

fn default_class() -> String {
    crate::policy::DEFAULT_DATA_CLASS.to_string()
}

fn yes() -> bool {
    true
}

pub(crate) type Handler =
    Box<dyn Fn(&mut State, &Ctx, Value) -> Result<Value, CmdError> + Send + Sync>;
pub(crate) type Validator = Box<dyn Fn(&Value) -> Result<(), CmdError> + Send + Sync>;

pub(crate) struct Command {
    pub descriptor: CommandDescriptor,
    pub handler: Handler,
    /// Checks the arguments' shape without running anything (before the
    /// policy is asked or a request is parked).
    pub validate: Validator,
}

fn decode<A: DeserializeOwned>(args: Value) -> Result<A, CmdError> {
    let args = if args.is_null() {
        Value::Object(Default::default())
    } else {
        args
    };
    serde_json::from_value(args).map_err(|e| CmdError::invalid(format!("invalid arguments: {e}")))
}

#[derive(Default)]
pub(crate) struct Registry {
    commands: BTreeMap<String, Command>,
}

/// The JSON Schema for `T`. Object schemas always carry `required` (empty
/// when nothing is), as MCP clients and the UI's form builder expect it.
fn schema<T: JsonSchema>() -> Value {
    let mut v = serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null);
    if let Value::Object(m) = &mut v
        && m.get("type").and_then(Value::as_str) == Some("object")
        && !m.contains_key("required")
    {
        m.insert("required".into(), Value::Array(Vec::new()));
    }
    v
}

impl Registry {
    /// Registers a typed handler. Arguments are deserialized strictly
    /// (unknown fields are rejected); a missing or `null` args value is `{}`.
    pub fn add<A, R>(
        &mut self,
        name: &str,
        description: &str,
        tier: Tier,
        f: fn(&mut State, &Ctx, A) -> Result<R, CmdError>,
    ) where
        A: DeserializeOwned + JsonSchema + 'static,
        R: Serialize + JsonSchema + 'static,
    {
        let descriptor = CommandDescriptor {
            name: name.to_string(),
            description: description.to_string(),
            tier,
            input_schema: schema::<A>(),
            output_schema: schema::<R>(),
            data_class: default_class(),
            mcp: true,
            user_only: false,
        };
        let handler: Handler = Box::new(move |state, ctx, args| {
            let a: A = decode(args)?;
            let r = f(state, ctx, a)?;
            Ok(serde_json::to_value(r)?)
        });
        let validate: Validator = Box::new(|args| decode::<A>(args.clone()).map(|_| ()));
        self.commands.insert(
            name.to_string(),
            Command {
                descriptor,
                handler,
                validate,
            },
        );
    }

    /// Registers an untyped command (for modules plugged in by the host).
    pub fn add_raw(
        &mut self,
        descriptor: CommandDescriptor,
        handler: Handler,
        validate: Validator,
    ) {
        self.commands.insert(
            descriptor.name.clone(),
            Command {
                descriptor,
                handler,
                validate,
            },
        );
    }

    fn tweak(&mut self, name: &str, f: impl Fn(&mut CommandDescriptor)) {
        if let Some(c) = self.commands.get_mut(name) {
            f(&mut c.descriptor);
        }
    }

    pub fn set_class(&mut self, name: &str, class: &str) {
        self.tweak(name, |d| d.data_class = class.to_string());
    }

    /// User only (implies hidden from MCP).
    pub fn user_only(&mut self, name: &str) {
        self.tweak(name, |d| {
            d.user_only = true;
            d.mcp = false;
        });
    }

    /// Not offered to MCP clients (still subject to the policy).
    pub fn hidden(&mut self, name: &str) {
        self.tweak(name, |d| d.mcp = false);
    }

    pub fn get(&self, name: &str) -> Option<&Command> {
        self.commands.get(name)
    }

    pub fn descriptors(&self) -> Vec<CommandDescriptor> {
        self.commands
            .values()
            .map(|c| c.descriptor.clone())
            .collect()
    }
}
