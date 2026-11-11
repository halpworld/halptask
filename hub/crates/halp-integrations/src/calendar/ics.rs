//! iCalendar (RFC 5545) feeds → [`Event`]s in a window.
//!
//! Parsing uses the `icalendar` crate's parser; recurrence expansion uses
//! `rrule`. Everything around them is defensive: the feed is size-capped
//! before it gets here, nesting depth and the number of `VEVENT`s are
//! checked on the raw text before the (recursive) parser runs, expansion is
//! bounded per series and in total, and parse errors never carry feed text.
//!
//! Recurring series: `RRULE` (several are unioned), `RDATE`, `EXDATE` and
//! overrides (`RECURRENCE-ID`) are applied; a cancelled override drops its
//! instance, a cancelled master drops the series. `HOURLY` and finer rules,
//! and rules asking for more than 24 times a day through `BYHOUR`,
//! `BYMINUTE` and `BYSECOND`, are not expanded (only `DTSTART` counts): a
//! calendar has no business with them and they are the expensive ones.
//! Expansion of a whole feed stops after two seconds; a rule that cannot be
//! read, rules past the fourth, or a stopped expansion make the batch
//! incomplete (nothing is archived from it).

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use icalendar::parser::{Component, Property, read_calendar, unfold};
use rrule::{RRule, RRuleSet, Tz, Unvalidated};

use super::{Attendee, Event, Hint, clean, conference_url, hash_key, local, normal_email, window};
use crate::connector::{SyncError, https_url};

/// Largest feed accepted (bytes).
pub const MAX_FEED_BYTES: usize = 5 * 1024 * 1024;
/// Most `VEVENT` components a feed may have (history included).
pub const MAX_FEED_COMPONENTS: usize = 20_000;
/// Most event instances kept from the window (the nearest to now win).
pub const MAX_EVENTS: usize = 2_000;
/// Deepest `BEGIN` nesting accepted (VCALENDAR > VEVENT > VALARM is 3).
pub const MAX_DEPTH: usize = 6;
/// Instances one series may produce in the window.
const MAX_PER_SERIES: u16 = 500;
/// Instances all series together may produce before expansion stops.
const MAX_OCCURRENCES: usize = 20_000;
/// Attendees kept per event (the count still covers all of them).
const MAX_ATTENDEES: usize = 100;
/// `RRULE`s read per series (more make the batch incomplete).
const MAX_RULES: usize = 4;
/// Occurrences per day a rule may ask for (`BYHOUR` × `BYMINUTE` ×
/// `BYSECOND`); more count as an `HOURLY`-or-finer rule.
const MAX_PER_DAY: usize = 24;
/// Wall time for expanding all series of one feed. `rrule` walks every
/// period from `DTSTART`, so a feed of series starting in the year 1 costs
/// tens of milliseconds each; past this the rest are not expanded and the
/// batch is incomplete.
const MAX_EXPAND_TIME: StdDuration = StdDuration::from_secs(2);

/// A parsed feed.
#[derive(Debug)]
pub struct Parsed {
    pub events: Vec<Event>,
    /// False when caps dropped instances.
    pub complete: bool,
}

/// A `DATE` or a `DATE-TIME` in its zone.
#[derive(Debug, Clone, Copy, PartialEq)]
enum When {
    Date(NaiveDate),
    At(DateTime<Tz>),
}

impl When {
    fn utc(&self, zone: Tz) -> Option<DateTime<Utc>> {
        match self {
            When::Date(d) => local(zone, d.and_hms_opt(0, 0, 0)?).map(|t| t.with_timezone(&Utc)),
            When::At(t) => Some(t.with_timezone(&Utc)),
        }
    }

    /// The same moment as `other` (dates compare as dates; a date against
    /// a time compares the time's local date).
    fn same(&self, other: &When) -> bool {
        match (self, other) {
            (When::Date(a), When::Date(b)) => a == b,
            (When::At(a), When::At(b)) => a.with_timezone(&Utc) == b.with_timezone(&Utc),
            (When::At(a), When::Date(b)) | (When::Date(b), When::At(a)) => a.date_naive() == *b,
        }
    }

    /// The instance part of an external key: `20261110` or
    /// `20261110T090000Z`.
    fn key(&self) -> String {
        match self {
            When::Date(d) => d.format("%Y%m%d").to_string(),
            When::At(t) => t.with_timezone(&Utc).format("%Y%m%dT%H%M%SZ").to_string(),
        }
    }
}

fn eq(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

fn prop<'a>(c: &'a Component<'a>, name: &str) -> Option<&'a Property<'a>> {
    c.properties.iter().find(|p| eq(p.name.as_str(), name))
}

fn props<'a>(c: &'a Component<'a>, name: &'a str) -> impl Iterator<Item = &'a Property<'a>> {
    c.properties
        .iter()
        .filter(move |p| eq(p.name.as_str(), name))
}

fn param<'a>(p: &'a Property<'a>, key: &str) -> Option<&'a str> {
    p.params
        .iter()
        .find(|x| eq(x.key.as_str(), key))
        .and_then(|x| x.val.as_ref())
        .map(|v| v.as_str().trim_matches('"'))
}

fn text(c: &Component<'_>, name: &str) -> Option<String> {
    prop(c, name).map(|p| p.val.clone().unescape_text().as_str().to_string())
}

fn value(c: &Component<'_>, name: &str) -> Option<String> {
    prop(c, name).map(|p| p.val.as_str().trim().to_string())
}

/// One date or date-time value.
fn when_str(v: &str, tzid: Option<&str>, is_date: bool, zone: Tz) -> Option<When> {
    let v = v.trim();
    if is_date || v.len() == 8 {
        return NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d")
            .ok()
            .map(When::Date);
    }
    let (body, utc) = match v.strip_suffix(['Z', 'z']) {
        Some(b) => (b, true),
        None => (v, false),
    };
    let n = NaiveDateTime::parse_from_str(body, "%Y%m%dT%H%M%S").ok()?;
    if utc {
        return Some(When::At(Tz::UTC.from_utc_datetime(&n)));
    }
    let z = tzid.and_then(super::tz::resolve).unwrap_or(zone);
    local(z, n).map(When::At)
}

fn when(p: &Property<'_>, zone: Tz) -> Option<When> {
    let is_date = param(p, "VALUE").is_some_and(|v| eq(v, "DATE"));
    when_str(p.val.as_str(), param(p, "TZID"), is_date, zone)
}

/// `RDATE` / `EXDATE`: comma lists; `PERIOD` values count by their start.
fn when_list(c: &Component<'_>, name: &'static str, zone: Tz) -> Vec<When> {
    let mut out = Vec::new();
    for p in props(c, name) {
        let is_date = param(p, "VALUE").is_some_and(|v| eq(v, "DATE"));
        for v in p.val.as_str().split(',').take(1000) {
            let v = v.split('/').next().unwrap_or(v);
            if let Some(w) = when_str(v, param(p, "TZID"), is_date, zone) {
                out.push(w);
            }
        }
    }
    out
}

/// ISO 8601 durations as RFC 5545 writes them (`PT1H30M`, `P1D`, `P2W`).
pub(crate) fn duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let (neg, s) = match s.strip_prefix('-') {
        Some(r) => (true, r),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let s = s.strip_prefix(['P', 'p'])?;
    let mut total = 0i64;
    let mut num = String::new();
    let mut in_time = false;
    for ch in s.chars() {
        match ch {
            '0'..='9' => num.push(ch),
            'T' | 't' => in_time = true,
            u => {
                let n: i64 = num.parse().ok()?;
                num.clear();
                let secs = match (u.to_ascii_uppercase(), in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total = total.checked_add(n.checked_mul(secs)?)?;
            }
        }
    }
    if !num.is_empty() || total > 366 * 86_400 {
        return None;
    }
    Some(Duration::seconds(if neg { -total } else { total }))
}

/// Checks nesting and counts events on the unfolded text, before the
/// recursive parser sees it.
fn prescan(text: &str) -> Result<(), SyncError> {
    let mut depth = 0usize;
    let mut events = 0usize;
    for line in text.lines() {
        let l = line.trim_end();
        if l.len() >= 6 && l[..6].eq_ignore_ascii_case("BEGIN:") {
            depth += 1;
            if depth > MAX_DEPTH {
                return Err(SyncError::Config(
                    "the calendar feed nests components too deeply".into(),
                ));
            }
            if l[6..].trim().eq_ignore_ascii_case("VEVENT") {
                events += 1;
                if events > MAX_FEED_COMPONENTS {
                    return Err(SyncError::Config(format!(
                        "the calendar feed has more than {MAX_FEED_COMPONENTS} events"
                    )));
                }
            }
        } else if l.len() >= 4 && l[..4].eq_ignore_ascii_case("END:") {
            depth = depth.saturating_sub(1);
        }
    }
    Ok(())
}

/// The feed text, unfolded, with lines that cannot be content lines
/// dropped (they would fail the whole parse).
fn normalise(bytes: &[u8]) -> Result<String, SyncError> {
    if bytes.len() > MAX_FEED_BYTES {
        return Err(SyncError::Config(
            "the calendar feed is larger than 5 MiB".into(),
        ));
    }
    let raw = String::from_utf8_lossy(bytes);
    let raw = raw.trim_start_matches('\u{feff}').trim_start();
    if !raw
        .get(..15)
        .is_some_and(|h| h.eq_ignore_ascii_case("BEGIN:VCALENDAR"))
    {
        return Err(SyncError::Config(
            "the feed URL did not return an iCalendar feed".into(),
        ));
    }
    let unfolded = unfold(&raw.replace("\r\n", "\n"));
    let mut out = String::with_capacity(unfolded.len() + 16);
    for line in unfolded.lines() {
        let l = line.trim_end_matches('\r');
        if l.trim().is_empty() || !l.contains(':') {
            continue;
        }
        let l = if l.len() >= 6 && (l[..6].eq_ignore_ascii_case("BEGIN:"))
            || l.len() >= 4 && l[..4].eq_ignore_ascii_case("END:")
        {
            // `END:vevent` / trailing blanks: the parser matches names exactly.
            let (k, v) = l.split_once(':').unwrap_or((l, ""));
            out.push_str(&k.to_ascii_uppercase());
            out.push(':');
            out.push_str(&v.trim().to_ascii_uppercase());
            out.push('\n');
            continue;
        } else {
            l
        };
        out.push_str(l);
        out.push('\n');
    }
    prescan(&out)?;
    Ok(out)
}

/// The normalised feed as calendars of at most [`CHUNK`] components each,
/// every one with the feed's own properties. The `icalendar` parser takes
/// time quadratic in the number of components (20 000 events: about 19 s);
/// in chunks it is linear. Text outside the first `VCALENDAR` is dropped,
/// as the parser did.
fn chunks(text: &str) -> Vec<String> {
    let mut head = String::from("BEGIN:VCALENDAR\n");
    let mut out: Vec<String> = Vec::new();
    let mut body = String::new();
    let mut in_body = 0usize;
    let mut depth = 0usize;
    let mut done = false;
    for line in text.lines() {
        if done {
            break;
        }
        let begin = line.starts_with("BEGIN:");
        let end = line.starts_with("END:");
        match (depth, begin, end) {
            (0, true, _) if line == "BEGIN:VCALENDAR" => depth = 1,
            (0, _, _) => {}
            (1, _, true) => done = true,
            (1, false, false) => {
                head.push_str(line);
                head.push('\n');
            }
            _ => {
                if begin {
                    depth += 1;
                } else if end {
                    depth -= 1;
                }
                body.push_str(line);
                body.push('\n');
                if depth == 1 {
                    in_body += 1;
                    if in_body >= CHUNK {
                        out.push(std::mem::take(&mut body));
                        in_body = 0;
                    }
                }
            }
        }
    }
    if !body.is_empty() || out.is_empty() {
        out.push(body);
    }
    out.into_iter()
        .map(|b| format!("{head}{b}END:VCALENDAR\n"))
        .collect()
}

/// Components per parse (see [`chunks`]).
const CHUNK: usize = 250;

/// What one `VEVENT` says, apart from when.
struct Details {
    title: String,
    location: Option<String>,
    url: Option<String>,
    description: Option<String>,
    busy: bool,
    private: bool,
    cancelled: bool,
    hint: Hint,
    organizer: Option<String>,
    attendees: Vec<Attendee>,
    attendee_count: u32,
}

fn details(c: &Component<'_>, all_day: bool) -> Details {
    let class = value(c, "CLASS").unwrap_or_default().to_ascii_uppercase();
    let private = class == "PRIVATE" || class == "CONFIDENTIAL";
    let transp = value(c, "TRANSP").unwrap_or_default().to_ascii_uppercase();
    let ms = value(c, "X-MICROSOFT-CDO-BUSYSTATUS")
        .or_else(|| value(c, "X-MICROSOFT-CDO-INTENDEDSTATUS"))
        .unwrap_or_default()
        .to_ascii_uppercase();
    let busy = match (transp.as_str(), ms.as_str()) {
        (_, "FREE") | ("TRANSPARENT", _) => false,
        ("OPAQUE", _) | (_, "BUSY" | "OOF" | "TENTATIVE") => true,
        _ => !all_day,
    };
    let hint = if ms == "OOF" {
        Hint::OutOfOffice
    } else {
        Hint::None
    };
    let cancelled = value(c, "STATUS").is_some_and(|s| eq(&s, "CANCELLED"));
    let title = text(c, "SUMMARY")
        .map(|s| clean(&s, 300, false))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(no title)".into());
    let location = text(c, "LOCATION")
        .map(|s| clean(&s, 200, false))
        .filter(|s| !s.is_empty());
    let description = text(c, "DESCRIPTION")
        .map(|s| clean(&s, 2000, true))
        .filter(|s| !s.is_empty());
    let url = https_url(value(c, "URL").as_deref())
        .or_else(|| https_url(value(c, "X-GOOGLE-CONFERENCE").as_deref()))
        .or_else(|| location.as_deref().and_then(conference_url))
        .or_else(|| description.as_deref().and_then(conference_url));
    let organizer = prop(c, "ORGANIZER").and_then(|p| normal_email(p.val.as_str()));
    let mut attendees = Vec::new();
    let mut attendee_count = 0u32;
    for p in props(c, "ATTENDEE") {
        let cutype = param(p, "CUTYPE").unwrap_or("").to_ascii_uppercase();
        let resource = matches!(cutype.as_str(), "ROOM" | "RESOURCE");
        if !resource {
            attendee_count = attendee_count.saturating_add(1);
        }
        if attendees.len() >= MAX_ATTENDEES {
            continue;
        }
        attendees.push(Attendee {
            email: normal_email(p.val.as_str()),
            name: param(p, "CN")
                .map(|n| clean(n, 120, false))
                .filter(|n| !n.is_empty()),
            declined: param(p, "PARTSTAT").is_some_and(|s| eq(s, "DECLINED")),
            resource,
            is_self: false,
        });
    }
    Details {
        title,
        location,
        url,
        description,
        busy,
        private,
        cancelled,
        hint,
        organizer,
        attendees,
        attendee_count: attendee_count.min(10_000),
    }
}

/// The span of a `VEVENT` from its start: days for all-day events, an
/// absolute duration otherwise.
fn span(c: &Component<'_>, start: &When, zone: Tz) -> Duration {
    let end = prop(c, "DTEND").and_then(|p| when(p, zone));
    let dur = value(c, "DURATION").and_then(|d| duration(&d));
    match start {
        When::Date(d) => {
            let days = match (end, dur) {
                (Some(When::Date(e)), _) => (e - *d).num_days(),
                (_, Some(x)) => x.num_days(),
                _ => 1,
            };
            Duration::days(days.clamp(1, 366))
        }
        When::At(s) => {
            let d = match (end, dur) {
                (Some(When::At(e)), _) => e.with_timezone(&Utc) - s.with_timezone(&Utc),
                (_, Some(x)) => x,
                _ => Duration::zero(),
            };
            d.clamp(Duration::zero(), Duration::days(366))
        }
    }
}

/// `UNTIL` as a UTC instant (dates are inclusive: the end of that day).
fn until_utc(v: &str, series: &When, zone: Tz) -> Option<DateTime<Utc>> {
    let series_zone = match series {
        When::At(t) => t.timezone(),
        When::Date(_) => zone,
    };
    match when_str(v, None, false, series_zone)? {
        When::Date(d) => {
            let next = d.succ_opt()?.and_hms_opt(0, 0, 0)?;
            match series {
                // All-day series expand in UTC (see `expand`).
                When::Date(_) => Some(Utc.from_utc_datetime(&next) - Duration::seconds(1)),
                When::At(_) => {
                    local(series_zone, next).map(|t| t.with_timezone(&Utc) - Duration::seconds(1))
                }
            }
        }
        When::At(t) => Some(t.with_timezone(&Utc)),
    }
}

/// What is left for expanding the rest of a feed.
struct Limits {
    /// Occurrences.
    budget: usize,
    deadline: Instant,
}

/// The starts of a series between `lo` and `hi`, `DTSTART` included.
fn expand(
    start: &When,
    rules: &[String],
    rdates: &[When],
    zone: Tz,
    lo: DateTime<Utc>,
    hi: DateTime<Utc>,
    limits: &mut Limits,
) -> (Vec<When>, bool) {
    let mut complete = true;
    // All-day series run in UTC at midnight so that dates stay dates.
    let s: DateTime<Tz> = match start {
        When::Date(d) => match d.and_hms_opt(0, 0, 0) {
            Some(n) => Tz::UTC.from_utc_datetime(&n),
            None => return (vec![], true),
        },
        When::At(t) => *t,
    };
    let to_when = |t: DateTime<Tz>| match start {
        When::Date(_) => When::Date(t.date_naive()),
        When::At(_) => When::At(t),
    };
    let mut out = vec![*start];
    let slack = Duration::days(2);
    if rules.len() > MAX_RULES {
        complete = false;
    }
    for rule in rules.iter().take(MAX_RULES) {
        if limits.budget == 0 || Instant::now() > limits.deadline {
            complete = false;
            break;
        }
        let mut until = None;
        let mut fine = false;
        // Times per day a rule asks for through BYHOUR × BYMINUTE ×
        // BYSECOND: a DAILY rule with long lists is an every-second rule in
        // disguise and costs as much to expand.
        let mut per_day = 1usize;
        let rest: Vec<&str> = rule
            .split(';')
            .filter(|part| {
                let (k, v) = part.split_once('=').unwrap_or((part, ""));
                let k = k.trim();
                if eq(k, "UNTIL") {
                    until = Some(v.trim().to_string());
                    return false;
                }
                if eq(k, "FREQ") {
                    fine |= matches!(
                        v.trim().to_ascii_uppercase().as_str(),
                        "HOURLY" | "MINUTELY" | "SECONDLY"
                    );
                }
                if eq(k, "BYHOUR") || eq(k, "BYMINUTE") || eq(k, "BYSECOND") {
                    per_day = per_day.saturating_mul(v.split(',').count().max(1));
                }
                !part.trim().is_empty()
            })
            .collect();
        if fine || per_day > MAX_PER_DAY {
            continue;
        }
        // A rule that cannot be read leaves occurrences out: the batch is
        // not the whole series, so nothing may be archived for it.
        let Ok(mut rr) = rest.join(";").parse::<RRule<Unvalidated>>() else {
            complete = false;
            continue;
        };
        if let Some(u) = until {
            match until_utc(&u, start, zone) {
                Some(u) if u >= s.with_timezone(&Utc) => {
                    rr = rr.until(u.with_timezone(&Tz::UTC));
                }
                // Ends before it starts: no occurrences from this rule.
                Some(_) => continue,
                None => {
                    complete = false;
                    continue;
                }
            }
        }
        let Ok(valid) = rr.validate(s) else {
            complete = false;
            continue;
        };
        let tz = s.timezone();
        let set = RRuleSet::new(s)
            .rrule(valid)
            .after((lo - slack).with_timezone(&tz))
            .before((hi + slack).with_timezone(&tz));
        let r = set.all(MAX_PER_SERIES.min(limits.budget.min(u16::MAX as usize) as u16));
        if r.limited && r.dates.len() >= MAX_PER_SERIES as usize {
            complete = false;
        }
        limits.budget = limits.budget.saturating_sub(r.dates.len());
        out.extend(r.dates.into_iter().map(to_when));
    }
    out.extend(rdates.iter().copied());
    let mut seen: Vec<When> = Vec::with_capacity(out.len());
    for w in out {
        if !seen.iter().any(|x| x.same(&w)) {
            seen.push(w);
        }
    }
    (seen, complete)
}

struct Raw<'a> {
    c: &'a Component<'a>,
    start: When,
    rid: Option<When>,
}

/// How new a revision of an event is: `SEQUENCE`, then the newer of
/// `LAST-MODIFIED` and `DTSTAMP` (UTC `YYYYMMDDTHHMMSSZ` text sorts by
/// time).
fn revision(c: &Component<'_>) -> (i64, String) {
    let seq = value(c, "SEQUENCE")
        .and_then(|v| v.trim().parse::<i64>().ok())
        .unwrap_or(0);
    let stamp = ["LAST-MODIFIED", "DTSTAMP"]
        .iter()
        .filter_map(|n| value(c, n))
        .map(|v| v.trim().to_ascii_uppercase())
        .max()
        .unwrap_or_default();
    (seq, stamp)
}

fn uid_of(c: &Component<'_>) -> String {
    match value(c, "UID").map(|u| clean(&u, 1000, false)) {
        Some(u) if !u.is_empty() && u.chars().count() <= 200 => u,
        Some(u) if !u.is_empty() => hash_key(&u),
        _ => hash_key(&format!(
            "{}\u{0}{}",
            value(c, "DTSTART").unwrap_or_default(),
            text(c, "SUMMARY").unwrap_or_default()
        )),
    }
}

/// Parses a feed and expands it into the window around `now`. `zone`
/// overrides the feed's own zone (`X-WR-TIMEZONE`); floating times use it.
pub fn parse(bytes: &[u8], zone: Option<Tz>, now: DateTime<Utc>) -> Result<Parsed, SyncError> {
    let text_ = normalise(bytes)?;
    let chunks = chunks(&text_);
    let cals = chunks
        .iter()
        .map(|c| read_calendar(c))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SyncError::Config("the calendar feed could not be parsed".into()))?;
    let feed_zone = cals
        .first()
        .into_iter()
        .flat_map(|c| c.properties.iter())
        .find(|p| eq(p.name.as_str(), "X-WR-TIMEZONE"))
        .and_then(|p| super::tz::resolve(p.val.as_str()));
    let zone = zone.or(feed_zone).unwrap_or(Tz::LOCAL);
    let (lo, hi) = window(now, zone);

    let mut masters: BTreeMap<String, Raw<'_>> = BTreeMap::new();
    let mut overrides: BTreeMap<String, Vec<Raw<'_>>> = BTreeMap::new();
    for c in cals
        .iter()
        .flat_map(|cal| cal.components.iter())
        .filter(|c| eq(c.name.as_str(), "VEVENT"))
    {
        let Some(start) = prop(c, "DTSTART").and_then(|p| when(p, zone)) else {
            continue;
        };
        let uid = uid_of(c);
        let rid = prop(c, "RECURRENCE-ID").and_then(|p| when(p, zone));
        let raw = Raw { c, start, rid };
        // The same UID (or UID and RECURRENCE-ID) more than once: the
        // newest revision wins (RFC 5545 SEQUENCE, then LAST-MODIFIED or
        // DTSTAMP), not whichever came first in the feed.
        if let Some(rid) = raw.rid {
            let list = overrides.entry(uid).or_default();
            match list
                .iter_mut()
                .find(|o| o.rid.as_ref().is_some_and(|r| r.same(&rid)))
            {
                Some(o) if revision(raw.c) > revision(o.c) => *o = raw,
                Some(_) => {}
                None => list.push(raw),
            }
        } else {
            match masters.entry(uid) {
                Entry::Vacant(v) => {
                    v.insert(raw);
                }
                Entry::Occupied(mut o) => {
                    if revision(raw.c) > revision(o.get().c) {
                        o.insert(raw);
                    }
                }
            }
        }
    }

    let mut events: Vec<Event> = Vec::new();
    let mut complete = true;
    let mut limits = Limits {
        budget: MAX_OCCURRENCES,
        deadline: Instant::now() + MAX_EXPAND_TIME,
    };
    let mut keys = BTreeSet::new();
    let mut push = |e: Event, events: &mut Vec<Event>| {
        if e.end_or_start() > lo && e.start < hi && keys.insert(e.key.clone()) {
            events.push(e);
        }
    };

    for (uid, m) in &masters {
        let all_day = matches!(m.start, When::Date(_));
        let d = details(m.c, all_day);
        let ovs = overrides.remove(uid).unwrap_or_default();
        if d.cancelled {
            continue;
        }
        let rules: Vec<String> = props(m.c, "RRULE")
            .map(|p| p.val.as_str().to_string())
            .collect();
        let rdates = when_list(m.c, "RDATE", zone);
        let recurring = !rules.is_empty() || !rdates.is_empty();
        let len = span(m.c, &m.start, zone);
        if !recurring {
            if let Some(e) = instance(uid.clone(), &m.start, len, zone, &d, false, all_day) {
                push(e, &mut events);
            }
            continue;
        }
        let exdates = when_list(m.c, "EXDATE", zone);
        let (starts, ok) = expand(&m.start, &rules, &rdates, zone, lo - len, hi, &mut limits);
        complete &= ok;
        let mut used = vec![false; ovs.len()];
        for s in starts {
            if exdates.iter().any(|x| x.same(&s)) {
                continue;
            }
            let key = format!("{uid}@{}", s.key());
            match ovs
                .iter()
                .position(|o| o.rid.as_ref().is_some_and(|r| r.same(&s)))
            {
                Some(i) => {
                    used[i] = true;
                    if let Some(e) = override_instance(key, &ovs[i], zone) {
                        push(e, &mut events);
                    }
                }
                None => {
                    if let Some(e) = instance(key, &s, len, zone, &d, true, all_day) {
                        push(e, &mut events);
                    }
                }
            }
        }
        // Overrides whose original slot fell outside the expansion (moved
        // into the window from elsewhere).
        for (i, o) in ovs.iter().enumerate() {
            if used[i] || exdates.iter().any(|x| o.rid.is_some_and(|r| x.same(&r))) {
                continue;
            }
            if let Some(r) = o.rid
                && let Some(e) = override_instance(format!("{uid}@{}", r.key()), o, zone)
            {
                push(e, &mut events);
            }
        }
    }
    // Overrides without a master in the feed (shared single instances).
    for (uid, ovs) in overrides {
        for o in &ovs {
            if let Some(r) = o.rid
                && let Some(e) = override_instance(format!("{uid}@{}", r.key()), o, zone)
            {
                push(e, &mut events);
            }
        }
    }

    if events.len() > MAX_EVENTS {
        events.sort_by_key(|e| (e.start - now).num_seconds().abs());
        events.truncate(MAX_EVENTS);
        complete = false;
    }
    events.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.key.cmp(&b.key)));
    Ok(Parsed { events, complete })
}

fn override_instance(key: String, o: &Raw<'_>, zone: Tz) -> Option<Event> {
    let all_day = matches!(o.start, When::Date(_));
    let d = details(o.c, all_day);
    if d.cancelled {
        return None;
    }
    let len = span(o.c, &o.start, zone);
    instance(key, &o.start, len, zone, &d, true, all_day)
}

fn instance(
    key: String,
    start: &When,
    len: Duration,
    zone: Tz,
    d: &Details,
    recurring: bool,
    all_day: bool,
) -> Option<Event> {
    let s = start.utc(zone)?;
    let e = match start {
        When::Date(day) => {
            let end_day = *day + Duration::days(len.num_days().max(1));
            local(zone, end_day.and_hms_opt(0, 0, 0)?)?.with_timezone(&Utc)
        }
        When::At(_) => s + len,
    };
    let private = d.private;
    Some(Event {
        key,
        title: if private {
            "Private event".into()
        } else {
            d.title.clone()
        },
        start: s,
        end: e,
        all_day,
        location: if private { None } else { d.location.clone() },
        url: if private { None } else { d.url.clone() },
        description: if private { None } else { d.description.clone() },
        busy: d.busy,
        recurring,
        private,
        hint: d.hint,
        organizer: d.organizer.clone(),
        organizer_is_self: false,
        attendees: d.attendees.clone(),
        attendee_count: d.attendee_count,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(duration("PT1H30M").unwrap(), Duration::minutes(90));
        assert_eq!(duration("P1D").unwrap(), Duration::days(1));
        assert_eq!(duration("P2W").unwrap(), Duration::days(14));
        assert_eq!(duration("-PT15M").unwrap(), Duration::minutes(-15));
        assert!(duration("PT").is_some());
        assert!(duration("1H").is_none());
        assert!(duration("P1H").is_none());
        assert!(duration("P99999W").is_none());
    }

    #[test]
    fn rejects_non_calendars_and_deep_nesting() {
        let now = Utc::now();
        assert!(parse(b"<html>nope</html>", None, now).is_err());
        let mut deep = String::from("BEGIN:VCALENDAR\n");
        for _ in 0..10 {
            deep.push_str("BEGIN:X\n");
        }
        let e = parse(deep.as_bytes(), None, now).unwrap_err();
        assert!(e.message().contains("deeply"));
        let big = vec![b' '; MAX_FEED_BYTES + 1];
        assert!(
            parse(&big, None, now)
                .unwrap_err()
                .message()
                .contains("5 MiB")
        );
    }
}
