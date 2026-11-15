//! The one door from the webview into the core.
//!
//! The webview is always the user: there is no way to pass an actor (or a
//! session) through this bridge. Agents reach the core over the MCP socket,
//! where the host authenticates them and sets their actor itself.

use halp_core::{Core, Ctx, Request, Response};
use serde::Deserialize;
use serde_json::{Value, json};

/// What the webview may send. Unknown fields (an `actor`, a `ctx`, ...) are
/// dropped here and never reach the core; the core's own [`Request`] then
/// checks the rest strictly.
#[derive(Debug, Deserialize)]
pub struct WebRequest {
    pub command: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub request_id: Option<String>,
}

/// Runs a webview request as the user and returns the contract envelope.
pub fn dispatch_as_user(core: &Core, req: WebRequest) -> Value {
    let args = if req.args.is_null() {
        json!({})
    } else {
        req.args
    };
    let request = Request {
        command: req.command,
        args,
        request_id: req.request_id,
    };
    core.dispatch(&Ctx::user(), request).to_value()
}

/// The envelope for a call made before the database is unlocked.
pub fn locked_response() -> Value {
    Response::Err(halp_core::CmdError::internal("the Hub is locked; unlock it first").into())
        .to_value()
}

#[cfg(test)]
mod tests {
    use super::*;
    use halp_core::SystemClock;
    use halp_store::{RawKeyProvider, Store};
    use std::sync::Arc;

    fn core() -> (tempfile::TempDir, Core) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store =
            Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).expect("store");
        let core = Core::with_store(store, Arc::new(SystemClock)).expect("core");
        (dir, core)
    }

    fn web(raw: Value) -> WebRequest {
        serde_json::from_value(raw).expect("web request")
    }

    #[test]
    fn dispatch_ignores_any_actor_from_the_webview() {
        let (_d, core) = core();
        let mut rx = core.subscribe();
        let res = dispatch_as_user(
            &core,
            web(json!({
                "command": "tasks.create",
                "args": {"title": "From the webview"},
                "request_id": "r1",
                "actor": "agent:mallory",
                "ctx": {"actor": "system"}
            })),
        );
        assert_eq!(res["ok"], json!(true), "{res}");
        assert_eq!(res["result"]["origin"], json!("user"));
        let ev = rx.try_recv().expect("an event");
        assert_eq!(ev.actor, "user");
        assert_eq!(ev.command, "tasks.create");
    }

    #[test]
    fn the_user_can_decide_consent_through_the_bridge() {
        let (_d, core) = core();
        let created = dispatch_as_user(
            &core,
            web(json!({"command": "tasks.create", "args": {"title": "Doomed"}})),
        );
        let id = created["result"]["id"].clone();
        // T3: the user's own delete still asks first.
        let first = dispatch_as_user(
            &core,
            web(json!({"command": "tasks.delete", "args": {"id": id}})),
        );
        assert_eq!(
            first["error"]["code"],
            json!("confirmation_required"),
            "{first}"
        );
        let cid = first["error"]["confirmation_id"].clone();
        let pending = dispatch_as_user(
            &core,
            web(json!({"command": "consent.get", "args": {"id": cid}})),
        );
        assert_eq!(pending["result"]["actor"], json!("user"));
        let ok = dispatch_as_user(
            &core,
            web(json!({"command": "consent.approve", "args": {"id": cid}})),
        );
        assert_eq!(ok["ok"], json!(true), "{ok}");
    }

    #[test]
    fn missing_args_default_to_an_empty_object() {
        let (_d, core) = core();
        let res = dispatch_as_user(&core, web(json!({"command": "role.list"})));
        assert_eq!(res["ok"], json!(true), "{res}");
    }

    #[test]
    fn locked_response_is_an_error_envelope() {
        let v = locked_response();
        assert_eq!(v["ok"], json!(false));
        assert_eq!(v["error"]["code"], json!("internal"));
    }

    /// Writes the real core's `commands.list` for `tests/contract.test.ts`:
    /// `HALP_COMMANDS_OUT=/path/commands.json cargo test -- --ignored dump_commands`
    #[test]
    #[ignore = "writes a file; run on demand"]
    fn dump_commands() {
        let (_d, core) = core();
        let out = std::env::var("HALP_COMMANDS_OUT").expect("HALP_COMMANDS_OUT");
        let res = dispatch_as_user(&core, web(json!({"command": "commands.list"})));
        assert_eq!(res["ok"], json!(true), "{res}");
        std::fs::write(
            out,
            serde_json::to_vec_pretty(&res["result"]).expect("json"),
        )
        .expect("write");
    }
}
