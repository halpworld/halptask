//! Electricity Maps (API v3): carbon intensity per grid zone.
//!
//! Per zone, one sync asks for
//!
//! - `GET /v3/carbon-intensity/latest?zone=Z` →
//!   `{zone, carbonIntensity, datetime, updatedAt, isEstimated, …}`;
//! - with `history`: `GET /v3/carbon-intensity/history?zone=Z` →
//!   `{zone, history: [{carbonIntensity, datetime, …}]}` (the last 24 hours);
//! - with `renewable`: `GET /v3/power-breakdown/latest?zone=Z` →
//!   `{zone, datetime, renewablePercentage, fossilFreePercentage, …}`.
//!
//! The token goes in the `auth-token` header (marked sensitive). The
//! parser takes only those fields, checks every number, and never quotes
//! the body in an error.

use chrono::{DateTime, Duration, Utc};
use reqwest::Method;
use reqwest::StatusCode;
use reqwest::header::{ACCEPT, HeaderName, HeaderValue};
use serde_json::Value;
use tokio_util::sync::CancellationToken;
use url::Url;

use super::{EnergySettings, Reading, Sample, gco2, instant, number, percent};
use crate::connector::SyncError;
use crate::http::{Http, HttpResponse};

pub const DEFAULT_API_URL: &str = "https://api.electricitymaps.com/";
/// Response caps: one record, and a day of hourly records.
pub const MAX_LATEST_BYTES: usize = 64 * 1024;
pub const MAX_HISTORY_BYTES: usize = 512 * 1024;
/// History records looked at.
const MAX_HISTORY_RECORDS: usize = 200;
/// The renewable share counts when it is this close to the intensity's
/// time.
const RENEWABLE_SLACK_MINUTES: i64 = 90;

/// Checks the API base: https (loopback http only in tests), no
/// credentials, query or fragment. Returned with a trailing `/`.
pub fn check_api_url(raw: &str, http: &Http) -> Result<String, String> {
    let bad = || "energy: api_url must be an https URL without credentials or query".to_string();
    let mut u = Url::parse(raw.trim()).map_err(|_| bad())?;
    if u.query().is_some() || u.fragment().is_some() {
        return Err(bad());
    }
    http.check(&u).map_err(|_| bad())?;
    if !u.path().ends_with('/') {
        let p = format!("{}/", u.path());
        u.set_path(&p);
    }
    Ok(u.to_string())
}

/// The latest carbon intensity: `(at, gCO2eq/kWh, isEstimated)`. The
/// flag is kept only when the answer gives a boolean: estimated values are
/// modelled, not measured, and are often revised later.
pub fn parse_latest(
    v: &Value,
    zone: &str,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, f64, Option<bool>)> {
    if !zone_matches(v, zone) {
        return None;
    }
    let at = instant(v.get("datetime")?, now)?;
    let g = gco2(number(v.get("carbonIntensity")?)?)?;
    Some((at, g, v.get("isEstimated").and_then(Value::as_bool)))
}

/// History samples, oldest first; bad records are skipped.
pub fn parse_history(v: &Value, zone: &str, now: DateTime<Utc>) -> Vec<Sample> {
    if !zone_matches(v, zone) {
        return Vec::new();
    }
    let Some(a) = v.get("history").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut out: Vec<Sample> = a
        .iter()
        .take(MAX_HISTORY_RECORDS)
        .filter(|r| zone_matches(r, zone))
        .filter_map(|r| {
            let at = instant(r.get("datetime")?, now)?;
            let g = gco2(number(r.get("carbonIntensity")?)?)?;
            Some(Sample {
                at,
                gco2: Some(g),
                power_kw: None,
            })
        })
        .collect();
    out.sort_by_key(|s| s.at);
    out
}

/// The renewable share, when it describes (about) the same time as `at`.
pub fn parse_renewable(
    v: &Value,
    zone: &str,
    at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Option<f64> {
    if !zone_matches(v, zone) {
        return None;
    }
    let t = instant(v.get("datetime")?, now)?;
    if (t - at).abs() > Duration::minutes(RENEWABLE_SLACK_MINUTES) {
        return None;
    }
    percent(number(v.get("renewablePercentage")?)?)
}

/// A record names no zone, or the asked one.
fn zone_matches(v: &Value, zone: &str) -> bool {
    match v.get("zone") {
        None | Some(Value::Null) => v.is_object(),
        Some(Value::String(z)) => z.eq_ignore_ascii_case(zone),
        Some(_) => false,
    }
}

/// How one request ended.
enum Got {
    Json(Value),
    /// This zone cannot be read (unknown zone, not in the plan, bad
    /// answer); other zones may still work.
    Zone(String),
}

fn header_token(token: &str) -> Result<HeaderValue, SyncError> {
    let mut v = HeaderValue::from_str(token).map_err(|_| {
        SyncError::Auth("the stored Electricity Maps token is not usable; set it again".into())
    })?;
    v.set_sensitive(true);
    Ok(v)
}

fn retry_after(r: &HttpResponse) -> Option<std::time::Duration> {
    r.header("retry-after")
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|s| std::time::Duration::from_secs(s.clamp(1, 3600)))
}

async fn get(
    http: &Http,
    base: &Url,
    path: &str,
    zone: &str,
    token: &HeaderValue,
    max: usize,
    cancel: &CancellationToken,
) -> Result<Got, SyncError> {
    let mut u = base
        .join(path)
        .map_err(|_| SyncError::Config("energy: api_url is not usable".into()))?;
    u.query_pairs_mut().append_pair("zone", zone);
    let headers = [
        (ACCEPT, HeaderValue::from_static("application/json")),
        (HeaderName::from_static("auth-token"), token.clone()),
    ];
    let r = http
        .send_limited(Method::GET, &u, &headers, None, max, cancel)
        .await?;
    let code = r.status.as_u16();
    match r.status {
        s if s.is_success() => match r.json() {
            Ok(v) if v.is_object() => Ok(Got::Json(v)),
            _ => Ok(Got::Zone(format!(
                "Electricity Maps gave an unreadable answer for {zone}"
            ))),
        },
        StatusCode::UNAUTHORIZED => Err(SyncError::Auth(
            "Electricity Maps refused the token (HTTP 401); set it again".into(),
        )),
        StatusCode::TOO_MANY_REQUESTS => Err(SyncError::Transient {
            message: "Electricity Maps is rate limiting (HTTP 429)".into(),
            retry_after: retry_after(&r),
        }),
        s if s.is_server_error() => Err(SyncError::Transient {
            message: format!("Electricity Maps answered HTTP {code}"),
            retry_after: retry_after(&r),
        }),
        // 400, 403 (zone not in the plan), 404 (unknown zone), others.
        _ => Ok(Got::Zone(format!(
            "Electricity Maps refused zone {zone} (HTTP {code})"
        ))),
    }
}

/// Readings for every configured zone that has one. The flag is whether
/// all zones produced one. Fails only when no zone did (with the first
/// zone's reason), or at once on a rejected token, rate limit or server
/// error.
pub async fn readings(
    s: &EnergySettings,
    token: &str,
    http: &Http,
    cancel: &CancellationToken,
    now: DateTime<Utc>,
) -> Result<(Vec<Reading>, bool), SyncError> {
    let base = Url::parse(s.api_url.as_deref().unwrap_or(DEFAULT_API_URL))
        .map_err(|_| SyncError::Config("energy: api_url is not usable".into()))?;
    let token = header_token(token)?;
    let mut out = Vec::new();
    let mut first_problem: Option<String> = None;
    let mut forbidden = 0usize;
    for zone in &s.zones {
        let latest = match get(
            http,
            &base,
            "v3/carbon-intensity/latest",
            zone,
            &token,
            MAX_LATEST_BYTES,
            cancel,
        )
        .await?
        {
            Got::Json(v) => parse_latest(&v, zone, now).ok_or_else(|| {
                format!("Electricity Maps has no current carbon intensity for {zone}")
            }),
            Got::Zone(m) => {
                if m.contains("HTTP 403") {
                    forbidden += 1;
                }
                Err(m)
            }
        };
        let (at, g, estimated) = match latest {
            Ok(x) => x,
            Err(m) => {
                first_problem.get_or_insert(m);
                continue;
            }
        };
        let samples = if s.history {
            match get(
                http,
                &base,
                "v3/carbon-intensity/history",
                zone,
                &token,
                MAX_HISTORY_BYTES,
                cancel,
            )
            .await?
            {
                Got::Json(v) => parse_history(&v, zone, now),
                Got::Zone(_) => Vec::new(),
            }
        } else {
            Vec::new()
        };
        let renewable_pct = if s.renewable {
            match get(
                http,
                &base,
                "v3/power-breakdown/latest",
                zone,
                &token,
                MAX_LATEST_BYTES,
                cancel,
            )
            .await?
            {
                Got::Json(v) => parse_renewable(&v, zone, at, now),
                Got::Zone(_) => None,
            }
        } else {
            None
        };
        out.push(Reading {
            source: "electricity_maps",
            zone: zone.clone(),
            zone_label: s.zone_labels.get(zone).cloned(),
            site: None,
            at,
            gco2: Some(g),
            power_kw: None,
            renewable_pct,
            estimated,
            samples,
        });
    }
    if out.is_empty() {
        let m = first_problem.unwrap_or_else(|| "no zones configured".into());
        return Err(if forbidden == s.zones.len() {
            SyncError::Auth(format!("{m}; check the token's plan"))
        } else {
            SyncError::Config(m)
        });
    }
    let all = out.len() == s.zones.len();
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

    #[test]
    fn latest_is_read_defensively() {
        let now = t("2026-11-11T12:30:00Z");
        let ok = json!({"zone": "DE", "carbonIntensity": 302, "datetime": "2026-11-11T12:00:00.000Z",
                        "updatedAt": "2026-11-11T12:10:00.000Z", "isEstimated": true});
        assert_eq!(
            parse_latest(&ok, "DE", now),
            Some((t("2026-11-11T12:00:00Z"), 302.0, Some(true)))
        );
        let mut measured = ok.clone();
        measured["isEstimated"] = json!(false);
        assert_eq!(parse_latest(&measured, "DE", now).unwrap().2, Some(false));
        measured["isEstimated"] = json!("yes");
        assert_eq!(parse_latest(&measured, "DE", now).unwrap().2, None);
        measured.as_object_mut().unwrap().remove("isEstimated");
        assert_eq!(parse_latest(&measured, "DE", now).unwrap().2, None);
        assert_eq!(parse_latest(&ok, "FR", now), None, "another zone's answer");
        for bad in [
            json!({"zone": "DE", "carbonIntensity": null, "datetime": "2026-11-11T12:00:00Z"}),
            json!({"zone": "DE", "carbonIntensity": -3, "datetime": "2026-11-11T12:00:00Z"}),
            json!({"zone": "DE", "carbonIntensity": 1e300, "datetime": "2026-11-11T12:00:00Z"}),
            json!({"zone": "DE", "carbonIntensity": "NaN", "datetime": "2026-11-11T12:00:00Z"}),
            json!({"zone": "DE", "carbonIntensity": 300, "datetime": "2030-01-01T00:00:00Z"}),
            json!({"zone": "DE", "carbonIntensity": 300}),
            json!({"zone": 7, "carbonIntensity": 300, "datetime": "2026-11-11T12:00:00Z"}),
            json!([1, 2]),
        ] {
            assert_eq!(parse_latest(&bad, "DE", now), None, "{bad}");
        }
    }

    #[test]
    fn history_skips_bad_records() {
        let now = t("2026-11-11T12:30:00Z");
        let v = json!({"zone": "DE", "history": [
            {"zone": "DE", "carbonIntensity": 310, "datetime": "2026-11-11T11:00:00.000Z"},
            {"zone": "DE", "carbonIntensity": 300, "datetime": "2026-11-11T10:00:00.000Z"},
            {"zone": "DE", "carbonIntensity": null, "datetime": "2026-11-11T09:00:00.000Z"},
            {"zone": "FR", "carbonIntensity": 50, "datetime": "2026-11-11T08:00:00.000Z"},
            "junk"
        ]});
        let h = parse_history(&v, "DE", now);
        assert_eq!(h.len(), 2);
        assert_eq!(h[0].at, t("2026-11-11T10:00:00Z"));
        assert_eq!(h[1].gco2, Some(310.0));
        assert!(parse_history(&json!({"history": "x"}), "DE", now).is_empty());
    }

    #[test]
    fn renewable_share_must_match_the_reading_time() {
        let now = t("2026-11-11T12:30:00Z");
        let at = t("2026-11-11T12:00:00Z");
        let v =
            json!({"zone": "DE", "datetime": "2026-11-11T12:00:00Z", "renewablePercentage": 61.27});
        assert_eq!(parse_renewable(&v, "DE", at, now), Some(61.3));
        let old =
            json!({"zone": "DE", "datetime": "2026-11-10T12:00:00Z", "renewablePercentage": 61});
        assert_eq!(parse_renewable(&old, "DE", at, now), None);
        let bad =
            json!({"zone": "DE", "datetime": "2026-11-11T12:00:00Z", "renewablePercentage": 140});
        assert_eq!(parse_renewable(&bad, "DE", at, now), None);
    }
}
