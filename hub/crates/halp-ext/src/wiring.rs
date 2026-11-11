//! P5-C2: Tier B wired to the core (docs/hub/P5-EXTENSIONS.md §5.4).
//!
//! - [`CoreInputs`] is the [`InputSource`] over [`Core::dispatch`] as
//!   `module:<pack_id>`. The policy engine allows a module T0 reads of the
//!   `general` class only and denies everything else outright (never
//!   parked), the core hides items of other classes, and every module read
//!   is audited (non-user T0 reads are). As a second lock, any item the
//!   module may not see is taken out of the result here, before the wasm
//!   ever runs.
//! - [`attach`] starts an [`ExtHost`] over the core and attaches it with
//!   [`Core::set_extension`]; the host holds only a `Weak<Core>`, so there
//!   is no reference cycle.
//! - [`run_computed`] runs the transform behind a placed `computed` widget:
//!   it looks the module up in the installed manifest (enabled packs whose
//!   trust may run code, wasm hashed again), takes the config from the
//!   widget on the current layout (never from the request) and runs it off
//!   the core lock.
//!
//! The core, `halpd` and the MCP host never link this crate (and so never
//! link wasmtime): only the desktop shell does.

use crate::error::{ExtError, clean_message};
use crate::host::{ExtHost, InputError, InputSource, RunOutput, TransformContext};
use crate::spec::{ALLOWED_INPUT_COMMANDS, InputSpec, ModuleSpec};
use halp_core::{Actor, Core, Ctx, ExtModule, PackTrust, Request, Response};
use serde_json::{Map, Value, json};
use std::sync::{Arc, Weak};

/// The widget kind of a Tier B widget (§5.1).
pub const COMPUTED_KIND: &str = "computed";

/// Deepest nesting the visibility filter walks. The core's own filter
/// stops at 70; anything deeper than this is refused rather than passed on
/// unchecked.
const MAX_FILTER_DEPTH: usize = 64;

/// The context a module's inputs run under.
pub fn module_ctx(pack_id: &str) -> Ctx {
    Ctx::new(Actor::Module(pack_id.to_string()))
}

/// Runs a module's inputs through the core as `module:<pack_id>`.
pub struct CoreInputs {
    core: Weak<Core>,
}

impl CoreInputs {
    pub fn new(core: &Arc<Core>) -> Self {
        Self {
            core: Arc::downgrade(core),
        }
    }
}

fn input_error(code: &str, message: impl Into<String>) -> InputError {
    InputError {
        code: code.to_string(),
        message: message.into(),
    }
}

impl InputSource for CoreInputs {
    fn fetch(&self, pack_id: &str, command: &str, args: &Value) -> Result<Value, InputError> {
        // The spec was checked already; check again at the boundary.
        if !ALLOWED_INPUT_COMMANDS.contains(&command) {
            return Err(input_error("denied", "modules may not call this command"));
        }
        let core = self
            .core
            .upgrade()
            .ok_or_else(|| input_error("internal", "the Hub is closed"))?;
        let ctx = module_ctx(pack_id);
        match core.dispatch(&ctx, Request::new(command, args.clone())) {
            Response::Ok(v) => only_visible(&core, &ctx, v),
            Response::Err(e) => {
                let code = serde_json::to_value(e.code)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_string))
                    .unwrap_or_else(|| "internal".into());
                Err(input_error(&code, e.message))
            }
        }
    }
}

/// An item view as the core returns it (`id`, `origin`, `type_id`): the
/// same shape the core's own filter recognises.
fn item_id(m: &Map<String, Value>) -> Option<&str> {
    if m.contains_key("origin") && m.contains_key("type_id") {
        m.get("id").and_then(Value::as_str)
    } else {
        None
    }
}

/// Takes every item `ctx` may not see out of a module's input: dropped
/// from lists, `null` in place, and `not_found` when the input itself is
/// such an item. For a module that is every item whose data class is not
/// `general` (the core already did this; this is the second lock, right
/// before the wasm call).
pub fn only_visible(core: &Core, ctx: &Ctx, mut v: Value) -> Result<Value, InputError> {
    if let Value::Object(m) = &v
        && let Some(id) = item_id(m)
        && !core.may_see_item(ctx, id)
    {
        return Err(input_error("not_found", "no such item"));
    }
    scrub(core, ctx, &mut v, 0)?;
    Ok(v)
}

fn scrub(core: &Core, ctx: &Ctx, v: &mut Value, depth: usize) -> Result<(), InputError> {
    if depth > MAX_FILTER_DEPTH {
        return Err(input_error("internal", "input nests too deeply to check"));
    }
    let hidden = |x: &Value| match x {
        Value::Object(m) => item_id(m).is_some_and(|id| !core.may_see_item(ctx, id)),
        _ => false,
    };
    match v {
        Value::Array(a) => {
            a.retain(|x| !hidden(x));
            for x in a {
                scrub(core, ctx, x, depth + 1)?;
            }
        }
        Value::Object(m) => {
            for x in m.values_mut() {
                if hidden(x) {
                    *x = Value::Null;
                } else {
                    scrub(core, ctx, x, depth + 1)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Starts the Tier B host over `core` and attaches it to the core.
pub fn attach(core: &Arc<Core>) -> Result<Arc<ExtHost>, ExtError> {
    let host = Arc::new(ExtHost::new(Arc::new(CoreInputs::new(core)))?);
    core.set_extension(host.clone());
    Ok(host)
}

/// Why a `computed` widget did not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComputedError {
    /// The pack is not installed, disabled, not trusted to run code, has no
    /// such module, or the widget is not a `computed` widget for it.
    Unavailable(String),
    /// The core is unavailable (stopped, or the host is not attached).
    Internal(String),
    /// The module itself failed (limits, trap, bad output, failed input).
    Module(ExtError),
}

/// A checked transform result, for the UI only.
#[derive(Debug, Clone, PartialEq)]
pub struct ComputedRun {
    pub output: Value,
    /// `halp.log` text (dev console only).
    pub log: String,
    pub trust: PackTrust,
    /// Untrusted text from the manifest.
    pub title: String,
    /// Untrusted text from the manifest.
    pub publisher: String,
}

/// The module spec the host runs, from the installed manifest's
/// declaration.
pub fn module_spec(m: &ExtModule) -> ModuleSpec {
    ModuleSpec {
        name: m.decl.name.clone(),
        wasm: m.decl.wasm.clone(),
        inputs: m
            .decl
            .inputs
            .iter()
            .map(|(k, i)| {
                let args = if i.args.is_null() {
                    json!({})
                } else {
                    i.args.clone()
                };
                (
                    k.clone(),
                    InputSpec {
                        command: i.command.clone(),
                        args,
                    },
                )
            })
            .collect(),
        output_schema: m.decl.output_schema.clone(),
    }
}

/// The config of `widget_id` on the current layout, when it is a
/// `computed` widget whose `transform` is `<pack_id>/<name>`.
fn widget_config(
    core: &Core,
    pack_id: &str,
    name: &str,
    widget_id: &str,
) -> Result<Value, ComputedError> {
    let layout = match core.dispatch(&Ctx::system(), Request::new("layout.get", json!({}))) {
        Response::Ok(v) => v,
        Response::Err(e) => return Err(ComputedError::Internal(clean_message(&e.message))),
    };
    let widget = layout["widgets"]
        .as_array()
        .and_then(|ws| ws.iter().find(|w| w["id"].as_str() == Some(widget_id)))
        .ok_or_else(|| ComputedError::Unavailable("no such widget on this layout".into()))?;
    if widget["kind"].as_str() != Some(COMPUTED_KIND) {
        return Err(ComputedError::Unavailable(
            "this widget is not a computed widget".into(),
        ));
    }
    let config = match &widget["config"] {
        Value::Object(m) => Value::Object(m.clone()),
        _ => json!({}),
    };
    let want = format!("{pack_id}/{name}");
    if config["transform"].as_str() != Some(want.as_str()) {
        return Err(ComputedError::Unavailable(
            "this widget does not use this transform".into(),
        ));
    }
    Ok(config)
}

/// Runs transform `<pack_id>/<name>` for the `computed` widget `widget_id`
/// on the current layout. Blocks until the run pool finishes; never holds
/// the core lock while the module runs.
pub fn run_computed(
    core: &Core,
    pack_id: &str,
    name: &str,
    widget_id: &str,
) -> Result<ComputedRun, ComputedError> {
    let host = core
        .extension::<ExtHost>()
        .ok_or_else(|| ComputedError::Internal("the extension host is not running".into()))?;
    let module = core.ext_module(pack_id, name).ok_or_else(|| {
        ComputedError::Unavailable(
            "the transform's pack is not installed, is disabled, or is not trusted to run code"
                .into(),
        )
    })?;
    let config = widget_config(core, pack_id, name, widget_id)?;
    let (now, today) = core
        .now_and_today()
        .ok_or_else(|| ComputedError::Internal("the core is unavailable".into()))?;
    let RunOutput { output, log, .. } = host
        .run_transform_blocking(
            pack_id,
            &module_spec(&module),
            &module.wasm,
            TransformContext { config, now, today },
        )
        .map_err(ComputedError::Module)?;
    Ok(ComputedRun {
        output,
        log,
        trust: module.trust,
        title: module.title,
        publisher: module.publisher,
    })
}
