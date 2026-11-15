//! `grants.set` accepts the built-in data classes and any class a task type
//! in the registry declares with `x-halp-data-class`; nothing else.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::{Core, Ctx, ErrorCode, FixedClock, Request, Response, packs};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

struct H {
    core: Core,
    _dir: TempDir,
}

fn h() -> H {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap();
    let clock = Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    });
    let h = H {
        core: Core::with_store(store, clock).unwrap(),
        _dir: dir,
    };
    h.ok("clients.pair", json!({"name": "bot", "id": "bot"}));
    h
}

impl H {
    fn call(&self, cmd: &str, args: Value) -> Response {
        self.core.dispatch(&Ctx::user(), Request::new(cmd, args))
    }
    fn ok(&self, cmd: &str, args: Value) -> Value {
        match self.call(cmd, args) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd} failed: {e:?}"),
        }
    }
    fn set(&self, classes: Value) -> Response {
        self.call(
            "grants.set",
            json!({"client_id": "bot", "data_classes": classes}),
        )
    }
    fn define(&self, id: &str, class: Value) -> Response {
        self.call(
            "types.define",
            json!({
                "id": id,
                "title": "T",
                "schema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object",
                    "x-halp-data-class": class,
                    "additionalProperties": false
                }
            }),
        )
    }
}

fn invalid(r: Response) {
    match r {
        Response::Err(e) => assert_eq!(e.code, ErrorCode::InvalidArgs, "{e:?}"),
        Response::Ok(v) => panic!("unexpectedly succeeded: {v}"),
    }
}

#[test]
fn built_in_classes_still_work() {
    let h = h();
    let all = json!(["general", "notes.private", "email.body", "secrets", "audit"]);
    let r = h.set(all);
    assert!(matches!(r, Response::Ok(_)), "{r:?}");
}

#[test]
fn a_class_declared_by_a_loaded_type_can_be_granted() {
    let h = h();
    // Before the type exists the class is unknown.
    invalid(h.set(json!(["team.okrs"])));
    assert!(matches!(
        h.define("team.okr", json!("team.okrs")),
        Response::Ok(_)
    ));
    let r = h.set(json!(["team.okrs", "general"]));
    let Response::Ok(v) = r else { panic!("{r:?}") };
    assert_eq!(v["grants"]["data_classes"], json!(["general", "team.okrs"]));
}

#[test]
fn an_undeclared_class_is_rejected() {
    let h = h();
    h.define("team.okr", json!("team.okrs"));
    invalid(h.set(json!(["team.other"])));
    invalid(h.set(json!(["general", "nope"])));
}

#[test]
fn malformed_class_strings_are_rejected() {
    let h = h();
    for bad in [
        "",
        "Notes.Private",
        "notes-private",
        "notes..private",
        ".notes",
        "notes.",
        "_notes",
        "a b",
        &"a".repeat(65),
    ] {
        invalid(h.set(json!([bad])));
        // A type cannot declare it either.
        invalid(h.define("team.bad", json!(bad)));
    }
    invalid(h.define("team.bad", json!(7)));
    // The longest allowed name works.
    let long = "a".repeat(64);
    assert!(matches!(
        h.define("team.long", json!(long)),
        Response::Ok(_)
    ));
    assert!(matches!(h.set(json!([long])), Response::Ok(_)));
}

#[test]
fn a_pack_declaring_a_malformed_class_does_not_load() {
    let pack = packs::builtin_packs()
        .unwrap()
        .into_iter()
        .find(|p| {
            p.task_types
                .iter()
                .any(|t| t.schema.get("x-halp-data-class").is_some())
        })
        .unwrap();
    let with_class = |class: &str| {
        let mut v = serde_json::to_value(&pack).unwrap();
        let t = v["task_types"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|t| t["schema"].get("x-halp-data-class").is_some())
            .unwrap();
        t["schema"]["x-halp-data-class"] = class.into();
        v.to_string()
    };
    assert!(packs::Pack::parse(&with_class("Bad Class")).is_err());
    assert!(packs::Pack::parse(&with_class("good.class_1")).is_ok());
}
