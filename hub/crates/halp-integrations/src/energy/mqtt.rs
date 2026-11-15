//! A site energy meter on an MQTT broker (MQTT 3.1.1).
//!
//! The runtime polls, so one sync connects, subscribes to the meters'
//! topics (QoS 0, clean session), reads what arrives within `wait_secs`
//! (the broker's retained message first, then live ones), and
//! disconnects. Packets are encoded and decoded by `rumqttc`'s codec; the
//! connection is ours: plain TCP only to a broker on this computer,
//! otherwise TLS with the operating system's trust store (rustls and the
//! platform verifier, as for HTTPS).
//!
//! A message counts only with a time: a live message is stamped when it
//! arrives; a retained message (which may be days old) only when the
//! payload carries its own timestamp (`at_pointer`). Payloads are capped
//! at [`MAX_PAYLOAD_BYTES`], packets at [`MAX_PACKET_BYTES`], a sync at
//! [`MAX_READ_BYTES`].

use std::sync::Arc;
use std::time::Duration;

use bytes::BytesMut;
use chrono::{DateTime, Utc};
use rumqttc::mqttbytes::v4::SubscribeReasonCode;
use rumqttc::mqttbytes::v4::{ConnAck, Connect, ConnectReturnCode, Packet, Subscribe};
use rumqttc::mqttbytes::{Error as MqttError, QoS};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::{MAX_ZONES, Reading, instant, number, power_kw, text, valid_zone};
use crate::connector::SyncError;

/// Largest payload read as a meter value.
pub const MAX_PAYLOAD_BYTES: usize = 4 * 1024;
/// Largest packet accepted from the broker (topic + payload).
pub const MAX_PACKET_BYTES: usize = 8 * 1024;
/// Most bytes read from the broker in one sync.
pub const MAX_READ_BYTES: usize = 1024 * 1024;
pub const DEFAULT_WAIT_SECS: u64 = 5;
pub const MAX_WAIT_SECS: u64 = 30;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const KEEP_ALIVE_SECS: u16 = 60;
const MAX_TOPIC_BYTES: usize = 256;
const MAX_POINTER_CHARS: usize = 256;

/// One meter: a topic and how to read it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Meter {
    /// An exact topic (no `+` or `#` wildcards).
    pub topic: String,
    /// The item's `zone` (its key): letters, digits and `_ . / : -`.
    pub zone: String,
    /// Display name of the site.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub site: Option<String>,
    /// JSON pointer to the value in a JSON payload (`/power_kw`); without
    /// it the payload is a plain number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<String>,
    /// JSON pointer to the reading's time (RFC 3339 or epoch seconds or
    /// milliseconds). Needed for retained messages to count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at_pointer: Option<String>,
    /// `kW` (default) or `W`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// The `mqtt` part of `energy` settings. The password (when `username` is
/// set) is the keychain secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MqttSettings {
    pub host: String,
    /// Default 8883 with TLS, 1883 without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Required (and the default) unless the broker is on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub meters: Vec<Meter>,
    /// How long one sync listens (default 5, at most 30 seconds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_secs: Option<u64>,
}

fn is_loopback_host(h: &str) -> bool {
    h == "localhost" || h.parse::<std::net::IpAddr>().is_ok_and(|a| a.is_loopback())
}

fn check_pointer(p: &Option<String>, what: &str) -> Result<Option<String>, String> {
    match p.as_deref().map(str::trim) {
        None => Ok(None),
        Some(p)
            if p.starts_with('/')
                && p.chars().count() <= MAX_POINTER_CHARS
                && !p.chars().any(char::is_control) =>
        {
            Ok(Some(p.to_string()))
        }
        Some(_) => Err(format!(
            "energy: {what} must be a JSON pointer like /power_kw"
        )),
    }
}

impl MqttSettings {
    pub fn check(mut self) -> Result<MqttSettings, String> {
        let host = self
            .host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let ip = host.parse::<std::net::IpAddr>().is_ok();
        if host.is_empty()
            || host.len() > 253
            || (!ip
                && !host
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
        {
            return Err(
                "energy: host must be a host name or IP address (no scheme or port)".into(),
            );
        }
        let local = is_loopback_host(&host);
        let tls = self.tls.unwrap_or(!local);
        if !tls && !local {
            return Err("energy: TLS is required unless the broker is on this computer".into());
        }
        self.host = host;
        self.tls = Some(tls);
        if self.port == Some(0) {
            return Err("energy: port must be 1 to 65535".into());
        }
        self.port = Some(self.port.unwrap_or(if tls { 8883 } else { 1883 }));
        if let Some(u) = &self.username {
            let u = u.trim();
            if u.is_empty() || u.len() > 256 || !u.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
                return Err("energy: username must be 1 to 256 printable ASCII characters".into());
            }
            self.username = Some(u.to_string());
        }
        let wait = self.wait_secs.unwrap_or(DEFAULT_WAIT_SECS);
        if !(1..=MAX_WAIT_SECS).contains(&wait) {
            return Err(format!("energy: wait_secs is 1 to {MAX_WAIT_SECS}"));
        }
        self.wait_secs = Some(wait);
        if self.meters.is_empty() || self.meters.len() > MAX_ZONES {
            return Err(format!("energy: list 1 to {MAX_ZONES} meters"));
        }
        let mut meters: Vec<Meter> = Vec::new();
        for m in &self.meters {
            let topic = m.topic.trim().to_string();
            if topic.is_empty()
                || topic.len() > MAX_TOPIC_BYTES
                || topic.contains(['+', '#'])
                || topic.chars().any(|c| c.is_control() || c == '\u{0}')
                || text(&topic, MAX_TOPIC_BYTES) != topic
            {
                return Err(
                    "energy: a meter topic must be an exact topic (no + or # wildcards), at most 256 bytes"
                        .into(),
                );
            }
            let zone = m.zone.trim().to_string();
            if !valid_zone(&zone) {
                return Err("energy: a meter zone is 1 to 64 letters, digits and _ . / : -".into());
            }
            if meters.iter().any(|x| x.zone == zone || x.topic == topic) {
                return Err("energy: meter zones and topics must be unique".into());
            }
            let site = m
                .site
                .as_deref()
                .map(|s| text(s, 120))
                .filter(|s| !s.is_empty());
            let pointer = check_pointer(&m.pointer, "pointer")?;
            let at_pointer = check_pointer(&m.at_pointer, "at_pointer")?;
            if at_pointer.is_some() && pointer.is_none() {
                return Err("energy: at_pointer needs a JSON payload (set pointer too)".into());
            }
            let unit = match m.unit.as_deref().map(str::trim) {
                None => None,
                Some(u) if u.eq_ignore_ascii_case("kw") => Some("kW".to_string()),
                Some(u) if u.eq_ignore_ascii_case("w") => Some("W".to_string()),
                Some(_) => return Err("energy: unit is kW or W".into()),
            };
            meters.push(Meter {
                topic,
                zone,
                site,
                pointer,
                at_pointer,
                unit,
            });
        }
        self.meters = meters;
        Ok(self)
    }
}

/// A meter value from a payload: `(at, kW)`. `received` is when a live
/// message arrived; a retained one (`retained`) needs its own timestamp.
pub fn parse_payload(
    m: &Meter,
    payload: &[u8],
    retained: bool,
    received: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, f64)> {
    if payload.len() > MAX_PAYLOAD_BYTES {
        return None;
    }
    let s = std::str::from_utf8(payload).ok()?.trim();
    let (raw, at) = match &m.pointer {
        Some(p) => {
            let v: Value = serde_json::from_str(s).ok()?;
            let raw = number(v.pointer(p)?)?;
            let at = match &m.at_pointer {
                Some(ap) => Some(instant(v.pointer(ap)?, now)?),
                None => None,
            };
            (raw, at)
        }
        None => {
            if s.is_empty()
                || s.len() > 64
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_digit() || matches!(b, b'+' | b'-' | b'.' | b'e' | b'E'))
            {
                return None;
            }
            (s.parse::<f64>().ok()?, None)
        }
    };
    let at = match at {
        Some(t) => t,
        None if retained => return None,
        None => DateTime::from_timestamp(received.timestamp(), 0)?,
    };
    let kw = match m.unit.as_deref() {
        Some("W") => raw / 1000.0,
        _ => raw,
    };
    Some((at, power_kw(kw)?))
}

// -------------------------------------------------------------- connection

trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

fn transient(m: impl Into<String>) -> SyncError {
    SyncError::transient(m)
}

fn tls_config() -> Result<rustls::ClientConfig, SyncError> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let fail = |_| transient("could not set up TLS for the broker");
    Ok(rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(fail)?
        .with_platform_verifier()
        .map_err(fail)?
        .with_no_client_auth())
}

async fn open(s: &MqttSettings, cancel: &CancellationToken) -> Result<Box<dyn Io>, SyncError> {
    let host = s.host.as_str();
    let port = s.port.unwrap_or(8883);
    let tls = s.tls.unwrap_or(true);
    let fut = async {
        let tcp = TcpStream::connect((host, port))
            .await
            .map_err(|e| transient(format!("could not connect to the broker ({})", e.kind())))?;
        let _ = tcp.set_nodelay(true);
        if !tls {
            return Ok(Box::new(tcp) as Box<dyn Io>);
        }
        let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| {
            SyncError::Config("energy: the broker host is not a valid TLS name".into())
        })?;
        let st = tokio_rustls::TlsConnector::from(Arc::new(tls_config()?))
            .connect(name, tcp)
            .await
            .map_err(|e| transient(format!("TLS with the broker failed: {e}")))?;
        Ok(Box::new(st) as Box<dyn Io>)
    };
    tokio::select! {
        _ = cancel.cancelled() => Err(SyncError::Cancelled),
        r = tokio::time::timeout(CONNECT_TIMEOUT, fut) => {
            r.map_err(|_| transient("timed out connecting to the broker"))?
        }
    }
}

struct Conn {
    io: Box<dyn Io>,
    buf: BytesMut,
    read: usize,
}

impl Conn {
    async fn send(&mut self, p: Packet) -> Result<(), SyncError> {
        let mut b = BytesMut::new();
        p.write(&mut b, MAX_PACKET_BYTES).map_err(|_| {
            SyncError::Config("energy: a packet for the broker is too large".into())
        })?;
        self.io
            .write_all(&b)
            .await
            .map_err(|e| transient(format!("writing to the broker failed ({})", e.kind())))?;
        self.io
            .flush()
            .await
            .map_err(|e| transient(format!("writing to the broker failed ({})", e.kind())))
    }

    /// The next packet, or `None` when `deadline` passes.
    async fn next(
        &mut self,
        deadline: Instant,
        cancel: &CancellationToken,
    ) -> Result<Option<Packet>, SyncError> {
        loop {
            match Packet::read(&mut self.buf, MAX_PACKET_BYTES) {
                Ok(p) => return Ok(Some(p)),
                Err(MqttError::InsufficientBytes(_)) => {}
                Err(MqttError::PayloadSizeLimitExceeded(_)) => {
                    return Err(transient(format!(
                        "the broker sent a message over {} KiB",
                        MAX_PACKET_BYTES / 1024
                    )));
                }
                Err(_) => return Err(transient("the broker sent a malformed packet")),
            }
            if self.read > MAX_READ_BYTES {
                return Err(transient("the broker sent too much data"));
            }
            self.buf.reserve(4096);
            let n = tokio::select! {
                _ = cancel.cancelled() => return Err(SyncError::Cancelled),
                r = tokio::time::timeout_at(deadline, self.io.read_buf(&mut self.buf)) => match r {
                    Err(_) => return Ok(None),
                    Ok(Ok(0)) => return Err(transient("the broker closed the connection")),
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => return Err(transient(format!("reading from the broker failed ({})", e.kind()))),
                },
            };
            self.read += n;
        }
    }
}

fn client_id() -> String {
    let mut b = [0u8; 8];
    let _ = getrandom::fill(&mut b);
    format!("halp-{}", hex::encode(b))
}

fn connack(a: &ConnAck, with_login: bool) -> Result<(), SyncError> {
    match a.code {
        ConnectReturnCode::Success => Ok(()),
        ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized if with_login => {
            Err(SyncError::Auth(
                "the broker refused the username or password; set the password again".into(),
            ))
        }
        ConnectReturnCode::BadUserNamePassword | ConnectReturnCode::NotAuthorized => Err(
            SyncError::Config("the broker needs a username and password".into()),
        ),
        ConnectReturnCode::ServiceUnavailable => Err(transient("the broker is unavailable")),
        ConnectReturnCode::RefusedProtocolVersion => Err(SyncError::Config(
            "the broker does not speak MQTT 3.1.1".into(),
        )),
        ConnectReturnCode::BadClientId => {
            Err(SyncError::Config("the broker refused the client id".into()))
        }
    }
}

/// Values per meter (index into `s.meters`), the latest of each.
async fn listen(
    s: &MqttSettings,
    password: Option<&str>,
    cancel: &CancellationToken,
    now: DateTime<Utc>,
) -> Result<Vec<Option<(DateTime<Utc>, f64)>>, SyncError> {
    let io = open(s, cancel).await?;
    let mut c = Conn {
        io,
        buf: BytesMut::with_capacity(4096),
        read: 0,
    };
    let mut hello = Connect::new(client_id());
    hello.keep_alive = KEEP_ALIVE_SECS;
    hello.clean_session = true;
    let with_login = s.username.is_some();
    if let Some(u) = &s.username {
        hello.set_login(u.as_str(), password.unwrap_or_default());
    }
    c.send(Packet::Connect(hello)).await?;
    let deadline = Instant::now() + CONNECT_TIMEOUT;
    match c.next(deadline, cancel).await? {
        Some(Packet::ConnAck(a)) => connack(&a, with_login)?,
        Some(_) => return Err(transient("the broker did not answer the connect")),
        None => return Err(transient("timed out waiting for the broker")),
    }
    let mut sub = Subscribe::new(s.meters[0].topic.clone(), QoS::AtMostOnce);
    for m in &s.meters[1..] {
        sub.add(m.topic.clone(), QoS::AtMostOnce);
    }
    sub.pkid = 1;
    c.send(Packet::Subscribe(sub)).await?;

    let wait = Duration::from_secs(s.wait_secs.unwrap_or(DEFAULT_WAIT_SECS));
    let deadline = Instant::now() + wait;
    let mut got: Vec<Option<(DateTime<Utc>, f64)>> = vec![None; s.meters.len()];
    let mut refused = vec![false; s.meters.len()];
    let mut acked = false;
    loop {
        let p = match c.next(deadline, cancel).await {
            Ok(Some(p)) => p,
            Ok(None) => break,
            // Keep what came before a broken or oversized packet.
            Err(e) if got.iter().any(Option::is_some) && !matches!(e, SyncError::Cancelled) => {
                break;
            }
            Err(e) => return Err(e),
        };
        match p {
            Packet::SubAck(a) if a.pkid == 1 => {
                acked = true;
                for (i, code) in a.return_codes.iter().enumerate().take(refused.len()) {
                    refused[i] = matches!(code, SubscribeReasonCode::Failure);
                }
                if refused.iter().all(|r| *r) {
                    return Err(SyncError::Config(
                        "the broker refused the subscription to every meter topic".into(),
                    ));
                }
            }
            Packet::Publish(p) => {
                let Some(i) = s.meters.iter().position(|m| m.topic == p.topic) else {
                    continue;
                };
                if let Some((at, kw)) =
                    parse_payload(&s.meters[i], &p.payload, p.retain, Utc::now(), now)
                    && got[i].is_none_or(|(old, _)| at >= old)
                {
                    got[i] = Some((at, kw));
                }
            }
            _ => {}
        }
        let all = got.iter().zip(&refused).all(|(g, r)| g.is_some() || *r);
        if acked && all {
            break;
        }
    }
    let _ = c.send(Packet::Disconnect).await;
    let _ = c.io.shutdown().await;
    Ok(got)
}

/// Readings for the meters that produced one this sync, and whether all
/// did. No reading at all is an error (the items keep their old time and
/// show as stale).
pub async fn readings(
    s: &MqttSettings,
    password: Option<&str>,
    cancel: &CancellationToken,
    now: DateTime<Utc>,
) -> Result<(Vec<Reading>, bool), SyncError> {
    if s.username.is_some() && password.is_none() {
        return Err(SyncError::Auth(
            "no broker password stored; add one in Settings".into(),
        ));
    }
    let got = listen(s, password, cancel, now).await?;
    let out: Vec<Reading> = s
        .meters
        .iter()
        .zip(got)
        .filter_map(|(m, g)| {
            let (at, kw) = g?;
            Some(Reading {
                source: "mqtt",
                zone: m.zone.clone(),
                zone_label: None,
                site: m.site.clone(),
                at,
                gco2: None,
                power_kw: Some(kw),
                renewable_pct: None,
                estimated: None,
                samples: Vec::new(),
            })
        })
        .collect();
    if out.is_empty() {
        return Err(transient(format!(
            "no usable meter reading arrived within {} s",
            s.wait_secs.unwrap_or(DEFAULT_WAIT_SECS)
        )));
    }
    let all = out.len() == s.meters.len();
    Ok((out, all))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn meter(pointer: Option<&str>, at: Option<&str>, unit: Option<&str>) -> Meter {
        Meter {
            topic: "site/main/power".into(),
            zone: "hq-main".into(),
            site: None,
            pointer: pointer.map(Into::into),
            at_pointer: at.map(Into::into),
            unit: unit.map(Into::into),
        }
    }

    fn settings(v: Value) -> Result<MqttSettings, String> {
        serde_json::from_value::<MqttSettings>(v)
            .map_err(|e| e.to_string())?
            .check()
    }

    #[test]
    fn plain_and_json_payloads() {
        let now = t("2026-11-11T12:00:00Z");
        let rx = t("2026-11-11T11:59:59.700Z");
        let plain = meter(None, None, None);
        assert_eq!(
            parse_payload(&plain, b" 12.5\n", false, rx, now),
            Some((t("2026-11-11T11:59:59Z"), 12.5))
        );
        for bad in [
            &b"NaN"[..],
            b"inf",
            b"-infinity",
            b"1e400",
            b"12 kW",
            b"",
            b"\xff",
        ] {
            assert_eq!(parse_payload(&plain, bad, false, rx, now), None, "{bad:?}");
        }
        assert_eq!(parse_payload(&plain, b"2000000", false, rx, now), None);
        // A retained plain number has no time: not used.
        assert_eq!(parse_payload(&plain, b"12.5", true, rx, now), None);

        let watts = meter(Some("/p"), None, Some("W"));
        assert_eq!(
            parse_payload(&watts, br#"{"p": 4500}"#, false, rx, now).map(|x| x.1),
            Some(4.5)
        );
        assert_eq!(
            parse_payload(&watts, br#"{"p": "x"}"#, false, rx, now),
            None
        );
        assert_eq!(parse_payload(&watts, br#"{"q": 1}"#, false, rx, now), None);
        assert_eq!(
            parse_payload(&watts, br#"{"p": 1e999}"#, false, rx, now),
            None
        );

        let stamped = meter(Some("/data/kw"), Some("/data/ts"), None);
        let p = json!({"data": {"kw": -3.2, "ts": "2026-11-11T11:00:00Z"}}).to_string();
        assert_eq!(
            parse_payload(&stamped, p.as_bytes(), true, rx, now),
            Some((t("2026-11-11T11:00:00Z"), -3.2))
        );
        let future = json!({"data": {"kw": 1, "ts": "2027-01-01T00:00:00Z"}}).to_string();
        assert_eq!(
            parse_payload(&stamped, future.as_bytes(), false, rx, now),
            None
        );
        let no_time = json!({"data": {"kw": 1}}).to_string();
        assert_eq!(
            parse_payload(&stamped, no_time.as_bytes(), false, rx, now),
            None
        );

        let big = format!(r#"{{"p": 1, "pad": "{}"}}"#, "x".repeat(MAX_PAYLOAD_BYTES));
        assert_eq!(parse_payload(&watts, big.as_bytes(), false, rx, now), None);
    }

    #[test]
    fn settings_require_tls_off_this_computer() {
        let m = json!([{"topic": "a/b", "zone": "hq"}]);
        let s = settings(json!({"host": "localhost", "meters": m})).unwrap();
        assert_eq!((s.tls, s.port), (Some(false), Some(1883)));
        let s = settings(json!({"host": "Broker.Example.com", "meters": m})).unwrap();
        assert_eq!(
            (s.tls, s.port, s.host.as_str()),
            (Some(true), Some(8883), "broker.example.com")
        );
        assert!(
            settings(json!({"host": "broker.example.com", "tls": false, "meters": m})).is_err()
        );
        assert!(settings(json!({"host": "192.168.1.5", "tls": false, "meters": m})).is_err());
        let s = settings(json!({"host": "[::1]", "tls": false, "meters": m})).unwrap();
        assert_eq!(s.host, "::1");
        assert!(settings(json!({"host": "mqtt://x", "meters": m})).is_err());
        assert!(settings(json!({"host": "x.com", "port": 0, "meters": m})).is_err());
        assert!(settings(json!({"host": "x.com", "meters": []})).is_err());
        assert!(settings(json!({"host": "x.com", "wait_secs": 99, "meters": m})).is_err());
        assert!(settings(json!({"host": "x.com", "username": "a b", "meters": m})).is_err());
        assert!(settings(json!({"host": "x.com", "password": "p", "meters": m})).is_err());
        for bad in [
            json!([{"topic": "a/+", "zone": "hq"}]),
            json!([{"topic": "a/#", "zone": "hq"}]),
            json!([{"topic": "a\u{202e}b", "zone": "hq"}]),
            json!([{"topic": "a", "zone": "h q"}]),
            json!([{"topic": "a", "zone": "hq"}, {"topic": "b", "zone": "hq"}]),
            json!([{"topic": "a", "zone": "hq", "pointer": "power"}]),
            json!([{"topic": "a", "zone": "hq", "at_pointer": "/ts"}]),
            json!([{"topic": "a", "zone": "hq", "unit": "MW"}]),
        ] {
            assert!(
                settings(json!({"host": "x.com", "meters": bad})).is_err(),
                "{bad}"
            );
        }
        let s = settings(json!({"host": "x.com", "meters":
            [{"topic": "a", "zone": "hq", "site": " HQ\u{200b} main ", "unit": "w"}]}))
        .unwrap();
        assert_eq!(s.meters[0].site.as_deref(), Some("HQ main"));
        assert_eq!(s.meters[0].unit.as_deref(), Some("W"));
    }
}
