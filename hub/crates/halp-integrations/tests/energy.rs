//! The energy connector end to end: Electricity Maps against wiremock,
//! MQTT against a small in-process broker (rumqttc's codec on a local
//! TCP socket), both through the runtime into the core.
#![allow(clippy::unwrap_used)]

mod common;

use std::sync::{Arc, Mutex};

use bytes::BytesMut;
use chrono::{DateTime, Duration, DurationRound, SecondsFormat, Utc};
use rumqttc::mqttbytes::QoS;
use rumqttc::mqttbytes::v4::{
    ConnAck, ConnectReturnCode, Packet, Publish, SubAck, SubscribeReasonCode,
};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "emaps-TOKENSECRET-0123456789abcdef";
const PASSWORD: &str = "mqtt-PASSWORDSECRET-42";

fn hour() -> DateTime<Utc> {
    Utc::now().duration_trunc(Duration::hours(1)).unwrap()
}

fn ts(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn latest(zone: &str, at: DateTime<Utc>, ci: Value) -> Value {
    json!({"zone": zone, "carbonIntensity": ci, "datetime": ts(at),
           "updatedAt": ts(Utc::now()), "emissionFactorType": "lifecycle",
           "isEstimated": true, "estimationMethod": "TIME_SLICER_AVERAGE"})
}

/// `n` hourly records ending an hour before `at`.
fn history(zone: &str, at: DateTime<Utc>, n: i64) -> Value {
    let h: Vec<Value> = (1..=n)
        .rev()
        .map(|i| {
            json!({"zone": zone, "carbonIntensity": 100 + i,
                   "datetime": ts(at - Duration::hours(i)), "isEstimated": false})
        })
        .collect();
    json!({"zone": zone, "history": h})
}

async fn mount(srv: &MockServer, p: &str, zone: &str, body: Value) {
    Mock::given(method("GET"))
        .and(path(p))
        .and(query_param("zone", zone))
        .and(header("auth-token", TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(srv)
        .await;
}

async fn serve_zone(srv: &MockServer, zone: &str, at: DateTime<Utc>, ci: Value, n: i64) {
    mount(
        srv,
        "/v3/carbon-intensity/latest",
        zone,
        latest(zone, at, ci),
    )
    .await;
    mount(
        srv,
        "/v3/carbon-intensity/history",
        zone,
        history(zone, at, n),
    )
    .await;
    mount(
        srv,
        "/v3/power-breakdown/latest",
        zone,
        json!({"zone": zone, "datetime": ts(at), "renewablePercentage": 61.23,
               "fossilFreePercentage": 90}),
    )
    .await;
}

fn connect_emaps(h: &common::H, srv: &MockServer) {
    h.ok(
        "integrations.connect",
        json!({"id": "grid", "kind": "energy", "title": "Grid carbon", "interval_secs": 3600,
               "roles": ["cso"],
               "settings": {"source": "electricity_maps", "zones": ["DE", "SE-SE3"],
                            "zone_labels": {"SE-SE3": "Stockholm\u{202e}"},
                            "api_url": srv.uri()}}),
    );
}

/// The integration's readings (the runtime also adds a group parent).
fn readings(h: &common::H, id: &str) -> Vec<Value> {
    h.items(id)
        .into_iter()
        .filter(|i| i["type_id"] == "cso.energy_reading")
        .collect()
}

fn by_zone(items: &[Value], zone: &str) -> Value {
    items
        .iter()
        .find(|i| i["fields"]["zone"] == zone)
        .cloned()
        .unwrap_or_else(|| panic!("no item for {zone}: {items:#?}"))
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

/// Syncs once more and waits for it to fail.
async fn resync_fails(h: &common::H, id: &str, state: &str) {
    h.ok("integrations.sync_now", json!({"id": id}));
    h.until(state, |h| h.state(id) == state).await;
}

fn assert_absent(h: &common::H, events: Vec<String>, needles: &[&str]) {
    assert!(!events.is_empty());
    for (what, text) in [
        ("events", events.join("\n")),
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
        ("items", h.ok("tasks.list", json!({})).to_string()),
    ] {
        for n in needles {
            assert!(!text.contains(n), "secret {n} leaked into {what}");
        }
    }
}

fn drain(events: &mut tokio::sync::broadcast::Receiver<halp_domain::Event>) -> Vec<String> {
    let mut seen = Vec::new();
    while let Ok(e) = events.try_recv() {
        seen.push(serde_json::to_string(&e).unwrap());
    }
    seen
}

// --------------------------------------------------------- Electricity Maps

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn electricity_maps_writes_one_reading_per_zone_and_keeps_the_token_secret() {
    let srv = MockServer::start().await;
    let at = hour();
    serve_zone(&srv, "DE", at, json!(412.6), 60).await;
    serve_zone(&srv, "SE-SE3", at, json!("23"), 3).await;
    let h = common::start();
    connect_emaps(&h, &srv);
    h.until("needs_auth without a token", |h| {
        h.state("grid") == "needs_auth"
    })
    .await;
    let mut events = h.core.subscribe();
    h.rt.set_token("grid", &format!("  {TOKEN}\n")).unwrap();
    h.until("first sync", |h| h.state("grid") == "ok").await;

    let items = readings(&h, "grid");
    assert_eq!(items.len(), 2, "{items:#?}");
    let de = by_zone(&items, "DE");
    assert_eq!(de["type_id"], "cso.energy_reading");
    assert_eq!(de["kind"], "bullet");
    assert_eq!(de["title"], "Grid carbon DE");
    assert_eq!(de["roles"], json!(["cso"]));
    let f = &de["fields"];
    assert_eq!(f["source"], "electricity_maps");
    assert_eq!(f["gco2_per_kwh"], 412.6);
    assert_eq!(f["renewable_pct"], 61.2);
    assert_eq!(f["estimated"], true, "isEstimated is kept");
    assert_eq!(
        DateTime::parse_from_rfc3339(f["at"].as_str().unwrap()).unwrap(),
        at
    );
    // 60 hours of history plus the latest, capped to the newest 48.
    let series = f["series"].as_array().unwrap();
    assert_eq!(series.len(), 48);
    let last = series.last().unwrap();
    assert_eq!(last["gco2_per_kwh"], 412.6);
    assert_eq!(last["at"], f["at"]);
    assert_eq!(series[0]["gco2_per_kwh"], 147.0);
    for s in series {
        let keys: Vec<&String> = s.as_object().unwrap().keys().collect();
        assert!(
            keys.iter().all(|k| *k == "at" || *k == "gco2_per_kwh"),
            "{s}"
        );
    }

    let se = by_zone(&items, "SE-SE3");
    assert_eq!(se["fields"]["zone_label"], "Stockholm");
    assert_eq!(se["title"], "Grid carbon Stockholm");
    assert_eq!(se["fields"]["gco2_per_kwh"], 23.0);
    assert_eq!(se["fields"]["series"].as_array().unwrap().len(), 4);

    // The same answers again write nothing.
    let n = |h: &common::H| {
        h.ok("history.list", json!({"limit": 1000}))["events"]
            .as_array()
            .unwrap()
            .len()
    };
    let before = n(&h);
    resync(&h, "grid").await;
    assert_eq!(n(&h), before, "an unchanged resync wrote events");

    // The token (trimmed: the mocks match it exactly) is in the keychain
    // only; the hint does not show it.
    if let Some(hint) = h.rt.secret_hint("grid") {
        assert!(!hint.contains("TOKENSECRET"), "{hint}");
    }
    assert_absent(&h, drain(&mut events), &[TOKEN, "TOKENSECRET"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_or_missing_values_leave_the_reading_stale() {
    let srv = MockServer::start().await;
    let at = hour() - Duration::hours(1);
    serve_zone(&srv, "DE", at, json!(300), 2).await;
    serve_zone(&srv, "SE-SE3", at, json!(30), 2).await;
    let h = common::start();
    connect_emaps(&h, &srv);
    h.rt.set_token("grid", TOKEN).unwrap();
    h.until("first sync", |h| h.state("grid") == "ok").await;
    let first = readings(&h, "grid");
    let fields = |items: &[Value], z: &str| by_zone(items, z)["fields"].clone();

    // Non-finite, huge, negative and missing intensities: no item moves.
    for bad in [
        json!("Infinity"),
        json!("NaN"),
        json!(1e308),
        json!(3001),
        json!(-5),
        Value::Null,
    ] {
        srv.reset().await;
        serve_zone(&srv, "DE", hour(), bad.clone(), 2).await;
        serve_zone(&srv, "SE-SE3", hour(), bad.clone(), 2).await;
        resync_fails(&h, "grid", "error").await;
        let now = readings(&h, "grid");
        for z in ["DE", "SE-SE3"] {
            assert_eq!(fields(&now, z), fields(&first, z), "{bad} changed {z}");
        }
        let err = h.entry("grid")["status"]["error"].to_string();
        assert!(err.contains("no current carbon intensity"), "{err}");
    }

    // One zone fails, the other moves on; nothing is archived because
    // the batch is not complete.
    srv.reset().await;
    serve_zone(&srv, "DE", hour(), json!(250), 2).await;
    serve_zone(&srv, "SE-SE3", hour(), json!("1e999"), 2).await;
    resync(&h, "grid").await;
    let now = readings(&h, "grid");
    assert_eq!(fields(&now, "DE")["gco2_per_kwh"], 250.0);
    assert_eq!(fields(&now, "SE-SE3"), fields(&first, "SE-SE3"));
    assert!(
        now.iter()
            .all(|i| !i["tags"].as_array().unwrap().contains(&json!("archived")))
    );

    // An older reading never replaces a newer one.
    srv.reset().await;
    serve_zone(&srv, "DE", hour() - Duration::hours(5), json!(999), 0).await;
    serve_zone(&srv, "SE-SE3", hour(), json!(31), 0).await;
    resync(&h, "grid").await;
    assert_eq!(fields(&readings(&h, "grid"), "DE")["gco2_per_kwh"], 250.0);

    // A future reading is not believed.
    srv.reset().await;
    serve_zone(&srv, "DE", hour() + Duration::hours(3), json!(10), 0).await;
    serve_zone(&srv, "SE-SE3", hour() + Duration::hours(3), json!(10), 0).await;
    resync_fails(&h, "grid", "error").await;
    assert_eq!(fields(&readings(&h, "grid"), "DE")["gco2_per_kwh"], 250.0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_token_asks_for_a_new_one_without_echoing_it() {
    let srv = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(401)
                .set_body_string(format!("invalid token {TOKEN}"))
                .insert_header("www-authenticate", TOKEN),
        )
        .mount(&srv)
        .await;
    let h = common::start();
    let mut events = h.core.subscribe();
    connect_emaps(&h, &srv);
    h.rt.set_token("grid", TOKEN).unwrap();
    h.until("needs_auth", |h| {
        h.state("grid") == "needs_auth" && !h.entry("grid")["status"]["error"].is_null()
    })
    .await;
    let err = h.entry("grid")["status"]["error"].to_string();
    assert!(err.contains("401"), "{err}");
    assert!(readings(&h, "grid").is_empty());
    assert_absent(&h, drain(&mut events), &[TOKEN, "TOKENSECRET"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn electricity_maps_settings_are_checked() {
    let h = common::start();
    let bad = |settings: Value| {
        let r = h.call(
            "integrations.connect",
            json!({"id": "x", "kind": "energy", "title": "x", "settings": settings}),
        );
        assert!(
            matches!(r, halp_core::Response::Err(_)),
            "accepted {settings}"
        );
    };
    bad(json!({"source": "electricity_maps", "zones": []}));
    bad(json!({"source": "electricity_maps", "zones": ["DE DE"]}));
    bad(json!({"source": "electricity_maps", "zones": ["DE"], "token": TOKEN}));
    bad(json!({"source": "electricity_maps", "zones": ["DE"],
               "api_url": "http://api.example.com"}));
    bad(json!({"source": "electricity_maps", "zones": ["DE"],
               "api_url": "https://u:p@api.example.com"}));
    bad(json!({"source": "solar", "zones": ["DE"]}));
    bad(json!({"source": "mqtt"}));
    let list = h.ok("integrations.list", json!({})).to_string();
    assert!(!list.contains(TOKEN));
}

// --------------------------------------------------------------------- MQTT

#[derive(Default)]
struct Seen {
    logins: Vec<(String, String)>,
    topics: Vec<String>,
}

/// A broker for tests: checks the password, acks the subscription and
/// publishes `msgs` (topic, payload, retained) on every connection.
async fn broker(msgs: Vec<(String, Vec<u8>, bool)>) -> (u16, Arc<Mutex<Seen>>) {
    let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s2 = seen.clone();
    tokio::spawn(async move {
        loop {
            let Ok((sock, _)) = l.accept().await else {
                return;
            };
            tokio::spawn(session(sock, msgs.clone(), s2.clone()));
        }
    });
    (port, seen)
}

async fn read(sock: &mut TcpStream, buf: &mut BytesMut) -> Option<Packet> {
    loop {
        if let Ok(p) = Packet::read(buf, 1 << 20) {
            return Some(p);
        }
        if sock.read_buf(buf).await.ok()? == 0 {
            return None;
        }
    }
}

async fn write(sock: &mut TcpStream, p: Packet) {
    let mut b = BytesMut::new();
    p.write(&mut b, 1 << 20).unwrap();
    sock.write_all(&b).await.unwrap();
}

async fn session(mut sock: TcpStream, msgs: Vec<(String, Vec<u8>, bool)>, seen: Arc<Mutex<Seen>>) {
    let mut buf = BytesMut::new();
    let Some(Packet::Connect(c)) = read(&mut sock, &mut buf).await else {
        return;
    };
    let login = c
        .login
        .map(|l| (l.username, l.password))
        .unwrap_or_default();
    let good = login.1 == PASSWORD;
    seen.lock().unwrap().logins.push(login);
    let code = if good {
        ConnectReturnCode::Success
    } else {
        ConnectReturnCode::BadUserNamePassword
    };
    write(&mut sock, Packet::ConnAck(ConnAck::new(code, false))).await;
    if !good {
        return;
    }
    let Some(Packet::Subscribe(s)) = read(&mut sock, &mut buf).await else {
        return;
    };
    let codes = s
        .filters
        .iter()
        .map(|f| {
            seen.lock().unwrap().topics.push(f.path.clone());
            SubscribeReasonCode::Success(QoS::AtMostOnce)
        })
        .collect();
    write(&mut sock, Packet::SubAck(SubAck::new(s.pkid, codes))).await;
    for (topic, payload, retain) in msgs {
        let mut p = Publish::new(topic, QoS::AtMostOnce, payload);
        p.retain = retain;
        write(&mut sock, Packet::Publish(p)).await;
    }
    // Wait for the client's DISCONNECT (or close).
    while let Some(p) = read(&mut sock, &mut buf).await {
        if matches!(p, Packet::Disconnect) {
            return;
        }
    }
}

fn connect_mqtt(h: &common::H, port: u16) {
    h.ok(
        "integrations.connect",
        json!({"id": "meter", "kind": "energy", "title": "Site meter", "interval_secs": 3600,
        "roles": ["cso"],
        "settings": {"source": "mqtt", "mqtt": {
            "host": "127.0.0.1", "port": port, "username": "hub", "wait_secs": 3,
            "meters": [
                {"topic": "site/hq/power", "zone": "hq-main", "site": "HQ\u{0007} main",
                 "pointer": "/data/w", "at_pointer": "/data/ts", "unit": "W"},
                {"topic": "site/lab/power", "zone": "lab"},
                {"topic": "site/old/power", "zone": "old"}
            ]}}}),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mqtt_meters_write_power_and_keep_the_password_secret() {
    let at = Utc::now().duration_trunc(Duration::minutes(1)).unwrap() - Duration::minutes(2);
    let (port, seen) = broker(vec![
        // Retained with its own time: used.
        (
            "site/hq/power".into(),
            json!({"data": {"w": 12_345.6, "ts": ts(at)}})
                .to_string()
                .into_bytes(),
            true,
        ),
        // Live plain number: stamped on arrival.
        ("site/lab/power".into(), b"7.25".to_vec(), false),
        // Retained without a time, then a huge live one: neither is used.
        ("site/old/power".into(), b"5".to_vec(), true),
        ("site/old/power".into(), b"1e12".to_vec(), false),
        // Not subscribed: ignored.
        ("other/topic".into(), b"1".to_vec(), false),
    ])
    .await;
    let h = common::start();
    let mut events = h.core.subscribe();
    connect_mqtt(&h, port);
    h.until("needs_auth without a password", |h| {
        h.state("meter") == "needs_auth"
    })
    .await;
    h.rt.set_token("meter", PASSWORD).unwrap();
    h.until("first sync", |h| h.state("meter") == "ok").await;

    let items = readings(&h, "meter");
    assert_eq!(items.len(), 2, "the silent meter has no item: {items:#?}");
    let hq = by_zone(&items, "hq-main");
    assert_eq!(hq["title"], "Site power HQ main");
    assert_eq!(hq["roles"], json!(["cso"]));
    let f = &hq["fields"];
    assert_eq!(f["source"], "mqtt");
    assert_eq!(f["site"], "HQ main");
    assert_eq!(f["power_kw"], 12.346);
    assert_eq!(
        DateTime::parse_from_rfc3339(f["at"].as_str().unwrap()).unwrap(),
        at
    );
    assert_eq!(f["series"], json!([{"at": f["at"], "power_kw": 12.346}]));
    assert!(f.get("gco2_per_kwh").is_none());
    let lab = by_zone(&items, "lab");
    assert_eq!(lab["fields"]["power_kw"], 7.25);
    let lab_at = DateTime::parse_from_rfc3339(lab["fields"]["at"].as_str().unwrap()).unwrap();
    assert!((Utc::now() - lab_at.with_timezone(&Utc)).num_seconds() < 60);

    {
        let s = seen.lock().unwrap();
        assert_eq!(s.logins[0], ("hub".to_string(), PASSWORD.to_string()));
        assert_eq!(
            s.topics[..3],
            ["site/hq/power", "site/lab/power", "site/old/power"]
        );
    }
    assert_absent(&h, drain(&mut events), &[PASSWORD, "PASSWORDSECRET"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_broker_password_asks_again_and_silence_is_an_error() {
    let (port, _) = broker(vec![]).await;
    let h = common::start();
    let mut events = h.core.subscribe();
    connect_mqtt(&h, port);
    h.rt.set_token("meter", "wrong-PASSWORDSECRET").unwrap();
    h.until("needs_auth after a refusal", |h| {
        h.state("meter") == "needs_auth" && !h.entry("meter")["status"]["error"].is_null()
    })
    .await;
    let err = h.entry("meter")["status"]["error"].to_string();
    assert!(err.contains("refused the username or password"), "{err}");

    // The right password, but no meter says anything: an error, no items.
    h.rt.set_token("meter", PASSWORD).unwrap();
    h.until("error on silence", |h| h.state("meter") == "error")
        .await;
    let err = h.entry("meter")["status"]["error"].to_string();
    assert!(err.contains("no usable meter reading"), "{err}");
    assert!(readings(&h, "meter").is_empty());
    assert_absent(&h, drain(&mut events), &[PASSWORD, "PASSWORDSECRET"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mqtt_settings_require_tls_off_this_computer() {
    let h = common::start();
    let r = h.call(
        "integrations.connect",
        json!({"id": "m", "kind": "energy", "title": "m",
               "settings": {"source": "mqtt", "mqtt": {"host": "broker.example.com", "tls": false,
                            "meters": [{"topic": "a", "zone": "a"}]}}}),
    );
    assert!(matches!(r, halp_core::Response::Err(_)));
    // Defaults (TLS on, port 8883) are covered by the unit tests; a
    // connect here would try to reach the host.
}
