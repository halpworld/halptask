//! Grid carbon and site power as sources (`energy` kind): one
//! `cso.energy_reading` item per zone or meter, for the CSO pack's
//! grid-carbon widget.
//!
//! Two sources:
//!
//! - `electricity_maps`: the Electricity Maps API ([`emaps`]), carbon
//!   intensity per grid zone (and the renewable share). The API token is
//!   the keychain secret (`integrations_set_token`); it is sent only in the
//!   `auth-token` header and never appears in settings, items, events or
//!   status text.
//! - `mqtt`: a site energy meter on an MQTT broker ([`mqtt`]), power in kW.
//!   The broker password (when a `username` is set) is the secret.
//!
//! Nothing is invented: a zone or meter without a usable reading this sync
//! is left out of the batch, so its item keeps its old `at` and the widget
//! shows it as stale. Batches are `complete` only when every configured
//! zone or meter produced a reading, so items of zones removed from the
//! settings are archived then, and never because a source was quiet.

pub mod emaps;
pub mod mqtt;

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::calendar::clean;
use crate::connector::{Batch, BoxFuture, Connector, SyncCx, SyncError};
use crate::http::Http;
use crate::secrets::check_user_token;
use halp_domain::integration::strip_controls;

pub use mqtt::{Meter, MqttSettings};

pub const ENERGY_READING: &str = "cso.energy_reading";
/// Samples kept in an item's rolling `series`.
pub const MAX_SERIES: usize = 48;
/// Zones (Electricity Maps) or meters (MQTT) per integration.
pub const MAX_ZONES: usize = 10;
/// The schema's bounds.
pub const MAX_GCO2: f64 = 3000.0;
pub const MAX_POWER_KW: f64 = 1_000_000.0;
/// Readings more than this far in the future are refused.
pub const FUTURE_SLACK_MINUTES: i64 = 5;
/// `tasks.list`'s largest page, for the previous readings (no paging).
const PREVIOUS_LIMIT: u32 = 1000;
const ROLE: &str = "cso";

// ----------------------------------------------------------------- numbers

/// A finite number within `lo..=hi`, or nothing.
pub fn in_range(v: f64, lo: f64, hi: f64) -> Option<f64> {
    (v.is_finite() && (lo..=hi).contains(&v)).then_some(v)
}

pub fn gco2(v: f64) -> Option<f64> {
    in_range(v, 0.0, MAX_GCO2).map(|x| round(x, 1))
}

pub fn power_kw(v: f64) -> Option<f64> {
    in_range(v, -MAX_POWER_KW, MAX_POWER_KW).map(|x| round(x, 3))
}

pub fn percent(v: f64) -> Option<f64> {
    in_range(v, 0.0, 100.0).map(|x| round(x, 1))
}

fn round(v: f64, places: i32) -> f64 {
    let f = 10f64.powi(places);
    let r = (v * f).round() / f;
    if r == 0.0 { 0.0 } else { r }
}

/// A JSON value as a finite number (a number, or a string holding one).
pub(crate) fn number(v: &Value) -> Option<f64> {
    let f = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) if s.len() <= 64 => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    f.is_finite().then_some(f)
}

/// A timestamp from text (RFC 3339) or a number (epoch seconds, or
/// milliseconds when it is that large), not further than a few minutes in
/// the future.
pub(crate) fn instant(v: &Value, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let t = any_instant(v)?;
    (t <= now + Duration::minutes(FUTURE_SLACK_MINUTES)).then_some(t)
}

/// [`instant`] without the check against the future.
fn any_instant(v: &Value) -> Option<DateTime<Utc>> {
    let t = match v {
        Value::String(s) if s.len() <= 64 => DateTime::parse_from_rfc3339(s.trim())
            .ok()?
            .with_timezone(&Utc),
        Value::Number(n) => {
            let f = n.as_f64()?;
            let secs = if f > 1e11 { f / 1000.0 } else { f };
            if !(0.0..1e11).contains(&secs) {
                return None;
            }
            DateTime::from_timestamp(secs as i64, 0)?
        }
        _ => return None,
    };
    DateTime::from_timestamp(t.timestamp(), 0)
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

/// Free text from settings or a source: controls, bidi and other format
/// characters removed, whitespace collapsed, at most `max` characters.
pub(crate) fn text(s: &str, max: usize) -> String {
    clean(&strip_controls(s), max, false)
}

/// A zone or meter key as the item schema allows it.
pub fn valid_zone(z: &str) -> bool {
    (1..=64).contains(&z.len())
        && z.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b':' | b'-'))
}

// ------------------------------------------------------------------ series

/// One sample of an item's `series`.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub at: DateTime<Utc>,
    pub gco2: Option<f64>,
    pub power_kw: Option<f64>,
}

impl Sample {
    fn to_json(&self) -> Value {
        let mut m = Map::new();
        m.insert("at".into(), json!(rfc3339(self.at)));
        if let Some(g) = self.gco2 {
            m.insert("gco2_per_kwh".into(), json!(g));
        }
        if let Some(p) = self.power_kw {
            m.insert("power_kw".into(), json!(p));
        }
        Value::Object(m)
    }
}

/// The samples of an existing item's `series`, re-checked (bad entries
/// dropped).
pub fn read_series(item: &Value, now: DateTime<Utc>) -> Vec<Sample> {
    let Some(a) = item.pointer("/fields/series").and_then(Value::as_array) else {
        return Vec::new();
    };
    a.iter()
        .take(4 * MAX_SERIES)
        .filter_map(|s| {
            let at = instant(s.get("at")?, now)?;
            let gco2 = s.get("gco2_per_kwh").and_then(number).and_then(gco2);
            let power_kw = s.get("power_kw").and_then(number).and_then(power_kw);
            (gco2.is_some() || power_kw.is_some()).then_some(Sample { at, gco2, power_kw })
        })
        .collect()
}

/// `old` and `new` merged by timestamp (`new` wins), oldest first, the
/// last [`MAX_SERIES`] kept.
pub fn merge_series(old: Vec<Sample>, new: Vec<Sample>, now: DateTime<Utc>) -> Vec<Sample> {
    let limit = now + Duration::minutes(FUTURE_SLACK_MINUTES);
    let mut by: BTreeMap<DateTime<Utc>, Sample> = BTreeMap::new();
    for s in old.into_iter().chain(new) {
        if s.at <= limit && (s.gco2.is_some() || s.power_kw.is_some()) {
            by.insert(s.at, s);
        }
    }
    let n = by.len().saturating_sub(MAX_SERIES);
    by.into_values().skip(n).collect()
}

// ----------------------------------------------------------------- reading

/// What one zone or meter reports this sync. Built only from values the
/// source gave; an absent value stays absent.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// `electricity_maps` or `mqtt`.
    pub source: &'static str,
    pub zone: String,
    pub zone_label: Option<String>,
    pub site: Option<String>,
    pub at: DateTime<Utc>,
    pub gco2: Option<f64>,
    pub power_kw: Option<f64>,
    pub renewable_pct: Option<f64>,
    /// The source says the value is estimated (`true`) or measured
    /// (`false`); absent when it does not say.
    pub estimated: Option<bool>,
    /// New samples (history), besides the reading itself.
    pub samples: Vec<Sample>,
}

/// This integration's existing item for `(source, zone)`.
pub fn existing<'a>(
    items: &'a [Value],
    origin: &str,
    source: &str,
    zone: &str,
) -> Option<&'a Value> {
    items.iter().find(|i| {
        i.get("origin").and_then(Value::as_str) == Some(origin)
            && i.get("type_id").and_then(Value::as_str) == Some(ENERGY_READING)
            && i.pointer("/fields/source").and_then(Value::as_str) == Some(source)
            && i.pointer("/fields/zone").and_then(Value::as_str) == Some(zone)
    })
}

/// The `ext.upsert` item for a reading, its series merged with the
/// previous item's. `None` when the reading is older than what the item
/// already shows (a stale retained message), so nothing goes backwards.
pub fn item(
    r: &Reading,
    previous: Option<&Value>,
    roles: &[String],
    now: DateTime<Utc>,
) -> Option<Value> {
    // Any stored time counts here, even one in the future (the clock went
    // back): `instant` would refuse it and let an older reading through.
    let old_at = previous
        .and_then(|p| p.pointer("/fields/at"))
        .and_then(any_instant);
    if old_at.is_some_and(|o| r.at < o) {
        return None;
    }
    let mut new = r.samples.clone();
    if r.gco2.is_some() || r.power_kw.is_some() {
        new.push(Sample {
            at: r.at,
            gco2: r.gco2,
            power_kw: r.power_kw,
        });
    }
    let old = previous.map(|p| read_series(p, now)).unwrap_or_default();
    let series = merge_series(old, new, now);

    let mut f = Map::new();
    f.insert("source".into(), json!(r.source));
    f.insert("zone".into(), json!(r.zone));
    if let Some(l) = &r.zone_label {
        f.insert("zone_label".into(), json!(l));
    }
    if let Some(s) = &r.site {
        f.insert("site".into(), json!(s));
    }
    f.insert("at".into(), json!(rfc3339(r.at)));
    if let Some(g) = r.gco2 {
        f.insert("gco2_per_kwh".into(), json!(g));
    }
    if let Some(p) = r.power_kw {
        f.insert("power_kw".into(), json!(p));
    }
    if let Some(p) = r.renewable_pct {
        f.insert("renewable_pct".into(), json!(p));
    }
    if let Some(e) = r.estimated {
        f.insert("estimated".into(), json!(e));
    }
    f.insert(
        "series".into(),
        Value::Array(series.iter().map(Sample::to_json).collect()),
    );
    let (prefix, what) = match r.source {
        "mqtt" => ("mqtt", "Site power"),
        _ => ("emaps", "Grid carbon"),
    };
    let name = r
        .site
        .as_deref()
        .or(r.zone_label.as_deref())
        .unwrap_or(&r.zone);
    let mut v = json!({
        "external_id": format!("{prefix}:{}", r.zone),
        "title": text(&format!("{what} {name}"), 300),
        "kind": "bullet",
        "type_id": ENERGY_READING,
        "tags": ["energy"],
        "fields": f,
    });
    if roles.iter().any(|x| x == ROLE) {
        v["roles"] = json!([ROLE]);
    }
    Some(v)
}

/// One batch of the readings that are there. `all` is whether every
/// configured zone or meter produced one (only then is it complete).
pub fn batch(
    readings: &[Reading],
    all: bool,
    previous: &[Value],
    origin: &str,
    roles: &[String],
    now: DateTime<Utc>,
) -> Batch {
    // `tasks.list` has no paging: a full page may lack this integration's
    // item, and writing without it would cut its series. Only readings
    // whose item was found are written then, and nothing is archived.
    let capped = previous.len() >= PREVIOUS_LIMIT as usize;
    let items = readings
        .iter()
        .filter_map(|r| {
            let prev = existing(previous, origin, r.source, &r.zone);
            if capped && prev.is_none() {
                return None;
            }
            item(r, prev, roles, now)
        })
        .collect::<Vec<_>>();
    Batch {
        scope: "readings".into(),
        complete: all && !capped && items.len() == readings.len(),
        items,
        patches: Vec::new(),
    }
}

// ---------------------------------------------------------------- settings

fn yes() -> bool {
    true
}

/// `settings` of an `energy` integration. Holds no secret: the
/// Electricity Maps token and the MQTT password live in the keychain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnergySettings {
    /// `electricity_maps` or `mqtt`.
    pub source: String,
    /// `electricity_maps`: the grid zones (`DE`, `SE-SE3`, `US-CAL-CISO`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zones: Vec<String>,
    /// `electricity_maps`: display names per zone.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub zone_labels: BTreeMap<String, String>,
    /// `electricity_maps`: the API base (default
    /// `https://api.electricitymaps.com`); https only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    /// `electricity_maps`: also fetch the last 24 hours (one request per
    /// zone; default true).
    #[serde(default = "yes")]
    pub history: bool,
    /// `electricity_maps`: also fetch the renewable share (one request per
    /// zone; default true).
    #[serde(default = "yes")]
    pub renewable: bool,
    /// `mqtt`: the broker and its meters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mqtt: Option<MqttSettings>,
}

impl EnergySettings {
    pub fn parse(v: &Value) -> Result<EnergySettings, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("energy settings: {e}"))
    }

    /// Checks and normalises.
    pub fn check(mut self, http: &Http) -> Result<EnergySettings, String> {
        match self.source.as_str() {
            "electricity_maps" => {
                if self.mqtt.is_some() {
                    return Err("energy: mqtt is for source mqtt".into());
                }
                if self.zones.is_empty() || self.zones.len() > MAX_ZONES {
                    return Err(format!("energy: list 1 to {MAX_ZONES} zones"));
                }
                let mut zones: Vec<String> = Vec::new();
                for z in &self.zones {
                    let z = z.trim().to_string();
                    if !valid_zone(&z) {
                        return Err(format!(
                            "energy: {:?} is not a zone (letters, digits and _ . / : -)",
                            text(&z, 64)
                        ));
                    }
                    if !zones.contains(&z) {
                        zones.push(z);
                    }
                }
                let mut labels = BTreeMap::new();
                for (z, l) in &self.zone_labels {
                    if !zones.contains(z) {
                        return Err(format!(
                            "energy: zone_labels names {:?}, which is not in zones",
                            text(z, 64)
                        ));
                    }
                    let l = text(l, 120);
                    if !l.is_empty() {
                        labels.insert(z.clone(), l);
                    }
                }
                self.zones = zones;
                self.zone_labels = labels;
                if let Some(u) = &self.api_url {
                    self.api_url = Some(emaps::check_api_url(u, http)?);
                }
            }
            "mqtt" => {
                if !self.zones.is_empty() || !self.zone_labels.is_empty() || self.api_url.is_some()
                {
                    return Err(
                        "energy: zones, zone_labels and api_url are for source electricity_maps"
                            .into(),
                    );
                }
                let Some(m) = self.mqtt.take() else {
                    return Err("energy: source mqtt needs mqtt settings".into());
                };
                self.mqtt = Some(m.check()?);
                self.history = true;
                self.renewable = true;
            }
            _ => return Err("energy: source is electricity_maps or mqtt".into()),
        }
        Ok(self)
    }

    fn needs_secret(&self) -> bool {
        match (self.source.as_str(), &self.mqtt) {
            ("mqtt", Some(m)) => m.username.is_some(),
            ("mqtt", None) => false,
            _ => true,
        }
    }
}

// --------------------------------------------------------------- connector

/// The `energy` connector.
pub struct Energy;

impl Energy {
    async fn run(&self, cx: SyncCx<'_>) -> Result<Vec<Batch>, SyncError> {
        let s = EnergySettings::parse(cx.settings)
            .and_then(|s| s.check(cx.http))
            .map_err(SyncError::Config)?;
        let secret = cx.token;
        let known: Vec<&str> = secret.into_iter().collect();
        let (readings, all) = match (s.source.as_str(), &s.mqtt) {
            ("mqtt", Some(m)) => mqtt::readings(m, secret, cx.cancel, cx.now)
                .await
                .map_err(|e| e.redacted(&known))?,
            _ => {
                let Some(token) = secret else {
                    return Err(SyncError::Auth(
                        "no Electricity Maps token stored; add one in Settings".into(),
                    ));
                };
                emaps::readings(&s, token, cx.http, cx.cancel, cx.now)
                    .await
                    .map_err(|e| e.redacted(&known))?
            }
        };
        if readings.is_empty() {
            return Ok(Vec::new());
        }
        // The previous items carry the rolling series. Without them a
        // write would cut the series short, so a failed read skips this
        // sync's write (the readings come again next time).
        let previous = cx
            .items
            .list(json!({ "type_id": ENERGY_READING, "limit": PREVIOUS_LIMIT }))
            .await?;
        let origin = format!("integration:{}", cx.id);
        let b = batch(&readings, all, &previous, &origin, cx.roles, cx.now);
        Ok(if b.items.is_empty() && !b.complete {
            Vec::new()
        } else {
            vec![b]
        })
    }
}

impl Connector for Energy {
    fn kind(&self) -> &'static str {
        "energy"
    }

    fn validate(&self, settings: &Value, http: &Http) -> Result<Value, String> {
        let s = EnergySettings::parse(settings)?.check(http)?;
        serde_json::to_value(&s).map_err(|e| e.to_string())
    }

    fn needs_token(&self, settings: &Value) -> bool {
        EnergySettings::parse(settings).map_or(true, |s| s.needs_secret())
    }

    fn secret_audience(&self, settings: &Value) -> String {
        match EnergySettings::parse(settings) {
            Ok(s) => match (s.source.as_str(), &s.mqtt) {
                ("mqtt", Some(m)) => format!(
                    "mqtt:{}:{}",
                    m.host.to_ascii_lowercase(),
                    m.port.map(|p| p.to_string()).unwrap_or_default()
                ),
                (src, _) => format!("{src}:{}", s.api_url.as_deref().unwrap_or("")),
            },
            Err(_) => "invalid".into(),
        }
    }

    fn check_secret(
        &self,
        _settings: &Value,
        secret: &str,
        _http: &Http,
    ) -> Result<String, &'static str> {
        let secret = secret.trim();
        check_user_token(secret)?;
        Ok(secret.to_string())
    }

    fn sync<'a>(&'a self, cx: SyncCx<'a>) -> BoxFuture<'a, Result<Vec<Batch>, SyncError>> {
        Box::pin(self.run(cx))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn reading(at: &str, g: f64) -> Reading {
        Reading {
            source: "electricity_maps",
            zone: "DE".into(),
            zone_label: Some("Germany".into()),
            site: None,
            at: t(at),
            gco2: Some(g),
            power_kw: None,
            renewable_pct: None,
            estimated: None,
            samples: Vec::new(),
        }
    }

    #[test]
    fn numbers_must_be_finite_and_in_range() {
        assert_eq!(gco2(f64::NAN), None);
        assert_eq!(gco2(f64::INFINITY), None);
        assert_eq!(gco2(-1.0), None);
        assert_eq!(gco2(3000.1), None);
        assert_eq!(gco2(302.04), Some(302.0));
        assert_eq!(power_kw(1e7), None);
        assert_eq!(power_kw(-12.34567), Some(-12.346));
        assert_eq!(percent(100.5), None);
        assert_eq!(number(&json!("1e400")), None);
        assert_eq!(number(&json!("NaN")), None);
        assert_eq!(number(&json!("inf")), None);
        assert_eq!(number(&json!(" 4.5 ")), Some(4.5));
        assert_eq!(number(&json!(true)), None);
    }

    #[test]
    fn instants_refuse_the_future_and_nonsense() {
        let now = t("2026-11-11T12:00:00Z");
        assert_eq!(
            instant(&json!("2026-11-11T11:00:00.350Z"), now),
            Some(t("2026-11-11T11:00:00Z"))
        );
        assert_eq!(instant(&json!("2026-11-11T13:00:00Z"), now), None);
        assert_eq!(
            instant(&json!(1_794_398_400_000_i64), now),
            Some(t("2026-11-11T12:00:00Z"))
        );
        assert_eq!(instant(&json!(-5), now), None);
        assert_eq!(instant(&json!("yesterday"), now), None);
    }

    #[test]
    fn the_series_is_rolling_and_capped() {
        let now = t("2026-11-11T12:00:00Z");
        let old: Vec<Sample> = (0..60)
            .map(|i| Sample {
                at: now - Duration::hours(60 - i),
                gco2: Some(100.0 + i as f64),
                power_kw: None,
            })
            .collect();
        let new = vec![
            Sample {
                at: now,
                gco2: Some(1.0),
                power_kw: None,
            },
            // The same hour again from history: the newer value wins.
            Sample {
                at: now - Duration::hours(1),
                gco2: Some(2.0),
                power_kw: None,
            },
            // Future and empty samples are dropped.
            Sample {
                at: now + Duration::hours(2),
                gco2: Some(3.0),
                power_kw: None,
            },
            Sample {
                at: now - Duration::minutes(30),
                gco2: None,
                power_kw: None,
            },
        ];
        let s = merge_series(old, new, now);
        assert_eq!(s.len(), MAX_SERIES);
        assert_eq!(s.last().unwrap().at, now);
        assert_eq!(s.last().unwrap().gco2, Some(1.0));
        assert_eq!(s[MAX_SERIES - 2].gco2, Some(2.0));
        assert!(s.windows(2).all(|w| w[0].at < w[1].at));
    }

    #[test]
    fn items_merge_with_the_previous_series_and_never_go_back() {
        let now = t("2026-11-11T12:00:00Z");
        let first = item(&reading("2026-11-11T10:00:00Z", 300.0), None, &[], now).unwrap();
        assert_eq!(first["external_id"], "emaps:DE");
        assert_eq!(first["title"], "Grid carbon Germany");
        assert_eq!(first["fields"]["gco2_per_kwh"], 300.0);
        assert_eq!(first["fields"]["at"], "2026-11-11T10:00:00Z");
        assert!(first.get("roles").is_none());
        assert!(first["fields"].get("renewable_pct").is_none());
        let mut prev = first.clone();
        prev["origin"] = json!("integration:e");
        let second = item(
            &reading("2026-11-11T11:00:00Z", 250.0),
            Some(&prev),
            &["cso".into()],
            now,
        )
        .unwrap();
        assert_eq!(second["fields"]["series"].as_array().unwrap().len(), 2);
        assert_eq!(second["roles"], json!(["cso"]));
        // An older reading than the item shows changes nothing.
        assert!(item(&reading("2026-11-11T09:00:00Z", 1.0), Some(&prev), &[], now).is_none());
        // A hostile previous series is re-checked.
        prev["fields"]["series"] = json!([
            {"at": "2026-11-11T09:00:00Z", "gco2_per_kwh": 1e9},
            {"at": "nope", "gco2_per_kwh": 5},
            {"at": "2026-11-11T08:00:00Z", "gco2_per_kwh": 5}
        ]);
        let third = item(
            &reading("2026-11-11T11:00:00Z", 250.0),
            Some(&prev),
            &[],
            now,
        )
        .unwrap();
        assert_eq!(
            third["fields"]["series"],
            json!([
                {"at": "2026-11-11T08:00:00Z", "gco2_per_kwh": 5.0},
                {"at": "2026-11-11T11:00:00Z", "gco2_per_kwh": 250.0}
            ])
        );
    }

    #[test]
    fn batches_are_complete_only_with_every_reading() {
        let now = t("2026-11-11T12:00:00Z");
        let r = [reading("2026-11-11T10:00:00Z", 300.0)];
        assert!(batch(&r, true, &[], "integration:e", &[], now).complete);
        assert!(!batch(&r, false, &[], "integration:e", &[], now).complete);
        // A full page of other items may hide this one: not written, not
        // complete. Found ones are still written.
        let other = json!({"origin": "integration:x", "type_id": ENERGY_READING,
                           "fields": {"source": "electricity_maps", "zone": "FR"}});
        let mut full = vec![other; PREVIOUS_LIMIT as usize];
        let b = batch(&r, true, &full, "integration:e", &[], now);
        assert!(!b.complete && b.items.is_empty());
        full[0] = json!({"origin": "integration:e", "type_id": ENERGY_READING,
                         "fields": {"source": "electricity_maps", "zone": "DE",
                                    "at": "2026-11-11T09:00:00Z"}});
        let b = batch(&r, true, &full, "integration:e", &[], now);
        assert!(!b.complete);
        assert_eq!(b.items.len(), 1);
    }

    #[test]
    fn a_future_stored_time_still_blocks_older_readings() {
        // The clock went back: the item shows a time ahead of `now`.
        let now = t("2026-11-11T12:00:00Z");
        let prev = json!({"fields": {"source": "electricity_maps", "zone": "DE",
                                     "at": "2026-11-11T14:00:00Z"}});
        assert!(item(&reading("2026-11-11T11:00:00Z", 1.0), Some(&prev), &[], now).is_none());
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(
            text("Ber\u{202e}lin\u{00ad} \u{200b}HQ\n", 120),
            "Berlin HQ"
        );
        assert!(valid_zone("US-CAL-CISO"));
        assert!(!valid_zone("DE DE"));
        assert!(!valid_zone(""));
        assert!(!valid_zone(&"a".repeat(65)));
    }
}
