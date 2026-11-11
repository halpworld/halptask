//! Integration commands through `Core::dispatch`: configuration, the
//! `ext.upsert` write path (idempotency, user edits, archive on remote
//! delete), status events and the host hook.
#![allow(clippy::unwrap_used)]

use std::sync::{Arc, Mutex};

use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::{Actor, Core, Ctx, ErrorCode, FixedClock, Request, Response};
use halp_domain::integration::{
    CaptureTicket, HostHandle, IntegrationConfig, IntegrationHost, IntegrationState,
    IntegrationStatus,
};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

#[derive(Default)]
struct FakeHost {
    log: Mutex<Vec<String>>,
}

impl IntegrationHost for FakeHost {
    fn validate(&self, kind: &str, settings: &Value) -> Result<Value, String> {
        if settings.get("bad").is_some() {
            return Err("bad setting".into());
        }
        self.log.lock().unwrap().push(format!("validate {kind}"));
        Ok(settings.clone())
    }
    fn configured(&self, cfg: &IntegrationConfig, reauthorize: bool) {
        self.log
            .lock()
            .unwrap()
            .push(format!("configured {} {reauthorize}", cfg.id));
    }
    fn removed(&self, id: &str) {
        self.log.lock().unwrap().push(format!("removed {id}"));
    }
    fn sync_now(&self, id: Option<&str>) -> Result<Vec<String>, String> {
        Ok(vec![id.unwrap_or("all").to_string()])
    }
    fn status(&self, _id: &str) -> Option<IntegrationStatus> {
        Some(IntegrationStatus {
            state: IntegrationState::Ok,
            has_secret: true,
            ..Default::default()
        })
    }
    fn capture_context(
        &self,
        cfg: &IntegrationConfig,
        repo_path: &str,
    ) -> Result<CaptureTicket, String> {
        Ok(CaptureTicket {
            integration: cfg.id.clone(),
            repo_path: repo_path.to_string(),
            queued: true,
        })
    }
}

struct H {
    core: Core,
    host: Arc<FakeHost>,
    _dir: TempDir,
}

fn h() -> H {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([9; 32])).unwrap();
    let clock = Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    });
    let core = Core::with_store(store, clock).unwrap();
    let host = Arc::new(FakeHost::default());
    core.set_extension(Arc::new(HostHandle(host.clone())));
    H {
        core,
        host,
        _dir: dir,
    }
}

fn gh() -> Ctx {
    Ctx::new(Actor::Integration("gh".into()))
}

impl H {
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
    fn connect_gh(&self) {
        self.ok(
            "integrations.connect",
            json!({"id": "gh", "kind": "github", "title": "GitHub", "settings": {"repos": []}}),
        );
    }
    fn upsert(&self, items: Value, complete: bool) -> Value {
        self.ok_as(
            &gh(),
            "ext.upsert",
            json!({"scope": "prs", "complete": complete, "items": items}),
        )
    }
    fn item(&self, id: &str) -> Value {
        self.ok("tasks.get", json!({"id": id}))
    }
    fn events(&self) -> usize {
        self.ok("history.list", json!({"limit": 1000}))["events"]
            .as_array()
            .unwrap()
            .len()
    }
}

fn pr(n: u32, title: &str) -> Value {
    json!({
        "external_id": format!("github:acme/api#{n}"),
        "title": title,
        "type_id": "dev.pull_request",
        "fields": {"repo": "acme/api", "number": n, "state": "open", "ci": "passing"},
        "tags": ["pr"],
    })
}

#[test]
fn connect_stores_config_and_tells_the_host() {
    let h = h();
    let r = h.ok(
        "integrations.connect",
        json!({"id": "gh", "kind": "github", "settings": {"repos": ["acme/*"]}}),
    );
    assert_eq!(r["config"]["kind"], "github");
    assert_eq!(r["config"]["roles"], json!(["developer"]));
    assert_eq!(r["status"]["state"], "ok");
    let log = h.host.log.lock().unwrap().clone();
    assert_eq!(log, vec!["validate github", "configured gh false"]);

    let l = h.ok("integrations.list", json!({}));
    assert_eq!(l["running"], true);
    assert_eq!(l["integrations"][0]["config"]["id"], "gh");

    // Settings the host rejects, unknown kinds and kind changes fail.
    let e = h.err_as(
        &Ctx::user(),
        "integrations.connect",
        json!({"id": "x", "kind": "github", "settings": {"bad": 1}}),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs);
    let e = h.err_as(
        &Ctx::user(),
        "integrations.connect",
        json!({"id": "x", "kind": "ftp"}),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs);
    let e = h.err_as(
        &Ctx::user(),
        "integrations.connect",
        json!({"id": "gh", "kind": "git"}),
    );
    assert_eq!(e.code, ErrorCode::Conflict);
    // Secrets are not an argument.
    let e = h.err_as(
        &Ctx::user(),
        "integrations.connect",
        json!({"id": "gh", "kind": "github", "secret": "ghp_x"}),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs);
}

#[test]
fn configuration_is_user_only() {
    let h = h();
    for ctx in [gh(), Ctx::new(Actor::Agent("a".into()))] {
        let e = h.err_as(
            &ctx,
            "integrations.connect",
            json!({"id": "gh", "kind": "github"}),
        );
        assert_eq!(e.code, ErrorCode::Denied);
        let e = h.err_as(&ctx, "integrations.disconnect", json!({"id": "gh"}));
        assert_eq!(e.code, ErrorCode::Denied);
    }
    let names: Vec<(String, bool)> = h
        .core
        .descriptors()
        .into_iter()
        .filter(|d| d.name.starts_with("integrations.") || d.name.starts_with("ext."))
        .map(|d| (d.name, d.mcp))
        .collect();
    assert!(names.contains(&("integrations.connect".into(), false)));
    assert!(names.contains(&("ext.upsert".into(), false)));
    assert!(names.contains(&("ext.status".into(), false)));
    assert!(names.contains(&("integrations.list".into(), true)));
}

#[test]
fn upsert_is_only_for_the_configured_integration() {
    let h = h();
    // Not configured yet.
    let e = h.err_as(
        &gh(),
        "ext.upsert",
        json!({"scope": "prs", "items": [pr(1, "A")]}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    h.connect_gh();
    // The user (or an agent) cannot write through it.
    let e = h.err_as(
        &Ctx::user(),
        "ext.upsert",
        json!({"scope": "prs", "items": [pr(1, "A")]}),
    );
    assert_eq!(e.code, ErrorCode::Denied);
    // Another integration id is not configured.
    let e = h.err_as(
        &Ctx::new(Actor::Integration("other".into())),
        "ext.upsert",
        json!({"scope": "prs", "items": []}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    // Disabled integrations cannot write.
    h.ok(
        "integrations.connect",
        json!({"id": "gh", "kind": "github", "enabled": false}),
    );
    let e = h.err_as(&gh(), "ext.upsert", json!({"scope": "prs", "items": []}));
    assert_eq!(e.code, ErrorCode::Denied);
}

#[test]
fn upsert_is_idempotent_and_tags_the_origin() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "Fix login"), pr(2, "Add cache")]), false);
    assert_eq!(r["created"], 2);
    let id1 = r["items"][0]["item_id"].as_str().unwrap().to_string();
    let it = h.item(&id1);
    assert_eq!(it["origin"], "integration:gh");
    assert_eq!(it["type_id"], "dev.pull_request");
    assert_eq!(it["roles"], json!(["developer"]));
    // Under the integration's root bullet.
    let root = h.item(it["parent_id"].as_str().unwrap());
    assert_eq!(root["title"], "GitHub");
    assert_eq!(root["kind"], "bullet");

    let before = h.events();
    let r = h.upsert(json!([pr(1, "Fix login"), pr(2, "Add cache")]), false);
    assert_eq!(r["created"], 0);
    assert_eq!(r["unchanged"], 2);
    assert_eq!(r["items"][0]["item_id"], id1.as_str());
    assert_eq!(h.events(), before, "a no-op sync records nothing");

    let r = h.upsert(json!([pr(1, "Fix login (v2)")]), false);
    assert_eq!(r["updated"], 1);
    assert_eq!(h.item(&id1)["title"], "Fix login (v2)");
    assert_eq!(h.events(), before + 1);

    // Sync writes never land on the user's undo stack.
    let ev = h.ok("history.list", json!({"limit": 1}));
    assert_eq!(ev["events"][0]["command"], "ext.upsert");
    assert_eq!(ev["events"][0]["undoable"], false);
}

#[test]
fn event_args_carry_counts_not_content() {
    let h = h();
    h.connect_gh();
    h.upsert(json!([pr(7, "Secret project codename")]), false);
    let ev = h.ok("history.list", json!({"limit": 1}));
    let e = &ev["events"][0];
    assert_eq!(e["entity"], json!({"kind": "integration", "id": "gh"}));
    assert_eq!(e["summary"], "Synced GitHub: 1 new, 0 updated, 0 archived");
    assert!(!ev.to_string().contains("codename"));
}

#[test]
fn user_edits_survive_until_the_remote_changes() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "Fix login")]), false);
    let id = r["items"][0]["item_id"].as_str().unwrap().to_string();
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"title": "my title", "tags": ["mine"]}}),
    );
    h.upsert(json!([pr(1, "Fix login")]), false);
    assert_eq!(h.item(&id)["title"], "my title");
    // A remote change wins for remote-owned fields; tags are unioned.
    h.upsert(json!([pr(1, "Fix login now")]), false);
    let it = h.item(&id);
    assert_eq!(it["title"], "Fix login now");
    assert_eq!(it["tags"], json!(["mine", "pr"]));
}

#[test]
fn deleted_items_stay_deleted_unless_the_remote_changes() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A")]), false);
    let id = r["items"][0]["item_id"].as_str().unwrap().to_string();
    let e = h.err_as(&Ctx::user(), "tasks.delete", json!({"id": id}));
    h.ok("consent.approve", json!({"id": e.confirmation_id.unwrap()}));
    let r = h.upsert(json!([pr(1, "A")]), false);
    assert_eq!(r["skipped"], 1);
    assert_eq!(r["created"], 0);
    let r = h.upsert(json!([pr(1, "A changed")]), false);
    assert_eq!(r["created"], 1);
}

#[test]
fn remote_deletes_archive_and_never_delete() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B")]), true);
    let id2 = r["items"][1]["item_id"].as_str().unwrap().to_string();
    // An incomplete batch archives nothing.
    let r = h.upsert(json!([pr(1, "A")]), false);
    assert_eq!(r["archived"], 0);
    // A complete batch archives what is missing.
    let r = h.upsert(json!([pr(1, "A")]), true);
    assert_eq!(r["archived"], 1);
    let it = h.item(&id2);
    assert_eq!(it["status"], "done");
    assert!(it["tags"].as_array().unwrap().contains(&json!("archived")));
    // Archived again: nothing more happens.
    let r = h.upsert(json!([pr(1, "A")]), true);
    assert_eq!(r["archived"], 0);
    // It comes back.
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B")]), true);
    assert_eq!(r["restored"], 1);
    let it = h.item(&id2);
    assert_eq!(it["status"], "todo");
    assert!(!it["tags"].as_array().unwrap().contains(&json!("archived")));
    // Other scopes are untouched by a complete batch.
    h.ok_as(
        &gh(),
        "ext.upsert",
        json!({"scope": "ci", "complete": true, "items": []}),
    );
    assert_eq!(h.item(&id2)["status"], "todo");
}

fn pr_state(h: &H, id: &str) -> Value {
    let it = h.item(id);
    json!({"status": it["status"], "tags": it["tags"], "fields": it["fields"], "title": it["title"]})
}

#[test]
fn a_rejected_item_in_a_complete_batch_is_never_archived() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B")]), true);
    let id1 = r["items"][0]["item_id"].as_str().unwrap().to_string();
    let id2 = r["items"][1]["item_id"].as_str().unwrap().to_string();
    let before1 = pr_state(&h, &id1);
    let before2 = pr_state(&h, &id2);
    let mut bad = pr(2, "B");
    bad["fields"]["state"] = json!("no-such-state");
    let r = h.upsert(json!([pr(1, "A"), bad]), true);
    assert_eq!(r["rejected"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["archived"], 0, "{r}");
    assert_eq!(pr_state(&h, &id2), before2);
    assert_eq!(pr_state(&h, &id1), before1);
    // The next good batch finds PR 2 live, not archived.
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B")]), true);
    assert_eq!(r["restored"], 0, "{r}");
    assert_eq!(r["archived"], 0, "{r}");
}

#[test]
fn a_duplicate_item_in_a_complete_batch_is_never_archived() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B")]), true);
    let id2 = r["items"][1]["item_id"].as_str().unwrap().to_string();
    // PR 2 appears twice, the first copy invalid: the second is a
    // duplicate. Either way PR 2 is still listed upstream.
    let mut bad = pr(2, "B");
    bad["fields"]["state"] = json!("no-such-state");
    let r = h.upsert(json!([pr(1, "A"), bad, pr(2, "B")]), true);
    assert_eq!(r["archived"], 0, "{r}");
    assert_eq!(h.item(&id2)["status"], "todo");
}

#[test]
fn an_item_without_a_usable_id_downgrades_a_complete_batch() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B")]), true);
    let id2 = r["items"][1]["item_id"].as_str().unwrap().to_string();
    let r = h.upsert(json!([pr(1, "A"), {"external_id": "", "title": "x"}]), true);
    assert_eq!(r["archived"], 0, "{r}");
    assert!(r["note"].as_str().unwrap().contains("incomplete"), "{r}");
    assert_eq!(h.item(&id2)["status"], "todo");
}

#[test]
fn a_really_missing_item_is_still_archived_next_to_a_rejected_one() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A"), pr(2, "B"), pr(3, "C")]), true);
    let id2 = r["items"][1]["item_id"].as_str().unwrap().to_string();
    let id3 = r["items"][2]["item_id"].as_str().unwrap().to_string();
    let mut bad = pr(3, "C");
    bad["fields"]["state"] = json!("no-such-state");
    // PR 2 is gone upstream; PR 3 is listed but invalid.
    let r = h.upsert(json!([pr(1, "A"), bad]), true);
    assert_eq!(r["archived"], 1, "{r}");
    assert!(r.get("note").is_none() || r["note"].is_null(), "{r}");
    assert_eq!(h.item(&id2)["status"], "done");
    assert_eq!(h.item(&id3)["status"], "todo");
}

#[test]
fn untrusted_content_is_cleaned_and_bad_items_rejected_one_by_one() {
    let h = h();
    h.connect_gh();
    let mut evil = pr(3, "Fix\u{1b}[31m it\u{202E}now");
    evil["notes"] = json!("line1\nline2\u{7}\u{2066}");
    evil["fields"]["author"] = json!("bob\u{0}");
    let bad_schema = json!({
        "external_id": "github:acme/api#4", "title": "x",
        "type_id": "dev.pull_request", "fields": {"repo": "not a repo"}
    });
    let r = h.upsert(
        json!([
            evil,
            bad_schema,
            {"external_id": "", "title": "x"},
            {"external_id": "a\nb", "title": "x"},
            {"external_id": "z", "title": "\u{7}"},
            pr(3, "dup"),
        ]),
        false,
    );
    assert_eq!(r["created"], 1, "{r}");
    let reasons: Vec<&str> = r["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["reason"].as_str().unwrap())
        .collect();
    assert_eq!(reasons.len(), 5, "{reasons:?}");
    assert!(reasons.contains(&"duplicate external_id in batch"));
    let it = h.item(r["items"][0]["item_id"].as_str().unwrap());
    assert_eq!(it["title"], "Fix[31m itnow");
    assert_eq!(it["notes"], "line1\nline2");
    assert_eq!(it["fields"]["author"], "bob");
}

#[test]
fn agents_reading_synced_items_are_tainted() {
    let h = h();
    h.connect_gh();
    h.upsert(json!([pr(1, "A")]), false);
    h.ok("clients.pair", json!({"name": "a", "id": "a"}));
    let a = Ctx::agent("a").with_session("s1");
    let listed = h.ok_as(&a, "tasks.list", json!({}));
    let id = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["origin"] == "integration:gh" && i["kind"] == "task")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    // Any confirmation the session asks for now says where the taint came from.
    let e = h.err_as(&a, "tasks.delete", json!({"id": id}));
    assert_eq!(e.code, ErrorCode::ConfirmationRequired, "{e:?}");
    let p = h.ok("consent.get", json!({"id": e.confirmation_id.unwrap()}));
    assert_eq!(p["tainted"], true);
    assert_eq!(p["taint_source"], "integration:gh");
}

#[test]
fn status_changes_are_events() {
    let h = h();
    h.connect_gh();
    h.ok_as(
        &gh(),
        "ext.status",
        json!({"state": "error", "error": "HTTP 502\u{1b} from api"}),
    );
    let ev = h.ok("history.list", json!({"limit": 1}));
    let e = &ev["events"][0];
    assert_eq!(e["command"], "integrations.status");
    assert_eq!(e["actor"], "integration:gh");
    assert_eq!(e["summary"], "GitHub: error (HTTP 502 from api)");
    let e = h.err_as(&Ctx::user(), "ext.status", json!({"state": "ok"}));
    assert_eq!(e.code, ErrorCode::Denied);
}

#[test]
fn sync_now_and_capture_go_to_the_host() {
    let h = h();
    h.connect_gh();
    let r = h.ok("integrations.sync_now", json!({"id": "gh"}));
    assert_eq!(r["queued"], json!(["gh"]));
    // No git integration yet.
    let e = h.err_as(
        &Ctx::user(),
        "context.capture",
        json!({"repo_path": "/tmp/x"}),
    );
    assert_eq!(e.code, ErrorCode::NotFound);
    h.ok(
        "integrations.connect",
        json!({"id": "git", "kind": "git", "settings": {"roots": ["/tmp"]}}),
    );
    let r = h.ok("context.capture", json!({"repo_path": "/tmp/x"}));
    assert_eq!(
        r,
        json!({"integration": "git", "repo_path": "/tmp/x", "queued": true})
    );
}

#[test]
fn disconnect_removes_config_and_optionally_items() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([pr(1, "A")]), false);
    let id = r["items"][0]["item_id"].as_str().unwrap().to_string();
    let e = h.err_as(&Ctx::user(), "integrations.disconnect", json!({"id": "gh"}));
    assert_eq!(e.code, ErrorCode::ConfirmationRequired);
    let r = h.ok("consent.approve", json!({"id": e.confirmation_id.unwrap()}));
    assert_eq!(r["removed_items"], 0);
    assert!(
        h.host
            .log
            .lock()
            .unwrap()
            .contains(&"removed gh".to_string())
    );
    assert_eq!(h.item(&id)["title"], "A");
    assert_eq!(
        h.ok("integrations.list", json!({}))["integrations"],
        json!([])
    );

    h.connect_gh();
    let r = h.upsert(json!([pr(2, "B")]), false);
    let id2 = r["items"][0]["item_id"].as_str().unwrap().to_string();
    let e = h.err_as(
        &Ctx::user(),
        "integrations.disconnect",
        json!({"id": "gh", "remove_items": true}),
    );
    let r = h.ok("consent.approve", json!({"id": e.confirmation_id.unwrap()}));
    assert_eq!(r["removed_items"], 2, "the item and the new root bullet");
    let e = h.err_as(&Ctx::user(), "tasks.get", json!({"id": id2}));
    assert_eq!(e.code, ErrorCode::NotFound);
}

#[test]
fn without_a_host_list_reports_stopped() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([9; 32])).unwrap();
    let clock = Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    });
    let core = Core::with_store(store, clock).unwrap();
    match core.dispatch(&Ctx::user(), Request::new("integrations.list", json!({}))) {
        Response::Ok(v) => assert_eq!(v["running"], false),
        Response::Err(e) => panic!("{e:?}"),
    }
    match core.dispatch(
        &Ctx::user(),
        Request::new(
            "integrations.connect",
            json!({"id": "gh", "kind": "github"}),
        ),
    ) {
        Response::Err(e) => assert_eq!(e.code, ErrorCode::InvalidArgs),
        Response::Ok(v) => panic!("{v}"),
    }
}

// ------------------------------------------------------- ext.patch_fields

fn cal() -> Ctx {
    Ctx::new(Actor::Integration("cal".into()))
}

impl H {
    fn connect_cal(&self, roles: Value) {
        self.ok(
            "integrations.connect",
            json!({"id": "cal", "kind": "calendar", "title": "Calendar", "roles": roles,
                   "settings": {}}),
        );
    }
    /// A 1:1 the calendar created; returns its id.
    fn cal_one_on_one(&self) -> String {
        let r = self.ok_as(
            &cal(),
            "ext.upsert",
            json!({"scope": "one_on_ones", "items": [{
                "external_id": "1on1:p1:ics:k", "title": "1:1 with Alice", "type_id": "em.one_on_one",
                "fields": {"person_id": "p1", "scheduled_for": "2026-11-11T14:00:00Z", "status": "scheduled",
                           "agenda": [{"text": "growth plan"}]}}]}),
        );
        r["items"][0]["item_id"].as_str().unwrap().to_string()
    }
}

fn patch(id: &str, fields: Value) -> Value {
    json!({"id": id, "type_id": "em.one_on_one", "fields": fields})
}

#[test]
fn patch_fields_moves_whitelisted_fields_of_own_items_undoably() {
    let h = h();
    h.connect_cal(json!(["eng-manager"]));
    let id = h.cal_one_on_one();
    let v0 = h.item(&id)["version"].as_u64().unwrap();
    let out = h.ok_as(
        &cal(),
        "ext.patch_fields",
        json!({"id": id, "type_id": "em.one_on_one", "if_version": v0,
               "fields": {"scheduled_for": "2026-11-12T10:00:00Z"}}),
    );
    assert_eq!(out["fields"]["scheduled_for"], "2026-11-12T10:00:00Z");
    // Merged: the rest of the fields (the agenda) are untouched.
    assert_eq!(out["fields"]["agenda"][0]["text"], "growth plan");
    assert_eq!(out["fields"]["status"], "scheduled");
    let ev = &h.ok("history.list", json!({"limit": 1}))["events"][0];
    assert_eq!(ev["command"], "ext.patch_fields");
    assert_eq!(ev["actor"], "integration:cal");
    assert_eq!(ev["undoable"], true);
    // The same value again writes nothing.
    let n = h.events();
    h.ok_as(
        &cal(),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "2026-11-12T10:00:00Z"})),
    );
    assert_eq!(h.events(), n);
    // The user can undo it.
    h.ok("history.undo", json!({}));
    assert_eq!(
        h.item(&id)["fields"]["scheduled_for"],
        "2026-11-11T14:00:00Z"
    );
    // A stale version is a conflict.
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        json!({"id": id, "type_id": "em.one_on_one", "if_version": v0,
               "fields": {"scheduled_for": "2026-11-13T10:00:00Z"}}),
    );
    assert_eq!(e.code, ErrorCode::Conflict, "{e:?}");
}

#[test]
fn patch_fields_refuses_other_fields_other_items_and_other_actors() {
    let h = h();
    h.connect_cal(json!(["eng-manager"]));
    let id = h.cal_one_on_one();
    // Not whitelisted.
    for f in [
        json!({"agenda": []}),
        json!({"status": "done"}),
        json!({"scheduled_for": "x", "agenda": []}),
    ] {
        let e = h.err_as(&cal(), "ext.patch_fields", patch(&id, f));
        assert_eq!(e.code, ErrorCode::Denied, "{e:?}");
    }
    // Schema still applies to whitelisted fields.
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "not a time"})),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs, "{e:?}");
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        patch(
            &id,
            json!({"scheduled_for": "2026-11-12T10:00:00Z\u{202E}"}),
        ),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs, "{e:?}");
    // The text check runs before the schema and names the cause.
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "2026-11-12T10:00:00Z\u{7}"})),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs, "{e:?}");
    assert!(e.message.contains("control characters"), "{e:?}");
    // Wrong type id: not found.
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        json!({"id": id, "type_id": "em.meeting", "fields": {"scheduled_for": "2026-11-12T10:00:00Z"}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    // A 1:1 the user created is not the integration's.
    let mine = h.ok(
        "tasks.create",
        json!({"title": "1:1 Bob", "type_id": "em.one_on_one", "fields": {"person_id": "p2"}}),
    );
    let mine = mine["id"].as_str().unwrap();
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        patch(mine, json!({"scheduled_for": "2026-11-12T10:00:00Z"})),
    );
    assert_eq!(e.code, ErrorCode::Denied, "{e:?}");
    // Another integration cannot patch the calendar's item.
    h.connect_gh();
    let e = h.err_as(
        &gh(),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "2026-11-12T10:00:00Z"})),
    );
    assert_eq!(e.code, ErrorCode::Denied, "{e:?}");
    // Agents and the user do not get the integration path.
    h.ok("clients.pair", json!({"name": "a", "id": "a"}));
    let e = h.err_as(
        &Ctx::agent("a"),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "2026-11-12T10:00:00Z"})),
    );
    assert_eq!(e.code, ErrorCode::Denied, "{e:?}");
    let e = h.err_as(
        &Ctx::user(),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "2026-11-12T10:00:00Z"})),
    );
    assert_eq!(e.code, ErrorCode::Denied, "{e:?}");
    // Hidden from MCP.
    let cmds = h.ok("commands.list", json!({}));
    let d = cmds["commands"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "ext.patch_fields")
        .unwrap();
    assert_eq!(d["mcp"], false);
    assert_eq!(d["tier"], "T1");
}

#[test]
fn patch_fields_never_reaches_private_items() {
    let h = h();
    h.connect_cal(json!(["eng-manager"]));
    let id = h.cal_one_on_one();
    // The user files the 1:1 under a private note: it is private now.
    let note = h.ok(
        "tasks.create",
        json!({"title": "Notes", "kind": "bullet", "type_id": "em.private_note",
               "fields": {"person_id": "p1", "body": "x"}}),
    );
    h.ok("tasks.move", json!({"id": id, "parent_id": note["id"]}));
    let e = h.err_as(
        &cal(),
        "ext.patch_fields",
        patch(&id, json!({"scheduled_for": "2026-11-12T10:00:00Z"})),
    );
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    assert_eq!(
        h.item(&id)["fields"]["scheduled_for"],
        "2026-11-11T14:00:00Z"
    );
}

#[test]
fn upsert_items_may_narrow_but_never_widen_roles() {
    let h = h();
    h.connect_cal(json!(["developer", "eng-manager"]));
    let item = |eid: &str, t: &str, roles: Value| {
        json!({"external_id": eid, "title": eid, "kind": "bullet", "type_id": t,
               "fields": {"starts_at": "2026-11-11T14:00:00Z", "ends_at": "2026-11-11T15:00:00Z"},
               "roles": roles})
    };
    let r = h.ok_as(
        &cal(),
        "ext.upsert",
        json!({"scope": "m", "items": [
            item("a", "dev.meeting", json!(["developer"])),
            item("b", "em.meeting", json!(["eng-manager", "eng-manager"])),
            item("c", "em.meeting", json!(["cso"])),
            item("d", "em.meeting", json!([])),
            {"external_id": "e", "title": "e", "kind": "bullet"},
        ]}),
    );
    assert_eq!(r["created"], 3, "{r}");
    let rejected: Vec<&str> = r["rejected"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["external_id"].as_str().unwrap())
        .collect();
    assert_eq!(rejected, ["c", "d"]);
    let roles = |eid: &str| {
        let id = r["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["external_id"] == eid)
            .unwrap()["item_id"]
            .as_str()
            .unwrap()
            .to_string();
        h.item(&id)["roles"].clone()
    };
    assert_eq!(roles("a"), json!(["developer"]));
    assert_eq!(roles("b"), json!(["eng-manager"]));
    assert_eq!(roles("e"), json!(["developer", "eng-manager"]));
    // The developer view has the dev copy only.
    let dev = h.ok(
        "tasks.list",
        json!({"role": "developer", "type_id": "em.meeting"}),
    );
    assert_eq!(dev["items"].as_array().unwrap().len(), 0);
}
