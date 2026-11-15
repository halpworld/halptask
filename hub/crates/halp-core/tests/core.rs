//! End-to-end tests through `Core::dispatch`: every command's happy path and
//! error codes, consent, undo/redo, grid rules, roles and focus, import.
#![allow(clippy::unwrap_used)]

use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::{
    Actor, ConsentOutcome, Core, Ctx, ErrorCode, FixedClock, Policy, Query, Request, Response,
    Tier, Verdict,
};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../halp-store/tests/fixtures");

struct H {
    core: Core,
    dir: TempDir,
}

fn clock() -> Arc<FixedClock> {
    Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        // A Tuesday.
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    })
}

fn open_at(path: &Path) -> Core {
    let store = Store::open(path, &RawKeyProvider::new([9; 32])).unwrap();
    Core::with_store(store, clock()).unwrap()
}

fn h() -> H {
    let dir = tempfile::tempdir().unwrap();
    let core = open_at(&dir.path().join("hub.db"));
    H { core, dir }
}

impl H {
    /// Runs a T3 call as the user and approves it.
    fn confirmed(&self, cmd: &str, args: Value) -> Value {
        let e = self.err_as(&Ctx::user(), cmd, args);
        assert_eq!(e.code, ErrorCode::ConfirmationRequired, "{cmd}: {e:?}");
        self.ok("consent.approve", json!({"id": e.confirmation_id.unwrap()}))
    }

    /// Pairs a client with this id; returns its token.
    fn pair(&self, id: &str) -> String {
        self.ok("clients.pair", json!({"name": id, "id": id}))["token"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn call(&self, ctx: &Ctx, cmd: &str, args: Value) -> Response {
        self.core.dispatch(ctx, Request::new(cmd, args))
    }

    fn ok_as(&self, ctx: &Ctx, cmd: &str, args: Value) -> Value {
        match self.call(ctx, cmd, args) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd} failed: {e:?}"),
        }
    }

    fn ok(&self, cmd: &str, args: Value) -> Value {
        self.ok_as(&Ctx::user(), cmd, args)
    }

    fn err_as(&self, ctx: &Ctx, cmd: &str, args: Value) -> halp_core::ErrorBody {
        match self.call(ctx, cmd, args) {
            Response::Ok(v) => panic!("{cmd} unexpectedly succeeded: {v}"),
            Response::Err(e) => e,
        }
    }

    fn err(&self, cmd: &str, args: Value) -> ErrorCode {
        self.err_as(&Ctx::user(), cmd, args).code
    }

    fn create(&self, args: Value) -> String {
        self.ok("tasks.create", args)["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn titles(&self, filter: Value) -> Vec<String> {
        self.ok("tasks.list", filter)["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap().to_string())
            .collect()
    }

    fn last_event(&self) -> Value {
        self.ok("history.list", json!({"limit": 1}))["events"][0].clone()
    }
}

fn overlaps(widgets: &[Value]) -> bool {
    let n = |w: &Value, k: &str| w[k].as_u64().unwrap();
    for (i, a) in widgets.iter().enumerate() {
        for b in &widgets[i + 1..] {
            if n(a, "x") < n(b, "x") + n(b, "w")
                && n(b, "x") < n(a, "x") + n(a, "w")
                && n(a, "y") < n(b, "y") + n(b, "h")
                && n(b, "y") < n(a, "y") + n(a, "h")
            {
                return true;
            }
        }
    }
    false
}

fn widget<'a>(layout: &'a Value, id: &str) -> &'a Value {
    layout["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == id)
        .unwrap()
}

// ---------------------------------------------------------------- envelope

#[test]
fn envelope_errors() {
    let h = h();
    assert_eq!(h.err("no.such", json!({})), ErrorCode::NotFound);
    let r = h.core.dispatch_json(&Ctx::user(), "{not json");
    assert_eq!(r.error().unwrap().code, ErrorCode::InvalidArgs);
    let r = h
        .core
        .dispatch_json(&Ctx::user(), r#"{"command":"tasks.list","extra":1}"#);
    assert_eq!(r.error().unwrap().code, ErrorCode::InvalidArgs);
    let big = format!(
        r#"{{"command":"tasks.list","args":{{"text":"{}"}}}}"#,
        "a".repeat(1024 * 1024 + 1)
    );
    assert_eq!(
        h.core
            .dispatch_json(&Ctx::user(), &big)
            .error()
            .unwrap()
            .code,
        ErrorCode::InvalidArgs
    );
    // Unknown args are rejected; missing args mean {}.
    assert_eq!(
        h.err("tasks.list", json!({"bogus": 1})),
        ErrorCode::InvalidArgs
    );
    let r = h.core.dispatch_json(
        &Ctx::user(),
        r#"{"command":"tasks.list","request_id":"r1"}"#,
    );
    assert!(r.is_ok());
    assert_eq!(r.to_value()["ok"], true);
}

// ------------------------------------------------------------------ tasks

#[test]
fn tasks_create_get_list() {
    let h = h();
    let work = h.create(json!({"title": "Work", "kind": "bullet", "tags": ["work"]}));
    let a = h.ok(
        "tasks.create",
        json!({"title": "Fix login", "parent_id": work, "tags": ["bug", "bug"], "due": "friday", "notes": "n"}),
    );
    assert_eq!(a["due"], "2026-11-13");
    assert_eq!(a["tags"], json!(["bug"]));
    assert_eq!(a["inherited_tags"], json!(["work"]));
    assert_eq!(a["status"], "todo");
    assert_eq!(a["version"], 1);
    let id = a["id"].as_str().unwrap();
    assert_eq!(h.ok("tasks.get", json!({"id": id}))["title"], "Fix login");
    h.create(json!({"title": "First", "parent_id": work, "index": 0, "status": "in_progress"}));
    h.create(json!({"title": "Security review", "roles": ["cso"]}));

    assert_eq!(h.titles(json!({"parent_id": work})), ["First", "Fix login"]);
    assert_eq!(
        h.titles(json!({"parent_id": null})),
        ["Work", "Security review"]
    );
    assert_eq!(
        h.titles(json!({"status": ["todo", "in_progress"], "kind": "task"})),
        ["First", "Fix login", "Security review"]
    );
    assert_eq!(
        h.titles(json!({"role": "developer"})),
        ["Work", "First", "Fix login"]
    );
    assert_eq!(h.titles(json!({"role": "cso"})).len(), 4);
    assert_eq!(
        h.titles(json!({"due_before": "2026-11-14", "due_after": "2026-11-12"})),
        ["Fix login"]
    );
    assert_eq!(h.titles(json!({"due_before": "2026-11-13"})).len(), 0);

    assert_eq!(
        h.err("tasks.get", json!({"id": "01NOPE"})),
        ErrorCode::NotFound
    );
    assert_eq!(h.err("tasks.get", json!({})), ErrorCode::InvalidArgs);
    assert_eq!(
        h.err("tasks.create", json!({"title": "  "})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.create", json!({"title": "x", "parent_id": "01NOPE"})),
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err(
            "tasks.create",
            json!({"title": "x", "due": "someday maybe"})
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err(
            "tasks.create",
            json!({"title": "x", "kind": "bullet", "status": "done"})
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.create", json!({"title": "x", "roles": ["pirate"]})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.list", json!({"role": "../x"})),
        ErrorCode::InvalidArgs
    );
}

#[test]
fn tasks_update() {
    let h = h();
    let id = h.create(json!({"title": "Draft", "due": "2026-11-20"}));
    let v = h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"title": "Draft memo", "due": null, "status": "in_progress", "roles": ["eng-manager"]}}),
    );
    assert_eq!(v["title"], "Draft memo");
    assert_eq!(v["due"], Value::Null);
    assert_eq!(v["status"], "in_progress");
    assert_eq!(v["roles"], json!(["eng-manager"]));
    assert_eq!(v["version"], 2);
    // No-op patches do not record anything.
    let before = h.last_event()["id"].clone();
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"title": "Draft memo"}}),
    );
    assert_eq!(h.last_event()["id"], before);

    assert_eq!(
        h.err(
            "tasks.update",
            json!({"id": id, "patch": {}, "if_version": 1})
        ),
        ErrorCode::Conflict
    );
    assert_eq!(
        h.err(
            "tasks.update",
            json!({"id": "01NOPE", "patch": {"title": "x"}})
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err("tasks.update", json!({"id": id, "patch": {"title": ""}})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.update", json!({"id": id, "title": "flat"})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.update", json!({"id": id, "patch": {"wat": 1}})),
        ErrorCode::InvalidArgs
    );

    // Typed fields are validated against the pack schema.
    let ok = json!({"id": id, "patch": {"type_id": "dev.pull_request", "fields": {"repo": "acme/web", "number": 7}}});
    assert_eq!(h.ok("tasks.update", ok)["type_id"], "dev.pull_request");
    let bad = json!({"id": id, "patch": {"fields": {"repo": "acme/web", "number": 0}}});
    assert_eq!(h.err("tasks.update", bad), ErrorCode::InvalidArgs);

    // A parent's status comes from its subtasks.
    h.create(json!({"title": "child", "parent_id": id}));
    assert_eq!(
        h.err(
            "tasks.update",
            json!({"id": id, "patch": {"status": "done"}})
        ),
        ErrorCode::InvalidArgs
    );
}

#[test]
fn tasks_move_complete_delete() {
    let h = h();
    let a = h.create(json!({"title": "A", "kind": "bullet"}));
    let b = h.create(json!({"title": "B"}));
    let c = h.create(json!({"title": "C", "parent_id": a}));
    let moved = h.ok("tasks.move", json!({"id": b, "parent_id": a, "order": 0}));
    assert_eq!(moved["parent_id"], a.as_str());
    assert_eq!(h.titles(json!({"parent_id": a})), ["B", "C"]);
    h.ok("tasks.move", json!({"id": b, "parent_id": null}));
    assert_eq!(h.titles(json!({"parent_id": null})), ["A", "B"]);
    assert_eq!(
        h.err("tasks.move", json!({"id": a, "parent_id": c})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.move", json!({"id": "01NOPE", "parent_id": null})),
        ErrorCode::NotFound
    );

    assert_eq!(h.ok("tasks.complete", json!({"id": b}))["status"], "done");
    assert_eq!(
        h.ok("tasks.complete", json!({"id": b, "done": false}))["status"],
        "todo"
    );
    assert_eq!(
        h.err("tasks.complete", json!({"id": a})),
        ErrorCode::InvalidArgs
    );
    h.ok("tasks.complete", json!({"id": a, "cascade": true}));
    assert_eq!(h.ok("tasks.get", json!({"id": c}))["status"], "done");
    assert_eq!(
        h.err("tasks.complete", json!({"id": "01NOPE"})),
        ErrorCode::NotFound
    );

    let d = h.confirmed("tasks.delete", json!({"id": a}));
    assert_eq!(d["deleted"].as_array().unwrap().len(), 2);
    assert_eq!(h.err("tasks.get", json!({"id": c})), ErrorCode::NotFound);
    assert_eq!(h.err("tasks.delete", json!({"id": a})), ErrorCode::NotFound);
    // Undo brings the whole subtree back.
    h.ok("history.undo", json!({}));
    assert_eq!(h.titles(json!({"parent_id": a})), ["C"]);
}

// ---------------------------------------------------------------- history

#[test]
fn history_undo_redo_list() {
    let h = h();
    assert_eq!(
        h.ok("history.undo", json!({})),
        json!({"event": null, "history_event": null})
    );
    assert_eq!(h.ok("history.redo", json!({}))["event"], Value::Null);

    let a = h.create(json!({"title": "A"}));
    let ea = h.last_event()["id"].as_str().unwrap().to_string();
    let b = h.create(json!({"title": "B"}));
    let eb = h.last_event()["id"].as_str().unwrap().to_string();
    h.ok("tasks.update", json!({"id": a, "patch": {"title": "A2"}}));

    let u = h.ok("history.undo", json!({}));
    assert_eq!(u["event"]["command"], "tasks.update");
    assert_eq!(u["history_event"]["command"], "history.undo");
    assert_eq!(h.ok("tasks.get", json!({"id": a}))["title"], "A");
    let r = h.ok("history.redo", json!({}));
    assert_eq!(r["event"]["command"], "tasks.update");
    assert_eq!(h.ok("tasks.get", json!({"id": a}))["title"], "A2");

    // Selective undo of an older event that nothing touched since.
    let u = h.ok("history.undo", json!({"event_id": eb}));
    assert_eq!(u["event"]["id"], eb.as_str());
    assert_eq!(h.err("tasks.get", json!({"id": b})), ErrorCode::NotFound);
    assert_eq!(h.ok("tasks.get", json!({"id": a}))["title"], "A2");
    h.ok("history.redo", json!({"event_id": eb}));
    assert_eq!(h.ok("tasks.get", json!({"id": b}))["title"], "B");
    // A was edited after it was created: undoing the create conflicts.
    assert_eq!(
        h.err("history.undo", json!({"event_id": ea})),
        ErrorCode::Conflict
    );
    assert_eq!(
        h.err("history.undo", json!({"event_id": "01NOPE"})),
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err("history.undo", json!({"event_id": "bad id!"})),
        ErrorCode::InvalidArgs
    );

    let l = h.ok("history.list", json!({"limit": 3}));
    let evs = l["events"].as_array().unwrap();
    assert_eq!(evs.len(), 3);
    assert_eq!(evs[0]["command"], "history.redo");
    assert_eq!(l["can_undo"], true);
    assert!(
        evs.iter()
            .all(|e| e["undoable"].is_boolean() && e["seq"].is_u64())
    );
    h.pair("bot");
    h.ok_as(
        &Ctx::agent("bot"),
        "tasks.create",
        json!({"title": "from bot"}),
    );
    let l = h.ok(
        "history.list",
        json!({"limit": 50, "actor_prefix": "agent:"}),
    );
    let evs = l["events"].as_array().unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(evs[0]["actor"], "agent:bot");
    let all = h.ok("history.list", json!({}))["events"]
        .as_array()
        .unwrap()
        .len();
    let older = h.ok("history.list", json!({"before": evs[0]["id"]}))["events"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(older, all - 1);
    // Limits above 500 are clamped.
    assert!(
        h.call(&Ctx::user(), "history.list", json!({"limit": 501}))
            .is_ok()
    );
    assert_eq!(
        h.err("history.list", json!({"limit": -1})),
        ErrorCode::InvalidArgs
    );
}

// ------------------------------------------------------------------ types

#[test]
fn types_list_get_define() {
    let h = h();
    let types = h.ok("types.list", json!({}))["types"]
        .as_array()
        .unwrap()
        .clone();
    let pr = types
        .iter()
        .find(|t| t["id"] == "dev.pull_request")
        .unwrap();
    assert_eq!(pr["origin"], "pack");
    assert!(types.iter().any(|t| t["origin"] == "builtin"));
    assert_eq!(
        h.ok("types.get", json!({"id": "dev.pull_request"}))["title"],
        "Pull request"
    );
    assert_eq!(
        h.err("types.get", json!({"id": "x.nope"})),
        ErrorCode::NotFound
    );

    let def = json!({
        "id": "me.book", "title": "Book",
        "schema": {"type": "object", "properties": {"pages": {"type": "integer"}}, "required": ["pages"]}
    });
    let t = h.ok("types.define", def);
    assert_eq!(t["origin"], "user");
    let id = h.create(json!({"title": "Dune", "type_id": "me.book", "fields": {"pages": 412}}));
    assert_eq!(h.ok("tasks.get", json!({"id": id}))["fields"]["pages"], 412);
    assert_eq!(
        h.err(
            "tasks.create",
            json!({"title": "x", "type_id": "me.book", "fields": {}})
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("tasks.create", json!({"title": "x", "type_id": "me.nope"})),
        ErrorCode::InvalidArgs
    );

    assert_eq!(
        h.err(
            "types.define",
            json!({"id": "dev.pull_request", "title": "Mine", "schema": {"type": "object"}})
        ),
        ErrorCode::Conflict
    );
    assert_eq!(
        h.err(
            "types.define",
            json!({"id": "NoDot", "title": "x", "schema": {}})
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err(
            "types.define",
            json!({"id": "me.bad", "title": "x", "schema": {"type": 5}})
        ),
        ErrorCode::InvalidArgs
    );

    // Undo removes the user type again.
    h.ok("history.undo", json!({}));
    h.ok("history.undo", json!({}));
    assert_eq!(
        h.err("types.get", json!({"id": "me.book"})),
        ErrorCode::NotFound
    );
}

// ----------------------------------------------------------------- layout

#[test]
fn layout_get_and_widgets() {
    let h = h();
    let l = h.ok("layout.get", json!({}));
    assert_eq!(l["role_id"], "developer");
    assert_eq!(
        h.ok("layout.get", json!({"role_id": "cso"}))["role_id"],
        "cso"
    );
    assert_eq!(
        h.err("layout.get", json!({"role_id": "pirate"})),
        ErrorCode::NotFound
    );

    let l = h.ok(
        "layout.add_widget",
        json!({"kind": "agenda", "title": "Later"}),
    );
    let ws = l["widgets"].as_array().unwrap();
    let new = ws
        .iter()
        .find(|w| w["id"].as_str().unwrap().starts_with("w-"))
        .unwrap();
    assert_eq!((new["w"].as_u64(), new["h"].as_u64()), (Some(4), Some(3)));
    assert_eq!(new["title"], "Later");
    assert!(!overlaps(ws));
    let nid = new["id"].as_str().unwrap().to_string();
    assert_eq!(
        h.err("layout.add_widget", json!({"kind": "risk-register"})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("layout.add_widget", json!({"kind": "agenda", "x": 1})),
        ErrorCode::InvalidArgs
    );

    let l = h.ok(
        "layout.update_widget",
        json!({"widget_id": nid, "patch": {"title": "Soon", "w": 6, "config": {"days": 3}}}),
    );
    let w = widget(&l, &nid);
    assert_eq!(
        (
            w["title"].as_str(),
            w["w"].as_u64(),
            w["config"]["days"].as_u64()
        ),
        (Some("Soon"), Some(6), Some(3))
    );
    assert!(!overlaps(l["widgets"].as_array().unwrap()));
    assert_eq!(
        h.err(
            "layout.update_widget",
            json!({"widget_id": "nope", "patch": {}})
        ),
        ErrorCode::NotFound
    );

    let l = h.ok("layout.remove_widget", json!({"widget_id": nid}));
    assert!(
        l["widgets"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["id"] != nid.as_str())
    );
    assert_eq!(
        h.err("layout.remove_widget", json!({"widget_id": nid})),
        ErrorCode::NotFound
    );
}

#[test]
fn layout_move_pushes_down_and_compacts() {
    let h = h();
    // Default developer grid: now(0,0,4x3) pr-queue(4,0,5x4)
    // focus-protector(9,0,3x3) context-restore(0,3,4x3) day-plan(9,3,3x7)
    // my-prs(4,4,5x3) ask-catcher(0,6,4x4) ci-radar(4,7,5x3).
    let l = h.ok(
        "layout.move_widget",
        json!({"widget_id": "now", "x": 4, "y": 0}),
    );
    let ws = l["widgets"].as_array().unwrap();
    assert!(!overlaps(ws));
    let now = widget(&l, "now");
    assert_eq!((now["x"].as_u64(), now["y"].as_u64()), (Some(4), Some(0)));
    // pr-queue was pushed below the moved widget.
    assert!(widget(&l, "pr-queue")["y"].as_u64().unwrap() >= 3);
    // context-restore floats up into the space "now" left.
    assert_eq!(widget(&l, "context-restore")["y"], 0);

    // Off-grid positions are refused before any grid math; a position that
    // fits the column but not the width is clamped into 12 columns.
    assert_eq!(
        h.err(
            "layout.move_widget",
            json!({"widget_id": "ci-radar", "x": 50, "y": 0})
        ),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err(
            "layout.move_widget",
            json!({"widget_id": "ci-radar", "x": 0, "y": 4294967295u64})
        ),
        ErrorCode::InvalidArgs
    );
    let l = h.ok(
        "layout.move_widget",
        json!({"widget_id": "ci-radar", "x": 11, "y": 0}),
    );
    // ci-radar is 5 wide, so x=11 clamps to 7.
    assert_eq!(widget(&l, "ci-radar")["x"], 7);
    assert!(!overlaps(l["widgets"].as_array().unwrap()));
    assert_eq!(
        h.err(
            "layout.move_widget",
            json!({"widget_id": "nope", "x": 0, "y": 0})
        ),
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err("layout.move_widget", json!({"widget_id": "now"})),
        ErrorCode::InvalidArgs
    );

    // Layout edits are undoable.
    h.ok("history.undo", json!({}));
    h.ok("history.undo", json!({}));
    let l = h.ok("layout.get", json!({}));
    assert_eq!(widget(&l, "now")["x"], 0);
}

// ---------------------------------------------------------- roles & focus

#[test]
fn roles_and_per_role_focus() {
    let h = h();
    let rl = h.ok("role.list", json!({}));
    let ids: Vec<&str> = rl["roles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["developer", "eng-manager", "cso"]);
    assert_eq!(rl["current"], "developer");
    assert_eq!(h.ok("role.get", json!({}))["id"], "developer");
    assert_eq!(h.ok("role.get", json!({"role_id": "cso"}))["id"], "cso");
    assert_eq!(
        h.err("role.get", json!({"id": "pirate"})),
        ErrorCode::NotFound
    );

    assert_eq!(h.ok("focus.get", json!({})), json!({"focus": null}));
    let a = h.create(json!({"title": "Ship it"}));
    let f = h.ok("focus.set", json!({"item_id": a}));
    assert_eq!(f["focus"]["id"], a.as_str());
    assert_eq!(
        h.err("focus.set", json!({"id": "01NOPE"})),
        ErrorCode::NotFound
    );

    let s = h.ok("role.switch", json!({"role_id": "cso"}));
    assert_eq!(s["role"]["id"], "cso");
    assert_eq!(s["layout"]["role_id"], "cso");
    assert_eq!(h.ok("focus.get", json!({}))["focus"], Value::Null);
    let day = h.ok("day.state", json!({}));
    assert_eq!(day["role"], "cso");
    assert_eq!(day["focus"], Value::Null);
    assert_eq!(
        h.err("role.switch", json!({"id": "pirate"})),
        ErrorCode::NotFound
    );

    h.ok("role.switch", json!({"id": "developer"}));
    assert_eq!(h.ok("focus.get", json!({}))["focus"]["id"], a.as_str());
    assert_eq!(h.ok("focus.clear", json!({})), json!({"focus": null}));
    assert_eq!(h.ok("focus.get", json!({}))["focus"], Value::Null);

    // Role and focus changes are logged but not undoable: undo skips them.
    let l = h.ok("history.list", json!({"limit": 1}));
    assert_eq!(l["events"][0]["undoable"], false);
    let u = h.ok("history.undo", json!({}));
    assert_eq!(u["event"]["command"], "tasks.create");
    assert_eq!(h.ok("role.get", json!({}))["id"], "developer");

    // The role survives a restart.
    h.ok("role.switch", json!({"id": "eng-manager"}));
    let path = h.dir.path().join("hub.db");
    drop(h.core);
    let core = open_at(&path);
    let r = core.dispatch(&Ctx::user(), Request::new("role.get", json!({})));
    assert_eq!(r.result().unwrap()["id"], "eng-manager");
}

#[test]
fn day_state_counts_and_agenda() {
    let h = h();
    h.create(json!({"title": "late", "due": "2026-11-09"}));
    h.create(json!({"title": "today", "due": "2026-11-10", "status": "in_progress"}));
    h.create(json!({"title": "next week", "due": "2026-11-17"}));
    h.create(json!({"title": "far", "due": "2026-12-30"}));
    h.create(json!({"title": "done", "due": "2026-11-09", "status": "done"}));
    h.create(json!({"title": "cso only", "due": "2026-11-09", "roles": ["cso"]}));
    let d = h.ok("day.state", json!({}));
    assert_eq!(d["date"], "2026-11-10");
    assert_eq!(
        d["counts"],
        json!({"overdue": 1, "today": 1, "in_progress": 1})
    );
    let agenda: Vec<&str> = d["agenda"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["title"].as_str().unwrap())
        .collect();
    assert_eq!(agenda, ["late", "today", "next week"]);
    assert_eq!(d["layout"]["role_id"], "developer");
    assert_eq!(
        h.ok("day.state", json!({"date": "2026-11-12"}))["counts"]["overdue"],
        2
    );
    assert_eq!(
        h.err("day.state", json!({"date": "tomorrow"})),
        ErrorCode::InvalidArgs
    );
}

// ---------------------------------------------------------------- capture

#[test]
fn capture_quick() {
    let h = h();
    let r = h.ok(
        "capture.quick",
        json!({"text": "Fix login #bug due:friday"}),
    );
    assert_eq!(
        r["parsed"],
        json!({"title": "Fix login", "tags": ["bug"], "due": "2026-11-13", "kind": "task", "status": "todo"})
    );
    assert_eq!(r["item"]["roles"], json!(["developer"]));
    assert_eq!(r["item"]["inherited_tags"], json!(["inbox"]));
    let inbox = r["item"]["parent_id"].clone();
    assert_eq!(h.titles(json!({"parent_id": null})), ["Inbox"]);

    let r = h.ok(
        "capture.quick",
        json!({"text": "- [~] Draft memo due:someday", "tags": ["memo"], "top": true}),
    );
    assert_eq!(r["item"]["title"], "Draft memo due:someday");
    assert_eq!(r["item"]["status"], "in_progress");
    assert_eq!(r["parsed"]["due"], Value::Null);
    assert_eq!(r["item"]["parent_id"], inbox);
    assert_eq!(
        h.titles(json!({"parent_id": inbox})),
        ["Draft memo due:someday", "Fix login"]
    );
    // One Inbox only.
    assert_eq!(h.titles(json!({"parent_id": null})), ["Inbox"]);

    assert_eq!(
        h.err("capture.quick", json!({"text": "   "})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("capture.quick", json!({"text": "x".repeat(4001)})),
        ErrorCode::InvalidArgs
    );
    assert_eq!(
        h.err("capture.quick", json!({"text": "x", "parent_id": "01NOPE"})),
        ErrorCode::NotFound
    );
    // Undoing the first capture removes the Inbox it created... once the
    // second capture is undone too.
    h.ok("history.undo", json!({}));
    h.ok("history.undo", json!({}));
    assert!(h.titles(json!({})).is_empty());
}

// --------------------------------------------------------------- commands

#[test]
fn commands_list_describes_every_command() {
    let h = h();
    let cmds = h.ok("commands.list", json!({}))["commands"]
        .as_array()
        .unwrap()
        .clone();
    let names: Vec<&str> = cmds.iter().map(|c| c["name"].as_str().unwrap()).collect();
    for n in [
        "tasks.create",
        "tasks.get",
        "tasks.list",
        "tasks.update",
        "tasks.move",
        "tasks.complete",
        "tasks.delete",
        "history.undo",
        "history.redo",
        "history.list",
        "types.list",
        "types.get",
        "types.define",
        "layout.get",
        "layout.add_widget",
        "layout.update_widget",
        "layout.move_widget",
        "layout.remove_widget",
        "role.list",
        "role.get",
        "role.switch",
        "focus.get",
        "focus.set",
        "focus.clear",
        "day.state",
        "capture.quick",
        "consent.list",
        "consent.get",
        "consent.approve",
        "consent.deny",
        "commands.list",
    ] {
        assert!(names.contains(&n), "missing {n}");
    }
    for c in &cmds {
        let s = &c["input_schema"];
        assert_eq!(s["type"], "object", "{}", c["name"]);
        assert!(
            s["required"].is_array(),
            "{} input_schema lacks required",
            c["name"]
        );
        assert!(!c["description"].as_str().unwrap().is_empty());
    }
    let tier = |n: &str| cmds.iter().find(|c| c["name"] == n).unwrap()["tier"].clone();
    assert_eq!(tier("tasks.list"), "T0");
    assert_eq!(tier("tasks.create"), "T1");
    assert_eq!(tier("tasks.delete"), "T3");
    let upd = cmds.iter().find(|c| c["name"] == "tasks.update").unwrap();
    assert_eq!(upd["input_schema"]["required"], json!(["id", "patch"]));
    assert_eq!(h.core.descriptors().len(), cmds.len());
}

// ------------------------------------------------------- policy & consent

struct DenyAgents;
impl Policy for DenyAgents {
    fn check(&self, q: &Query<'_>, _now: std::time::Instant) -> Verdict {
        if q.ctx.actor != Actor::User && q.tier >= Tier::T1 {
            Verdict::Deny {
                reason: "agents are read-only here".into(),
            }
        } else {
            Verdict::Allow {
                reason: "test".into(),
            }
        }
    }
}

#[test]
fn policy_deny() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([9; 32])).unwrap();
    let core = Core::with_policy(store, clock(), Arc::new(DenyAgents)).unwrap();
    let h = H { core, dir };
    let bot = Ctx::agent("bot");
    let e = h.err_as(&bot, "tasks.create", json!({"title": "x"}));
    assert_eq!(e.code, ErrorCode::Denied);
    assert_eq!(e.message, "agents are read-only here");
    assert!(h.call(&bot, "tasks.list", json!({})).is_ok());
    assert!(
        h.call(&Ctx::user(), "tasks.create", json!({"title": "x"}))
            .is_ok()
    );
}

#[test]
fn consent_approve_runs_as_original_actor() {
    let h = h();
    h.pair("bot");
    let mut rx = h.core.subscribe();
    let id = h.create(json!({"title": "Old"}));
    let bot = Ctx::agent("bot");

    let e = h.err_as(&bot, "tasks.delete", json!({"id": id}));
    assert_eq!(e.code, ErrorCode::ConfirmationRequired);
    let cid = e.confirmation_id.unwrap();
    // Not deleted yet.
    assert!(h.call(&Ctx::user(), "tasks.get", json!({"id": id})).is_ok());

    let p = h.ok("consent.list", json!({}))["pending"].clone();
    assert_eq!(p.as_array().unwrap().len(), 1);
    assert_eq!(p[0]["id"], cid.as_str());
    assert_eq!(p[0]["actor"], "agent:bot");
    assert_eq!(p[0]["tier"], "T3");
    assert_eq!(p[0]["args"], json!({"id": id}));
    assert_eq!(
        h.ok_as(&bot, "consent.get", json!({"confirmation_id": cid}))["command"],
        "tasks.delete"
    );

    // Only the user may decide.
    assert_eq!(
        h.err_as(&bot, "consent.approve", json!({"id": cid})).code,
        ErrorCode::Denied
    );
    assert_eq!(
        h.err_as(&bot, "consent.deny", json!({"id": cid})).code,
        ErrorCode::Denied
    );

    let r = h.ok("consent.approve", json!({"confirmation_id": cid}));
    assert_eq!(r["deleted"], json!([id]));
    assert_eq!(h.err("tasks.get", json!({"id": id})), ErrorCode::NotFound);
    // The approval is recorded after (and with) its outcome.
    let evs = h.ok("history.list", json!({"limit": 2}))["events"].clone();
    assert_eq!(
        (evs[0]["command"].as_str(), evs[0]["actor"].as_str()),
        (Some("consent.approve"), Some("user"))
    );
    assert_eq!(
        (evs[1]["command"].as_str(), evs[1]["actor"].as_str()),
        (Some("tasks.delete"), Some("agent:bot"))
    );
    // Only the requester can collect the outcome.
    assert!(h.core.take_consent_outcome(&Ctx::user(), &cid).is_none());
    match h.core.take_consent_outcome(&bot, &cid) {
        Some(ConsentOutcome::Approved(resp)) => {
            assert_eq!(resp.result().unwrap()["deleted"], json!([id]))
        }
        other => panic!("unexpected outcome {other:?}"),
    }
    assert!(h.core.take_consent_outcome(&bot, &cid).is_none());
    assert!(
        h.ok("consent.list", json!({}))["pending"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        h.err("consent.approve", json!({"id": cid})),
        ErrorCode::NotFound
    );

    let mut seen = Vec::new();
    while let Ok(e) = rx.try_recv() {
        seen.push((e.command, e.actor));
    }
    let want = |c: &str, a: &str| (c.to_string(), a.to_string());
    let pos = |x: &(String, String)| seen.iter().position(|s| s == x).unwrap();
    let (req, appr, del) = (
        pos(&want("consent.requested", "agent:bot")),
        pos(&want("consent.approve", "user")),
        pos(&want("tasks.delete", "agent:bot")),
    );
    assert!(req < del && del < appr);
}

#[test]
fn consent_deny_and_user_t3() {
    let h = h();
    let id = h.create(json!({"title": "Keep"}));
    // T3 needs confirmation even from the user.
    let e = h.err_as(&Ctx::user(), "tasks.delete", json!({"id": id}));
    assert_eq!(e.code, ErrorCode::ConfirmationRequired);
    let cid = e.confirmation_id.unwrap();
    assert_eq!(
        h.ok("consent.deny", json!({"id": cid})),
        json!({"denied": cid})
    );
    assert_eq!(
        h.core.take_consent_outcome(&Ctx::user(), &cid),
        Some(ConsentOutcome::Denied)
    );
    assert!(h.call(&Ctx::user(), "tasks.get", json!({"id": id})).is_ok());
    assert_eq!(h.last_event()["command"], "consent.deny");
    assert_eq!(
        h.err("consent.get", json!({"id": cid})),
        ErrorCode::NotFound
    );
    assert_eq!(
        h.err("consent.deny", json!({"id": cid})),
        ErrorCode::NotFound
    );
    assert_eq!(h.err("consent.approve", json!({})), ErrorCode::InvalidArgs);
    // Lower tiers pass straight through.
    h.pair("bot");
    assert!(
        h.call(&Ctx::agent("bot"), "tasks.create", json!({"title": "x"}))
            .is_ok()
    );
}

// ------------------------------------------------------------------ events

#[test]
fn changes_are_broadcast() {
    let h = h();
    let mut rx = h.core.subscribe();
    let id = h.create(json!({"title": "A"}));
    let e = rx.try_recv().unwrap();
    assert_eq!(
        (
            e.command.as_str(),
            e.entity.kind.as_str(),
            e.entity.id.as_str()
        ),
        ("tasks.create", "item", id.as_str())
    );
    // Reads and failed writes broadcast nothing.
    h.ok("tasks.list", json!({}));
    h.err(
        "tasks.update",
        json!({"id": "01NOPE", "patch": {"title": "x"}}),
    );
    assert!(rx.try_recv().is_err());
    h.ok("history.undo", json!({}));
    assert_eq!(rx.try_recv().unwrap().command, "history.undo");
}

// ------------------------------------------------------------------ import

fn import_args(path: &str, passphrase: Option<&str>) -> Value {
    let bytes = std::fs::read(format!("{FIXTURES}/{path}")).unwrap();
    let mut a = json!({
        "content_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
        "file_name": path,
        "utc_offset_minutes": 0,
    });
    if let Some(p) = passphrase {
        a["passphrase"] = json!(p);
    }
    a
}

#[test]
fn import_tui_plain_and_encrypted() {
    let h = h();
    h.create(json!({"title": "Existing"}));
    let r = h.confirmed("import.tui", import_args("tui_plain.pb", None));
    assert_eq!(r["items"], 14);
    assert_eq!(r["roots"], 4);
    let roots = h.titles(json!({"parent_id": null}));
    assert_eq!(roots[0], "Existing");
    assert_eq!(roots.len(), 5);
    let fix = h.titles(json!({"text": "Fix login"}));
    assert_eq!(fix, ["Fix login"]);
    let ev = h.ok("history.list", json!({"limit": 20}))["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"] == "import.tui")
        .unwrap()
        .clone();
    assert_eq!(ev["undoable"], true);
    // One undo removes the whole import.
    h.ok("history.undo", json!({}));
    assert_eq!(h.titles(json!({})), ["Existing"]);

    let fail = |args: Value| {
        let e = h.err_as(&Ctx::user(), "import.tui", args);
        assert_eq!(e.code, ErrorCode::ConfirmationRequired);
        let e = h.err_as(
            &Ctx::user(),
            "consent.approve",
            json!({"id": e.confirmation_id.unwrap()}),
        );
        assert_eq!(e.code, ErrorCode::InvalidArgs, "{e:?}");
    };
    fail(import_args("tui_encrypted.pb", None));
    fail(import_args("tui_encrypted.pb", Some("wrong")));
    let r = h.confirmed(
        "import.tui",
        import_args("tui_encrypted.pb", Some("correct horse battery staple")),
    );
    assert!(r["items"].as_u64().unwrap() > 0);
    fail(
        json!({"content_base64": base64::engine::general_purpose::STANDARD.encode(b"\x00\x01garbage"), "file_name": "x.pb"}),
    );
    // Agents cannot import at all, and the consent prompt hides secrets.
    h.pair("bot");
    let e = h.err_as(
        &Ctx::agent("bot"),
        "import.tui",
        import_args("tui_plain.pb", None),
    );
    assert_eq!(e.code, ErrorCode::Denied);
    let e = h.err_as(
        &Ctx::user(),
        "import.tui",
        import_args("tui_encrypted.pb", Some("s3cret")),
    );
    let p = h.ok("consent.get", json!({"id": e.confirmation_id.unwrap()}));
    assert_eq!(p["args"]["passphrase"], "[redacted]");
    assert!(
        p["args"]["content_base64"]
            .as_str()
            .unwrap()
            .starts_with('<')
    );
    assert!(!p.to_string().contains("s3cret"));
}

/// `docs/fixtures/commands.json` is `halpd schema`; regenerate it with
/// `cargo run -p halpd -- schema > docs/fixtures/commands.json`.
#[test]
fn the_commands_fixture_is_current() {
    let dir = tempfile::tempdir().unwrap();
    let store = halp_store::Store::open(
        &dir.path().join("hub.db"),
        &halp_store::RawKeyProvider::new([0; 32]),
    )
    .unwrap();
    let core = Core::with_store(store, Arc::new(halp_core::SystemClock)).unwrap();
    let mut cmds = core.descriptors();
    cmds.sort_by(|a, b| a.name.cmp(&b.name));
    let want: serde_json::Value =
        serde_json::from_str(include_str!("../../../docs/fixtures/commands.json")).unwrap();
    assert_eq!(
        serde_json::to_value(serde_json::json!({ "commands": cmds })).unwrap(),
        want,
        "regenerate docs/fixtures/commands.json with `cargo run -p halpd -- schema`"
    );
}
