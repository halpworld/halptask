//! Calendar MCP servers' list-events answers → [`Event`]s. Two shapes are
//! understood, plus a plain one:
//!
//! - Google Calendar (`events.list` / `instances`): `summary`,
//!   `start.dateTime|date` (+ `timeZone`), `attendees[].self`,
//!   `responseStatus`, `resource`, `status: cancelled`, `transparency`,
//!   `eventType` (`focusTime`, `outOfOffice`), `hangoutLink`,
//!   `conferenceData`, `visibility`, `recurringEventId`.
//! - Microsoft Graph (`calendarView`): `subject`, `start {dateTime,
//!   timeZone}` (Windows zone names too), `isAllDay`, `isCancelled`,
//!   `showAs`, `sensitivity`, `attendees[].emailAddress`, `type:
//!   resource`, `status.response`, `onlineMeeting.joinUrl`,
//!   `seriesMasterId`, `isOrganizer`.
//! - Plain: `id`, `title`, `start`, `end` as RFC 3339 strings.
//!
//! Everything is untrusted text: cleaned, capped, and only `https` links
//! kept. Cancelled events and Google `workingLocation` entries are left
//! out (so their items get archived).

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, Utc};
use rrule::Tz;
use serde_json::Value;

use super::{Attendee, Event, Hint, clean, conference_url, hash_key, local, normal_email, window};
use crate::connector::https_url;

const MAX_ATTENDEES: usize = 100;

fn s<'a>(v: &'a Value, path: &str) -> Option<&'a str> {
    v.pointer(path)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|x| !x.is_empty())
}

fn first<'a>(v: &'a Value, paths: &[&str]) -> Option<&'a str> {
    paths.iter().find_map(|p| s(v, p))
}

fn flag(v: &Value, path: &str) -> bool {
    v.pointer(path).and_then(Value::as_bool).unwrap_or(false)
}

/// A start or end: `{dateTime, timeZone}`, `{date}` or a string. The bool
/// says it was a date (all-day).
fn moment(v: Option<&Value>, zone: Tz) -> Option<(DateTime<Utc>, bool)> {
    let v = v?;
    let (dt, date, tz) = match v {
        Value::String(x) => (Some(x.as_str()), None, None),
        Value::Object(_) => (
            s(v, "/dateTime"),
            s(v, "/date"),
            s(v, "/timeZone").or_else(|| s(v, "/time_zone")),
        ),
        _ => return None,
    };
    if let Some(d) = date
        && let Ok(day) = NaiveDate::parse_from_str(d, "%Y-%m-%d")
    {
        return local(zone, day.and_hms_opt(0, 0, 0)?).map(|t| (t.with_timezone(&Utc), true));
    }
    let x = dt?;
    if let Ok(t) = DateTime::parse_from_rfc3339(x) {
        return Some((t.with_timezone(&Utc), false));
    }
    if let Ok(day) = NaiveDate::parse_from_str(x, "%Y-%m-%d") {
        return local(zone, day.and_hms_opt(0, 0, 0)?).map(|t| (t.with_timezone(&Utc), true));
    }
    // Graph: `2026-11-10T09:00:00.0000000` in `timeZone`.
    let n = NaiveDateTime::parse_from_str(x, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(x, "%Y-%m-%dT%H:%M"))
        .ok()?;
    let z = tz.and_then(super::tz::resolve).unwrap_or(zone);
    local(z, n).map(|t| (t.with_timezone(&Utc), false))
}

fn attendees(r: &Value) -> (Vec<Attendee>, u32) {
    let mut out = Vec::new();
    let mut count = 0u32;
    for a in r
        .get("attendees")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .take(10_000)
    {
        let email =
            first(a, &["/email", "/emailAddress/address", "/address"]).and_then(normal_email);
        let name = first(a, &["/displayName", "/emailAddress/name", "/name"])
            .map(|n| clean(n, 120, false))
            .filter(|n| !n.is_empty());
        let resource = flag(a, "/resource")
            || s(a, "/type").is_some_and(|t| t.eq_ignore_ascii_case("resource"));
        let response = first(a, &["/responseStatus", "/status/response", "/response"])
            .unwrap_or("")
            .to_ascii_lowercase();
        if !resource {
            count = count.saturating_add(1);
        }
        if out.len() < MAX_ATTENDEES {
            out.push(Attendee {
                email,
                name,
                declined: response == "declined",
                resource,
                is_self: flag(a, "/self"),
            });
        }
    }
    (out, count.min(10_000))
}

/// One record, if it is a meeting to show.
pub fn event(r: &Value, zone: Tz) -> Option<Event> {
    let cancelled = s(r, "/status").is_some_and(|x| x.eq_ignore_ascii_case("cancelled"))
        || flag(r, "/isCancelled");
    let kind = s(r, "/eventType").unwrap_or("");
    if cancelled || kind == "workingLocation" || s(r, "/type") == Some("seriesMaster") {
        return None;
    }
    let (start, all_day_start) = moment(r.get("start"), zone)?;
    let all_day = all_day_start || flag(r, "/isAllDay");
    let end = moment(r.get("end"), zone)
        .map(|(t, _)| t)
        .filter(|e| *e >= start)
        .unwrap_or(if all_day {
            start + Duration::days(1)
        } else {
            start
        });
    if end - start > Duration::days(366) {
        return None;
    }
    let id = first(r, &["/id", "/iCalUId", "/iCalUID", "/uid"])?;
    let key = if id.chars().count() <= 200 && !id.chars().any(char::is_control) {
        id.to_string()
    } else {
        hash_key(id)
    };
    let private = matches!(
        first(r, &["/visibility", "/sensitivity"])
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("private" | "confidential")
    );
    let show_as = s(r, "/showAs").unwrap_or("").to_ascii_lowercase();
    let hint = match (kind, show_as.as_str()) {
        ("focusTime", _) => Hint::Focus,
        ("outOfOffice", _) | (_, "oof") => Hint::OutOfOffice,
        _ => Hint::None,
    };
    let busy = match (s(r, "/transparency"), show_as.as_str()) {
        (Some("transparent"), _) | (_, "free" | "workingelsewhere") => false,
        (Some("opaque"), _) | (_, "busy" | "tentative" | "oof") => true,
        _ => !all_day,
    };
    let title = first(r, &["/summary", "/subject", "/title", "/name"])
        .map(|t| clean(t, 300, false))
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "(no title)".into());
    let location = first(r, &["/location/displayName", "/location"])
        .map(|l| clean(l, 200, false))
        .filter(|l| !l.is_empty());
    let description = first(r, &["/description", "/bodyPreview"])
        .map(|d| clean(d, 2000, true))
        .filter(|d| !d.is_empty());
    let video = r
        .pointer("/conferenceData/entryPoints")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find(|e| s(e, "/entryPointType") == Some("video"))
        .and_then(|e| s(e, "/uri"));
    let url = https_url(s(r, "/hangoutLink"))
        .or_else(|| https_url(video))
        .or_else(|| {
            https_url(first(
                r,
                &[
                    "/onlineMeeting/joinUrl",
                    "/onlineMeetingUrl",
                    "/meetingUrl",
                    "/url",
                ],
            ))
        })
        .or_else(|| location.as_deref().and_then(conference_url))
        .or_else(|| description.as_deref().and_then(conference_url));
    let organizer = first(
        r,
        &[
            "/organizer/email",
            "/organizer/emailAddress/address",
            "/organizer",
        ],
    )
    .and_then(normal_email);
    let organizer_is_self = flag(r, "/organizer/self") || flag(r, "/isOrganizer");
    let recurring = r.get("recurringEventId").is_some()
        || r.get("seriesMasterId").is_some_and(|v| !v.is_null())
        || r.get("recurrence").is_some_and(|v| !v.is_null())
        || matches!(s(r, "/type"), Some("occurrence" | "exception"));
    let (attendees, attendee_count) = attendees(r);
    Some(Event {
        key,
        title: if private {
            "Private event".into()
        } else {
            title
        },
        start,
        end,
        all_day,
        location: if private { None } else { location },
        url: if private { None } else { url },
        description: if private { None } else { description },
        busy,
        recurring,
        private,
        hint,
        organizer,
        organizer_is_self,
        attendees,
        attendee_count,
    })
}

/// The records' events that touch the window around `now`, by start.
pub fn events(records: &[Value], zone: Tz, now: DateTime<Utc>) -> Vec<Event> {
    let (lo, hi) = window(now, zone);
    let mut out: Vec<Event> = Vec::new();
    for r in records {
        if let Some(e) = event(r, zone)
            && e.end_or_start() > lo
            && e.start < hi
            && !out.iter().any(|x| x.key == e.key)
        {
            out.push(e);
        }
    }
    out.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.key.cmp(&b.key)));
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn google_and_graph_shapes() {
        let z = Tz::Tz(chrono_tz::Europe::Stockholm);
        let g = json!({
            "id": "abc_20261110T080000Z", "summary": "Ana / me", "status": "confirmed",
            "start": {"dateTime": "2026-11-10T09:00:00+01:00", "timeZone": "Europe/Stockholm"},
            "end": {"dateTime": "2026-11-10T09:30:00+01:00"},
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "recurringEventId": "abc",
            "organizer": {"email": "me@acme.com", "self": true},
            "attendees": [
                {"email": "me@acme.com", "self": true, "responseStatus": "accepted"},
                {"email": "ana@acme.com", "displayName": "Ana", "responseStatus": "needsAction"},
                {"email": "room@resource.calendar.google.com", "resource": true}
            ]
        });
        let e = event(&g, z).unwrap();
        assert_eq!(e.start.to_rfc3339(), "2026-11-10T08:00:00+00:00");
        assert_eq!(
            e.url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert!(e.recurring && e.organizer_is_self);
        assert_eq!(e.attendee_count, 2);
        assert!(e.attendees[0].is_self);

        let m = json!({
            "id": "AAMk1", "subject": "Planning", "isCancelled": false, "showAs": "busy",
            "start": {"dateTime": "2026-11-10T09:00:00.0000000", "timeZone": "W. Europe Standard Time"},
            "end": {"dateTime": "2026-11-10T10:00:00.0000000", "timeZone": "W. Europe Standard Time"},
            "onlineMeeting": {"joinUrl": "https://teams.microsoft.com/l/meetup-join/x"},
            "organizer": {"emailAddress": {"name": "Bo", "address": "bo@acme.com"}},
            "attendees": [{"emailAddress": {"address": "me@acme.com"}, "status": {"response": "declined"}, "type": "required"}],
            "sensitivity": "normal"
        });
        let e = event(&m, z).unwrap();
        assert_eq!(e.start.to_rfc3339(), "2026-11-10T08:00:00+00:00");
        assert_eq!(e.end - e.start, Duration::hours(1));
        assert!(e.attendees[0].declined);

        assert!(
            event(
                &json!({"id": "x", "status": "cancelled", "start": "2026-11-10T09:00:00Z"}),
                z
            )
            .is_none()
        );
        assert!(
            event(
                &json!({"id": "x", "isCancelled": true, "start": "2026-11-10T09:00:00Z"}),
                z
            )
            .is_none()
        );
        let p = event(
            &json!({"id": "p", "summary": "Doctor", "visibility": "private", "location": "Clinic",
                    "start": {"date": "2026-11-11"}, "end": {"date": "2026-11-12"}}),
            z,
        )
        .unwrap();
        assert_eq!(p.title, "Private event");
        assert!(p.all_day && p.location.is_none() && !p.busy);
        let f = event(
            &json!({"id": "f", "summary": "Heads down", "eventType": "focusTime",
                    "start": "2026-11-10T13:00:00Z", "end": "2026-11-10T15:00:00Z"}),
            z,
        )
        .unwrap();
        assert_eq!(f.hint, Hint::Focus);
    }
}
