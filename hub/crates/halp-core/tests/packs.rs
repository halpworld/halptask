//! P5-B: installed packs in the core and the encrypted store (v1.10):
//! the trust split, retired types, re-verification on load, the
//! `packs.*` commands, dev mode and the Tier C lookups.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::halp_pack::{self as hp, SigningKey};
use halp_core::{
    CommandDescriptor, Core, Ctx, ErrorBody, ErrorCode, FixedClock, Request, Response, Tier,
};
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

#[test]
fn builtin_packs_are_listed_with_the_trust_split() {
    let h = h();
    let list = h.ok("packs.list", json!({}));
    let dev = list["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "halp.developer")
        .unwrap();
    assert_eq!(dev["trust"], "builtin");
    assert_eq!(dev["status"], "ok");
    assert_eq!(dev["role_id"], "developer");
    // A built-in role pack type: origin pack, trust builtin.
    let types = h.ok("types.list", json!({}));
    let t = types["types"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["origin"] == "pack")
        .unwrap();
    assert_eq!(t["trust"], "builtin");
    assert!(t["pack"].as_str().unwrap().starts_with("halp."));
    code(
        &h.err("packs.disable", json!({"id": "halp.developer"})),
        ErrorCode::Conflict,
    );
    code(
        &h.err("packs.uninstall", json!({"id": "halp.developer"})),
        ErrorCode::Conflict,
    );
}

#[test]
fn install_a_signed_pack_and_use_its_type() {
    let (h, k) = with_acme();
    let p = h.pack("acme.sre");
    assert_eq!(p["trust"], "community");
    assert_eq!(p["status"], "ok");
    assert_eq!(p["extends"], json!(["halp.developer"]));
    assert_eq!(p["key_fp"].as_str().unwrap(), fp(&k));
    assert_eq!(p["description"], "On-call tooling");
    let t = h.type_info("acme.incident");
    assert_eq!(t["origin"], "pack");
    assert_eq!(t["trust"], "community");
    assert_eq!(t["pack"], "acme.sre");
    let roles = h.ok("role.list", json!({}));
    assert!(roles.to_string().contains("acme-sre"));
    // The role inherits the parent's types and has its layout.
    h.ok("layout.get", json!({"role_id": "acme-sre"}));
    h.incident(json!({"sev": "1"}));
    // The key lists it.
    let keys = h.ok("packs.keys_list", json!({}));
    assert_eq!(keys["keys"][0]["packs"], 1);
    assert_eq!(keys["keys"][0]["fingerprint"], fp(&k));
    assert!(fp(&k).starts_with(keys["keys"][0]["key_id"].as_str().unwrap()));
    // The event carries no bundle or manifest text.
    let ev = h.ok("history.list", json!({"limit": 5}));
    assert!(!ev.to_string().contains("bundle_base64"));
    // A second install of the same version is refused.
    code(
        &h.install_err(&bundle(manifest("1.0.0"), &no_files(), Some(&k))),
        ErrorCode::Conflict,
    );
}

#[test]
fn every_pack_write_is_user_only() {
    let (h, _k) = with_acme();
    let bot = bot();
    for (cmd, args) in [
        ("packs.preview", json!({"bundle_base64": ""})),
        ("packs.install", json!({"preview_id": "pv_x"})),
        ("packs.enable", json!({"id": "acme.sre"})),
        ("packs.disable", json!({"id": "acme.sre"})),
        ("packs.rollback", json!({"id": "acme.sre"})),
        ("packs.uninstall", json!({"id": "acme.sre"})),
        ("packs.reset_layout", json!({"id": "acme.sre"})),
        ("packs.keys_list", json!({})),
        ("packs.keys_add", json!({"pubkey_b64": "", "label": "x"})),
        ("packs.keys_revoke", json!({"key_id": "x"})),
        ("packs.dev_set", json!({"path": null})),
        ("packs.dev_get", json!({})),
        ("packs.dev_reload", json!({})),
    ] {
        code(&h.err_as(&bot, cmd, args), ErrorCode::Denied);
    }
    let d = h.core().descriptors();
    for c in d.iter().filter(|c| c.name.starts_with("packs.")) {
        let read = c.name == "packs.list" || c.name == "packs.get";
        assert_eq!(c.user_only, !read, "{}", c.name);
        assert_eq!(c.mcp, read, "{}", c.name);
    }
    // Agents see the list without the user-only fields.
    let l = match h.call_as(&bot, "packs.list", json!({})) {
        Response::Ok(v) => v,
        Response::Err(e) => panic!("{e:?}"),
    };
    let acme = l["packs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "acme.sre")
        .unwrap();
    assert!(acme.get("description").is_none());
    assert!(acme.get("publisher").is_none());
    assert!(acme.get("previous_version").is_none());
    assert_eq!(h.pack("acme.sre")["status"], "ok");
}

#[test]
fn retired_types_keep_their_rules() {
    let (h, _k) = with_acme();
    let id = h.incident(json!({"sev": "1"}));
    let empty = h.incident(json!({}));
    h.ok("packs.uninstall", json!({"id": "acme.sre"}));
    let t = h.type_info("acme.incident");
    assert_eq!(t["origin"], "retired");
    assert_eq!(t["trust"], "community");
    assert_eq!(t["pack"], "acme.sre");
    // No new items of a retired type.
    let e = h.err(
        "tasks.create",
        json!({"title": "x", "type_id": "acme.incident", "fields": {}}),
    );
    code(&e, ErrorCode::InvalidArgs);
    assert!(e.message.contains("retired"), "{e:?}");
    // The user may still edit its items (validated against the last
    // definition) and clear the type of an item without fields.
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": {"sev": "2"}}}),
    );
    code(
        &h.err(
            "tasks.update",
            json!({"id": id, "patch": {"fields": {"sev": "9"}}}),
        ),
        ErrorCode::InvalidArgs,
    );
    h.ok(
        "tasks.update",
        json!({"id": empty, "patch": {"type_id": null}}),
    );
    // Retired survives a restart (the retired_types row).
    let mut h = h;
    h.reopen();
    assert_eq!(h.type_info("acme.incident")["origin"], "retired");
    assert_eq!(h.fields(&id), json!({"sev": "2"}));
}

fn private_manifest() -> Value {
    let mut m = manifest("1.0.0");
    m["task_types"] = json!([{
        "id": "acme.secret",
        "title": "Secret",
        "schema": {
            "type": "object",
            "x-halp-data-class": "acme.private",
            "x-halp-immutable-when": {"status": ["closed"]},
            "properties": {"status": {"enum": ["open", "closed"]}},
            "additionalProperties": false
        }
    }]);
    m
}

#[test]
fn private_and_locked_items_stay_so_when_retired() {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(private_manifest(), &no_files(), Some(&k)));
    let a = h.ok(
        "tasks.create",
        json!({"title": "s", "type_id": "acme.secret", "fields": {"status": "closed"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    assert_eq!(h.type_info("acme.secret")["origin"], "retired");
    // Private class: an agent without the grant does not see it.
    code(
        &h.err_as(&bot(), "tasks.get", json!({"id": a})),
        ErrorCode::NotFound,
    );
    // Locked: an agent with the grant still cannot change it.
    h.ok(
        "grants.set",
        json!({"client_id": "bot", "data_classes": ["general", "acme.private"], "t1": true}),
    );
    let e = h.err_as(
        &bot(),
        "tasks.update",
        json!({"id": a, "patch": {"title": "changed"}}),
    );
    code(&e, ErrorCode::Denied);
    // Re-enabled, the type is live again.
    h.ok("packs.enable", json!({"id": "acme.sre"}));
    assert_eq!(h.type_info("acme.secret")["origin"], "pack");
}

#[test]
fn an_unsigned_pack_cannot_carry_privileged_annotations() {
    let h = h();
    let e = h.install_err(&bundle(private_manifest(), &no_files(), None));
    code(&e, ErrorCode::Denied);
    // Without them it installs as unsigned, and its types cannot lock.
    let p = h.install(&bundle(manifest("1.0.0"), &no_files(), None));
    assert_eq!(p["trust"], "unsigned");
    assert_eq!(h.type_info("acme.incident")["trust"], "unsigned");
}

#[test]
fn an_unknown_key_needs_trust_key() {
    let h = h();
    let k = signer();
    let b = bundle(private_manifest(), &no_files(), Some(&k));
    let p = match h.preview(&b) {
        Response::Ok(v) => v,
        Response::Err(e) => panic!("{e:?}"),
    };
    assert_eq!(p["signature"], "unknown_key");
    assert_eq!(p["trust"], "community");
    assert_eq!(p["key_fp"].as_str().unwrap(), fp(&k));
    let e = h.err("packs.install", json!({"preview_id": p["preview_id"]}));
    code(&e, ErrorCode::Denied);
    let v = h.ok(
        "packs.install",
        json!({"preview_id": p["preview_id"], "trust_key": true}),
    );
    assert_eq!(v["trust"], "community");
    let keys = h.ok("packs.keys_list", json!({}));
    assert_eq!(keys["keys"][0]["label"], "Acme");
}

#[test]
fn a_tampered_bundle_is_not_loaded_and_its_types_retire() {
    let (mut h, _k) = with_acme();
    let id = h.incident(json!({"sev": "1"}));
    let sha = h.ok("packs.list", json!({})).to_string();
    assert!(sha.contains("acme.sre"));
    h.with_store(|s| {
        // Flip a byte of every stored bundle.
        for sha in all_blob_shas(s) {
            let mut b = s.get_blob(&sha).unwrap().unwrap();
            let n = b.len() / 2;
            b[n] ^= 1;
            s.overwrite_blob_unchecked(&sha, &b).unwrap();
        }
    });
    // Startup went fine; the pack is in error and its type retired.
    let p = h.pack("acme.sre");
    assert_eq!(p["status"], "error");
    assert!(p["error"].as_str().unwrap().contains("hash"), "{p}");
    assert_eq!(h.type_info("acme.incident")["origin"], "retired");
    let e = h.err(
        "tasks.create",
        json!({"title": "x", "type_id": "acme.incident", "fields": {}}),
    );
    code(&e, ErrorCode::InvalidArgs);
    assert_eq!(h.fields(&id), json!({"sev": "1"}));
    // The role is gone.
    assert!(
        !h.ok("role.list", json!({}))
            .to_string()
            .contains("acme-sre")
    );
    // It cannot be enabled again.
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    assert!(
        h.call("packs.enable", json!({"id": "acme.sre"}))
            .error()
            .is_some()
    );
}

fn all_blob_shas(s: &Store) -> Vec<String> {
    let rows: Vec<Value> = s.load(halp_store::EntityKind::Pack).unwrap();
    rows.iter()
        .map(|r| r["current"]["bundle_sha256"].as_str().unwrap().to_string())
        .collect()
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

#[test]
fn update_migrates_and_rollback_restores() {
    let (h, k) = with_acme();
    let a = h.incident(json!({"sev": "1", "service": "api"}));
    let b = h.incident(json!({"service": "db"}));
    // A downgrade, a different key and an unsigned update are refused.
    code(
        &h.install_err(&bundle(manifest("0.9.0"), &no_files(), Some(&k))),
        ErrorCode::Conflict,
    );
    let other = signer();
    h.trust_label(&other, "Other");
    code(
        &h.install_err(&bundle(v110(), &no_files(), Some(&other))),
        ErrorCode::Denied,
    );
    code(
        &h.install_err(&bundle(v110(), &no_files(), None)),
        ErrorCode::Denied,
    );
    // The preview reports the migration.
    let p = match h.preview(&bundle(v110(), &no_files(), Some(&k))) {
        Response::Ok(v) => v,
        Response::Err(e) => panic!("{e:?}"),
    };
    assert_eq!(p["installed_version"], "1.0.0");
    assert_eq!(p["migrations"]["items_checked"], 2);
    assert_eq!(p["migrations"]["items_migrated"], 1);
    assert_eq!(p["migrations"]["items_failed"], 0);
    assert!(p["permission_diff"].is_object());
    let v = h.ok("packs.install", json!({"preview_id": p["preview_id"]}));
    assert_eq!(v["version"], "1.1.0");
    assert_eq!(v["previous_version"], "1.0.0");
    assert_eq!(h.fields(&a), json!({"severity": "1", "service": "api"}));
    assert_eq!(h.fields(&b), json!({"service": "db"}));
    // The install event is not undoable, and the migration wrote as the
    // pack's module.
    let w = h.ok("history.list", json!({"limit": 5}));
    let ev = w["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"] == "packs.install")
        .unwrap();
    assert_eq!(ev["undoable"], false);
    // Rollback restores the fields and the version.
    let r = h.ok("packs.rollback", json!({"id": "acme.sre"}));
    assert_eq!(r["version"], "1.0.0");
    assert_eq!(r["previous_version"], Value::Null);
    assert_eq!(h.fields(&a), json!({"sev": "1", "service": "api"}));
    code(
        &h.err("packs.rollback", json!({"id": "acme.sre"})),
        ErrorCode::Conflict,
    );
}

/// Items a migration changed count as written by `module:<pack>`: an agent
/// that reads one is tainted by it.
#[test]
fn a_migration_writes_as_the_pack_module() {
    let (mut h, k) = with_acme();
    h.core
        .as_mut()
        .unwrap()
        .register_command(
            CommandDescriptor {
                name: "test.send".into(),
                description: "test command".into(),
                tier: Tier::T2,
                input_schema: json!({
                    "type": "object",
                    "properties": {"to": {"type": "string"}},
                    "required": ["to"],
                    "additionalProperties": false
                }),
                output_schema: json!({"type": "object"}),
                data_class: "general".into(),
                mcp: true,
                user_only: false,
            },
            Box::new(|_, a| Ok(json!({"sent": a}))),
        )
        .unwrap();
    h.ok(
        "rules.create",
        json!({"client_id": "bot", "command": "test.send", "args": {"to": "alice"}}),
    );
    let a = h.incident(json!({"sev": "1"}));
    let r = h.call_as(&bot(), "tasks.get", json!({"id": a}));
    assert!(r.is_ok(), "{r:?}");
    assert!(
        h.call_as(&bot(), "test.send", json!({"to": "alice"}))
            .is_ok()
    );
    h.install(&bundle(v110(), &no_files(), Some(&k)));
    let r = h.call_as(&bot(), "tasks.get", json!({"id": a}));
    assert!(r.is_ok(), "{r:?}");
    let e = h.err_as(&bot(), "test.send", json!({"to": "alice"}));
    code(&e, ErrorCode::ConfirmationRequired);
    let p = h.ok("consent.get", json!({"id": e.confirmation_id.unwrap()}));
    assert_eq!(p["taint_source"], "module:acme.sre", "{p}");
}

impl H {
    fn trust_label(&self, k: &SigningKey, label: &str) {
        self.ok(
            "packs.keys_add",
            json!({"pubkey_b64": pubkey_b64(k), "label": label}),
        );
    }
}

#[test]
fn rollback_is_refused_when_a_migrated_item_changed() {
    let (h, k) = with_acme();
    let a = h.incident(json!({"sev": "1"}));
    h.install(&bundle(v110(), &no_files(), Some(&k)));
    h.ok(
        "tasks.update",
        json!({"id": a, "patch": {"fields": {"severity": "2"}}}),
    );
    code(
        &h.err("packs.rollback", json!({"id": "acme.sre"})),
        ErrorCode::Conflict,
    );
    assert_eq!(h.pack("acme.sre")["version"], "1.1.0");
}

#[test]
fn an_update_is_refused_if_any_item_would_fail() {
    let (h, k) = with_acme();
    let a = h.incident(json!({"sev": "1"}));
    let ok = h.incident(json!({"service": "x"}));
    // 1.1.0 drops `sev` without a migration: item a would not validate.
    let mut m = v110();
    m["migrations"] = json!([]);
    let e = h.install_err(&bundle(m, &no_files(), Some(&k)));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains(&a), "{e:?}");
    assert!(!e.message.contains(&ok), "{e:?}");
    assert_eq!(h.pack("acme.sre")["version"], "1.0.0");
    assert_eq!(h.fields(&a), json!({"sev": "1"}));
}

#[test]
fn a_locked_item_is_not_migrated() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = manifest("1.0.0");
    m["task_types"][0]["schema"]["x-halp-immutable-when"] = json!({"status": ["closed"]});
    h.install(&bundle(m.clone(), &no_files(), Some(&k)));
    let a = h.incident(json!({"sev": "1", "status": "closed"}));
    let mut m2 = v110();
    m2["task_types"][0]["schema"]["x-halp-immutable-when"] = json!({"status": ["closed"]});
    let e = h.install_err(&bundle(m2, &no_files(), Some(&k)));
    code(&e, ErrorCode::Conflict);
    assert_eq!(h.fields(&a), json!({"sev": "1", "status": "closed"}));
}

#[test]
fn uninstall_is_refused_while_another_pack_extends_it() {
    let (h, k) = with_acme();
    let mut child = manifest("1.0.0");
    child["id"] = "acme.oncall".into();
    child["extends"] = json!({"id": "acme.sre", "version": "^1"});
    child["role"] = json!({"id": "acme-oncall", "title": "On-call", "accent": "#223344"});
    child["task_types"] = json!([]);
    let v = h.install(&bundle(child, &no_files(), Some(&k)));
    assert_eq!(v["extends"], json!(["acme.sre", "halp.developer"]));
    let e = h.err("packs.uninstall", json!({"id": "acme.sre"}));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains("acme.oncall"));
    // A disabled parent still serves its child (only its own role goes).
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    assert_eq!(h.pack("acme.oncall")["status"], "ok");
    assert_eq!(h.type_info("acme.incident")["origin"], "pack");
    h.ok("packs.enable", json!({"id": "acme.sre"}));
    h.ok("packs.uninstall", json!({"id": "acme.oncall"}));
    h.ok("packs.uninstall", json!({"id": "acme.sre"}));
    code(
        &h.err("packs.get", json!({"id": "acme.sre"})),
        ErrorCode::NotFound,
    );
    assert!(
        !h.ok("role.list", json!({}))
            .to_string()
            .contains("acme-sre")
    );
}

#[test]
fn a_pack_cannot_take_over_a_user_type() {
    let h = h();
    h.ok(
        "types.define",
        json!({"id": "acme.incident", "title": "Mine", "schema": {"type": "object"}}),
    );
    let k = signer();
    h.trust(&k);
    code(
        &h.install_err(&bundle(manifest("1.0.0"), &no_files(), Some(&k))),
        ErrorCode::Conflict,
    );
}

#[test]
fn undo_never_reverts_a_pack_change() {
    let (h, _k) = with_acme();
    // The agent's own change, then the user disables the pack.
    h.ok(
        "grants.set",
        json!({"client_id": "bot", "data_classes": ["general"], "t1": true}),
    );
    let r = h.call_as(&bot(), "tasks.create", json!({"title": "agent task"}));
    assert!(r.is_ok(), "{r:?}");
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    // The agent's undo undoes its own task, not the disable.
    let e = h.err_as(&bot(), "history.undo", json!({}));
    code(&e, ErrorCode::ConfirmationRequired);
    assert!(e.message.contains("agent task"), "{e:?}");
    h.ok("consent.approve", json!({"id": e.confirmation_id.unwrap()}));
    assert_eq!(h.pack("acme.sre")["enabled"], false);
    // Nor does the user's.
    let _ = h.call("history.undo", json!({}));
    assert_eq!(h.pack("acme.sre")["enabled"], false);
    assert_eq!(h.type_info("acme.incident")["origin"], "retired");
}

#[test]
fn keys_add_list_and_revoke() {
    let (h, k) = with_acme();
    let e = h.err(
        "packs.keys_add",
        json!({"pubkey_b64": pubkey_b64(&k), "label": "again"}),
    );
    code(&e, ErrorCode::Conflict);
    code(
        &h.err(
            "packs.keys_add",
            json!({"pubkey_b64": "AAAA", "label": "x"}),
        ),
        ErrorCode::InvalidArgs,
    );
    let r = h.ok("packs.keys_revoke", json!({"key_id": k.key_id()}));
    assert_eq!(r["disabled_packs"], json!(["acme.sre"]));
    assert_eq!(r["key"]["revoked"], true);
    let p = h.pack("acme.sre");
    assert_eq!(p["enabled"], false);
    assert_ne!(p["status"], "ok");
    assert_eq!(h.type_info("acme.incident")["origin"], "retired");
    // A pack signed by a revoked key does not come back.
    assert!(
        h.call("packs.enable", json!({"id": "acme.sre"}))
            .error()
            .is_some()
    );
    code(
        &h.err("packs.keys_revoke", json!({"key_id": "nope"})),
        ErrorCode::NotFound,
    );
}

#[test]
fn reset_layout_restores_the_pack_default() {
    let (h, _k) = with_acme();
    let before = h.ok("layout.get", json!({"role_id": "acme-sre"}));
    let w = before["widgets"][0]["id"].as_str().unwrap().to_string();
    h.ok(
        "layout.remove_widget",
        json!({"role_id": "acme-sre", "widget_id": w}),
    );
    let r = h.ok("packs.reset_layout", json!({"id": "acme.sre"}));
    assert_eq!(r["role_id"], "acme-sre");
    assert_eq!(h.ok("layout.get", json!({"role_id": "acme-sre"})), before);
}

fn widget_manifest() -> Value {
    let mut m = manifest("1.0.0");
    m["ui_widgets"] = json!([{
        "name": "rota",
        "title": "Rota",
        "root": "ui/rota",
        "calls": ["tasks_list"]
    }]);
    m
}

fn widget_files() -> BTreeMap<String, Vec<u8>> {
    let mut f = BTreeMap::new();
    f.insert(
        "ui/rota/index.html".into(),
        b"<!doctype html><p>hi</p>".to_vec(),
    );
    f.insert("ui/rota/app.js".into(), b"console.log(1)".to_vec());
    f.insert("ui/rota/notes.txt".into(), b"plain".to_vec());
    f.insert("other/x.js".into(), b"nope".to_vec());
    f
}

#[test]
fn tier_c_lookups_serve_only_verified_files_under_the_root() {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(widget_manifest(), &widget_files(), Some(&k)));
    let w = h.core().ext_ui_widget("acme.sre", "rota").unwrap();
    assert_eq!(w.kind, "x.acme.sre.rota");
    assert_eq!(w.entry, "ui/rota/index.html");
    assert_eq!(w.calls, vec!["tasks_list".to_string()]);
    assert!(w.trust.may_run_code());
    let f = h
        .core()
        .ext_ui_file("acme.sre", "rota", "ui/rota/index.html")
        .unwrap();
    assert!(f.mime.starts_with("text/html"));
    assert_eq!(f.sha256, hp::ids::sha256_hex(&f.bytes));
    assert!(
        h.core()
            .ext_ui_file("acme.sre", "rota", "ui/rota/app.js")
            .is_some()
    );
    for bad in [
        "other/x.js",
        "ui/rota/notes.txt",
        "ui/rota/missing.js",
        "ui/rota/../../other/x.js",
        "ui/rotax/index.html",
        "ui/rota",
    ] {
        assert!(
            h.core().ext_ui_file("acme.sre", "rota", bad).is_none(),
            "{bad}"
        );
    }
    assert!(h.core().ext_ui_widget("acme.sre", "nope").is_none());
    assert!(h.core().ext_ui_widget("halp.developer", "rota").is_none());
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    assert!(h.core().ext_ui_widget("acme.sre", "rota").is_none());
    assert!(
        h.core()
            .ext_ui_file("acme.sre", "rota", "ui/rota/index.html")
            .is_none()
    );
}

#[test]
fn dev_mode_loads_a_folder_and_is_not_persisted() {
    let mut h = h();
    let src = tempfile::tempdir().unwrap();
    let pack = src.path().join("sre");
    std::fs::create_dir_all(pack.join("ui")).unwrap();
    let mut m = manifest("0.1.0");
    m["task_types"][0]["id"] = "acme.devtype".into();
    std::fs::write(
        pack.join("halp-pack.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();
    std::fs::write(pack.join("ui/readme.md"), "hi").unwrap();
    code(
        &h.err("packs.dev_set", json!({"path": "relative/path"})),
        ErrorCode::InvalidArgs,
    );
    // `path` is required (null turns dev mode off; a missing key does not).
    code(&h.err("packs.dev_set", json!({})), ErrorCode::InvalidArgs);
    let r = h.ok(
        "packs.dev_set",
        json!({"path": src.path().display().to_string()}),
    );
    assert_eq!(r["packs"], 1);
    let g = h.ok("packs.dev_get", json!({}));
    assert_eq!(g["packs"][0]["id"], "dev.acme.sre");
    assert_eq!(g["packs"][0]["trust"], "dev");
    assert_eq!(g["packs"][0]["role_id"], "dev-acme-sre");
    assert_eq!(h.type_info("acme.devtype")["trust"], "dev");
    let id = h.ok(
        "tasks.create",
        json!({"title": "d", "type_id": "acme.devtype", "fields": {"sev": "1"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Edit and reload.
    m["title"] = "Acme SRE v2".into();
    std::fs::write(
        pack.join("halp-pack.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();
    h.ok("packs.dev_reload", json!({}));
    assert_eq!(h.pack("dev.acme.sre")["title"], "Acme SRE v2");
    // A restart turns dev mode off; the dev type stays as retired.
    h.reopen();
    assert_eq!(h.ok("packs.dev_get", json!({}))["path"], Value::Null);
    assert_eq!(h.type_info("acme.devtype")["origin"], "retired");
    assert_eq!(h.fields(&id), json!({"sev": "1"}));
    code(&h.err("packs.dev_reload", json!({})), ErrorCode::Conflict);
}

#[test]
fn preview_ids_are_bound_and_expire_with_a_change() {
    let h = h();
    let k = signer();
    h.trust(&k);
    code(
        &h.err("packs.install", json!({"preview_id": "pv_nope"})),
        ErrorCode::NotFound,
    );
    code(
        &h.err("packs.preview", json!({"bundle_base64": "!!"})),
        ErrorCode::InvalidArgs,
    );
    let p = match h.preview(&bundle(manifest("1.0.0"), &no_files(), Some(&k))) {
        Response::Ok(v) => v,
        Response::Err(e) => panic!("{e:?}"),
    };
    assert_eq!(p["signature"], "verified");
    assert_eq!(p["installed_version"], Value::Null);
    assert_eq!(p["permission_diff"], Value::Null);
    assert_eq!(p["chain"], json!(["halp.developer"]));
    h.ok("packs.install", json!({"preview_id": p["preview_id"]}));
    // Used once.
    code(
        &h.err("packs.install", json!({"preview_id": p["preview_id"]})),
        ErrorCode::NotFound,
    );
}

fn module_manifest() -> Value {
    let mut m = manifest("1.0.0");
    m["modules"] = json!([{
        "name": "burn",
        "wasm": "wasm/burn.wasm",
        "inputs": {"open": {"command": "tasks.list", "args": {}}},
        "output_schema": {"type": "object"}
    }]);
    m
}

fn module_files() -> BTreeMap<String, Vec<u8>> {
    let mut f = BTreeMap::new();
    f.insert("wasm/burn.wasm".into(), b"\0asm\x01\0\0\0".to_vec());
    f
}

#[test]
fn tier_b_lookup_returns_verified_wasm_only_for_packs_that_may_run_code() {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(module_manifest(), &module_files(), Some(&k)));
    let m = h.core().ext_module("acme.sre", "burn").unwrap();
    assert_eq!(m.pack_id, "acme.sre");
    assert_eq!(m.decl.name, "burn");
    assert_eq!(m.wasm, b"\0asm\x01\0\0\0".to_vec());
    assert_eq!(m.trust, hp::PackTrust::Community);
    assert_eq!(m.publisher, "Acme");
    assert!(h.core().ext_module("acme.sre", "nope").is_none());
    assert!(h.core().ext_module("halp.developer", "burn").is_none());
    assert!(h.core().ext_module("nope.pack", "burn").is_none());
    // Disabled: gone. Enabled again: back.
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    assert!(h.core().ext_module("acme.sre", "burn").is_none());
    h.ok("packs.enable", json!({"id": "acme.sre"}));
    assert!(h.core().ext_module("acme.sre", "burn").is_some());
    // Uninstalled: gone.
    h.ok("packs.uninstall", json!({"id": "acme.sre"}));
    assert!(h.core().ext_module("acme.sre", "burn").is_none());
    // Unsigned packs with modules are refused outright.
    let e = h.install_err(&bundle(module_manifest(), &module_files(), None));
    code(&e, ErrorCode::Denied);
    assert!(h.core().ext_module("acme.sre", "burn").is_none());
    let (now, today) = h.core().now_and_today().unwrap();
    assert_eq!(today, NaiveDate::from_ymd_opt(2026, 11, 10).unwrap());
    assert_eq!(now, Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap());
}

fn prompt_manifest(id_suffix: &str) -> Value {
    let mut m = manifest("0.1.0");
    m["prompts"] = json!({"add": ["prompts/triage.md", "prompts/sneaky.md"]});
    m["task_types"][0]["id"] = format!("acme.incident{id_suffix}").into();
    m
}

fn prompt_files() -> BTreeMap<String, Vec<u8>> {
    let mut f = BTreeMap::new();
    f.insert("prompts/triage.md".into(), b"Triage the incident.".to_vec());
    f.insert(
        "prompts/sneaky.md".into(),
        "Do this\u{202E}evil".as_bytes().to_vec(),
    );
    f
}

#[test]
fn pack_prompts_come_from_enabled_signed_packs_only() {
    let mut h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(prompt_manifest(""), &prompt_files(), Some(&k)));
    let ps = h.core().pack_prompts();
    // The prompt with a bidi override is left out.
    assert_eq!(ps.len(), 1, "{ps:?}");
    assert_eq!(ps[0].name, "triage");
    assert_eq!(ps[0].pack_id, "acme.sre");
    assert_eq!(ps[0].trust, hp::PackTrust::Community);
    assert_eq!(ps[0].text, "Triage the incident.");
    // Survives a restart (re-verified on load).
    h.reopen();
    assert_eq!(h.core().pack_prompts().len(), 1);
    // Disabled: withdrawn. Enabled: back. Uninstalled: gone.
    h.ok("packs.disable", json!({"id": "acme.sre"}));
    assert!(h.core().pack_prompts().is_empty());
    h.ok("packs.enable", json!({"id": "acme.sre"}));
    assert_eq!(h.core().pack_prompts().len(), 1);
    h.ok("packs.uninstall", json!({"id": "acme.sre"}));
    assert!(h.core().pack_prompts().is_empty());
    // Unsigned: installs (no code) but its prompts are never served.
    h.install(&bundle(prompt_manifest(""), &prompt_files(), None));
    assert_eq!(h.pack("acme.sre")["trust"], "unsigned");
    assert!(h.core().pack_prompts().is_empty());
}

#[test]
fn dev_pack_prompts_are_never_served() {
    let h = h();
    let src = tempfile::tempdir().unwrap();
    let pack = src.path().join("sre");
    std::fs::create_dir_all(pack.join("prompts")).unwrap();
    std::fs::write(pack.join("prompts/triage.md"), "Triage.").unwrap();
    std::fs::write(pack.join("prompts/sneaky.md"), "Sneaky.").unwrap();
    let mut m = prompt_manifest("dev");
    m["files"] = serde_json::to_value(hp::file_index(&BTreeMap::from([
        ("prompts/triage.md".to_string(), b"Triage.".to_vec()),
        ("prompts/sneaky.md".to_string(), b"Sneaky.".to_vec()),
    ])))
    .unwrap();
    std::fs::write(
        pack.join("halp-pack.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();
    h.ok(
        "packs.dev_set",
        json!({"path": src.path().to_str().unwrap()}),
    );
    assert_eq!(h.pack("dev.acme.sre")["trust"], "dev");
    assert!(h.core().pack_prompts().is_empty());
}

// ---- review fixes: type changes keep items valid, private and locked ----

/// The refusal a preview reports for a bundle that cannot be installed.
fn preview_warning(h: &H, bytes: &[u8]) -> String {
    match h.preview(bytes) {
        Response::Ok(p) => p["warnings"].to_string(),
        Response::Err(e) => e.message,
    }
}

/// F1: a rollback validates every item of the pack's types against the
/// version it returns to, not only the items the update migrated.
#[test]
fn rollback_validates_every_item_of_the_packs_types() {
    let (h, k) = with_acme();
    h.install(&bundle(v110(), &no_files(), Some(&k)));
    // Written under 1.1.0: 1.0.0 has no `severity`.
    let a = h.incident(json!({"severity": "2"}));
    let fine = h.incident(json!({"service": "api"}));
    let e = h.err("packs.rollback", json!({"id": "acme.sre"}));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains(&a), "{e:?}");
    assert!(!e.message.contains(&fine), "{e:?}");
    assert_eq!(h.pack("acme.sre")["version"], "1.1.0");
    // Fixed, the rollback goes through.
    h.ok("tasks.update", json!({"id": a, "patch": {"fields": {}}}));
    assert_eq!(
        h.ok("packs.rollback", json!({"id": "acme.sre"}))["version"],
        "1.0.0"
    );
}

/// acme.sre widening `dev.pull_request`: a new `state` value and a
/// `team` property.
fn override_manifest(version: &str) -> Value {
    let mut m = manifest(version);
    m["type_overrides"] = json!([{
        "type": "dev.pull_request",
        "add_enum": {"state": ["abandoned"]},
        "add_properties": {"team": {"type": "string"}}
    }]);
    m
}

fn pull_request(h: &H, fields: Value) -> String {
    h.ok(
        "tasks.create",
        json!({"title": "PR", "type_id": "dev.pull_request", "fields": fields}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// F2: an override going away (uninstall, disable, an update without it)
/// is refused while items of the overridden type need it.
#[test]
fn dropping_a_type_override_is_refused_while_items_need_it() {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(override_manifest("1.0.0"), &no_files(), Some(&k)));
    let a = pull_request(
        &h,
        json!({"repo": "acme/api", "number": 1, "state": "abandoned"}),
    );
    let b = pull_request(&h, json!({"repo": "acme/api", "number": 2, "team": "sre"}));
    let plain = pull_request(&h, json!({"repo": "acme/api", "number": 3}));
    for cmd in ["packs.uninstall", "packs.disable"] {
        let e = h.err(cmd, json!({"id": "acme.sre"}));
        code(&e, ErrorCode::Conflict);
        assert!(e.message.contains(&a) && e.message.contains(&b), "{e:?}");
        assert!(!e.message.contains(&plain), "{e:?}");
    }
    let e = h.install_err(&bundle(manifest("1.1.0"), &no_files(), Some(&k)));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains(&a), "{e:?}");
    assert!(
        preview_warning(&h, &bundle(manifest("1.1.0"), &no_files(), Some(&k))).contains(&a),
        "the preview names the items"
    );
    let p = h.pack("acme.sre");
    assert_eq!(
        (p["version"].clone(), p["enabled"].clone()),
        (json!("1.0.0"), json!(true))
    );
    // Every item still validates.
    h.ok(
        "tasks.update",
        json!({"id": a, "patch": {"fields": {"repo": "acme/api", "number": 1, "state": "closed"}}}),
    );
    h.ok(
        "tasks.update",
        json!({"id": b, "patch": {"fields": {"repo": "acme/api", "number": 2}}}),
    );
    h.ok("packs.uninstall", json!({"id": "acme.sre"}));
}

/// F2 for a key revocation, which is never refused: the pack is disabled
/// anyway and the items the override kept valid are reported.
#[test]
fn a_key_revocation_reports_items_its_override_kept_valid() {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(override_manifest("1.0.0"), &no_files(), Some(&k)));
    let a = pull_request(
        &h,
        json!({"repo": "acme/api", "number": 1, "state": "abandoned"}),
    );
    let r = h.ok("packs.keys_revoke", json!({"key_id": k.key_id()}));
    assert_eq!(r["disabled_packs"], json!(["acme.sre"]));
    assert_eq!(r["invalid_items"], json!([a]));
    // The item keeps its data; the user can bring it back in line.
    assert_eq!(h.fields(&a)["state"], "abandoned");
    h.ok(
        "tasks.update",
        json!({"id": a, "patch": {"fields": {"repo": "acme/api", "number": 1, "state": "closed"}}}),
    );
}

fn secret_manifest(version: &str, class: Option<&str>) -> Value {
    let mut m = manifest(version);
    let mut schema = json!({
        "type": "object",
        "properties": {"note": {"type": "string"}},
        "additionalProperties": false
    });
    if let Some(c) = class {
        schema["x-halp-data-class"] = json!(c);
    }
    m["task_types"] = json!([{"id": "acme.secret", "title": "Secret", "schema": schema}]);
    m
}

fn secret(h: &H) -> String {
    h.ok(
        "tasks.create",
        json!({"title": "s", "type_id": "acme.secret", "fields": {"note": "hush"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// F3a: an update may not change a type's data class, signed or
/// unsigned; the preview says why.
#[test]
fn an_update_cannot_change_a_types_data_class() {
    for signed in [true, false] {
        let h = h();
        let k = signer();
        h.trust(&k);
        let key = signed.then_some(&k);
        h.install(&bundle(
            secret_manifest("1.0.0", Some("acme.private")),
            &no_files(),
            key,
        ));
        let s = secret(&h);
        code(
            &h.err_as(&bot(), "tasks.get", json!({"id": s})),
            ErrorCode::NotFound,
        );
        for class in [None, Some("acme.other")] {
            let b = bundle(secret_manifest("1.1.0", class), &no_files(), key);
            let w = preview_warning(&h, &b);
            assert!(w.contains("data class"), "{w}");
            let e = h.install_err(&b);
            code(&e, ErrorCode::Conflict);
            assert!(e.message.contains("data class"), "{e:?}");
        }
        assert_eq!(h.pack("acme.sre")["version"], "1.0.0");
        code(
            &h.err_as(&bot(), "tasks.get", json!({"id": s})),
            ErrorCode::NotFound,
        );
    }
}

/// F3a, preview: a class dropped from a type that stays is an added
/// capability, never a removed one.
#[test]
fn a_weakened_data_class_previews_as_an_escalation() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = secret_manifest("1.0.0", Some("acme.private"));
    m["task_types"]
        .as_array_mut()
        .unwrap()
        .push(manifest("1.0.0")["task_types"][0].clone());
    h.install(&bundle(m.clone(), &no_files(), Some(&k)));
    let mut m2 = m.clone();
    m2["version"] = json!("1.1.0");
    m2["task_types"][0]["schema"]
        .as_object_mut()
        .unwrap()
        .remove("x-halp-data-class");
    let p = match h.preview(&bundle(m2, &no_files(), Some(&k))) {
        Response::Ok(p) => p,
        Response::Err(e) => panic!("{e:?}"),
    };
    let d = &p["permission_diff"];
    assert!(
        !d["removed"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "data_class"),
        "{d}"
    );
    assert!(
        d["added"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["kind"] == "data_class" && c["id"] == "acme.secret"),
        "{d}"
    );
    assert!(p["warnings"].to_string().contains("data class"), "{p}");
}

/// F3b: a migration may not retype items into a type of another data
/// class (refused when the manifest is checked).
#[test]
fn a_migration_cannot_retype_into_another_data_class() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = secret_manifest("1.0.0", Some("acme.private"));
    m["task_types"].as_array_mut().unwrap().push(json!({
        "id": "acme.open", "title": "Open",
        "schema": {"type": "object", "properties": {"note": {"type": "string"}}, "additionalProperties": false}
    }));
    h.install(&bundle(m.clone(), &no_files(), Some(&k)));
    let s = secret(&h);
    let mut m2 = m;
    m2["version"] = json!("1.1.0");
    m2["migrations"] = json!([{
        "type": "acme.secret", "from": "<1.1.0", "ops": [{"op": "retype", "to": "acme.open"}]
    }]);
    let e = h.install_err(&bundle(m2, &no_files(), Some(&k)));
    assert!(e.message.contains("data class"), "{e:?}");
    assert_eq!(
        h.ok("tasks.get", json!({"id": s}))["type_id"],
        "acme.secret"
    );
    code(
        &h.err_as(&bot(), "tasks.get", json!({"id": s})),
        ErrorCode::NotFound,
    );
}

/// F3, rollback: a rollback that would change a type's data class is
/// refused (a record written before the rule: 1.0.0 general, 1.1.0
/// private).
#[test]
fn a_rollback_cannot_change_a_types_data_class() {
    let h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(
        secret_manifest("1.0.0", None),
        &no_files(),
        Some(&k),
    ));
    let newer = bundle(
        secret_manifest("1.1.0", Some("acme.private")),
        &no_files(),
        Some(&k),
    );
    let mut h = h;
    h.with_store(|s| {
        let sha = s.put_blob(&newer).unwrap();
        let community = [hp::CommunityKey {
            key_id: k.key_id(),
            pubkey: k.public_key(),
            revoked: false,
        }];
        let vb = hp::verify(&newer, &community).unwrap();
        let before = s
            .get(halp_store::EntityKind::Pack, "acme.sre")
            .unwrap()
            .unwrap();
        let mut after = before.clone();
        after["previous"] = before["current"].clone();
        after["current"]["version"] = json!("1.1.0");
        after["current"]["bundle_sha256"] = json!(sha);
        after["current"]["manifest_sha256"] = json!(vb.manifest_sha256);
        after["current"]["types"] = serde_json::to_value(&vb.manifest.task_types).unwrap();
        s.commit(
            halp_store::NewEvent {
                actor: "system".into(),
                command: "test.forge".into(),
                entity: halp_domain::EntityRef {
                    kind: "pack".into(),
                    id: "acme.sre".into(),
                },
                summary: "forge".into(),
                args: json!({}),
                changes: vec![halp_store::Change {
                    kind: halp_store::EntityKind::Pack,
                    id: "acme.sre".into(),
                    before: Some(before),
                    after: Some(after),
                }],
                undoable: false,
            },
            Utc::now(),
        )
        .unwrap();
    });
    assert_eq!(h.pack("acme.sre")["version"], "1.1.0");
    let s = secret(&h);
    let e = h.err("packs.rollback", json!({"id": "acme.sre"}));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains("data class"), "{e:?}");
    code(
        &h.err_as(&bot(), "tasks.get", json!({"id": s})),
        ErrorCode::NotFound,
    );
}

/// F4: an update may not drop or change `x-halp-immutable-when` of a type
/// that has locked items (it would unlock them).
#[test]
fn an_update_cannot_unlock_locked_items() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = manifest("1.0.0");
    m["task_types"][0]["schema"]["x-halp-immutable-when"] = json!({"status": ["closed"]});
    h.install(&bundle(m.clone(), &no_files(), Some(&k)));
    let a = h.incident(json!({"sev": "1", "status": "closed"}));
    let drop_lock = |v: &str| {
        let mut m2 = manifest(v);
        m2["task_types"][0]["schema"]
            .as_object_mut()
            .unwrap()
            .remove("x-halp-immutable-when");
        m2
    };
    let mut relax = manifest("1.1.0");
    relax["task_types"][0]["schema"]["x-halp-immutable-when"] = json!({"sev": ["9"]});
    for m2 in [drop_lock("1.1.0"), relax] {
        let b = bundle(m2, &no_files(), Some(&k));
        assert!(preview_warning(&h, &b).contains("locked"));
        let e = h.install_err(&b);
        code(&e, ErrorCode::Conflict);
        assert!(e.message.contains("locked"), "{e:?}");
    }
    assert_eq!(h.pack("acme.sre")["version"], "1.0.0");
    // Still locked for an agent with the grant.
    h.ok(
        "grants.set",
        json!({"client_id": "bot", "data_classes": ["general"], "t1": true}),
    );
    code(
        &h.err_as(
            &bot(),
            "tasks.update",
            json!({"id": a, "patch": {"title": "x"}}),
        ),
        ErrorCode::Denied,
    );
    // With nothing locked, the predicate may change.
    h.ok("tasks.delete", json!({"id": a}));
    h.install(&bundle(drop_lock("1.2.0"), &no_files(), Some(&k)));
}

/// The preview lists what an update's type overrides change, with the
/// description text, so the user sees what agents will be shown.
#[test]
fn the_preview_lists_type_overrides() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = override_manifest("1.0.0");
    m["type_overrides"][0]["property_descriptions"] = json!({"repo": "owner/name"});
    m["type_overrides"][0]["title"] = json!("Change");
    let p = match h.preview(&bundle(m, &no_files(), Some(&k))) {
        Response::Ok(p) => p,
        Response::Err(e) => panic!("{e:?}"),
    };
    assert_eq!(
        p["overrides"],
        json!([{
            "type_id": "dev.pull_request",
            "title": "Change",
            "icon": null,
            "property_descriptions": {"repo": "owner/name"},
            "added_properties": ["team"],
            "added_enum_values": {"state": ["abandoned"]}
        }])
    );
}

/// LOW #2: a pack's pin is the key's full fingerprint.
#[test]
fn a_pack_is_pinned_to_the_full_fingerprint() {
    let (mut h, k) = with_acme();
    let mut pinned = String::new();
    h.with_store(|s| {
        let r = s
            .get(halp_store::EntityKind::Pack, "acme.sre")
            .unwrap()
            .unwrap();
        pinned = r["current"]["key_fp"].as_str().unwrap().to_string();
    });
    assert_eq!(pinned, fp(&k));
    // Updates still verify against it.
    h.install(&bundle(v110(), &no_files(), Some(&k)));
}

/// No installed pack's item may come to rely on a dev override: dev mode
/// is never saved, so a restart (or turning it off) would leave the item
/// invalid. Writes are refused, and turning dev mode off is a backstop.
#[test]
fn dev_mode_may_not_leave_installed_items_invalid() {
    let mut h = h();
    h.install(&bundle(manifest("1.0.0"), &no_files(), None));
    let src = tempfile::tempdir().unwrap();
    let pack = src.path().join("child");
    std::fs::create_dir_all(&pack).unwrap();
    let child = json!({
        "format": 1,
        "id": "acme.child",
        "version": "0.1.0",
        "title": "Child",
        "description": "Dev override",
        "license": "MIT",
        "publisher": {"name": "Acme"},
        "min_hub": "0.x",
        "extends": {"id": "acme.sre", "version": "^1.0"},
        "role": {"id": "acme-child", "title": "Child", "accent": "#112233"},
        "type_overrides": [{
            "type": "acme.incident",
            "add_properties": {"team": {"type": "string"}}
        }]
    });
    std::fs::write(
        pack.join("halp-pack.json"),
        serde_json::to_string_pretty(&child).unwrap(),
    )
    .unwrap();
    let path = src.path().display().to_string();
    assert_eq!(h.ok("packs.dev_set", json!({"path": path}))["packs"], 1);
    // The dev override is visible, but installed items cannot use it.
    let e = h.err(
        "tasks.create",
        json!({"title": "P", "type_id": "acme.incident", "fields": {"sev": "1", "team": "sre"}}),
    );
    code(&e, ErrorCode::InvalidArgs);
    assert!(e.message.contains("dev mode is not saved"), "{e:?}");
    let a = h.incident(json!({"sev": "1"}));
    code(
        &h.err(
            "tasks.update",
            json!({"id": a, "patch": {"fields": {"sev": "1", "team": "sre"}}}),
        ),
        ErrorCode::InvalidArgs,
    );
    // Installed packs and keys do not change while dev mode is on.
    for (cmd, args) in [
        ("packs.disable", json!({"id": "acme.sre"})),
        ("packs.uninstall", json!({"id": "acme.sre"})),
        ("packs.rollback", json!({"id": "acme.sre"})),
    ] {
        let e = h.err(cmd, args);
        code(&e, ErrorCode::Conflict);
        assert!(e.message.contains("dev mode"), "{cmd}: {e:?}");
    }
    let e = h.install_err(&bundle(manifest("1.1.0"), &no_files(), None));
    assert!(e.message.contains("dev mode"), "{e:?}");
    let k = signer();
    let e = h.err(
        "packs.keys_add",
        json!({"pubkey_b64": pubkey_b64(&k), "label": "Acme"}),
    );
    assert!(e.message.contains("dev mode"), "{e:?}");
    // A restart turns dev mode off and every item still validates.
    h.reopen();
    assert!(h.ok("packs.dev_get", json!({}))["path"].is_null());
    h.ok("tasks.update", json!({"id": a, "patch": {"title": "t2"}}));
    // Turning dev mode on and off again works as nothing relies on it.
    h.ok("packs.dev_set", json!({"path": path}));
    h.ok("packs.dev_reload", json!({}));
    h.ok("packs.dev_set", json!({"path": null}));
}

/// A widget entry must be an `.html` file, and SVG is never served (it
/// can run script as a document without a ticket).
#[test]
fn widget_entries_are_html_and_svg_is_not_served() {
    let h = h();
    let k = signer();
    h.trust(&k);
    let mut m = widget_manifest();
    m["ui_widgets"][0]["entry"] = "x.svg".into();
    let mut f = widget_files();
    f.insert("ui/rota/x.svg".into(), b"<svg/>".to_vec());
    let e = h.install_err(&bundle(m, &f, Some(&k)));
    assert!(e.message.contains(".html"), "{e:?}");
    h.install(&bundle(widget_manifest(), &f, Some(&k)));
    assert!(
        h.core()
            .ext_ui_file("acme.sre", "rota", "ui/rota/x.svg")
            .is_none()
    );
}

/// A dev pack never takes over a retired type of a real pack: its items
/// are the user's, and the retired definition must survive.
#[test]
fn a_dev_pack_cannot_take_over_a_retired_type() {
    let mut h = h();
    let k = signer();
    h.trust(&k);
    h.install(&bundle(manifest("1.0.0"), &no_files(), Some(&k)));
    let a = h.incident(json!({"sev": "1"}));
    h.ok("packs.uninstall", json!({"id": "acme.sre"}));
    let src = tempfile::tempdir().unwrap();
    let pack = src.path().join("other");
    std::fs::create_dir_all(&pack).unwrap();
    let mut m = manifest("0.1.0");
    m["id"] = "acme.other".into();
    m["role"]["id"] = "acme-other".into();
    m["task_types"][0]["schema"]["properties"]["sev"] = json!({"enum": ["3"]});
    std::fs::write(
        pack.join("halp-pack.json"),
        serde_json::to_string_pretty(&m).unwrap(),
    )
    .unwrap();
    let path = src.path().display().to_string();
    // The dev pack is blocked, or turning dev mode on is refused; either
    // way the retired type keeps its pack, trust and schema.
    let _ = h.call("packs.dev_set", json!({"path": path}));
    let t = h.type_info("acme.incident");
    assert_eq!(t["origin"], "retired", "{t}");
    assert_eq!(t["trust"], "community", "{t}");
    h.ok("tasks.update", json!({"id": a, "patch": {"title": "t2"}}));
    let _ = h.call("packs.dev_set", json!({"path": null}));
    h.reopen();
    let t = h.type_info("acme.incident");
    assert_eq!(
        (t["origin"].clone(), t["trust"].clone()),
        (json!("retired"), json!("community")),
        "{t}"
    );
    h.ok("tasks.update", json!({"id": a, "patch": {"title": "t3"}}));
}

/// Redefining a user type may not leave its items invalid.
#[test]
fn redefining_a_user_type_checks_its_items() {
    let h = h();
    let def = |ty: &str| {
        json!({"id": "mine.t", "title": "T", "schema": {
            "type": "object", "properties": {"a": {"type": ty}}, "additionalProperties": false
        }})
    };
    h.ok("types.define", def("string"));
    let id = h.ok(
        "tasks.create",
        json!({"title": "x", "type_id": "mine.t", "fields": {"a": "s"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = h.err("types.define", def("integer"));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains(&id), "{e:?}");
    h.ok("tasks.update", json!({"id": id, "patch": {"title": "y"}}));
    h.ok("types.define", def("string"));
}

#[test]
fn undo_and_redo_may_not_leave_items_invalid() {
    let h = h();
    let def = |extra: bool| {
        let mut props = json!({"a": {"type": "string"}});
        if extra {
            props["b"] = json!({"type": "integer"});
        }
        json!({"id": "mine.u", "title": "U", "schema": {
            "type": "object", "properties": props, "additionalProperties": false
        }})
    };
    let event = |h: &H, command: &str| -> String {
        h.ok("history.list", json!({"limit": 50}))["events"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["command"].as_str() == Some(command))
            .map(|e| e["id"].as_str().unwrap().to_string())
            .expect("event")
    };
    h.ok("types.define", def(true));
    let defined = event(&h, "types.define");
    let id = h.ok(
        "tasks.create",
        json!({"title": "x", "type_id": "mine.u", "fields": {"a": "s", "b": 1}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Undoing the definition would leave the item with an unknown type.
    let e = h.err("history.undo", json!({"event_id": defined}));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains(&id), "{e:?}");

    // Drop `b`, then narrow the type (allowed: no item uses `b`).
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": {"a": "s"}}}),
    );
    let edit = event(&h, "tasks.update");
    h.ok("types.define", def(false));
    // Undoing the edit would bring `b` back under a schema without it.
    let e = h.err("history.undo", json!({"event_id": edit}));
    code(&e, ErrorCode::Conflict);
    assert!(e.message.contains(&id), "{e:?}");
    // Undo the narrowing first, then the edit: both are fine.
    h.ok("history.undo", json!({}));
    h.ok("history.undo", json!({}));
    let item = h.ok("tasks.get", json!({"id": id}));
    assert_eq!(item["fields"]["b"], json!(1), "{item}");
    // Redo the edit, then try to redo the narrowing after `b` is back.
    h.ok("history.redo", json!({}));
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": {"a": "s", "b": 2}}}),
    );
    // The new edit cleared the redo stack; narrowing again is refused by
    // define itself.
    code(&h.err("types.define", def(false)), ErrorCode::Conflict);
}

#[test]
fn items_of_a_type_that_fails_to_load_fail_closed() {
    let mut h = h();
    h.ok(
        "types.define",
        json!({"id": "mine.w", "title": "W", "schema": {"type": "object"}}),
    );
    let id = h.ok(
        "tasks.create",
        json!({"title": "w item", "type_id": "mine.w", "fields": {}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok_as(&bot(), "tasks.get", json!({"id": id}));
    // The stored definition is damaged: the type no longer loads.
    h.with_store(|s| {
        let before = s.get(halp_store::EntityKind::TaskType, "mine.w").unwrap();
        let mut after = before.clone().unwrap();
        after["schema"] = json!({"type": 5});
        s.commit(
            halp_store::NewEvent {
                actor: "system".into(),
                command: "test.forge".into(),
                entity: halp_domain::EntityRef {
                    kind: "task_type".into(),
                    id: "mine.w".into(),
                },
                summary: "forge".into(),
                args: json!({}),
                changes: vec![halp_store::Change {
                    kind: halp_store::EntityKind::TaskType,
                    id: "mine.w".into(),
                    before,
                    after: Some(after),
                }],
                undoable: false,
            },
            Utc::now(),
        )
        .unwrap();
    });
    // Its items are now treated as the most private class.
    code(
        &h.err_as(&bot(), "tasks.get", json!({"id": id})),
        ErrorCode::NotFound,
    );
    assert!(
        !h.ok_as(&bot(), "tasks.list", json!({}))
            .to_string()
            .contains("w item")
    );
    h.ok("tasks.get", json!({"id": id}));
}
