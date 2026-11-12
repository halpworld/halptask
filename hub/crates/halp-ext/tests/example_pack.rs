//! P5-E2: the committed example pack `hub/examples/packs/acme-sre`
//! (`acme.platform-sre`, signed with the test-only key in
//! `hub/examples/keys/`) proves the P5 goal end to end: verify the signed
//! bundle, install it into a core, place its widgets, run its Tier B
//! transform and serve its Tier C entry.
//!
//! Lives in halp-ext because it needs the real wasm host and `wat`.
//! `HALP_REGEN_EXAMPLE=1 cargo test -p halp-ext --test example_pack`
//! recompiles `wasm/burn.wasm` from `wasm/burn.wat` and rewrites the
//! manifest's `files` index and re-signs the bundle with the test key.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::halp_pack::{self as hp, CommunityKey, PackTrust};
use halp_core::{Core, Ctx, ErrorCode, FixedClock, Request, Response};
use halp_ext::wiring;
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};

const PACK: &str = "acme.platform-sre";

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

fn pack_dir() -> PathBuf {
    examples().join("packs/acme-sre")
}

fn collect(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) {
    for e in std::fs::read_dir(dir).unwrap() {
        let e = e.unwrap();
        let name = e.file_name().into_string().unwrap();
        if name.starts_with('.') {
            continue;
        }
        let path = if rel.is_empty() {
            name.clone()
        } else {
            format!("{rel}/{name}")
        };
        if path == "halp-pack.json" {
            continue;
        }
        if e.file_type().unwrap().is_dir() {
            collect(&e.path(), &path, out);
        } else {
            out.insert(path, std::fs::read(e.path()).unwrap());
        }
    }
}

fn dir_files() -> BTreeMap<String, Vec<u8>> {
    let mut f = BTreeMap::new();
    collect(&pack_dir(), "", &mut f);
    f
}

fn pubkey() -> [u8; 32] {
    let b64 = std::fs::read_to_string(examples().join("keys/NEVER-TRUST-test-only.pub")).unwrap();
    hp::crypto::decode_pubkey(b64.trim()).unwrap()
}

fn community_key() -> CommunityKey {
    let pk = pubkey();
    CommunityKey {
        key_id: hp::key_id(&pk),
        pubkey: pk,
        revoked: false,
    }
}

fn bundle_bytes() -> Vec<u8> {
    std::fs::read(examples().join("packs/acme-sre.halppack")).unwrap()
}

/// Rebuilds the module, the manifest's file index and the signed bundle on
/// request.
#[test]
fn regen_when_asked() {
    if std::env::var_os("HALP_REGEN_EXAMPLE").is_none() {
        return;
    }
    let wat = std::fs::read_to_string(pack_dir().join("wasm/burn.wat")).unwrap();
    std::fs::write(
        pack_dir().join("wasm/burn.wasm"),
        wat::parse_str(&wat).unwrap(),
    )
    .unwrap();
    let mpath = pack_dir().join("halp-pack.json");
    let mut m: Value = serde_json::from_str(&std::fs::read_to_string(&mpath).unwrap()).unwrap();
    m["files"] = serde_json::to_value(hp::file_index(&dir_files())).unwrap();
    let manifest = serde_json::to_string_pretty(&m).unwrap() + "\n";
    std::fs::write(&mpath, &manifest).unwrap();
    // Sign it as `halpd pack sign` does. (`halpd pack sign` itself checks
    // the result without the signer's key trusted, which a pack with code
    // cannot pass; this test signs directly.)
    let key = std::fs::read_to_string(examples().join("keys/NEVER-TRUST-test-only.key")).unwrap();
    let key = hp::SigningKey::from_pkcs8(&STANDARD.decode(key.trim()).unwrap()).unwrap();
    let bytes = hp::build_bundle(&manifest, &dir_files(), Some(&key)).unwrap();
    hp::verify(&bytes, &[community_key()]).unwrap();
    std::fs::write(examples().join("packs/acme-sre.halppack"), bytes).unwrap();
}

#[test]
fn sources_are_in_sync() {
    // The committed wasm is exactly the WAT's compilation.
    let wat = std::fs::read_to_string(pack_dir().join("wasm/burn.wat")).unwrap();
    assert_eq!(
        wat::parse_str(&wat).unwrap(),
        std::fs::read(pack_dir().join("wasm/burn.wasm")).unwrap(),
        "wasm/burn.wasm is stale: run with HALP_REGEN_EXAMPLE=1"
    );
    // The manifest's index covers every file.
    let text = std::fs::read_to_string(pack_dir().join("halp-pack.json")).unwrap();
    let (m, _) = hp::Manifest::parse(&text, PackTrust::Community).unwrap();
    assert_eq!(
        m.files,
        hp::file_index(&dir_files()),
        "file index is stale: run with HALP_REGEN_EXAMPLE=1"
    );
    // The module may only import what halp-ext allows: nothing here.
    let imports = wasm_imports(&std::fs::read(pack_dir().join("wasm/burn.wasm")).unwrap());
    assert!(imports.is_empty(), "the example module imports {imports:?}");
}

/// Import names of a wasm module (section 2), without a parser crate: the
/// example has none, and a module with any import fails halp-ext's check.
fn wasm_imports(wasm: &[u8]) -> Vec<String> {
    let mut i = 8;
    let mut out = Vec::new();
    while i < wasm.len() {
        let id = wasm[i];
        i += 1;
        let (mut size, mut shift) = (0usize, 0);
        loop {
            let b = wasm[i];
            i += 1;
            size |= usize::from(b & 0x7f) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                break;
            }
        }
        if id == 2 && size > 1 {
            out.push("an import section".to_string());
        }
        i += size;
    }
    out
}

#[test]
fn the_committed_bundle_verifies_and_matches_the_folder() {
    let bytes = bundle_bytes();
    // With the test key trusted: a community pack.
    let v = hp::verify(&bytes, &[community_key()]).unwrap();
    assert_eq!(v.trust, PackTrust::Community);
    assert_eq!(v.manifest.id, PACK);
    let signer = v.signer.as_ref().unwrap();
    assert_eq!(signer.key_id, hp::key_id(&pubkey()));
    // Without the key trusted, a pack with code is refused outright.
    let err = hp::verify(&bytes, &[]).unwrap_err();
    assert!(err.message.contains("trusted key"), "{}", err.message);
    // It is exactly the folder (re-sign after editing).
    assert_eq!(v.files, dir_files(), "bundle is stale: re-sign the folder");
    let text = std::fs::read_to_string(pack_dir().join("halp-pack.json")).unwrap();
    let (m, _) = hp::Manifest::parse(&text, PackTrust::Community).unwrap();
    assert_eq!(v.manifest, m, "bundle manifest differs from the folder");
}

struct H {
    core: Arc<Core>,
    _dir: tempfile::TempDir,
}

impl H {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([5; 32])).unwrap();
        let clock = Arc::new(FixedClock {
            now: Utc.with_ymd_and_hms(2026, 11, 11, 9, 0, 0).unwrap(),
            today: NaiveDate::from_ymd_opt(2026, 11, 11).unwrap(),
        });
        Self {
            core: Arc::new(Core::with_store(store, clock).unwrap()),
            _dir: dir,
        }
    }
    fn ok(&self, cmd: &str, args: Value) -> Value {
        let ctx = Ctx::user();
        match self.core.dispatch(&ctx, Request::new(cmd, args)) {
            Response::Err(e) if e.code == ErrorCode::ConfirmationRequired => {
                match self.core.dispatch(
                    &ctx,
                    Request::new("consent.approve", json!({"id": e.confirmation_id.unwrap()})),
                ) {
                    Response::Ok(v) => v,
                    Response::Err(e) => panic!("{cmd}: {e:?}"),
                }
            }
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd}: {e:?}"),
        }
    }
}

#[test]
fn install_place_run_and_serve_the_example() {
    let h = H::new();
    // Settings > Packs: preview, then install trusting the key.
    let p = h.ok(
        "packs.preview",
        json!({"bundle_base64": STANDARD.encode(bundle_bytes())}),
    );
    assert_eq!(p["trust"], "community", "{p}");
    h.ok(
        "packs.install",
        json!({"preview_id": p["preview_id"], "trust_key": true}),
    );
    let pack = h.ok("packs.get", json!({"id": PACK}));
    assert_eq!(pack["trust"], "community");
    assert_eq!(pack["extends"][0], "halp.developer");
    assert_eq!(pack["capabilities"]["modules"], 1);
    assert_eq!(pack["capabilities"]["ui_widgets"], 1);
    assert_eq!(pack["capabilities"]["prompts"], 1);

    // The prompt is served to MCP, owned and labelled by the pack.
    let prompts = h.core.pack_prompts();
    assert_eq!(prompts.len(), 1, "{prompts:?}");
    assert_eq!(prompts[0].name, "oncall-handoff");
    assert_eq!(prompts[0].pack_id, PACK);
    assert!(prompts[0].text.contains("untrusted"));

    // Place both widgets in the pack's role.
    h.ok("role.switch", json!({"id": "acme-platform-sre"}));
    let l = h.ok(
        "layout.add_widget",
        json!({"kind": "computed", "config": {"transform": "acme.platform-sre/burn", "view": "metric"}}),
    );
    let widget = l["widgets"].as_array().unwrap().last().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let l = h.ok(
        "layout.add_widget",
        json!({"kind": "x.acme.platform-sre.oncall"}),
    );
    assert_eq!(
        l["widgets"].as_array().unwrap().last().unwrap()["kind"],
        "x.acme.platform-sre.oncall"
    );

    // Two open incidents, one done, one unrelated task.
    let mk = |title: &str, fields: Value| {
        h.ok(
            "tasks.create",
            json!({"title": title, "type_id": "acme.incident", "fields": fields}),
        )["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    mk("API 5xx", json!({"sev": "1", "service": "api"}));
    mk("Slow queue", json!({"sev": "3", "service": "jobs"}));
    let done = mk("Old outage", json!({"sev": "2"}));
    h.ok("tasks.complete", json!({"id": done}));
    h.ok("tasks.create", json!({"title": "Water the plants"}));

    // Tier B: run the transform through the real host and wiring.
    wiring::attach(&h.core).unwrap();
    let run = wiring::run_computed(&h.core, PACK, "burn", &widget).unwrap();
    assert_eq!(run.trust, PackTrust::Community);
    assert_eq!(run.output["value"], 2, "{}", run.output);
    assert_eq!(run.output["label"], "Open incidents");
    // It parses as the `metric` view (computed.ts: a numeric or text
    // `value`, optional short label and hint).
    assert!(run.output["value"].is_number());
    assert!(run.output["label"].as_str().unwrap().chars().count() <= 80);
    assert!(run.output["hint"].as_str().unwrap().chars().count() <= 200);
    // The module's reads were audited as the module.
    let audit = h.ok("audit.list", json!({"limit": 200}));
    assert!(
        audit["records"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["actor"] == format!("module:{PACK}") && r["command"] == "tasks.list")
    );

    // Tier C: the entry document and its script are served, nothing else.
    let w = h.core.ext_ui_widget(PACK, "oncall").unwrap();
    assert_eq!(w.kind, "x.acme.platform-sre.oncall");
    assert_eq!(w.entry, "ui/oncall/index.html");
    assert_eq!(w.calls, vec!["tasks_list".to_string()]);
    let html = h.core.ext_ui_file(PACK, "oncall", &w.entry).unwrap();
    assert!(html.mime.starts_with("text/html"));
    let doc = String::from_utf8(html.bytes).unwrap();
    // No inline script: the served CSP allows only the pack's own files.
    assert!(doc.contains("<script src=\"oncall.js\"></script>"));
    assert!(!doc.contains("<script>"));
    let js = h
        .core
        .ext_ui_file(PACK, "oncall", "ui/oncall/oncall.js")
        .unwrap();
    assert!(js.mime.starts_with("text/javascript"));
    let js = String::from_utf8(js.bytes).unwrap();
    assert!(js.contains("ui/initialize") && js.contains("tools/call"));
    assert!(!js.contains("innerHTML") && !js.contains("eval("));
    assert!(
        h.core
            .ext_ui_file(PACK, "oncall", "wasm/burn.wasm")
            .is_none()
    );
    assert!(
        h.core
            .ext_ui_file(PACK, "oncall", "prompts/oncall-handoff.md")
            .is_none()
    );

    // Disabling withdraws everything.
    h.ok("packs.disable", json!({"id": PACK}));
    assert!(h.core.ext_ui_file(PACK, "oncall", &w.entry).is_none());
    assert!(h.core.pack_prompts().is_empty());
}
