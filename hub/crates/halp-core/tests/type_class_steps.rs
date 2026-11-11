//! Undo/redo and `types.define` carry the class of every item a type
//! change touches, so an agent cannot read, reclassify or probe items it
//! may not see (security review, release round).
#![allow(clippy::unwrap_used, clippy::expect_used)]
use std::sync::Arc;

use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::{Core, Ctx, FixedClock, Request, Response};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};

fn core() -> (tempfile::TempDir, Core) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap();
    let clock = Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    });
    let core = Core::with_store(store, clock).unwrap();
    (dir, core)
}
fn call(c: &Core, ctx: &Ctx, cmd: &str, a: Value) -> Response {
    c.dispatch(ctx, Request::new(cmd, a))
}
fn ok(c: &Core, ctx: &Ctx, cmd: &str, a: Value) -> Value {
    match call(c, ctx, cmd, a) {
        Response::Ok(v) => v,
        Response::Err(e) => panic!("{cmd}: {e:?}"),
    }
}
fn schema(pin: Value) -> Value {
    json!({"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object",
           "properties":{"pin": pin}})
}
fn bot(c: &Core) -> Ctx {
    ok(
        c,
        &Ctx::user(),
        "clients.pair",
        json!({"name":"bot","id":"bot"}),
    );
    Ctx::agent("bot").with_session("s1")
}
/// A private note with an `acme.x` child holding a secret; returns the child.
fn private_child(c: &Core) -> String {
    let u = Ctx::user();
    let note = ok(
        c,
        &u,
        "tasks.create",
        json!({"title":"Private note","type_id":"em.private_note",
        "fields":{"person_id":"ana","body":"b"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        c,
        &u,
        "tasks.create",
        json!({"title":"PIN","type_id":"acme.x","parent_id":note,
        "fields":{"pin":"4711-SECRET"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn undo_of_a_type_with_private_items_is_hidden_and_leaks_nothing() {
    let (_d, c) = core();
    let b = bot(&c);
    ok(
        &c,
        &b,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string","enum":["x"]}))}),
    );
    ok(
        &c,
        &b,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string"}))}),
    );
    let child = private_child(&c);
    assert!(matches!(
        call(&c, &b, "tasks.get", json!({"id": child})),
        Response::Err(_)
    ));
    let r = format!("{:?}", call(&c, &b, "history.undo", json!({})));
    assert!(r.contains("Err"), "undo must be refused: {r}");
    assert!(!r.contains("4711"), "field value leaked: {r}");
    assert!(!r.contains(&child), "private id leaked: {r}");
}

#[test]
fn undo_conflict_names_item_ids_only() {
    // The user's own undo still gets a conflict, without field values.
    let (_d, c) = core();
    let u = Ctx::user();
    ok(
        &c,
        &u,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string","enum":["x"]}))}),
    );
    ok(
        &c,
        &u,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string"}))}),
    );
    let it = ok(
        &c,
        &u,
        "tasks.create",
        json!({"title":"t","type_id":"acme.x","fields":{"pin":"4711"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Undo the second definition by its event id: the item no longer fits.
    let hist = ok(&c, &u, "history.list", json!({}));
    let ev = hist["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"] == "types.define")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let r = format!(
        "{:?}",
        call(&c, &u, "history.undo", json!({"event_id": ev}))
    );
    assert!(r.contains(&it), "conflict should name the item: {r}");
    assert!(!r.contains("4711"), "field value leaked: {r}");
}

#[test]
fn types_define_is_no_oracle_on_private_items() {
    let (_d, c) = core();
    let b = bot(&c);
    ok(
        &c,
        &b,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string"}))}),
    );
    let child = private_child(&c);
    for pat in ["^4", "^5"] {
        let r = format!(
            "{:?}",
            call(
                &c,
                &b,
                "types.define",
                json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string","pattern":pat}))})
            )
        );
        assert!(
            r.contains("Err"),
            "redefine over private items must be refused: {r}"
        );
        assert!(!r.contains(&child), "private id leaked: {r}");
    }
}

#[test]
fn undo_cannot_drop_a_type_class_after_the_grant_is_revoked() {
    let (_d, c) = core();
    let u = Ctx::user();
    let b = bot(&c);
    ok(
        &c,
        &b,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": schema(json!({"type":"string"}))}),
    );
    ok(
        &c,
        &u,
        "types.define",
        json!({"id":"acme.decl","title":"D","schema": {"type":"object","x-halp-data-class":"acme.priv"}}),
    );
    ok(
        &c,
        &u,
        "grants.set",
        json!({"client_id":"bot","data_classes":["general","acme.priv"]}),
    );
    let mut s = schema(json!({"type":"string"}));
    s["x-halp-data-class"] = json!("acme.priv");
    ok(
        &c,
        &b,
        "types.define",
        json!({"id":"acme.x","title":"X","schema": s}),
    );
    let it = ok(
        &c,
        &u,
        "tasks.create",
        json!({"title":"Private thing","type_id":"acme.x","fields":{"pin":"9999"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    ok(
        &c,
        &u,
        "grants.set",
        json!({"client_id":"bot","data_classes":["general"]}),
    );
    assert!(matches!(
        call(&c, &b, "tasks.get", json!({"id": it})),
        Response::Err(_)
    ));
    let r = format!("{:?}", call(&c, &b, "history.undo", json!({})));
    assert!(r.contains("Err"), "undo must be refused: {r}");
    let r = format!("{:?}", call(&c, &b, "tasks.get", json!({"id": it})));
    assert!(!r.contains("9999"), "item became readable: {r}");
}
