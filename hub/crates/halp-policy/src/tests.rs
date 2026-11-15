//! One test per policy rule.
#![allow(clippy::unwrap_used)]

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::secret::token_hash;
use crate::*;

fn client(id: &str, token: &str) -> Client {
    Client {
        id: id.into(),
        name: id.into(),
        token_hash: token_hash(token),
        created_at: "2026-01-01T00:00:00Z".into(),
        revoked_at: None,
        grants: Grants::default(),
    }
}

fn engine(clients: Vec<Client>, rules: Vec<Rule>) -> Engine {
    let e = Engine::default();
    e.load(clients, rules);
    e
}

fn q<'a>(ctx: &'a Ctx, command: &'a str, tier: Tier, args: &'a Value) -> Query<'a> {
    Query {
        ctx,
        command,
        tier,
        data_class: DEFAULT_DATA_CLASS,
        args,
    }
}

fn is_allow(v: &Verdict) -> bool {
    matches!(v, Verdict::Allow { .. })
}
fn is_deny(v: &Verdict) -> bool {
    matches!(v, Verdict::Deny { .. })
}
fn is_confirm(v: &Verdict) -> bool {
    matches!(v, Verdict::Confirm { .. })
}

fn rule(client_id: &str, command: &str, args: Value) -> Rule {
    let args: BTreeMap<String, Value> = serde_json::from_value(args).unwrap();
    Rule {
        id: "r1".into(),
        client_id: client_id.into(),
        command: command.into(),
        args,
        created_at: "2026-01-01T00:00:00Z".into(),
        created_by: "user".into(),
    }
}

const A: &Value = &Value::Null;

#[test]
fn t0_is_allowed_for_a_paired_agent() {
    let e = engine(vec![client("c", "t")], vec![]);
    let ctx = Ctx::agent("c");
    assert!(is_allow(
        &e.check(&q(&ctx, "tasks.list", Tier::T0, A), Instant::now())
    ));
}

#[test]
fn unknown_or_revoked_clients_are_denied() {
    let mut r = client("old", "t2");
    r.revoked_at = Some("2026-01-02T00:00:00Z".into());
    let e = engine(vec![client("c", "t"), r], vec![]);
    let now = Instant::now();
    let ghost = Ctx::agent("ghost");
    assert!(is_deny(
        &e.check(&q(&ghost, "tasks.list", Tier::T0, A), now)
    ));
    let old = Ctx::agent("old");
    assert!(is_deny(&e.check(&q(&old, "tasks.list", Tier::T0, A), now)));
}

#[test]
fn t1_needs_the_t1_grant() {
    let mut c = client("c", "t");
    let e = engine(vec![c.clone()], vec![]);
    let ctx = Ctx::agent("c");
    let now = Instant::now();
    assert!(is_allow(
        &e.check(&q(&ctx, "tasks.create", Tier::T1, A), now)
    ));
    c.grants.t1 = false;
    e.load(vec![c], vec![]);
    assert!(is_confirm(
        &e.check(&q(&ctx, "tasks.create", Tier::T1, A), now)
    ));
}

#[test]
fn t2_confirms_unless_a_narrow_rule_matches() {
    let args = json!({"widget_id": "w1", "role_id": "developer"});
    let now = Instant::now();
    let ctx = Ctx::agent("c");
    let e = engine(vec![client("c", "t")], vec![]);
    assert!(is_confirm(
        &e.check(&q(&ctx, "x.send", Tier::T2, &args), now)
    ));

    let e = engine(
        vec![client("c", "t")],
        vec![rule("c", "x.send", json!({"widget_id": "w1"}))],
    );
    assert!(is_allow(&e.check(&q(&ctx, "x.send", Tier::T2, &args), now)));
    // Another argument value, another command or another client: no match.
    let other = json!({"widget_id": "w2"});
    assert!(is_confirm(
        &e.check(&q(&ctx, "x.send", Tier::T2, &other), now)
    ));
    assert!(is_confirm(
        &e.check(&q(&ctx, "x.post", Tier::T2, &args), now)
    ));
    let e2 = engine(
        vec![client("c", "t"), client("d", "u")],
        vec![rule("d", "x.send", json!({"widget_id": "w1"}))],
    );
    assert!(is_confirm(
        &e2.check(&q(&ctx, "x.send", Tier::T2, &args), now)
    ));
}

#[test]
fn an_empty_rule_matches_nothing() {
    let e = engine(vec![client("c", "t")], vec![rule("c", "x.send", json!({}))]);
    let ctx = Ctx::agent("c");
    assert!(is_confirm(&e.check(
        &q(&ctx, "x.send", Tier::T2, &json!({})),
        Instant::now()
    )));
}

#[test]
fn rules_never_apply_to_t3() {
    let e = engine(
        vec![client("c", "t")],
        vec![rule("c", "tasks.delete", json!({"id": "x"}))],
    );
    let ctx = Ctx::agent("c");
    let args = json!({"id": "x"});
    assert!(is_confirm(&e.check(
        &q(&ctx, "tasks.delete", Tier::T3, &args),
        Instant::now()
    )));
}

#[test]
fn t3_always_confirms_even_for_the_user() {
    let e = engine(vec![client("c", "t")], vec![]);
    let now = Instant::now();
    for ctx in [Ctx::user(), Ctx::system(), Ctx::agent("c")] {
        assert!(is_confirm(
            &e.check(&q(&ctx, "tasks.delete", Tier::T3, A), now)
        ));
    }
    let u = Ctx::user();
    assert!(is_allow(&e.check(&q(&u, "x.send", Tier::T2, A), now)));
}

#[test]
fn data_classes_need_a_grant() {
    let mut c = client("c", "t");
    let e = engine(vec![c.clone()], vec![]);
    let ctx = Ctx::agent("c");
    let now = Instant::now();
    let mut query = q(&ctx, "audit.list", Tier::T0, A);
    query.data_class = "audit";
    assert!(is_deny(&e.check(&query, now)));
    c.grants.data_classes.push("audit".into());
    e.load(vec![c], vec![]);
    assert!(is_allow(&e.check(&query, now)));
    // The user has every class.
    let u = Ctx::user();
    let mut uq = q(&u, "audit.list", Tier::T0, A);
    uq.data_class = "secrets";
    assert!(is_allow(&e.check(&uq, now)));
}

/// Item data classes (`x-halp-data-class`) follow the same grants.
#[test]
fn allows_class_follows_the_client_grants() {
    let mut c = client("c", "t");
    let e = engine(vec![c.clone()], vec![]);
    let ctx = Ctx::agent("c");
    assert!(e.allows_class(&ctx, "general"));
    assert!(!e.allows_class(&ctx, "notes.private"));
    assert!(!e.allows_class(&ctx, "any.custom-class"));
    assert!(!e.allows_class(&Ctx::agent("stranger"), "general"));
    assert!(e.allows_class(&Ctx::user(), "notes.private"));
    c.grants.data_classes.push("notes.private".into());
    e.load(vec![c.clone()], vec![]);
    assert!(e.allows_class(&ctx, "notes.private"));
    c.revoked_at = Some("2026-01-02T00:00:00Z".into());
    e.load(vec![c], vec![]);
    assert!(!e.allows_class(&ctx, "notes.private"));
}

#[test]
fn taint_forces_confirmation_of_t2_and_is_reported() {
    let e = engine(
        vec![client("c", "t")],
        vec![rule("c", "x.send", json!({"to": "me"}))],
    );
    let ctx = Ctx::agent("c").with_session("s1");
    let args = json!({"to": "me"});
    let now = Instant::now();
    assert!(is_allow(&e.check(&q(&ctx, "x.send", Tier::T2, &args), now)));
    // Its own and the user's content do not taint.
    e.note_read(&ctx, "user");
    e.note_read(&ctx, "agent:c");
    assert_eq!(e.taint(&ctx), None);
    e.note_read(&ctx, "integration:gmail");
    e.note_read(&ctx, "agent:other");
    assert_eq!(e.taint(&ctx).as_deref(), Some("integration:gmail"));
    match e.check(&q(&ctx, "x.send", Tier::T2, &args), now) {
        Verdict::Confirm { tainted, .. } => {
            assert_eq!(tainted.as_deref(), Some("integration:gmail"));
        }
        other => panic!("expected confirm, got {other:?}"),
    }
    // T3 confirmations carry the taint too; another session is clean.
    match e.check(&q(&ctx, "tasks.delete", Tier::T3, A), now) {
        Verdict::Confirm { tainted, .. } => assert!(tainted.is_some()),
        other => panic!("expected confirm, got {other:?}"),
    }
    // Another session of the same client is the same client: still tainted.
    let other = Ctx::agent("c").with_session("s2");
    assert_eq!(e.taint(&other).as_deref(), Some("integration:gmail"));
    assert_eq!(e.taint(&Ctx::agent("d")), None);
    // The user is never tainted.
    e.note_read(&Ctx::user(), "integration:gmail");
    assert_eq!(e.taint(&Ctx::user()), None);
    e.clear_taint(&ctx);
    assert_eq!(e.taint(&ctx), None);
}

#[test]
fn taint_decays_after_the_last_tainted_read() {
    let e = Engine::new(Limits {
        taint_ttl: Duration::from_secs(60),
        ..Limits::default()
    });
    let ctx = Ctx::agent("c").with_session("s1");
    let t0 = Instant::now();
    e.note_read_at(&ctx, "integration:gmail", t0);
    assert!(e.taint_at(&ctx, t0 + Duration::from_secs(59)).is_some());
    // Another tainted read restarts the clock.
    e.note_read_at(&ctx, "agent:other", t0 + Duration::from_secs(50));
    assert_eq!(
        e.taint_at(&ctx, t0 + Duration::from_secs(100)).as_deref(),
        Some("integration:gmail")
    );
    assert_eq!(e.taint_at(&ctx, t0 + Duration::from_secs(111)), None);
    // A clean read does not taint it again.
    e.note_read_at(&ctx, "user", t0 + Duration::from_secs(120));
    assert_eq!(e.taint_at(&ctx, t0 + Duration::from_secs(120)), None);
    assert_eq!(Limits::default().taint_ttl, Duration::from_secs(30 * 60));
}

#[test]
fn calls_are_rate_limited_per_client() {
    let e = Engine::new(Limits {
        calls: (3, 1.0),
        ..Limits::default()
    });
    e.load(vec![client("c", "t"), client("d", "u")], vec![]);
    let ctx = Ctx::agent("c");
    let now = Instant::now();
    for _ in 0..3 {
        assert!(is_allow(&e.check(&q(&ctx, "tasks.list", Tier::T0, A), now)));
    }
    assert!(is_deny(&e.check(&q(&ctx, "tasks.list", Tier::T0, A), now)));
    // Another client has its own bucket; tokens come back with time.
    let d = Ctx::agent("d");
    assert!(is_allow(&e.check(&q(&d, "tasks.list", Tier::T0, A), now)));
    let later = now + Duration::from_secs(2);
    assert!(is_allow(
        &e.check(&q(&ctx, "tasks.list", Tier::T0, A), later)
    ));
}

#[test]
fn writes_have_their_own_tighter_bucket() {
    let e = Engine::new(Limits {
        writes: (2, 0.0),
        ..Limits::default()
    });
    e.load(vec![client("c", "t")], vec![]);
    let ctx = Ctx::agent("c");
    let now = Instant::now();
    assert!(is_allow(
        &e.check(&q(&ctx, "tasks.create", Tier::T1, A), now)
    ));
    assert!(is_allow(
        &e.check(&q(&ctx, "tasks.create", Tier::T1, A), now)
    ));
    assert!(is_deny(
        &e.check(&q(&ctx, "history.undo", Tier::T1, A), now)
    ));
    assert!(is_allow(&e.check(&q(&ctx, "tasks.list", Tier::T0, A), now)));
}

#[test]
fn parking_is_rate_limited() {
    let e = Engine::new(Limits {
        parks: (2, 0.0),
        ..Limits::default()
    });
    e.load(vec![client("c", "t")], vec![]);
    let ctx = Ctx::agent("c");
    let now = Instant::now();
    assert!(e.may_park(&ctx, now).is_ok());
    assert!(e.may_park(&ctx, now).is_ok());
    assert!(e.may_park(&ctx, now).is_err());
    assert!(e.may_park(&Ctx::user(), now).is_ok());
}

#[test]
fn circuit_breaker_trips_on_bulk_destructive_requests() {
    let e = Engine::new(Limits {
        breaker_threshold: 3,
        ..Limits::default()
    });
    e.load(vec![client("c", "t")], vec![]);
    let ctx = Ctx::agent("c");
    let now = Instant::now();
    for _ in 0..3 {
        assert!(is_confirm(
            &e.check(&q(&ctx, "tasks.delete", Tier::T3, A), now)
        ));
    }
    assert!(is_deny(
        &e.check(&q(&ctx, "tasks.delete", Tier::T3, A), now)
    ));
    assert!(e.breaker_open("c", now));
    // Reads and T1 still work; T2/T3 are refused until the cooldown ends.
    assert!(is_allow(&e.check(&q(&ctx, "tasks.list", Tier::T0, A), now)));
    assert!(is_deny(&e.check(&q(&ctx, "x.send", Tier::T2, A), now)));
    let later = now + Duration::from_secs(301);
    assert!(!e.breaker_open("c", later));
    assert!(is_confirm(
        &e.check(&q(&ctx, "tasks.delete", Tier::T3, A), later)
    ));
}

#[test]
fn breaker_window_forgets_old_requests() {
    let e = Engine::new(Limits {
        breaker_threshold: 2,
        ..Limits::default()
    });
    e.load(vec![client("c", "t")], vec![]);
    let ctx = Ctx::agent("c");
    let t0 = Instant::now();
    for i in 0..6u64 {
        let t = t0 + Duration::from_secs(i * 40);
        assert!(is_confirm(
            &e.check(&q(&ctx, "tasks.delete", Tier::T3, A), t)
        ));
    }
}

#[test]
fn tokens_authenticate_only_active_clients() {
    let mut r = client("old", "tok-old");
    r.revoked_at = Some("x".into());
    let e = engine(vec![client("c", "tok-c"), r], vec![]);
    assert_eq!(e.authenticate("tok-c").as_deref(), Some("c"));
    assert_eq!(e.authenticate("tok-old"), None);
    assert_eq!(e.authenticate("nope"), None);
    assert_eq!(e.authenticate(""), None);
}

#[test]
fn modules_get_general_reads_and_are_denied_everything_else() {
    let e = Engine::default();
    let now = Instant::now();
    let m = Ctx::new(Actor::Module("acme.sre".into()));
    assert!(is_allow(&e.check(&q(&m, "tasks.list", Tier::T0, A), now)));
    // Every write tier is denied outright: never confirmed, so a module
    // cannot raise a consent prompt.
    for (cmd, tier) in [
        ("tasks.create", Tier::T1),
        ("tasks.update", Tier::T1),
        ("history.undo", Tier::T2),
        ("tasks.delete", Tier::T3),
    ] {
        let v = e.check(&q(&m, cmd, tier, A), now);
        assert!(is_deny(&v), "{cmd}: {v:?}");
    }
    // A read of any class but `general` is denied too.
    for class in ["secrets", "notes.private", "email.body", "acme.private"] {
        let mut sq = q(&m, "tasks.list", Tier::T0, A);
        sq.data_class = class;
        assert!(is_deny(&e.check(&sq, now)), "{class}");
        assert!(!e.allows_class(&m, class), "{class}");
    }
    assert!(e.allows_class(&m, crate::DEFAULT_DATA_CLASS));
    // Even a tainted module session is denied, not confirmed.
    e.note_read_at(&m, "agent:bot", now);
    assert!(is_deny(&e.check(&q(&m, "tasks.create", Tier::T1, A), now)));
    // And it may never park a request, however few it has asked.
    assert!(e.may_park(&m, now).is_err());
}

#[test]
fn integrations_get_reads_and_confirm_the_rest() {
    let e = Engine::default();
    let now = Instant::now();
    let i = Ctx::new(Actor::Integration("gh".into()));
    assert!(is_allow(&e.check(&q(&i, "tasks.list", Tier::T0, A), now)));
    assert!(is_confirm(
        &e.check(&q(&i, "tasks.create", Tier::T1, A), now)
    ));
    let mut sq = q(&i, "x", Tier::T0, A);
    sq.data_class = "secrets";
    assert!(is_confirm(&e.check(&sq, now)));
}

#[test]
fn integrations_may_write_only_through_their_sync_commands() {
    let e = Engine::default();
    let now = Instant::now();
    let i = Ctx::new(Actor::Integration("gh".into()));
    assert!(is_allow(&e.check(&q(&i, "ext.upsert", Tier::T1, A), now)));
    assert!(is_allow(&e.check(&q(&i, "ext.status", Tier::T1, A), now)));
    assert!(is_allow(
        &e.check(&q(&i, "ext.patch_fields", Tier::T1, A), now)
    ));
    let mut pq = q(&i, "ext.patch_fields", Tier::T1, A);
    pq.data_class = "notes.private";
    assert!(is_confirm(&e.check(&pq, now)));
    // Any other write still needs the user.
    assert!(is_confirm(
        &e.check(&q(&i, "tasks.create", Tier::T1, A), now)
    ));
    assert!(is_confirm(
        &e.check(&q(&i, "tasks.delete", Tier::T3, A), now)
    ));
    // The same names at a higher tier or another data class are not covered.
    assert!(is_confirm(&e.check(&q(&i, "ext.upsert", Tier::T2, A), now)));
    let mut sq = q(&i, "ext.upsert", Tier::T1, A);
    sq.data_class = "secrets";
    assert!(is_confirm(&e.check(&sq, now)));
    // A module calling them gets nothing special (denied, as any write).
    let m = Ctx::new(Actor::Module("m".into()));
    assert!(is_deny(&e.check(&q(&m, "ext.upsert", Tier::T1, A), now)));
}

#[test]
fn integration_writes_are_rate_limited() {
    let e = Engine::default();
    let now = Instant::now();
    let i = Ctx::new(Actor::Integration("gh".into()));
    let mut allowed = 0;
    for _ in 0..200 {
        if is_allow(&e.check(&q(&i, "ext.upsert", Tier::T1, A), now)) {
            allowed += 1;
        }
    }
    assert!((1..200).contains(&allowed), "{allowed}");
    // Another integration has its own bucket.
    let j = Ctx::new(Actor::Integration("linear".into()));
    assert!(is_allow(&e.check(&q(&j, "ext.upsert", Tier::T1, A), now)));
}

#[test]
fn actor_strings_round_trip_and_reject_junk() {
    for s in [
        "user",
        "system",
        "agent:claude-code",
        "module:m.1",
        "integration:gmail",
    ] {
        assert_eq!(Actor::parse(s).unwrap().to_string(), s);
    }
    for s in ["", "agent:", "agent:a b", "root", "agent:../x", "user:x"] {
        assert!(Actor::parse(s).is_none(), "{s}");
    }
}

#[test]
fn bucket_refills_but_never_beyond_capacity() {
    let t = Instant::now();
    let mut b = Bucket::new(2, 1.0, t);
    assert!(b.take(t) && b.take(t) && !b.take(t));
    let later = t + Duration::from_secs(100);
    assert!(b.take(later) && b.take(later) && !b.take(later));
    // A clock going backwards does not mint tokens.
    assert!(!b.take(t));
}

#[test]
fn activity_reports_last_call_and_taint_expiry() {
    let e = Engine::new(Limits {
        taint_ttl: Duration::from_secs(60),
        ..Limits::default()
    });
    e.load(vec![client("c", "t")], vec![]);
    let t0 = Instant::now();
    assert_eq!(e.activity("c", t0), Activity::default());
    let ctx = Ctx::agent("c").with_session("s1");
    let args = json!({});
    e.check(&q(&ctx, "tasks.list", Tier::T0, &args), t0);
    e.note_read_at(&ctx, "integration:gmail", t0);
    let a = e.activity("c", t0 + Duration::from_secs(10));
    assert_eq!(a.idle, Some(Duration::from_secs(10)));
    assert_eq!(a.tainted_for, Some(Duration::from_secs(50)));
    let a = e.activity("c", t0 + Duration::from_secs(61));
    assert_eq!(a.tainted_for, None);
    // Another client's calls are not this one's.
    assert_eq!(e.activity("d", t0), Activity::default());
}
