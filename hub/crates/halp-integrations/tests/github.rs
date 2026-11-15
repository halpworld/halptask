//! The GitHub connector end to end: wiremock fixtures → runtime → core.
#![allow(clippy::unwrap_used)]

mod common;

use serde_json::{Value, json};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "ghp_TESTTOKEN0123456789abcdefghijABCDEF";

fn fixture(name: &str) -> Value {
    let p = format!(
        "{}/tests/fixtures/github/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap()
}

async fn github() -> MockServer {
    let s = MockServer::start().await;
    mount(&s, "search_author.json").await;
    s
}

async fn mount(s: &MockServer, author: &str) {
    let auth = format!("Bearer {TOKEN}");
    let q = |w: &str| format!("is:pr is:open archived:false {w}");
    for (qual, file) in [
        ("review-requested:@me", "search_review.json"),
        ("author:@me", author),
    ] {
        Mock::given(method("GET"))
            .and(path("/search/issues"))
            .and(query_param("q", q(qual).as_str()))
            .and(header("authorization", auth.as_str()))
            .and(header("user-agent", "HalpTaskHub/0.1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(file)))
            .mount(s)
            .await;
    }
    // pulls/12 supports conditional requests.
    Mock::given(method("GET"))
        .and(path("/repos/acme/api/pulls/12"))
        .and(header("if-none-match", "\"p12\""))
        .respond_with(ResponseTemplate::new(304))
        .with_priority(1)
        .expect(1..)
        .mount(s)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/api/pulls/12"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"p12\"")
                .set_body_json(fixture("pull_12.json")),
        )
        .with_priority(5)
        .mount(s)
        .await;
    for (p, file) in [
        ("/repos/acme/api/pulls/12/reviews", "reviews_12.json"),
        ("/repos/acme/api/actions/runs", "runs_12.json"),
        ("/repos/acme/web/pulls/7", "pull_7.json"),
        ("/repos/acme/web/pulls/7/reviews", "reviews_7.json"),
        ("/repos/acme/web/actions/runs", "runs_7.json"),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .and(header("authorization", auth.as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(fixture(file)))
            .mount(s)
            .await;
    }
}

fn connect(h: &common::H, api: &str) {
    h.ok(
        "integrations.connect",
        json!({"id": "gh", "kind": "github", "title": "GitHub", "interval_secs": 3600,
               "settings": {"api_url": format!("{api}/")}}),
    );
}

fn by_ext<'a>(items: &'a [Value], h: &common::H, key: &str) -> &'a Value {
    let _ = h;
    items
        .iter()
        .find(|i| i["title"].as_str().is_some_and(|t| t.contains(key)))
        .unwrap_or_else(|| panic!("no item {key} in {items:#?}"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prs_and_ci_runs_are_mapped_and_resyncs_are_idempotent() {
    let gh = github().await;
    let h = common::start();
    connect(&h, &gh.uri());
    h.until("needs_auth without a token", |h| {
        h.state("gh") == "needs_auth"
    })
    .await;
    let mut events = h.core.subscribe();

    h.rt.set_token("gh", TOKEN).unwrap();
    h.until("first sync", |h| h.state("gh") == "ok").await;

    let items = h.items("gh");
    let pr12 = by_ext(&items, &h, "acme/api#12");
    assert_eq!(pr12["kind"], "task");
    assert_eq!(pr12["type_id"], "dev.pull_request");
    let f = &pr12["fields"];
    assert_eq!(f["repo"], "acme/api");
    assert_eq!(f["number"], 12);
    assert_eq!(f["state"], "review_requested");
    assert_eq!(f["requested_from_me"], true);
    assert_eq!(f["authored_by_me"], false);
    assert_eq!(
        f["ci"], "pending",
        "the latest CI run is still running: {f}"
    );
    assert_eq!(f["branch"], "retries");
    assert_eq!(f["mergeable"], "blocked");
    assert_eq!(f["reviewers"], json!(["me"]));

    let pr7 = by_ext(&items, &h, "acme/web#7");
    assert_eq!(pr7["fields"]["state"], "approved");
    assert_eq!(pr7["fields"]["authored_by_me"], true);
    assert_eq!(pr7["fields"]["ci"], "unknown");
    let t7 = pr7["title"].as_str().unwrap();
    assert!(!t7.contains('\u{1b}') && !t7.contains('\u{202e}'), "{t7:?}");

    assert!(
        items
            .iter()
            .all(|i| !i["title"].as_str().unwrap().contains("not a PR"))
    );
    let ci = by_ext(&items, &h, "CI on retries");
    assert_eq!(ci["kind"], "bullet");
    assert_eq!(ci["type_id"], "dev.ci_run");
    assert_eq!(ci["fields"]["status"], "running");
    assert_eq!(ci["fields"]["pr_number"], 12);
    assert!(ci["fields"].get("finished_at").is_none());
    assert_eq!(
        by_ext(&items, &h, "Lint on retries")["fields"]["status"],
        "passing"
    );

    // Same remote state again: no new history events from the upsert.
    let before = h.ok("history.list", json!({"limit": 1000}))["events"]
        .as_array()
        .unwrap()
        .len();
    let last = h.entry("gh")["status"]["last_sync"].clone();
    h.ok("integrations.sync_now", json!({"id": "gh"}));
    h.until("second sync", |h| {
        h.entry("gh")["status"]["last_sync"] != last
    })
    .await;
    let after = h.ok("history.list", json!({"limit": 1000}))["events"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(before, after, "an unchanged resync wrote events");
    assert_eq!(h.items("gh").len(), items.len());
    assert_eq!(h.entry("gh")["status"]["items"], json!(4));

    // PR 7 merged: gone from the search, so it is archived (not deleted).
    let conditional = gh
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| {
            r.url.path() == "/repos/acme/api/pulls/12" && r.headers.get("if-none-match").is_some()
        })
        .count();
    assert!(conditional >= 1, "the resync did not send If-None-Match");
    gh.reset().await;
    mount(&gh, "search_empty.json").await;
    let last = h.entry("gh")["status"]["last_sync"].clone();
    h.ok("integrations.sync_now", json!({"id": "gh"}));
    h.until("third sync", |h| {
        h.entry("gh")["status"]["last_sync"] != last
    })
    .await;
    let items = h.items("gh");
    let pr7 = by_ext(&items, &h, "acme/web#7");
    assert!(
        pr7["tags"].as_array().unwrap().contains(&json!("archived")),
        "{pr7}"
    );
    assert_eq!(pr7["status"], "done");
    let pr12 = by_ext(&items, &h, "acme/api#12");
    assert!(
        !pr12["tags"]
            .as_array()
            .unwrap()
            .contains(&json!("archived"))
    );

    // The token appears nowhere the core keeps or broadcasts.
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        seen.push(serde_json::to_string(&e).unwrap());
    }
    assert!(!seen.is_empty());
    for (what, text) in [
        ("events", seen.join("\n")),
        (
            "history",
            h.ok("history.list", json!({"limit": 1000})).to_string(),
        ),
        (
            "audit",
            h.ok("audit.list", json!({"limit": 1000})).to_string(),
        ),
        ("list", h.ok("integrations.list", json!({})).to_string()),
        ("items", json!(h.items("gh")).to_string()),
    ] {
        assert!(!text.contains(TOKEN), "token leaked into {what}");
        assert!(!text.contains("TESTTOKEN"), "token leaked into {what}");
    }
    h.rt.shutdown(std::time::Duration::from_secs(2)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rejected_token_needs_auth_and_is_not_echoed() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_json(json!({"message": format!("Bad credentials {TOKEN}")})),
        )
        .mount(&s)
        .await;
    let h = common::start();
    connect(&h, &s.uri());
    h.rt.set_token("gh", TOKEN).unwrap();
    h.until("needs_auth", |h| {
        h.state("gh") == "needs_auth" && h.entry("gh")["status"]["has_secret"] == true
    })
    .await;
    let all = format!(
        "{}{}{}",
        h.ok("integrations.list", json!({})),
        h.ok("audit.list", json!({"limit": 1000})),
        h.ok("history.list", json!({"limit": 1000}))
    );
    assert!(!all.contains("TESTTOKEN"));
    assert!(all.contains("needs_auth"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rate_limits_back_off_and_token_input_is_checked() {
    let s = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/search/issues"))
        .respond_with(
            ResponseTemplate::new(403)
                .insert_header("x-ratelimit-remaining", "0")
                .insert_header("x-ratelimit-reset", "9999999999"),
        )
        .mount(&s)
        .await;
    let h = common::start();
    connect(&h, &s.uri());
    h.rt.set_token("gh", TOKEN).unwrap();
    h.until("error", |h| h.state("gh") == "error").await;
    let e = h.entry("gh");
    assert!(
        e["status"]["error"]
            .as_str()
            .unwrap()
            .contains("rate limit")
    );
    assert!(!e["status"]["next_sync"].is_null());

    // Token setting checks.
    assert_eq!(h.rt.set_token("nope", TOKEN), Err("unknown_integration"));
    assert_eq!(h.rt.set_token("gh", ""), Err("empty"));
    assert_eq!(h.rt.set_token("gh", "a\nb"), Err("not_printable_ascii"));
    assert_eq!(h.rt.set_token("gh", &"x".repeat(5000)), Err("too_long"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn settings_are_validated_by_the_runtime() {
    let h = common::start();
    let bad = |settings: Value| match h.call(
        "integrations.connect",
        json!({"id": "x", "kind": "github", "settings": settings}),
    ) {
        halp_core::Response::Err(e) => e.message,
        halp_core::Response::Ok(v) => panic!("accepted {v}"),
    };
    assert!(bad(json!({"repos": ["nope"]})).contains("owner/name"));
    assert!(bad(json!({"api_url": "http://example.com/"})).contains("https"));
    assert!(bad(json!({"token": "ghp_x"})).contains("unknown field"));
    assert!(bad(json!({"auth": "oauth"})).contains("oauth"));
}

/// GitHub lists reviews oldest first: on a PR with several pages the
/// newest decision is on the last page and must win.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reviews_are_read_to_the_last_page() {
    let s = MockServer::start().await;
    let reviews = "/repos/acme/web/pulls/7/reviews";
    let last = format!("<{}{reviews}?per_page=100&page=3>; rel=\"last\"", s.uri());
    for (page, body) in [
        (
            "1",
            json!([{"user": {"login": "dan"}, "state": "APPROVED"}]),
        ),
        (
            "2",
            json!([{"user": {"login": "dan"}, "state": "COMMENTED"}]),
        ),
        (
            "3",
            json!([{"user": {"login": "dan"}, "state": "CHANGES_REQUESTED"}]),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(reviews))
            .and(query_param("page", page))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("link", last.as_str())
                    .set_body_json(body),
            )
            .with_priority(1)
            .expect(1..)
            .mount(&s)
            .await;
    }
    mount(&s, "search_author.json").await;
    let h = common::start();
    connect(&h, &s.uri());
    h.rt.set_token("gh", TOKEN).unwrap();
    h.until("first sync", |h| h.state("gh") == "ok").await;
    let items = h.items("gh");
    let pr7 = by_ext(&items, &h, "acme/web#7");
    assert_eq!(pr7["fields"]["state"], "changes_requested");
}

/// A token stored for one GitHub host is deleted when the settings point
/// at another one, so it is never sent there; other edits keep it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moving_to_another_host_drops_the_token() {
    use halp_integrations::SecretStore;
    let h = common::start();
    let a = MockServer::start().await;
    let b = MockServer::start().await;
    connect(&h, &a.uri());
    h.rt.set_token("gh", TOKEN).unwrap();
    assert!(h.entry("gh")["status"]["has_secret"].as_bool().unwrap());
    h.ok(
        "integrations.connect",
        json!({"id": "gh", "kind": "github", "title": "GitHub", "interval_secs": 3600,
               "settings": {"api_url": format!("{}/", a.uri()), "ci": false}}),
    );
    assert!(h.secrets.get("gh").unwrap().is_some(), "same host keeps it");
    connect(&h, &b.uri());
    assert!(h.secrets.get("gh").unwrap().is_none(), "new host drops it");
    assert_eq!(h.entry("gh")["status"]["has_secret"], false);
    assert!(b.received_requests().await.unwrap().iter().all(|r| {
        r.headers
            .get("authorization")
            .is_none_or(|v| !v.to_str().unwrap_or("").contains(TOKEN))
    }));
}
