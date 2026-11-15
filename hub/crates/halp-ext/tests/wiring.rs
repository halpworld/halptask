//! P5-C2: Tier B wired to a real core (docs/hub/P5-EXTENSIONS.md §5.4).
//!
//! The acceptance tests: a module's input runs as `module:<pack>` and is
//! audited; a non-`general` item never reaches the module; a module T1
//! attempt is denied and nothing is parked. Plus the `computed` widget
//! binding (`run_computed`).
#![allow(clippy::unwrap_used)]

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::halp_pack::{self as hp, SigningKey};
use halp_core::{Core, Ctx, ErrorCode, FixedClock, PackTrust, Request, Response};
use halp_ext::InputSource;
use halp_ext::wiring::{self, ComputedError, CoreInputs};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

const KEY: [u8; 32] = [7; 32];
const PACK: &str = "acme.sre";

struct H {
    core: Arc<Core>,
    _dir: TempDir,
}

fn signer() -> SigningKey {
    SigningKey::from_pkcs8(&hp::generate_pkcs8().unwrap()).unwrap()
}

/// A signed community pack with one module (`burn`, the echo module: its
/// output is its whole input envelope), the `computed` widget kind, and a
/// type of the private class `acme.private`.
fn manifest() -> Value {
    json!({
        "format": 1,
        "id": PACK,
        "version": "1.0.0",
        "title": "Acme SRE",
        "description": "On-call tooling",
        "license": "MIT",
        "publisher": {"name": "Acme"},
        "min_hub": "0.x",
        "extends": {"id": "halp.developer", "version": "^0.1"},
        "role": {"id": "acme-sre", "title": "SRE", "accent": "#112233"},
        "widgets": {"add": ["computed"]},
        "task_types": [{
            "id": "acme.secret",
            "title": "Secret",
            "schema": {
                "type": "object",
                "x-halp-data-class": "acme.private",
                "properties": {"note": {"type": "string"}},
                "additionalProperties": false
            }
        }],
        "modules": [{
            "name": "burn",
            "wasm": "wasm/burn.wasm",
            "inputs": {
                "open": {"command": "tasks.list", "args": {}},
                "types": {"command": "types.list"}
            },
            "output_schema": {"type": "object"}
        }]
    })
}

fn bundle(key: &SigningKey) -> Vec<u8> {
    let mut files = BTreeMap::new();
    files.insert("wasm/burn.wasm".to_string(), common::echo_module());
    let mut m = manifest();
    m["files"] = serde_json::to_value(hp::file_index(&files)).unwrap();
    hp::build_bundle(
        &serde_json::to_string_pretty(&m).unwrap(),
        &files,
        Some(key),
    )
    .unwrap()
}

impl H {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new(KEY)).unwrap();
        let clock = Arc::new(FixedClock {
            now: Utc.with_ymd_and_hms(2026, 11, 11, 9, 0, 0).unwrap(),
            today: NaiveDate::from_ymd_opt(2026, 11, 11).unwrap(),
        });
        let core = Arc::new(Core::with_store(store, clock).unwrap());
        Self { core, _dir: dir }
    }

    fn call(&self, cmd: &str, args: Value) -> Response {
        match self.core.dispatch(&Ctx::user(), Request::new(cmd, args)) {
            Response::Err(e) if e.code == ErrorCode::ConfirmationRequired => self.core.dispatch(
                &Ctx::user(),
                Request::new("consent.approve", json!({"id": e.confirmation_id.unwrap()})),
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

    /// Installs the pack, switches to its role and places a `computed`
    /// widget for `acme.sre/burn`. Returns the widget id.
    fn setup(&self) -> String {
        let k = signer();
        self.ok(
            "packs.keys_add",
            json!({"pubkey_b64": hp::crypto::encode_pubkey(&k.public_key()), "label": "Acme"}),
        );
        let p = self.ok(
            "packs.preview",
            json!({"bundle_base64": STANDARD.encode(bundle(&k))}),
        );
        self.ok("packs.install", json!({"preview_id": p["preview_id"]}));
        self.ok("role.switch", json!({"id": "acme-sre"}));
        self.place(json!({"transform": "acme.sre/burn", "view": "metric"}))
    }

    fn place(&self, config: Value) -> String {
        let l = self.ok(
            "layout.add_widget",
            json!({"kind": "computed", "config": config}),
        );
        l["widgets"].as_array().unwrap().last().unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn task(&self, title: &str, type_id: Option<&str>) -> String {
        let mut args = json!({"title": title});
        if let Some(t) = type_id {
            args["type_id"] = json!(t);
            args["fields"] = json!({"note": "classified"});
        }
        self.ok("tasks.create", args)["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn audit(&self) -> Vec<Value> {
        self.ok("audit.list", json!({"limit": 200}))["records"]
            .as_array()
            .unwrap()
            .clone()
    }
}

#[test]
fn module_inputs_run_as_the_module_and_are_audited() {
    let h = H::new();
    let widget = h.setup();
    wiring::attach(&h.core).unwrap();
    let run = wiring::run_computed(&h.core, PACK, "burn", &widget).unwrap();
    assert_eq!(run.trust, PackTrust::Community);
    assert_eq!(run.publisher, "Acme");
    assert_eq!(run.title, "Acme SRE");
    // The echo module returns its envelope: inputs, config, now, today.
    assert_eq!(run.output["config"]["transform"], "acme.sre/burn");
    assert_eq!(run.output["config"]["view"], "metric");
    assert_eq!(run.output["today"], "2026-11-11");
    assert!(run.output["inputs"]["open"].is_object() || run.output["inputs"]["open"].is_array());
    assert!(run.output["inputs"]["types"].is_object() || run.output["inputs"]["types"].is_array());
    let module_reads: Vec<_> = h
        .audit()
        .into_iter()
        .filter(|r| r["actor"] == "module:acme.sre")
        .collect();
    for cmd in ["tasks.list", "types.list"] {
        assert!(
            module_reads
                .iter()
                .any(|r| r["command"] == cmd && r["decision"] == "allow"),
            "{cmd} by the module is audited: {module_reads:?}"
        );
    }
}

#[test]
fn non_general_items_never_reach_the_module() {
    let h = H::new();
    let widget = h.setup();
    let open = h.task("visible chore", None);
    let secret = h.task("hidden secret", Some("acme.secret"));
    wiring::attach(&h.core).unwrap();
    let run = wiring::run_computed(&h.core, PACK, "burn", &widget).unwrap();
    let text = run.output["inputs"].to_string();
    assert!(text.contains(&open), "general item reaches the module");
    assert!(!text.contains(&secret), "private item never does: {text}");
    assert!(!text.contains("hidden secret"));
    assert!(!text.contains("classified"));

    // Directly through the input source: a get of the private item is not
    // found, and a list leaves it out.
    let src = CoreInputs::new(&h.core);
    let e = src
        .fetch(PACK, "tasks.get", &json!({"id": secret}))
        .unwrap_err();
    assert_eq!(e.code, "not_found");
    let got = src.fetch(PACK, "tasks.get", &json!({"id": open})).unwrap();
    assert_eq!(got["id"], open);
    let list = src.fetch(PACK, "tasks.list", &json!({})).unwrap();
    assert!(!list.to_string().contains(&secret));

    // The second lock on its own: a hand-made value holding the private
    // item (as a list entry, a field and the top level) is scrubbed.
    let item = h.ok("tasks.get", json!({"id": secret}));
    let general = h.ok("tasks.get", json!({"id": open}));
    let ctx = wiring::module_ctx(PACK);
    let v = wiring::only_visible(
        &h.core,
        &ctx,
        json!({"items": [item.clone(), general.clone()], "one": item.clone()}),
    )
    .unwrap();
    assert_eq!(v["items"].as_array().unwrap().len(), 1);
    assert_eq!(v["items"][0]["id"], open);
    assert!(v["one"].is_null());
    let e = wiring::only_visible(&h.core, &ctx, item.clone()).unwrap_err();
    assert_eq!(e.code, "not_found");
    // The user may see it: the filter is about the module, not the item.
    let v = wiring::only_visible(&h.core, &Ctx::user(), item).unwrap();
    assert_eq!(v["id"], secret);
}

#[test]
fn a_module_write_is_denied_and_nothing_is_parked() {
    let h = H::new();
    h.setup();
    let ctx = wiring::module_ctx(PACK);
    let r = h.core.dispatch(
        &ctx,
        Request::new("tasks.create", json!({"title": "from a module"})),
    );
    match r {
        Response::Err(e) => {
            assert_eq!(e.code, ErrorCode::Denied);
            assert!(e.confirmation_id.is_none());
        }
        Response::Ok(v) => panic!("module write succeeded: {v}"),
    }
    let pending = h.ok("consent.list", json!({}));
    assert!(
        !pending.to_string().contains("module:"),
        "nothing parked: {pending}"
    );
    assert!(
        !h.ok("tasks.list", json!({}))
            .to_string()
            .contains("from a module")
    );
    let rec = h
        .audit()
        .into_iter()
        .find(|r| r["actor"] == "module:acme.sre" && r["command"] == "tasks.create")
        .unwrap();
    assert_eq!(rec["decision"], "deny");
    // And the input source refuses anything outside the read allow-list
    // before it reaches the core.
    let e = CoreInputs::new(&h.core)
        .fetch(PACK, "tasks.create", &json!({"title": "x"}))
        .unwrap_err();
    assert_eq!(e.code, "denied");
}

#[test]
fn run_computed_is_bound_to_an_enabled_pack_and_a_placed_widget() {
    let h = H::new();
    let widget = h.setup();
    // No host attached yet.
    assert!(matches!(
        wiring::run_computed(&h.core, PACK, "burn", &widget),
        Err(ComputedError::Internal(_))
    ));
    wiring::attach(&h.core).unwrap();
    assert!(wiring::run_computed(&h.core, PACK, "burn", &widget).is_ok());
    let unavailable = |pack: &str, name: &str, w: &str| {
        matches!(
            wiring::run_computed(&h.core, pack, name, w),
            Err(ComputedError::Unavailable(_))
        )
    };
    // Unknown module, unknown pack, unknown widget.
    assert!(unavailable(PACK, "nope", &widget));
    assert!(unavailable("halp.developer", "burn", &widget));
    assert!(unavailable(PACK, "burn", "w-nope"));
    // A widget whose transform names something else.
    let other = h.place(json!({"transform": "acme.sre/other", "view": "list"}));
    assert!(unavailable(PACK, "burn", &other));
    // A widget that is not `computed`.
    let l = h.ok("layout.get", json!({}));
    let not_computed = l["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] != "computed")
        .map(|w| w["id"].as_str().unwrap().to_string());
    if let Some(id) = not_computed {
        assert!(unavailable(PACK, "burn", &id));
    }
    // Disabled: unavailable. Enabled again: runs.
    h.ok("packs.disable", json!({"id": PACK}));
    assert!(unavailable(PACK, "burn", &widget));
    h.ok("packs.enable", json!({"id": PACK}));
    h.ok("role.switch", json!({"id": "acme-sre"}));
    assert!(wiring::run_computed(&h.core, PACK, "burn", &widget).is_ok());
    // Uninstalled: unavailable.
    h.ok("packs.uninstall", json!({"id": PACK}));
    assert!(unavailable(PACK, "burn", &widget));
}
