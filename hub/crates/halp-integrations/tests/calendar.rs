//! The calendar connector: ICS parsing against a fixed clock, then the
//! feed end to end (wiremock → runtime → core).
#![allow(clippy::unwrap_used)]

mod common;

use chrono::{DateTime, Duration, Utc};
use halp_integrations::calendar::{self, Event, ics};
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fixture(name: &str) -> Vec<u8> {
    let p = format!(
        "{}/tests/fixtures/calendar/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read(p).unwrap()
}

fn at(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
}

fn find<'a>(evs: &'a [Event], key: &str) -> Option<&'a Event> {
    evs.iter().find(|e| e.key == key)
}

fn starts(evs: &[Event], uid: &str) -> Vec<String> {
    let mut v: Vec<String> = evs
        .iter()
        .filter(|e| e.key == uid || e.key.starts_with(&format!("{uid}@")))
        .map(|e| e.start.to_rfc3339())
        .collect();
    v.sort();
    v
}

// ------------------------------------------------------------------ parsing

#[test]
fn recurrence_exdate_overrides_and_dst() {
    // Window (Europe/Stockholm): 2026-10-21 .. 2026-11-12. Stockholm leaves
    // summer time on 25 October, New York on 1 November.
    let now = at("2026-10-28T12:00:00Z");
    let p = ics::parse(&fixture("feed.ics"), None, now).unwrap();
    assert!(p.complete);
    let evs = &p.events;

    // Weekly 09:00 Stockholm: 07:00Z before the change, 08:00Z after;
    // 5 November is an EXDATE, 12 November is past the window.
    assert_eq!(
        starts(evs, "sthlm-weekly"),
        ["2026-10-22T07:00:00+00:00", "2026-10-29T08:00:00+00:00"]
    );
    let w = find(evs, "sthlm-weekly@20261029T080000Z").unwrap();
    assert_eq!(w.end - w.start, Duration::minutes(30));
    assert!(w.recurring);
    assert_eq!(w.location.as_deref(), Some("Room 4"));

    // Mondays and Fridays 09:30 New York: 13:30Z (EDT), then 14:30Z (EST).
    // 30 October moved to 11:00 by an override, 6 November cancelled.
    assert_eq!(
        starts(evs, "ny-series"),
        [
            "2026-10-23T13:30:00+00:00",
            "2026-10-26T13:30:00+00:00",
            "2026-10-30T15:00:00+00:00",
            "2026-11-02T14:30:00+00:00",
            "2026-11-09T14:30:00+00:00",
        ]
    );
    let moved = find(evs, "ny-series@20261030T133000Z").unwrap();
    assert_eq!(moved.title, "NY sync (moved)");
    assert_eq!(moved.end - moved.start, Duration::minutes(30));
    assert!(find(evs, "ny-series@20261106T143000Z").is_none());

    // All day, in the calendar's zone, not busy (transparent).
    let d = find(evs, "allday-1").unwrap();
    assert!(d.all_day);
    assert_eq!(d.start, at("2026-10-29T23:00:00Z"));
    assert_eq!(d.end, at("2026-10-30T23:00:00Z"));
    assert!(!d.busy);

    // Cancelled and long-past events are not there.
    assert!(find(evs, "cancelled-1").is_none());
    assert!(find(evs, "old-1").is_none());

    // Private events keep their time, nothing else.
    let pv = find(evs, "private-1").unwrap();
    assert!(pv.private);
    assert_eq!(pv.title, "Private event");
    assert!(pv.location.is_none() && pv.description.is_none() && pv.url.is_none());

    // Escapes, folding, bidi stripped; a Windows zone name resolved.
    let x = find(evs, "escaped-1").unwrap();
    assert_eq!(
        x.title,
        "Plan, review ; and ship the long title that is folded"
    );
    assert!(!x.title.contains('\u{202e}'));
    assert_eq!(x.start, at("2026-10-27T09:00:00Z"));
    assert_eq!(x.description.as_deref(), Some("Line one\nLine two"));
    assert_eq!(x.url.as_deref(), Some("https://example.com/plan"));

    // A 1:1 with attendees and a conference link.
    let o = find(evs, "one-on-one-1").unwrap();
    assert_eq!(o.attendee_count, 2);
    assert_eq!(o.organizer.as_deref(), Some("me@acme.com"));
    assert_eq!(
        o.url.as_deref(),
        Some("https://meet.google.com/abc-defg-hij")
    );

    // Stable, sorted output: parsing twice gives the same keys.
    let again = ics::parse(&fixture("feed.ics"), None, now).unwrap();
    let k1: Vec<&str> = evs.iter().map(|e| e.key.as_str()).collect();
    let k2: Vec<&str> = again.events.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(k1, k2);
}

#[test]
fn a_zone_override_moves_all_day_events() {
    let now = at("2026-10-28T12:00:00Z");
    let ny = calendar::tz::resolve("America/New_York").unwrap();
    let p = ics::parse(&fixture("feed.ics"), Some(ny), now).unwrap();
    let d = find(&p.events, "allday-1").unwrap();
    assert_eq!(d.start, at("2026-10-30T04:00:00Z"));
}

#[test]
fn malformed_and_hostile_feeds_are_rejected() {
    let now = at("2026-10-28T12:00:00Z");
    let msg = |b: &[u8]| match ics::parse(b, None, now) {
        Ok(p) => panic!("accepted: {} events", p.events.len()),
        Err(e) => format!("{e:?}"),
    };
    assert!(msg(b"<html>login</html>").contains("did not return an iCalendar feed"));
    assert!(msg(b"").contains("did not return an iCalendar feed"));
    // Truncated: no END, broken lines. The parser's text (which quotes
    // the input) never reaches the error.
    let bad = msg(b"BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nDTSTART:SECRETJUNK\r\n");
    assert!(!bad.contains("SECRETJUNK"), "{bad}");
    // Too deep.
    let mut deep = String::from("BEGIN:VCALENDAR\r\n");
    for _ in 0..20 {
        deep.push_str("BEGIN:X-A\r\n");
    }
    for _ in 0..20 {
        deep.push_str("END:X-A\r\n");
    }
    deep.push_str("END:VCALENDAR\r\n");
    assert!(msg(deep.as_bytes()).contains("too deeply"));
    // Too big.
    let mut big = b"BEGIN:VCALENDAR\r\n".to_vec();
    big.resize(ics::MAX_FEED_BYTES + 1, b'x');
    assert!(msg(&big).contains("5 MiB"));
    // Too many events.
    let mut many = String::from("BEGIN:VCALENDAR\r\n");
    for i in 0..=ics::MAX_FEED_COMPONENTS {
        many.push_str(&format!(
            "BEGIN:VEVENT\r\nUID:{i}\r\nDTSTART:20261028T100000Z\r\nEND:VEVENT\r\n"
        ));
    }
    many.push_str("END:VCALENDAR\r\n");
    assert!(ics::parse(many.as_bytes(), None, now).is_err());
}

#[test]
fn a_daily_series_is_capped() {
    // Every minute of the window is far more than the cap: kept instances
    // are the nearest, and the batch is marked incomplete.
    let now = at("2026-10-28T12:00:00Z");
    let mut f = String::from("BEGIN:VCALENDAR\r\nX-WR-TIMEZONE:UTC\r\n");
    for i in 0..3000 {
        f.push_str(&format!(
            "BEGIN:VEVENT\r\nUID:e{i}\r\nDTSTART:20261028T{:02}{:02}00Z\r\nDURATION:PT1M\r\nSUMMARY:x\r\nEND:VEVENT\r\n",
            (i / 60) % 24,
            i % 60
        ));
    }
    f.push_str("END:VCALENDAR\r\n");
    let p = ics::parse(f.as_bytes(), None, now).unwrap();
    assert!(!p.complete);
    assert!(p.events.len() <= ics::MAX_EVENTS);
}

/// A feed with the same UID twice (an update appended, or reordered by a
/// server): the highest SEQUENCE wins whatever the order, then the newer
/// DTSTAMP. The same for an override's UID and RECURRENCE-ID.
#[test]
fn duplicate_uids_keep_the_newest_revision() {
    let now = at("2026-10-28T12:00:00Z");
    let ev = |uid: &str, extra: &str, seq: u32, stamp: &str, start: &str, title: &str| {
        format!(
            "BEGIN:VEVENT\r\nUID:{uid}\r\n{extra}SEQUENCE:{seq}\r\nDTSTAMP:{stamp}\r\nDTSTART:{start}\r\nDURATION:PT30M\r\nSUMMARY:{title}\r\nEND:VEVENT\r\n"
        )
    };
    let feed = [
        "BEGIN:VCALENDAR\r\nX-WR-TIMEZONE:UTC\r\n".to_string(),
        ev("a", "", 1, "20261025T000000Z", "20261029T090000Z", "old"),
        ev("a", "", 3, "20261020T000000Z", "20261029T100000Z", "new"),
        ev(
            "b",
            "",
            2,
            "20261020T000000Z",
            "20261029T110000Z",
            "older stamp",
        ),
        ev(
            "b",
            "",
            2,
            "20261021T000000Z",
            "20261029T120000Z",
            "newer stamp",
        ),
        ev(
            "s",
            "RRULE:FREQ=DAILY;COUNT=3\r\n",
            0,
            "20261001T000000Z",
            "20261028T080000Z",
            "series",
        ),
        ev(
            "s",
            "RECURRENCE-ID:20261029T080000Z\r\n",
            1,
            "20261003T000000Z",
            "20261029T140000Z",
            "moved early",
        ),
        ev(
            "s",
            "RECURRENCE-ID:20261029T080000Z\r\n",
            2,
            "20261002T000000Z",
            "20261029T150000Z",
            "moved late",
        ),
        "END:VCALENDAR\r\n".to_string(),
    ]
    .concat();
    let p = ics::parse(feed.as_bytes(), None, now).unwrap();
    let a = find(&p.events, "a").unwrap();
    assert_eq!(
        (a.title.as_str(), a.start),
        ("new", at("2026-10-29T10:00:00Z"))
    );
    let b = find(&p.events, "b").unwrap();
    assert_eq!(b.title, "newer stamp");
    let moved = find(&p.events, "s@20261029T080000Z").unwrap();
    assert_eq!(moved.title, "moved late");
    assert_eq!(starts(&p.events, "s").len(), 3);
}

/// A recurrence rule the parser cannot use, or more rules than are read,
/// leaves occurrences out: the batch must not count as the whole feed.
#[test]
fn unreadable_or_dropped_rules_make_the_batch_incomplete() {
    let now = at("2026-10-28T12:00:00Z");
    let feed = |rules: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nX-WR-TIMEZONE:UTC\r\nBEGIN:VEVENT\r\nUID:r\r\nDTSTART:20261028T090000Z\r\nDURATION:PT30M\r\nSUMMARY:x\r\n{rules}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    let ok = ics::parse(feed("RRULE:FREQ=WEEKLY\r\n").as_bytes(), None, now).unwrap();
    assert!(ok.complete);
    // Ended before it started: no occurrences, nothing missing.
    let ended = feed("RRULE:FREQ=WEEKLY;UNTIL=20261001T000000Z\r\n");
    assert!(ics::parse(ended.as_bytes(), None, now).unwrap().complete);
    for bad in [
        "RRULE:FREQ=FORTNIGHTLY\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=XX\r\n",
        "RRULE:FREQ=WEEKLY;UNTIL=garbage\r\n",
        "RRULE:FREQ=WEEKLY;BYDAY=MO\r\nRRULE:FREQ=WEEKLY;BYDAY=TU\r\nRRULE:FREQ=WEEKLY;BYDAY=WE\r\nRRULE:FREQ=WEEKLY;BYDAY=TH\r\nRRULE:FREQ=WEEKLY;BYDAY=FR\r\n",
    ] {
        let p = ics::parse(feed(bad).as_bytes(), None, now).unwrap();
        assert!(!p.complete, "{bad:?} counted as complete");
        assert!(
            find(&p.events, "r@20261028T090000Z").is_some(),
            "DTSTART still counts"
        );
    }
}

/// Pathological rules stay cheap: an every-second rule disguised as DAILY
/// is not expanded, and a feed of series that `rrule` must walk from the
/// year 1 stops expanding after the time cap (the batch is incomplete).
#[test]
fn pathological_rules_are_bounded_in_time() {
    let now = at("2026-10-28T12:00:00Z");
    let feed = |n: usize, start: &str, rule: &str| {
        let mut f = String::from("BEGIN:VCALENDAR\r\nX-WR-TIMEZONE:UTC\r\n");
        for i in 0..n {
            f.push_str(&format!(
                "BEGIN:VEVENT\r\nUID:p{i}\r\nDTSTART:{start}\r\nDURATION:PT1M\r\nSUMMARY:x\r\nRRULE:{rule}\r\nEND:VEVENT\r\n"
            ));
        }
        f.push_str("END:VCALENDAR\r\n");
        f
    };
    let all: Vec<String> = (0..60).map(|i| i.to_string()).collect();
    let secondly = format!("FREQ=DAILY;BYMINUTE={0};BYSECOND={0}", all.join(","));
    let t = std::time::Instant::now();
    let p = ics::parse(feed(1, "19000101T000000Z", &secondly).as_bytes(), None, now).unwrap();
    assert!(p.events.is_empty());
    assert!(
        t.elapsed() < std::time::Duration::from_secs(1),
        "{:?}",
        t.elapsed()
    );

    let t = std::time::Instant::now();
    let never = feed(
        2000,
        "00010101T000000Z",
        "FREQ=YEARLY;BYMONTH=2;BYMONTHDAY=30",
    );
    let p = ics::parse(never.as_bytes(), None, now).unwrap();
    assert!(!p.complete);
    assert!(
        t.elapsed() < std::time::Duration::from_secs(10),
        "{:?}",
        t.elapsed()
    );
}

// --------------------------------------------------------------- end to end

const SECRET: &str = "SECRETPATHq7Zx9876";

fn ics_time(t: DateTime<Utc>) -> String {
    t.format("%Y%m%dT%H%M%SZ").to_string()
}

/// A feed relative to today (UTC calendar): a weekly series, a 1:1 with
/// Alice tomorrow and, when `extra`, one more meeting.
fn live_feed(extra: bool) -> String {
    let today = Utc::now()
        .date_naive()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc();
    let series = today - Duration::days(21);
    let one = today + Duration::days(1) + Duration::hours(5);
    let x = today + Duration::days(2);
    let mut s = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-TIMEZONE:UTC\r\n\
BEGIN:VEVENT\r\nUID:weekly\r\nDTSTART:{}\r\nDURATION:PT30M\r\nRRULE:FREQ=WEEKLY;COUNT=10\r\n\
SUMMARY:Team retro\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:alice-1on1\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Alice / Me\r\n\
ORGANIZER;CN=Me:mailto:me@acme.com\r\n\
ATTENDEE;CN=Alice Andersson:mailto:alice@acme.com\r\n\
ATTENDEE;CN=Me:mailto:me@acme.com\r\nEND:VEVENT\r\n",
        ics_time(series),
        ics_time(one),
        ics_time(one + Duration::minutes(30)),
    );
    if extra {
        s.push_str(&format!(
            "BEGIN:VEVENT\r\nUID:extra\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:Vendor demo\r\n\
ORGANIZER:mailto:sales@vendor.example\r\n\
ATTENDEE:mailto:me@acme.com\r\nATTENDEE:mailto:bob@acme.com\r\n\
ATTENDEE:mailto:sales@vendor.example\r\nEND:VEVENT\r\n",
            ics_time(x),
            ics_time(x + Duration::hours(1)),
        ));
    }
    s.push_str("END:VCALENDAR\r\n");
    s
}

async fn serve(s: &MockServer, body: String) {
    Mock::given(method("GET"))
        .and(path(format!("/feeds/{SECRET}/basic.ics")))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/calendar")
                .set_body_string(body),
        )
        .mount(s)
        .await;
}

fn connect(h: &common::H, id: &str) {
    h.ok(
        "integrations.connect",
        json!({"id": id, "kind": "calendar", "title": "Work calendar", "interval_secs": 3600,
               "roles": ["developer", "eng-manager"],
               "settings": {"source": "ics", "me": ["Me@Acme.com"], "domains": ["acme.com"]}}),
    );
}

fn of_type(items: &[Value], t: &str) -> Vec<Value> {
    items
        .iter()
        .filter(|i| i["type_id"] == t)
        .cloned()
        .collect()
}

fn is_archived(i: &Value) -> bool {
    i["tags"].as_array().unwrap().contains(&json!("archived"))
}

async fn resync(h: &common::H, id: &str) {
    let last = h.entry(id)["status"]["last_sync"].clone();
    h.ok("integrations.sync_now", json!({"id": id}));
    h.until("resync", |h| {
        let e = h.entry(id);
        e["status"]["last_sync"] != last && e["status"]["state"] == "ok"
    })
    .await;
}

/// Connected for developers only (the default roles): no `em.*` item is
/// written, so an `em.meeting` never lands in the developer role, and no
/// 1:1 is created.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn em_types_need_the_eng_manager_role() {
    let srv = MockServer::start().await;
    serve(&srv, live_feed(true)).await;
    let h = common::start();
    h.ok(
        "tasks.create",
        json!({"title": "Alice Andersson", "type_id": "em.person",
               "fields": {"name": "Alice Andersson"}}),
    );
    h.ok(
        "integrations.connect",
        json!({"id": "dev", "kind": "calendar", "title": "Work calendar", "interval_secs": 3600,
               "settings": {"source": "ics", "me": ["Me@Acme.com"], "domains": ["acme.com"]}}),
    );
    assert_eq!(h.entry("dev")["config"]["roles"], json!(["developer"]));
    h.rt.set_token("dev", &format!("{}/feeds/{SECRET}/basic.ics", srv.uri()))
        .unwrap();
    h.until("first sync", |h| h.state("dev") == "ok").await;
    let items = h.items("dev");
    assert!(!of_type(&items, "dev.meeting").is_empty());
    assert!(of_type(&items, "em.meeting").is_empty());
    assert!(of_type(&items, "em.one_on_one").is_empty());
    assert!(
        items
            .iter()
            .all(|i| !i["type_id"].as_str().unwrap_or_default().starts_with("em.")),
        "{items:#?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feed_sync_maps_links_resyncs_and_archives() {
    let srv = MockServer::start().await;
    serve(&srv, live_feed(true)).await;
    let h = common::start();
    let alice = h.ok(
        "tasks.create",
        json!({"title": "Alice Andersson", "type_id": "em.person",
               "fields": {"name": "Alice Andersson"}}),
    );
    let alice_id = alice["item"]["id"]
        .as_str()
        .or_else(|| alice["id"].as_str())
        .unwrap()
        .to_string();

    connect(&h, "cal");
    h.until("needs_auth without a URL", |h| {
        h.state("cal") == "needs_auth"
    })
    .await;
    let mut events = h.core.subscribe();
    let url = format!("{}/feeds/{SECRET}/basic.ics", srv.uri());
    h.rt.set_token("cal", &url).unwrap();
    h.until("first sync", |h| h.state("cal") == "ok").await;

    let items = h.items("cal");
    let em = of_type(&items, "em.meeting");
    let dev = of_type(&items, "dev.meeting");
    // 4 weekly instances (-7, 0, +7, +14 days), the 1:1, the demo.
    assert_eq!(em.len(), 6, "{em:#?}");
    assert_eq!(dev.len(), 6, "{dev:#?}");
    let cat = |title: &str| {
        em.iter()
            .find(|i| i["title"] == title)
            .unwrap_or_else(|| panic!("no {title}"))["fields"]["category"]
            .clone()
    };
    assert_eq!(cat("Team retro"), "team");
    assert_eq!(cat("Alice / Me"), "1on1");
    assert_eq!(cat("Vendor demo"), "external");
    let one = em.iter().find(|i| i["title"] == "Alice / Me").unwrap();
    assert_eq!(one["kind"], "bullet");
    assert_eq!(one["fields"]["organizer_is_me"], true);
    assert_eq!(one["fields"]["attendees_count"], 2);
    assert!(one["tags"].as_array().unwrap().contains(&json!("calendar")));
    assert!(dev[0]["fields"].get("category").is_none());

    // The 1:1 is linked to Alice: one em.one_on_one, scheduled for it.
    let ones = of_type(&items, "em.one_on_one");
    assert_eq!(ones.len(), 1, "{ones:#?}");
    assert_eq!(ones[0]["fields"]["person_id"], alice_id.as_str());
    assert_eq!(
        ones[0]["fields"]["scheduled_for"],
        one["fields"]["starts_at"]
    );
    assert_eq!(ones[0]["fields"]["status"], "scheduled");
    assert_eq!(ones[0]["title"], "1:1 with Alice Andersson");

    // Same feed again: nothing written, nothing duplicated.
    let before = h.ok("history.list", json!({"limit": 1000}))["events"]
        .as_array()
        .unwrap()
        .len();
    resync(&h, "cal").await;
    let after = h.ok("history.list", json!({"limit": 1000}))["events"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(before, after, "an unchanged resync wrote events");
    let again = h.items("cal");
    assert_eq!(again.len(), items.len());
    assert_eq!(of_type(&again, "em.one_on_one").len(), 1);

    // The demo leaves the feed: archived in both types, the rest kept.
    srv.reset().await;
    serve(&srv, live_feed(false)).await;
    resync(&h, "cal").await;
    let items = h.items("cal");
    for t in ["em.meeting", "dev.meeting"] {
        let v = of_type(&items, t);
        let demo = v.iter().find(|i| i["title"] == "Vendor demo").unwrap();
        assert!(is_archived(demo), "{demo}");
        assert!(
            v.iter()
                .filter(|i| i["title"] != "Vendor demo")
                .all(|i| !is_archived(i))
        );
    }
    assert!(!is_archived(&of_type(&items, "em.one_on_one")[0]));

    // The hint shows the host only (no path, no characters of it).
    let hint = h.rt.secret_hint("cal").unwrap();
    assert_eq!(hint, "127.0.0.1", "{hint}");

    // The URL appears nowhere the core keeps or broadcasts.
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
        (
            "integrations",
            h.ok("integrations.list", json!({})).to_string(),
        ),
        ("items", serde_json::to_string(&items).unwrap()),
    ] {
        assert!(!text.contains("SECRETPATH"), "feed URL leaked into {what}");
        assert!(!text.contains("/feeds/"), "feed URL leaked into {what}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_and_oversized_feeds_keep_the_url_secret() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(format!("/gone/{SECRET}/basic.ics")))
        .respond_with(ResponseTemplate::new(404).set_body_string(format!("no feed {SECRET}")))
        .mount(&srv)
        .await;
    let mut huge = String::from("BEGIN:VCALENDAR\r\n");
    huge.push_str(&"X".repeat(ics::MAX_FEED_BYTES + 10));
    Mock::given(method("GET"))
        .and(path(format!("/huge/{SECRET}/basic.ics")))
        .respond_with(ResponseTemplate::new(200).set_body_string(huge))
        .mount(&srv)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/html/{SECRET}/basic.ics")))
        .respond_with(ResponseTemplate::new(200).set_body_string(format!("<html>{SECRET}</html>")))
        .mount(&srv)
        .await;

    let h = common::start();
    for id in ["gone", "huge", "html"] {
        connect(&h, id);
        h.rt.set_token(id, &format!("{}/{id}/{SECRET}/basic.ics", srv.uri()))
            .unwrap();
    }
    h.until("refused feed needs auth", |h| {
        h.state("gone") == "needs_auth" && h.entry("gone")["status"]["error"].is_string()
    })
    .await;
    h.until("huge feed fails", |h| h.state("huge") == "error")
        .await;
    h.until("html feed fails", |h| h.state("html") == "error")
        .await;
    let err = |id: &str| h.entry(id)["status"]["error"].to_string();
    assert!(err("gone").contains("HTTP 404"), "{}", err("gone"));
    assert!(err("huge").contains("5 MiB"), "{}", err("huge"));
    assert!(err("html").contains("iCalendar"), "{}", err("html"));
    assert!(h.items("huge").is_empty());

    let all = format!(
        "{}{}{}",
        h.ok("integrations.list", json!({})),
        h.ok("audit.list", json!({"limit": 1000})),
        h.ok("history.list", json!({"limit": 1000}))
    );
    assert!(!all.contains("SECRETPATH"), "the feed URL leaked: {all}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_https_feed_urls_are_stored() {
    let h = common::start();
    connect(&h, "cal");
    // Loopback http is allowed in tests only; other http is not.
    assert_eq!(
        h.rt.set_token("cal", "http://example.com/feed.ics"),
        Err("invalid_url")
    );
    assert_eq!(h.rt.set_token("cal", "not a url"), Err("invalid_url"));
    assert_eq!(
        h.rt.set_token("cal", "ftp://example.com/feed.ics"),
        Err("invalid_url")
    );
    assert!(h.rt.secret_hint("cal").is_none());
    // webcal:// is https.
    h.rt.set_token(
        "cal",
        "webcal://calendar.example.com/private-abcdef123456/basic.ics",
    )
    .unwrap();
    assert_eq!(
        h.rt.secret_hint("cal").as_deref(),
        Some("calendar.example.com")
    );
    assert_eq!(
        h.rt.set_token("nope", "https://example.com/a.ics"),
        Err("unknown_integration")
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mcp_calendar_servers_map_google_shaped_events() {
    use std::os::unix::fs::PermissionsExt;
    let start = Utc::now() + Duration::days(1);
    let end = start + Duration::minutes(45);
    let events = json!({"items": [
        {"id": "g1", "summary": "Sprint planning\u{202e}", "status": "confirmed",
         "start": {"dateTime": start.to_rfc3339()}, "end": {"dateTime": end.to_rfc3339()},
         "hangoutLink": "https://meet.google.com/aaa-bbbb-ccc",
         "organizer": {"email": "me@acme.com", "self": true},
         "attendees": [{"email": "me@acme.com", "self": true}, {"email": "bob@acme.com"},
                       {"email": "carol@acme.com"}]},
        {"id": "g2", "summary": "Gone", "status": "cancelled",
         "start": {"dateTime": start.to_rfc3339()}, "end": {"dateTime": end.to_rfc3339()}},
        {"id": "g3", "summary": "Heads down", "eventType": "focusTime",
         "start": {"dateTime": start.to_rfc3339()}, "end": {"dateTime": end.to_rfc3339()}}
    ]});
    let script = format!(
        "#!/bin/sh\nwhile IFS= read -r line; do\n  id=$(printf '%s' \"$line\" | sed -n 's/.*\"id\":\\([0-9][0-9]*\\).*/\\1/p')\n  case \"$line\" in\n    *'\"method\":\"initialize\"'*)\n      printf '{{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":{{\"protocolVersion\":\"2025-06-18\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"cal\",\"version\":\"1\"}}}}}}\\n' \"$id\" ;;\n    *'\"method\":\"tools/call\"'*)\n      printf '{{\"jsonrpc\":\"2.0\",\"id\":%s,\"result\":{{\"content\":[],\"structuredContent\":{}}}}}\\n' \"$id\" ;;\n  esac\ndone\n",
        events.to_string().replace('%', "%%").replace('\'', "")
    );
    let h = common::start();
    let path = h.dir.path().join("cal.sh");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    h.ok(
        "integrations.connect",
        json!({"id": "gcal", "kind": "calendar", "title": "Google", "interval_secs": 3600,
               "roles": ["eng-manager"],
               "settings": {"source": "mcp", "item_types": ["em.meeting"], "me": ["me@acme.com"],
                            "mcp": {"transport": "stdio", "command": path.to_str().unwrap(),
                                    "tool": "list_events"}}}),
    );
    h.until("mcp calendar sync", |h| h.state("gcal") == "ok")
        .await;
    let all = h.items("gcal");
    let items = of_type(&all, "em.meeting");
    assert_eq!(items.len(), 2, "{all:#?}");
    assert!(of_type(&all, "dev.meeting").is_empty());
    let plan = items
        .iter()
        .find(|i| i["title"] == "Sprint planning")
        .unwrap_or_else(|| panic!("{items:#?}"));
    assert_eq!(plan["fields"]["category"], "team");
    assert_eq!(plan["fields"]["organizer_is_me"], true);
    assert_eq!(
        plan["fields"]["url"],
        "https://meet.google.com/aaa-bbbb-ccc"
    );
    let focus = items.iter().find(|i| i["title"] == "Heads down").unwrap();
    assert_eq!(focus["fields"]["category"], "focus");
}

/// A feed with a 1:1 with Alice `days` days from today and one with Carol.
fn one_on_one_feed(days: i64) -> String {
    let today = Utc::now()
        .date_naive()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc();
    let ev = |uid: &str, who: &str, mail: &str, at: DateTime<Utc>| {
        format!(
            "BEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:{who} / Me\r\n\
ORGANIZER;CN=Me:mailto:me@acme.com\r\n\
ATTENDEE;CN={who}:mailto:{mail}\r\n\
ATTENDEE;CN=Me:mailto:me@acme.com\r\nEND:VEVENT\r\n",
            ics_time(at),
            ics_time(at + Duration::minutes(30)),
        )
    };
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nX-WR-TIMEZONE:UTC\r\n{}{}END:VCALENDAR\r\n",
        ev(
            "alice",
            "Alice Andersson",
            "alice@acme.com",
            today + Duration::days(days)
        ),
        ev(
            "carol",
            "Carol Chen",
            "carol@acme.com",
            today + Duration::days(3)
        ),
    )
}

fn id_of(v: &Value) -> String {
    v["item"]["id"]
        .as_str()
        .or_else(|| v["id"].as_str())
        .unwrap()
        .to_string()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn moved_one_on_ones_follow_the_feed_and_items_narrow_to_their_role() {
    let srv = MockServer::start().await;
    serve(&srv, one_on_one_feed(1)).await;
    let h = common::start();
    let person = |name: &str, mail: &str| {
        id_of(&h.ok(
            "tasks.create",
            json!({"title": name, "type_id": "em.person",
                   "fields": {"name": name, "email": mail}}),
        ))
    };
    let alice = person("Alice Andersson", "alice@acme.com");
    let carol = person("Carol Chen", "carol@acme.com");
    // The user already planned Carol's next 1:1 by hand.
    let mine = id_of(&h.ok(
        "tasks.create",
        json!({"title": "Carol 1:1", "kind": "task", "type_id": "em.one_on_one",
               "fields": {"person_id": carol, "scheduled_for": "2030-01-01T10:00:00Z",
                          "status": "scheduled"}}),
    ));

    h.ok(
        "integrations.connect",
        json!({"id": "cal", "kind": "calendar", "title": "Work calendar", "interval_secs": 3600,
               "roles": ["developer", "eng-manager"],
               "settings": {"source": "ics", "me": ["me@acme.com"], "domains": ["acme.com"]}}),
    );
    let url = format!("{}/feeds/{SECRET}/basic.ics", srv.uri());
    h.rt.set_token("cal", &url).unwrap();
    h.until("first sync", |h| h.state("cal") == "ok").await;

    let items = h.items("cal");
    for i in of_type(&items, "em.meeting") {
        assert_eq!(i["roles"], json!(["eng-manager"]), "{i}");
    }
    for i in of_type(&items, "dev.meeting") {
        assert_eq!(i["roles"], json!(["developer"]), "{i}");
    }
    let ones = of_type(&items, "em.one_on_one");
    assert_eq!(ones.len(), 1, "only Alice gets a new 1:1: {ones:#?}");
    assert_eq!(ones[0]["fields"]["person_id"], alice.as_str());
    assert_eq!(ones[0]["roles"], json!(["eng-manager"]));
    let first = ones[0]["fields"]["scheduled_for"].clone();

    // Alice's meeting moves a day later: the 1:1 item follows.
    srv.reset().await;
    serve(&srv, one_on_one_feed(2)).await;
    resync(&h, "cal").await;
    let ones = of_type(&h.items("cal"), "em.one_on_one");
    assert_eq!(ones.len(), 1);
    let moved = ones[0]["fields"]["scheduled_for"].clone();
    assert_ne!(moved, first);
    let want = Utc::now()
        .date_naive()
        .and_hms_opt(9, 0, 0)
        .unwrap()
        .and_utc()
        + Duration::days(2);
    assert_eq!(
        DateTime::parse_from_rfc3339(moved.as_str().unwrap()).unwrap(),
        want
    );
    let ev = h.ok("history.list", json!({"limit": 1000}))["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["command"] == "ext.patch_fields")
        .cloned()
        .expect("a patch event");
    assert_eq!(ev["actor"], "integration:cal");

    // The user's own 1:1 with Carol is never touched.
    let c = h.ok("tasks.get", json!({"id": mine}));
    let c = if c.get("item").is_some() {
        c["item"].clone()
    } else {
        c
    };
    assert_eq!(c["fields"]["scheduled_for"], "2030-01-01T10:00:00Z");
    assert_eq!(c["origin"], "user");
}
