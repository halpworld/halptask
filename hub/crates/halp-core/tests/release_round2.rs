//! Release review, round 2: pack events after a retype migration, item
//! titles in non-item events, `day.state` counts, revoked keys, and the
//! fail-closed paths for unloaded types.
#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::halp_pack::{self as hp, SigningKey};
use halp_core::{Core, Ctx, ErrorBody, ErrorCode, FixedClock, Request, Response};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

const KEY: [u8; 32] = [9; 32];

struct H {
    core: Option<Core>,
    dir: TempDir,
}

fn clock() -> Arc<FixedClock> {
    Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    })
}

fn open(dir: &Path) -> Core {
    let store = Store::open(&dir.join("hub.db"), &RawKeyProvider::new(KEY)).unwrap();
    Core::with_store(store, clock()).unwrap()
}

fn h() -> H {
    let dir = tempfile::tempdir().unwrap();
    let core = open(dir.path());
    let h = H {
        core: Some(core),
        dir,
    };
    h.ok("clients.pair", json!({"name": "bot", "id": "bot"}));
    h
}

fn bot() -> Ctx {
    Ctx::agent("bot").with_session("s1")
}

fn signer() -> SigningKey {
    SigningKey::from_pkcs8(&hp::generate_pkcs8().unwrap()).unwrap()
}

/// The fingerprint as the core reports it: 64 hex.
fn fp(k: &SigningKey) -> String {
    hp::fingerprint(&k.public_key()).replace(' ', "")
}

fn pubkey_b64(k: &SigningKey) -> String {
    hp::crypto::encode_pubkey(&k.public_key())
}

/// `acme.sre`, a child of `halp.developer` (as in halp-pack's fixtures).
fn manifest(version: &str) -> Value {
    json!({
        "format": 1,
        "id": "acme.sre",
        "version": version,
        "title": "Acme SRE",
        "description": "On-call tooling",
        "license": "MIT",
        "publisher": {"name": "Acme"},
        "min_hub": "0.x",
        "extends": {"id": "halp.developer", "version": "^0.1"},
        "role": {"id": "acme-sre", "title": "SRE", "accent": "#112233"},
        "task_types": [{
            "id": "acme.incident",
            "title": "Incident",
            "schema": {
                "type": "object",
                "properties": {
                    "sev": {"enum": ["1", "2"]},
                    "service": {"type": "string"},
                    "status": {"enum": ["open", "closed"]}
                },
                "additionalProperties": false
            }
        }]
    })
}

fn bundle(mut m: Value, files: &BTreeMap<String, Vec<u8>>, key: Option<&SigningKey>) -> Vec<u8> {
    m["files"] = serde_json::to_value(hp::file_index(files)).unwrap();
    hp::build_bundle(&serde_json::to_string_pretty(&m).unwrap(), files, key).unwrap()
}

fn no_files() -> BTreeMap<String, Vec<u8>> {
    BTreeMap::new()
}

impl H {
    fn core(&self) -> &Core {
        self.core.as_ref().unwrap()
    }
    fn call_as(&self, ctx: &Ctx, cmd: &str, args: Value) -> Response {
        self.core().dispatch(ctx, Request::new(cmd, args))
    }
    /// As the user; a T3 confirmation is approved in place (as the UI does).
    fn call(&self, cmd: &str, args: Value) -> Response {
        match self.call_as(&Ctx::user(), cmd, args) {
            Response::Err(e) if e.code == ErrorCode::ConfirmationRequired => self.call_as(
                &Ctx::user(),
                "consent.approve",
                json!({"id": e.confirmation_id.unwrap()}),
            ),
            r => r,
        }
    }
    fn ok(&self, cmd: &str, args: Value) -> Value {
        match self.call(cmd, args) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd} failed: {e:?}"),
        }
    }
    fn err(&self, cmd: &str, args: Value) -> ErrorBody {
        match self.call(cmd, args) {
            Response::Ok(v) => panic!("{cmd} unexpectedly succeeded: {v}"),
            Response::Err(e) => e,
        }
    }
    fn ok_as(&self, ctx: &Ctx, cmd: &str, args: Value) -> Value {
        match self.call_as(ctx, cmd, args) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd} failed: {e:?}"),
        }
    }
    fn err_as(&self, ctx: &Ctx, cmd: &str, args: Value) -> ErrorBody {
        match self.call_as(ctx, cmd, args) {
            Response::Ok(v) => panic!("{cmd} unexpectedly succeeded: {v}"),
            Response::Err(e) => e,
        }
    }
    fn preview(&self, bytes: &[u8]) -> Response {
        self.call(
            "packs.preview",
            json!({"bundle_base64": STANDARD.encode(bytes)}),
        )
    }
    fn install(&self, bytes: &[u8]) -> Value {
        let p = match self.preview(bytes) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("preview failed: {e:?}"),
        };
        self.ok("packs.install", json!({"preview_id": p["preview_id"]}))
    }
    fn install_err(&self, bytes: &[u8]) -> ErrorBody {
        match self.preview(bytes) {
            Response::Err(e) => e,
            Response::Ok(p) => self.err("packs.install", json!({"preview_id": p["preview_id"]})),
        }
    }
    fn trust(&self, k: &SigningKey) {
        self.ok(
            "packs.keys_add",
            json!({"pubkey_b64": pubkey_b64(k), "label": "Acme"}),
        );
    }
    fn pack(&self, id: &str) -> Value {
        self.ok("packs.get", json!({"id": id}))
    }
    fn type_info(&self, id: &str) -> Value {
        self.ok("types.get", json!({"id": id}))
    }
    fn incident(&self, fields: Value) -> String {
        self.ok(
            "tasks.create",
            json!({"title": "Pager", "type_id": "acme.incident", "fields": fields}),
        )["id"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn fields(&self, id: &str) -> Value {
        self.ok("tasks.get", json!({"id": id}))["fields"].clone()
    }
    /// Closes the core and opens it again on the same store.
    fn reopen(&mut self) {
        self.core = None;
        self.core = Some(open(self.dir.path()));
    }
    fn with_store(&mut self, f: impl FnOnce(&mut Store)) {
        self.core = None;
        let mut s =
            Store::open(&self.dir.path().join("hub.db"), &RawKeyProvider::new(KEY)).unwrap();
        f(&mut s);
        drop(s);
        self.core = Some(open(self.dir.path()));
    }
}

fn code(e: &ErrorBody, c: ErrorCode) {
    assert_eq!(e.code, c, "{e:?}");
}

/// A trusted (community) acme.sre 1.0.0 installed.
fn with_acme() -> (H, SigningKey) {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(manifest("1.0.0"), &no_files(), Some(&k)));
    (h, k)
}

fn ev(h: &H, command: &str) -> String {
    h.ok("history.list", json!({"limit": 100}))["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"].as_str() == Some(command))
        .map(|e| e["id"].as_str().unwrap().to_string())
        .expect("event")
}

fn forge_type(h: &mut H, id: &str) {
    let id = id.to_string();
    h.with_store(|s| {
        let before = s.get(halp_store::EntityKind::TaskType, &id).unwrap();
        let mut after = before.clone().unwrap();
        after["schema"] = json!({"type": 5});
        s.commit(
            halp_store::NewEvent {
                actor: "system".into(),
                command: "test.forge".into(),
                entity: halp_domain::EntityRef {
                    kind: "task_type".into(),
                    id: id.clone(),
                },
                summary: "forge".into(),
                args: json!({}),
                changes: vec![halp_store::Change {
                    kind: halp_store::EntityKind::TaskType,
                    id: id.clone(),
                    before,
                    after: Some(after),
                }],
                undoable: false,
            },
            Utc::now(),
        )
        .unwrap();
    });
}

/// Undo, plain or by event id, keeps the dev-mode rule (CONTRACT note 17).
#[test]
fn the_dev_mode_rule_applies_to_undo() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = manifest("1.0.0");
    m["task_types"][0]["schema"]["properties"]["team"] = json!({"type": "string"});
    h.install(&bundle(m, &no_files(), None));
    let a = h.incident(json!({"sev": "1", "team": "sre"}));
    h.ok(
        "tasks.update",
        json!({"id": a, "patch": {"fields": {"sev": "1"}}}),
    );
    let upd = ev(&h, "tasks.update");
    h.install(&bundle(manifest("1.1.0"), &no_files(), None));
    let src = tempfile::tempdir().unwrap();
    let pack = src.path().join("child");
    std::fs::create_dir_all(&pack).unwrap();
    let child = json!({
        "format": 1, "id": "acme.child", "version": "0.1.0", "title": "Child",
        "description": "Dev override", "license": "MIT", "publisher": {"name": "Acme"},
        "min_hub": "0.x", "extends": {"id": "acme.sre", "version": "^1.0"},
        "role": {"id": "acme-child", "title": "Child", "accent": "#112233"},
        "type_overrides": [{"type": "acme.incident", "add_properties": {"team": {"type": "string"}}}]
    });
    std::fs::write(
        pack.join("halp-pack.json"),
        serde_json::to_string_pretty(&child).unwrap(),
    )
    .unwrap();
    let path = src.path().display().to_string();
    h.ok("packs.dev_set", json!({"path": path}));
    let e = h.err("history.undo", json!({}));
    code(&e, ErrorCode::Conflict);
    let e = h.err("history.undo", json!({"event_id": upd}));
    code(&e, ErrorCode::Conflict);
}
fn v110() -> Value {
    let mut m = manifest("1.1.0");
    m["task_types"][0]["schema"]["properties"] = json!({
        "severity": {"enum": ["1", "2"]},
        "service": {"type": "string"},
        "status": {"enum": ["open", "closed"]}
    });
    m["migrations"] = json!([{
        "type": "acme.incident",
        "from": "<1.1.0",
        "ops": [{"op": "rename", "from": "sev", "to": "severity"}]
    }]);
    m
}

/// v1.1.0 brings `acme.incident2` and retypes every incident into it.
fn v110_retype() -> Value {
    let mut m = manifest("1.1.0");
    let mut t2 = m["task_types"][0].clone();
    t2["id"] = json!("acme.incident2");
    t2["title"] = json!("Incident 2");
    m["task_types"].as_array_mut().unwrap().push(t2);
    m["migrations"] = json!([{
        "type": "acme.incident",
        "from": "<1.1.0",
        "ops": [{"op": "retype", "to": "acme.incident2"}]
    }]);
    m
}

#[test]
fn a_retype_into_a_new_pack_type_is_logged_as_a_pack_update() {
    let (h, k) = with_acme();
    let a = h.incident(json!({"sev": "1"}));
    h.install(&bundle(v110_retype(), &no_files(), Some(&k)));
    assert_eq!(
        h.ok("tasks.get", json!({"id": a}))["type_id"],
        "acme.incident2"
    );
    let events = h.ok("history.list", json!({"limit": 5}))["events"].clone();
    let e = events
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"] == "packs.install")
        .unwrap();
    assert_eq!(
        e["summary"], "Updated pack acme.sre from 1.0.0 to 1.1.0",
        "{e}"
    );
}

/// A private type (class `secrets`), one item of it due today and focused.
fn private_focus(h: &H) -> String {
    h.ok(
        "types.define",
        json!({"id": "mine.p", "title": "P", "schema": {"type": "object", "x-halp-data-class": "secrets"}}),
    );
    let id = h.ok(
        "tasks.create",
        json!({"title": "SECRET TITLE", "kind": "task", "type_id": "mine.p", "fields": {}, "due": "2026-11-10"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok("focus.set", json!({"id": id}));
    id
}

#[test]
fn focusing_a_private_item_keeps_its_title_out_of_history() {
    let h = h();
    private_focus(&h);
    let s = h
        .ok_as(&bot(), "history.list", json!({"limit": 100}))
        .to_string();
    assert!(!s.contains("SECRET TITLE"), "{s}");
    // The user still sees that something was focused.
    let s = h.ok("history.list", json!({"limit": 100})).to_string();
    assert!(s.contains("Focused a private item"), "{s}");
}

#[test]
fn day_state_counts_only_items_the_caller_may_see() {
    let h = h();
    private_focus(&h);
    let v = h.ok_as(&bot(), "day.state", json!({}));
    assert_eq!(v["counts"]["today"], 0, "{v}");
    assert!(v["focus"].is_null(), "{v}");
    let v = h.ok("day.state", json!({}));
    assert_eq!(v["counts"]["today"], 1, "{v}");
}

#[test]
fn unloaded_types_fail_closed_on_every_path() {
    let mut h = h();
    h.ok(
        "types.define",
        json!({"id": "mine.w", "title": "W", "schema": {"type": "object"}}),
    );
    let root = h.ok("tasks.create", json!({"title": "root general"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let id = h.ok("tasks.create", json!({"title": "w item", "kind": "task", "parent_id": root, "type_id": "mine.w", "fields": {}, "due": "2026-11-10"}))["id"].as_str().unwrap().to_string();
    let child = h.ok(
        "tasks.create",
        json!({"title": "child of w", "parent_id": id}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Focused (and so named in an event summary) while the type still loaded.
    h.ok("focus.set", json!({"id": id}));
    forge_type(&mut h, "mine.w");
    let b = bot();
    for (cmd, args) in [
        ("tasks.get", json!({"id": id})),
        ("tasks.get", json!({"id": child})),
        ("tasks.update", json!({"id": id, "patch": {"title": "z"}})),
        ("tasks.create", json!({"title": "n", "parent_id": id})),
        ("tasks.delete", json!({"id": root})),
        // Redefining the unloaded type would reclassify its items.
        (
            "types.define",
            json!({"id": "mine.w", "title": "W", "schema": {"type": "object"}}),
        ),
    ] {
        code(&h.err_as(&b, cmd, args), ErrorCode::NotFound);
    }
    for (cmd, args) in [
        ("tasks.list", json!({})),
        ("day.state", json!({})),
        ("focus.get", json!({})),
        ("history.list", json!({"limit": 100})),
        ("tasks.get", json!({"id": root})),
    ] {
        let s = h.ok_as(&b, cmd, args).to_string();
        assert!(
            !s.contains("w item") && !s.contains("child of w"),
            "{cmd} leaked: {s}"
        );
    }
    assert_eq!(h.ok_as(&b, "day.state", json!({}))["counts"]["today"], 0);
}

#[test]
fn an_agent_naming_an_unknown_type_gets_invalid_args_not_not_found() {
    let h = h();
    let e = h.err_as(
        &bot(),
        "tasks.create",
        json!({"title": "n", "type_id": "nope.x", "fields": {}}),
    );
    code(&e, ErrorCode::InvalidArgs);
    // A new type id is not hidden either: the agent may define and use it.
    h.ok_as(
        &bot(),
        "types.define",
        json!({"id": "bot.t", "title": "T", "schema": {"type": "object"}}),
    );
    h.ok_as(
        &bot(),
        "tasks.create",
        json!({"title": "n", "type_id": "bot.t", "fields": {}}),
    );
}

#[test]
fn a_revoked_key_stays_revoked() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let kid = hp::key_id(&k.public_key());
    h.ok("packs.keys_revoke", json!({"key_id": kid}));
    let e = h.err(
        "packs.keys_add",
        json!({"pubkey_b64": pubkey_b64(&k), "label": "again"}),
    );
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains("revoked"), "{e:?}");
}

#[test]
fn consent_summaries_do_not_carry_titles_to_agents() {
    let h = h();
    let id = h.ok("tasks.create", json!({"title": "LEAKY2"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    // A T3 request from the bot is parked, and its summary quotes the title.
    let e = h.err_as(&bot(), "tasks.delete", json!({"id": id}));
    code(&e, ErrorCode::ConfirmationRequired);
    h.ok("consent.deny", json!({"id": e.confirmation_id.unwrap()}));
    // The item then becomes private.
    h.ok("types.define", json!({"id": "mine.p", "title": "P", "schema": {"type": "object", "x-halp-data-class": "secrets"}}));
    let p = h.ok(
        "tasks.create",
        json!({"title": "vault", "type_id": "mine.p", "fields": {}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok("tasks.move", json!({"id": id, "parent_id": p}));
    let s = h
        .ok_as(&bot(), "history.list", json!({"limit": 100}))
        .to_string();
    assert!(!s.contains("LEAKY2"), "{s}");
    assert!(
        s.contains("Asked: a request") && s.contains("Declined: a request"),
        "{s}"
    );
    // The user keeps the full log.
    let s = h.ok("history.list", json!({"limit": 100})).to_string();
    assert!(s.contains("LEAKY2"), "{s}");
}

#[test]
fn a_deleted_items_history_stays_hidden_from_agents() {
    let h = h();
    let id = h.ok("tasks.create", json!({"title": "GONE TITLE"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok("tasks.delete", json!({"id": id}));
    let s = h
        .ok_as(&bot(), "history.list", json!({"limit": 100}))
        .to_string();
    assert!(!s.contains("GONE TITLE"), "{s}");
    let s = h.ok("history.list", json!({"limit": 100})).to_string();
    assert!(s.contains("GONE TITLE"), "{s}");
}
