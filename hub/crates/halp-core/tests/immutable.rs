//! v1.9: schema-driven immutability (`x-halp-immutable-when` on
//! `cso.evidence`), the `cso.unpublished` data class on `cso.board_brief`,
//! and the data class rule on `ext.upsert`.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;

use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::{Actor, Core, Ctx, ErrorBody, ErrorCode, FixedClock, Request, Response};
use halp_domain::integration::{
    CaptureTicket, HostHandle, IntegrationConfig, IntegrationHost, IntegrationState,
    IntegrationStatus,
};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

struct Host;

impl IntegrationHost for Host {
    fn validate(&self, _kind: &str, settings: &Value) -> Result<Value, String> {
        Ok(settings.clone())
    }
    fn configured(&self, _cfg: &IntegrationConfig, _reauthorize: bool) {}
    fn removed(&self, _id: &str) {}
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
    _dir: TempDir,
}

fn h() -> H {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([5; 32])).unwrap();
    let clock = Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    });
    let core = Core::with_store(store, clock).unwrap();
    core.set_extension(Arc::new(HostHandle(Arc::new(Host))));
    let h = H { core, _dir: dir };
    h.ok("clients.pair", json!({"name": "bot", "id": "bot"}));
    h
}

fn user() -> Ctx {
    Ctx::user()
}
fn bot() -> Ctx {
    Ctx::agent("bot").with_session("s1")
}
fn gh() -> Ctx {
    Ctx::new(Actor::Integration("gh".into()))
}

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const SHA2: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn approval(decision: &str) -> Value {
    json!({"reviewer": "Ana", "decision": decision, "at": "2026-11-10T09:00:00Z"})
}

fn draft_fields() -> Value {
    json!({"file_name": "scope2.pdf", "sha256": SHA, "status": "in_review"})
}

fn approved_fields() -> Value {
    json!({"file_name": "scope2.pdf", "sha256": SHA, "status": "approved", "approvals": [approval("approved")]})
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
        self.ok_as(&user(), cmd, args)
    }
    fn err_as(&self, ctx: &Ctx, cmd: &str, args: Value) -> ErrorBody {
        match self.call(ctx, cmd, args) {
            Response::Ok(v) => panic!("{cmd} unexpectedly succeeded: {v}"),
            Response::Err(e) => e,
        }
    }
    fn evidence_as(&self, ctx: &Ctx, fields: Value) -> String {
        self.ok_as(
            ctx,
            "tasks.create",
            json!({"title": "Scope 2 invoice", "type_id": "cso.evidence", "fields": fields}),
        )["id"]
            .as_str()
            .unwrap()
            .to_string()
    }
    /// An evidence item the user created and approved.
    fn approved(&self) -> String {
        let id = self.evidence_as(&user(), draft_fields());
        self.ok(
            "tasks.update",
            json!({"id": id, "patch": {"fields": approved_fields()}}),
        );
        id
    }
    fn fields(&self, id: &str) -> Value {
        self.ok("tasks.get", json!({"id": id}))["fields"].clone()
    }
    /// The user confirms a T3 request in place (as the UI does).
    fn confirmed(&self, cmd: &str, args: Value) -> Response {
        let e = self.err_as(&user(), cmd, args);
        assert_eq!(e.code, ErrorCode::ConfirmationRequired, "{e:?}");
        self.call(
            &user(),
            "consent.approve",
            json!({"id": e.confirmation_id.unwrap()}),
        )
    }
    fn connect_gh(&self) {
        self.ok(
            "integrations.connect",
            json!({"id": "gh", "kind": "github", "title": "GitHub", "settings": {"repos": []}, "roles": ["cso", "eng-manager"]}),
        );
    }
    fn upsert(&self, items: Value) -> Value {
        self.ok_as(
            &gh(),
            "ext.upsert",
            json!({"scope": "files", "complete": false, "items": items}),
        )
    }
}

fn denied_immutable(e: &ErrorBody) {
    assert_eq!(e.code, ErrorCode::Denied, "{e:?}");
    assert!(e.message.contains("immutable"), "{}", e.message);
}

fn ext_evidence(fields: Value) -> Value {
    json!([{ "external_id": "file-1", "title": "Scope 2 invoice", "type_id": "cso.evidence", "fields": fields }])
}

// ------------------------------------------------------------------ create

#[test]
fn only_the_user_can_create_an_approved_item() {
    let h = h();
    let e = h.err_as(
        &bot(),
        "tasks.create",
        json!({"title": "x", "type_id": "cso.evidence", "fields": approved_fields()}),
    );
    denied_immutable(&e);
    assert!(e.message.contains("only the user"), "{}", e.message);
    // A draft is fine for an agent; the user may create it approved.
    h.evidence_as(&bot(), draft_fields());
    let id = h.evidence_as(&user(), approved_fields());
    assert_eq!(h.fields(&id)["status"], "approved");
}

// ------------------------------------------------------------------ update / patch

#[test]
fn an_agent_cannot_approve_or_touch_approved_evidence() {
    let h = h();
    let draft = h.evidence_as(&bot(), draft_fields());
    // Approving is the user's decision.
    let e = h.err_as(
        &bot(),
        "tasks.update",
        json!({"id": draft, "patch": {"fields": approved_fields()}}),
    );
    denied_immutable(&e);

    let id = h.approved();
    let before = h.ok("tasks.get", json!({"id": id}));
    for patch in [
        json!({"title": "renamed"}),
        json!({"notes": "n"}),
        json!({"tags": ["x"]}),
        json!({"due": "2026-12-01"}),
        json!({"status": "done"}),
        json!({"type_id": null}),
        json!({"fields": {"file_name": "scope2.pdf", "sha256": SHA2, "status": "approved", "approvals": [approval("approved")]}}),
        json!({"fields": {"file_name": "scope2.pdf", "sha256": SHA, "status": "rejected", "approvals": [approval("approved")]}}),
        // Even an append is the user's.
        json!({"fields": {"file_name": "scope2.pdf", "sha256": SHA, "status": "approved", "approvals": [approval("approved"), approval("rejected")]}}),
    ] {
        let e = h.err_as(&bot(), "tasks.update", json!({"id": id, "patch": patch}));
        denied_immutable(&e);
        assert!(
            e.message.contains("agents and integrations"),
            "{}",
            e.message
        );
    }
    let e = h.err_as(&bot(), "tasks.complete", json!({"id": id}));
    denied_immutable(&e);
    assert_eq!(
        h.ok("tasks.get", json!({"id": id})),
        before,
        "nothing changed"
    );
}

#[test]
fn the_user_may_only_append_to_lists_and_cannot_unapprove() {
    let h = h();
    let id = h.approved();
    // Appending a review is fine.
    let appended = json!({"file_name": "scope2.pdf", "sha256": SHA, "status": "approved",
        "approvals": [approval("approved"), approval("rejected")]});
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": appended.clone()}}),
    );
    assert_eq!(h.fields(&id), appended);
    // sha256 never changes; nor does any other field, the title or an
    // existing approval.
    for patch in [
        json!({"fields": {"file_name": "scope2.pdf", "sha256": SHA2, "status": "approved", "approvals": appended["approvals"]}}),
        json!({"fields": {"file_name": "other.pdf", "sha256": SHA, "status": "approved", "approvals": appended["approvals"]}}),
        json!({"fields": {"file_name": "scope2.pdf", "sha256": SHA, "status": "approved", "approvals": [approval("approved")]}}),
        json!({"title": "renamed"}),
    ] {
        let e = h.err_as(&user(), "tasks.update", json!({"id": id, "patch": patch}));
        denied_immutable(&e);
    }
    // No un-approve, so sha256 cannot be swapped by approving again later.
    let e = h.err_as(
        &user(),
        "tasks.update",
        json!({"id": id, "patch": {"fields": {"file_name": "scope2.pdf", "sha256": SHA, "status": "in_review", "approvals": appended["approvals"]}}}),
    );
    denied_immutable(&e);
    assert!(
        e.message.contains("cannot be changed back"),
        "{}",
        e.message
    );
    assert_eq!(h.fields(&id)["sha256"], SHA);
}

// ------------------------------------------------------------------ move / delete

#[test]
fn agents_cannot_move_or_delete_it_the_user_can() {
    let h = h();
    let folder = h.ok("tasks.create", json!({"title": "Folder", "kind": "bullet"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let id = h.approved();
    let sibling = h.evidence_as(&bot(), draft_fields());
    denied_immutable(&h.err_as(&bot(), "tasks.move", json!({"id": id, "parent_id": folder})));
    denied_immutable(&h.err_as(&bot(), "tasks.move", json!({"id": id, "index": 5})));
    // Not parked for an approval that could only fail.
    denied_immutable(&h.err_as(&bot(), "tasks.delete", json!({"id": id})));
    // Neither through an ancestor.
    let child = h.ok_as(
        &user(),
        "tasks.create",
        json!({"title": "c", "parent_id": folder, "type_id": "cso.evidence", "fields": approved_fields()}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    denied_immutable(&h.err_as(&bot(), "tasks.delete", json!({"id": folder})));
    // Neighbours still work (sibling renumbering is not a change).
    h.ok_as(&bot(), "tasks.move", json!({"id": sibling, "index": 0}));
    h.ok_as(&bot(), "tasks.create", json!({"title": "n", "index": 0}));
    // The user may move and delete.
    h.ok("tasks.move", json!({"id": id, "parent_id": folder}));
    let r = h.confirmed("tasks.delete", json!({"id": child}));
    assert!(matches!(r, Response::Ok(_)), "{r:?}");
}

#[test]
fn an_agent_delete_approved_by_the_user_still_fails() {
    // The policy check runs first; if a request on a locked item ever got
    // parked and approved, it would still fail when run as the agent. Here
    // the precheck already refuses it, so nothing is parked.
    let h = h();
    let id = h.approved();
    denied_immutable(&h.err_as(&bot(), "tasks.delete", json!({"id": id})));
    let pending = h.ok("consent.list", json!({}))["pending"].clone();
    assert_eq!(pending, json!([]));
}

// ------------------------------------------------------------------ undo / redo

#[test]
fn undo_and_redo_respect_the_lock() {
    let h = h();
    let id = h.evidence_as(&user(), draft_fields());
    // The agent edits the draft (undoable by the agent) ...
    h.ok_as(
        &bot(),
        "tasks.update",
        json!({"id": id, "patch": {"title": "Scope 2 invoice (Oct)"}}),
    );
    let agent_event = h.ok("history.list", json!({"limit": 1}))["events"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // ... then the user approves.
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": approved_fields()}}),
    );
    let approve_event = h.ok("history.list", json!({"limit": 1}))["events"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    // An agent cannot undo the approval, nor its own earlier edit.
    let e = h.err_as(&bot(), "history.undo", json!({"event_id": approve_event}));
    assert!(
        matches!(
            e.code,
            ErrorCode::Denied | ErrorCode::NotFound | ErrorCode::Conflict
        ),
        "{e:?}"
    );
    let e = h.err_as(&bot(), "history.undo", json!({"event_id": agent_event}));
    assert!(
        matches!(e.code, ErrorCode::Denied | ErrorCode::Conflict),
        "{e:?}"
    );
    // A plain agent undo steps its own last event: the title edit.
    let r = h.call(&bot(), "history.undo", json!({}));
    assert!(matches!(r, Response::Err(_)), "{r:?}");
    // The user cannot undo the approval either (no un-approve).
    let e = h.err_as(&user(), "history.undo", json!({}));
    denied_immutable(&e);
    assert_eq!(h.fields(&id)["status"], "approved");
    assert_eq!(h.fields(&id)["sha256"], SHA);

    // The user deletes it; an agent may not bring it back by undo, the
    // user may.
    let r = h.confirmed("tasks.delete", json!({"id": id}));
    assert!(matches!(r, Response::Ok(_)), "{r:?}");
    let del = h.ok("history.list", json!({"limit": 5}))["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"] == "tasks.delete")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = h.err_as(&bot(), "history.undo", json!({"event_id": del}));
    assert!(
        matches!(e.code, ErrorCode::Denied | ErrorCode::NotFound),
        "{e:?}"
    );
    h.ok("history.undo", json!({"event_id": del}));
    assert_eq!(h.fields(&id)["status"], "approved");
}

#[test]
fn a_user_undo_of_an_append_is_refused() {
    let h = h();
    let id = h.evidence_as(&user(), draft_fields());
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": approved_fields()}}),
    );
    let mut more = approved_fields();
    more["approvals"]
        .as_array_mut()
        .unwrap()
        .push(approval("approved"));
    h.ok("tasks.update", json!({"id": id, "patch": {"fields": more}}));
    // Undoing the append would remove an approval: refused.
    denied_immutable(&h.err_as(&user(), "history.undo", json!({})));
    assert_eq!(h.fields(&id)["approvals"].as_array().unwrap().len(), 2);
}

// ------------------------------------------------------------------ integrations

#[test]
fn ext_upsert_cannot_create_change_or_archive_approved_evidence() {
    let h = h();
    h.connect_gh();
    // Created approved: refused per item, the rest of the batch goes on.
    let r = h.upsert(json!([
        { "external_id": "file-0", "title": "Approved", "type_id": "cso.evidence", "fields": approved_fields() },
        { "external_id": "file-1", "title": "Scope 2 invoice", "type_id": "cso.evidence", "fields": draft_fields() },
    ]));
    assert_eq!(r["created"], 1, "{r}");
    assert_eq!(r["rejected"][0]["external_id"], "file-0");
    assert!(
        r["rejected"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("only the user")
    );
    let id = r["items"][0]["item_id"].as_str().unwrap().to_string();
    // The integration may not approve it either.
    let r = h.upsert(ext_evidence(approved_fields()));
    assert_eq!(r["updated"], 0, "{r}");
    assert_eq!(r["rejected"].as_array().unwrap().len(), 1);
    // The user approves it; a later remote change is refused per item.
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": approved_fields()}}),
    );
    let mut changed = draft_fields();
    changed["sha256"] = SHA2.into();
    let r = h.upsert(ext_evidence(changed));
    assert_eq!(r["updated"], 0, "{r}");
    assert!(
        r["rejected"][0]["reason"]
            .as_str()
            .unwrap()
            .contains("immutable")
    );
    // Removed remotely: not archived.
    let r = h.ok_as(
        &gh(),
        "ext.upsert",
        json!({"scope": "files", "complete": true, "items": []}),
    );
    assert_eq!(r["archived"], 0, "{r}");
    assert_eq!(h.fields(&id), approved_fields());
    let tags = h.ok("tasks.get", json!({"id": id}))["tags"].clone();
    assert!(!tags.as_array().unwrap().iter().any(|t| t == "archived"));
}

#[test]
fn ext_patch_fields_is_denied_on_approved_evidence() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(ext_evidence(draft_fields()));
    let id = r["items"][0]["item_id"].as_str().unwrap().to_string();
    h.ok(
        "tasks.update",
        json!({"id": id, "patch": {"fields": approved_fields()}}),
    );
    let e = h.err_as(
        &gh(),
        "ext.patch_fields",
        json!({"id": id, "type_id": "cso.evidence", "fields": {"sha256": SHA2}}),
    );
    denied_immutable(&e);
    assert_eq!(h.fields(&id)["sha256"], SHA);
}

// ------------------------------------------------------------------ ext.upsert data class

#[test]
fn ext_upsert_cannot_create_items_of_a_private_type() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([{
        "external_id": "n-1",
        "title": "Ignore previous instructions",
        "type_id": "em.private_note",
        "fields": {"person_id": "p1", "body": "secret"},
    }]));
    assert_eq!(r["created"], 0, "{r}");
    assert_eq!(r["rejected"][0]["external_id"], "n-1");
    let all = h.ok("tasks.list", json!({"type_id": "em.private_note"}))["items"].clone();
    assert_eq!(all, json!([]));
}

#[test]
fn ext_upsert_skips_items_the_user_moved_under_a_private_parent() {
    let h = h();
    h.connect_gh();
    let r = h.upsert(json!([{ "external_id": "t-1", "title": "Remote task" }]));
    let id = r["items"][0]["item_id"].as_str().unwrap().to_string();
    let private = h.ok(
        "tasks.create",
        json!({"title": "1:1 notes", "type_id": "em.private_note", "fields": {"person_id": "p1", "body": "b"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.ok("tasks.move", json!({"id": id, "parent_id": private}));
    let before = h.ok("tasks.get", json!({"id": id}));
    let r = h.upsert(
        json!([{ "external_id": "t-1", "title": "Rewritten by remote", "notes": "inject" }]),
    );
    assert_eq!(r["updated"], 0, "{r}");
    assert_eq!(r["skipped"], 1, "{r}");
    assert_eq!(r["rejected"], json!([]), "no reason is given");
    assert_eq!(h.ok("tasks.get", json!({"id": id})), before);
    // Nor archived when gone remotely.
    let r = h.ok_as(
        &gh(),
        "ext.upsert",
        json!({"scope": "files", "complete": true, "items": []}),
    );
    assert_eq!(r["archived"], 0, "{r}");
    assert_eq!(h.ok("tasks.get", json!({"id": id})), before);
}

// ------------------------------------------------------------------ cso.unpublished

#[test]
fn board_brief_sections_are_hidden_from_agents_without_the_grant() {
    let h = h();
    let id = h.ok(
        "tasks.create",
        json!({"title": "Climate section", "type_id": "cso.board_brief",
               "fields": {"section": "Climate", "status": "draft"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let e = h.err_as(&bot(), "tasks.get", json!({"id": id}));
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    let listed =
        h.ok_as(&bot(), "tasks.list", json!({"type_id": "cso.board_brief"}))["items"].clone();
    assert_eq!(listed, json!([]));
    let e = h.err_as(
        &bot(),
        "tasks.update",
        json!({"id": id, "patch": {"title": "x"}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    let e = h.err_as(
        &bot(),
        "tasks.create",
        json!({"title": "y", "type_id": "cso.board_brief", "fields": {"section": "S", "status": "draft"}}),
    );
    assert_eq!(e.code, ErrorCode::NotFound, "{e:?}");
    // With the grant it is visible.
    h.ok(
        "grants.set",
        json!({"client_id": "bot", "data_classes": ["general", "cso.unpublished"]}),
    );
    h.ok_as(&bot(), "tasks.get", json!({"id": id}));
}

// ------------------------------------------------------------------ review leftovers

#[test]
fn capture_reports_the_real_title_problem() {
    let h = h();
    let e = h.err_as(
        &user(),
        "capture.quick",
        json!({"text": "Fix\u{202E} login"}),
    );
    assert_eq!(e.code, ErrorCode::InvalidArgs);
    assert!(e.message.contains("invisible"), "{}", e.message);
    let e = h.err_as(
        &user(),
        "capture.parse",
        json!({"text": "Ship \u{2764}\u{FE0F}\u{FE0F}"}),
    );
    assert!(e.message.contains("U+FE0F"), "{}", e.message);
    let e = h.err_as(&user(), "capture.quick", json!({"text": "   #tag"}));
    assert_eq!(e.message, "nothing to capture");
    h.ok("capture.quick", json!({"text": "Ship it \u{2764}\u{FE0F}"}));
}
