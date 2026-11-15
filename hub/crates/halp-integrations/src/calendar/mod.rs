//! Calendars as sources (`calendar` kind): meetings in a window around
//! today ([`window`]: seven days back, fourteen ahead) become
//! `dev.meeting` and/or `em.meeting` items, and upcoming 1:1 meetings are
//! linked to the Engineering Manager pack's `em.one_on_one` items.
//!
//! Two sources:
//!
//! - `ics`: an iCalendar feed URL. The URL is a credential (Google's and
//!   Outlook's "secret address" grants read access to the calendar), so it
//!   is stored in the OS keychain like a token (`integrations_set_token`)
//!   and never appears in settings, events, audit entries or status text;
//!   the app shows only [`url_hint`].
//! - `mcp`: a calendar MCP server's list-events tool (Google Calendar or
//!   Microsoft Graph shaped answers, see [`remote`]), configured like the
//!   `mcp` kind.
//!
//! Items are keyed by UID plus recurrence instance, so a resync is
//! idempotent; each type's batch is complete (events gone from the feed
//! or the window, or cancelled, are archived) unless a cap cut it.

pub mod ics;
pub mod remote;
pub mod tz;

use std::collections::BTreeMap;
use std::sync::LazyLock;

use chrono::{DateTime, Duration, NaiveDateTime, SecondsFormat, TimeZone, Utc};
use regex::Regex;
use reqwest::Method;
use reqwest::header::{ACCEPT, HeaderValue};
use rrule::Tz;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::connector::{
    Batch, BoxFuture, Connector, FieldPatch, SyncCx, SyncError, cap, https_url,
};
use crate::http::{Http, HttpErrorKind};
use crate::mcp::{self, McpSettings};
use crate::secrets::check_user_token;

pub const DEV_MEETING: &str = "dev.meeting";
pub const EM_MEETING: &str = "em.meeting";
pub const ONE_ON_ONE: &str = "em.one_on_one";
pub const PERSON: &str = "em.person";
/// `tasks.list`'s largest page (it has no paging).
const LIST_LIMIT: u32 = 1000;
/// Days before today the window starts.
pub const DAYS_BACK: i64 = 7;
/// Days after today the window covers (today + 14 inclusive).
pub const DAYS_AHEAD: i64 = 14;
const MAX_ME: usize = 10;
const MAX_URL_CHARS: usize = 4096;

// ------------------------------------------------------------------ events

/// What the source says about the user's time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Hint {
    #[default]
    None,
    Focus,
    OutOfOffice,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Attendee {
    pub email: Option<String>,
    pub name: Option<String>,
    pub declined: bool,
    /// A room or equipment.
    pub resource: bool,
    /// The source marks this attendee as the calendar's owner.
    pub is_self: bool,
}

/// One meeting instance, source independent.
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// `uid` or `uid@<instance>`; stable across syncs.
    pub key: String,
    pub title: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub all_day: bool,
    pub location: Option<String>,
    pub url: Option<String>,
    pub description: Option<String>,
    pub busy: bool,
    pub recurring: bool,
    pub private: bool,
    pub hint: Hint,
    pub organizer: Option<String>,
    pub organizer_is_self: bool,
    pub attendees: Vec<Attendee>,
    /// Non-resource attendees (all of them, not only the kept ones).
    pub attendee_count: u32,
}

impl Event {
    /// The end, or a second after the start for zero-length events.
    pub(crate) fn end_or_start(&self) -> DateTime<Utc> {
        if self.end > self.start {
            self.end
        } else {
            self.start + Duration::seconds(1)
        }
    }
}

// ----------------------------------------------------------------- helpers

fn invisible(c: char) -> bool {
    matches!(c,
        '\u{200B}'..='\u{200F}'
        | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{2069}'
        | '\u{061C}'
        | '\u{180E}'
        | '\u{FEFF}')
}

/// Text from a calendar: controls and bidi/zero-width characters removed,
/// whitespace collapsed (line breaks kept when `multiline`), at most `max`
/// characters.
pub(crate) fn clean(s: &str, max: usize, multiline: bool) -> String {
    let mut out = String::new();
    let mut n = 0usize;
    let mut last_space = true;
    for ch in s.chars() {
        if n >= max {
            break;
        }
        if invisible(ch) {
            continue;
        }
        let ch = if ch == '\n' && multiline {
            '\n'
        } else if ch.is_control() || ch.is_whitespace() {
            ' '
        } else {
            ch
        };
        if ch == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = ch == '\n';
            if ch == '\n' && out.ends_with(' ') {
                out.pop();
                n -= 1;
            }
        }
        out.push(ch);
        n += 1;
    }
    out.trim().to_string()
}

/// A short stable key for an over-long or missing UID.
pub(crate) fn hash_key(s: &str) -> String {
    let h = Sha256::digest(s.as_bytes());
    format!("h-{}", &hex::encode(h)[..32])
}

/// A lower-case address from `mailto:` or plain text, if it looks like one.
pub(crate) fn normal_email(v: &str) -> Option<String> {
    let v = v.trim();
    let v = match v.get(..7) {
        Some(p) if p.eq_ignore_ascii_case("mailto:") => &v[7..],
        _ => v,
    };
    let v = v.trim().to_lowercase();
    let (local, domain) = v.split_once('@')?;
    let ok = !local.is_empty()
        && domain.contains('.')
        && !domain.contains('@')
        && v.len() <= 254
        && !v
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, ':' | '/' | '<' | '>'));
    ok.then_some(v)
}

/// A local time in `zone`; a time in a DST gap moves an hour on.
pub(crate) fn local(zone: Tz, n: NaiveDateTime) -> Option<DateTime<Tz>> {
    zone.from_local_datetime(&n).earliest().or_else(|| {
        zone.from_local_datetime(&(n + Duration::hours(1)))
            .earliest()
    })
}

/// The sync window in `zone`: midnight seven days ago to midnight after
/// today + 14.
pub fn window(now: DateTime<Utc>, zone: Tz) -> (DateTime<Utc>, DateTime<Utc>) {
    let today = now.with_timezone(&zone).date_naive();
    let at = |days: i64, fallback: DateTime<Utc>| {
        (today + Duration::days(days))
            .and_hms_opt(0, 0, 0)
            .and_then(|n| local(zone, n))
            .map_or(fallback, |t| t.with_timezone(&Utc))
    };
    (
        at(-DAYS_BACK, now - Duration::days(DAYS_BACK)),
        at(DAYS_AHEAD + 1, now + Duration::days(DAYS_AHEAD + 1)),
    )
}

static CONFERENCE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"https://(?:meet\.google\.com|(?:[a-z0-9-]+\.)*zoom\.us|teams\.microsoft\.com|teams\.live\.com|(?:[a-z0-9-]+\.)*webex\.com|(?:[a-z0-9-]+\.)*whereby\.com)/[^\s<>\x22']*",
    )
    .ok()
});

/// The first video-call link in free text.
pub(crate) fn conference_url(s: &str) -> Option<String> {
    let m = CONFERENCE.as_ref()?.find(s)?;
    https_url(Some(m.as_str().trim_end_matches(['.', ',', ')', ']', '>'])))
}

fn rfc3339(t: DateTime<Utc>) -> String {
    t.to_rfc3339_opts(SecondsFormat::Secs, true)
}

// ---------------------------------------------------------------- settings

fn default_types() -> Vec<String> {
    vec![DEV_MEETING.into(), EM_MEETING.into()]
}

fn yes() -> bool {
    true
}

/// `settings` of a `calendar` integration. Holds no URL for `ics`: the
/// feed URL is the keychain secret.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalendarSettings {
    /// `ics` or `mcp`.
    pub source: String,
    /// `mcp` only: the server and its list-events tool (adapter `calendar`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<McpSettings>,
    /// The item types to write (`dev.meeting`, `em.meeting` or both).
    #[serde(default = "default_types")]
    pub item_types: Vec<String>,
    /// The user's own addresses (for declined events, 1:1s, organizer).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub me: Vec<String>,
    /// Internal mail domains (default: those of `me`); an organizer from
    /// elsewhere makes a meeting `external`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
    /// IANA zone for floating times and the window (default: the feed's
    /// `X-WR-TIMEZONE`, else the computer's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    /// Copy event descriptions into item notes (default false).
    #[serde(default)]
    pub include_description: bool,
    /// Leave out events the user declined (default true).
    #[serde(default = "yes")]
    pub skip_declined: bool,
    /// Link upcoming 1:1 meetings to `em.one_on_one` items (default true).
    #[serde(default = "yes")]
    pub link_one_on_ones: bool,
}

impl CalendarSettings {
    pub fn parse(v: &Value) -> Result<CalendarSettings, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("calendar settings: {e}"))
    }

    /// Checks and normalises (lower-cases addresses, sets the MCP adapter).
    pub fn check(mut self, http: &Http) -> Result<CalendarSettings, String> {
        match (self.source.as_str(), &mut self.mcp) {
            ("ics", None) => {}
            ("ics", Some(_)) => return Err("calendar: mcp is for source mcp".into()),
            ("mcp", Some(m)) => {
                if !m.adapter.is_empty() && m.adapter != "calendar" {
                    return Err("calendar: the mcp adapter is calendar".into());
                }
                m.adapter = "calendar".into();
                m.check_transport(http)?;
            }
            ("mcp", None) => return Err("calendar: source mcp needs mcp settings".into()),
            _ => return Err("calendar: source is ics or mcp".into()),
        }
        let mut types = Vec::new();
        for t in &self.item_types {
            if t != DEV_MEETING && t != EM_MEETING {
                return Err("calendar: item_types are dev.meeting and/or em.meeting".into());
            }
            if !types.contains(t) {
                types.push(t.clone());
            }
        }
        if types.is_empty() {
            return Err("calendar: item_types must not be empty".into());
        }
        self.item_types = types;
        if self.me.len() > MAX_ME || self.domains.len() > MAX_ME {
            return Err(format!(
                "calendar: at most {MAX_ME} addresses and {MAX_ME} domains"
            ));
        }
        let mut me = Vec::new();
        for m in &self.me {
            let e = normal_email(m).ok_or_else(|| format!("calendar: {m:?} is not an address"))?;
            if !me.contains(&e) {
                me.push(e);
            }
        }
        self.me = me;
        let mut domains = Vec::new();
        for d in &self.domains {
            let d = d.trim().trim_start_matches('@').to_lowercase();
            if d.is_empty()
                || d.len() > 253
                || !d.contains('.')
                || !d
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
            {
                return Err(format!("calendar: {d:?} is not a domain"));
            }
            if !domains.contains(&d) {
                domains.push(d);
            }
        }
        self.domains = domains;
        if let Some(z) = &self.timezone
            && z.parse::<chrono_tz::Tz>().is_err()
        {
            return Err("calendar: timezone must be an IANA name like Europe/Stockholm".into());
        }
        Ok(self)
    }

    fn zone(&self) -> Option<Tz> {
        self.timezone
            .as_deref()
            .and_then(|z| z.parse::<chrono_tz::Tz>().ok())
            .map(Tz::Tz)
    }

    fn is_me(&self, a: &Attendee) -> bool {
        a.is_self || a.email.as_ref().is_some_and(|e| self.me.contains(e))
    }

    fn my_domains(&self) -> Vec<String> {
        if !self.domains.is_empty() {
            return self.domains.clone();
        }
        let mut d: Vec<String> = self
            .me
            .iter()
            .filter_map(|e| e.split_once('@').map(|(_, d)| d.to_string()))
            .collect();
        d.dedup();
        d
    }

    fn declined(&self, e: &Event) -> bool {
        e.attendees.iter().any(|a| a.declined && self.is_me(a))
    }

    fn organizer_is_me(&self, e: &Event) -> bool {
        e.organizer_is_self || e.organizer.as_ref().is_some_and(|o| self.me.contains(o))
    }
}

/// Checks a feed URL given as the secret; `webcal://` becomes `https://`.
pub fn check_feed_url(raw: &str, http: &Http) -> Result<String, &'static str> {
    let raw = raw.trim();
    if raw.chars().count() > MAX_URL_CHARS {
        return Err("invalid_url");
    }
    check_user_token(raw).map_err(|_| "invalid_url")?;
    let fixed = match raw.get(..9) {
        Some(p) if p.eq_ignore_ascii_case("webcal://") => format!("https://{}", &raw[9..]),
        _ => raw.to_string(),
    };
    let u = Url::parse(&fixed).map_err(|_| "invalid_url")?;
    http.check(&u).map_err(|_| "invalid_url")?;
    Ok(u.to_string())
}

/// What the app may show of a feed URL: its host only
/// (`calendar.google.com`). The path and query are the secret.
pub fn url_hint(secret: &str) -> Option<String> {
    let u = Url::parse(secret).ok()?;
    u.host_str().map(str::to_string)
}

// ----------------------------------------------------------------- mapping

/// The `em.meeting` category.
pub fn category(e: &Event, s: &CalendarSettings) -> &'static str {
    let title = e.title.to_lowercase();
    let has = |words: &[&str]| words.iter().any(|w| title.contains(w));
    let others: Vec<&Attendee> = e
        .attendees
        .iter()
        .filter(|a| !a.resource && !s.is_me(a))
        .collect();
    let organizer_other = e.organizer.as_ref().is_some_and(|o| {
        !s.organizer_is_me(e) && !e.attendees.iter().any(|a| a.email.as_ref() == Some(o))
    });
    let n_others = others.len() + usize::from(organizer_other);
    // Team rituals often come without an attendee list (shared calendars).
    let team = has(&[
        "standup",
        "stand-up",
        "stand up",
        "daily",
        "retro",
        "planning",
        "sprint",
        "refinement",
        "grooming",
    ]);
    if e.hint != Hint::None
        || has(&[
            "focus",
            "deep work",
            "no meetings",
            "no-meeting",
            "out of office",
            "ooo",
            "vacation",
            "holiday",
            "pto",
        ])
        || (n_others == 0 && e.attendee_count <= 1 && !team)
    {
        return "focus";
    }
    if n_others == 1 && e.attendee_count <= 2 {
        return "1on1";
    }
    let mine = s.my_domains();
    if let Some(o) = &e.organizer
        && !mine.is_empty()
        && o.split_once('@')
            .is_some_and(|(_, d)| !mine.iter().any(|m| m == d))
    {
        return "external";
    }
    if team {
        return "team";
    }
    "cross-team"
}

fn item(e: &Event, type_id: &str, prefix: &str, s: &CalendarSettings) -> Value {
    let short = if type_id == EM_MEETING { "em" } else { "dev" };
    let mut f = Map::new();
    f.insert("starts_at".into(), json!(rfc3339(e.start)));
    f.insert("ends_at".into(), json!(rfc3339(e.end.max(e.start))));
    if let Some(l) = &e.location {
        f.insert("location".into(), json!(cap(l, 200)));
    }
    if let Some(u) = &e.url {
        f.insert("url".into(), json!(u));
    }
    f.insert("busy".into(), json!(e.busy));
    if type_id == EM_MEETING {
        f.insert(
            "attendees_count".into(),
            json!(e.attendee_count.min(10_000)),
        );
        f.insert("organizer_is_me".into(), json!(s.organizer_is_me(e)));
        f.insert("recurring".into(), json!(e.recurring));
        f.insert("category".into(), json!(category(e, s)));
    }
    let mut v = json!({
        "external_id": cap(&format!("{prefix}:{short}:{}", e.key), 300),
        "title": cap(&e.title, 300),
        "kind": "bullet",
        "type_id": type_id,
        "tags": ["calendar"],
        "fields": f,
    });
    if s.include_description
        && let Some(d) = &e.description
    {
        v["notes"] = json!(cap(d, 2000));
    }
    v
}

/// The role a calendar item type belongs to (its pack's role).
fn type_role(type_id: &str) -> Option<&'static str> {
    match type_id.split_once('.').map(|(p, _)| p) {
        Some("dev") => Some("developer"),
        Some("em") => Some("eng-manager"),
        _ => None,
    }
}

/// Whether the integration is connected for the role that owns a calendar
/// item type (no roles = every role). Types of no pack are always fine.
fn type_allowed(type_id: &str, connected: &[String]) -> bool {
    type_role(type_id).is_none_or(|r| connected.is_empty() || connected.iter().any(|c| c == r))
}

/// Shows an item only in its pack's role (`dev.meeting` to developers,
/// `em.*` to engineering managers). `false` when the integration is not
/// connected for that role: the item must not be written at all (an
/// `em.meeting` would otherwise land in the developer role).
pub fn set_roles(item: &mut Value, connected: &[String]) -> bool {
    let Some(t) = item.get("type_id").and_then(Value::as_str) else {
        return true;
    };
    if !type_allowed(t, connected) {
        return false;
    }
    if let Some(role) = type_role(t) {
        item["roles"] = json!([role]);
    }
    true
}

/// One complete batch per item type, nearest events first when capped.
pub fn batches(
    events: &[Event],
    s: &CalendarSettings,
    prefix: &str,
    complete: bool,
    now: DateTime<Utc>,
) -> Vec<Batch> {
    let max = halp_domain::integration::MAX_UPSERT_ITEMS;
    let mut kept: Vec<&Event> = events
        .iter()
        .filter(|e| !(s.skip_declined && s.declined(e)))
        .collect();
    let mut complete = complete;
    if kept.len() > max {
        kept.sort_by_key(|e| (e.start - now).num_seconds().abs());
        kept.truncate(max);
        complete = false;
    }
    kept.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.key.cmp(&b.key)));
    s.item_types
        .iter()
        .map(|t| Batch {
            scope: t.clone(),
            complete,
            items: kept.iter().map(|e| item(e, t, prefix, s)).collect(),
            patches: Vec::new(),
        })
        .collect()
}

// ------------------------------------------------------------ 1:1 linking

fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn words(s: &str) -> String {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

struct Person {
    id: String,
    name: String,
    email: Option<String>,
}

fn archived(v: &Value) -> bool {
    v.get("tags").and_then(Value::as_array).is_some_and(|t| {
        t.iter()
            .any(|x| x == halp_domain::integration::ARCHIVED_TAG)
    })
}

fn person_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn persons(items: &[Value]) -> Vec<Person> {
    items
        .iter()
        .filter(|v| v.get("type_id").and_then(Value::as_str) == Some(PERSON) && !archived(v))
        .filter_map(|v| {
            let id = v.get("id")?.as_str()?.to_string();
            if !person_id_ok(&id) {
                return None;
            }
            let name = v
                .pointer("/fields/name")
                .and_then(Value::as_str)
                .or_else(|| v.get("title").and_then(Value::as_str))
                .unwrap_or_default()
                .to_string();
            let email = v
                .pointer("/fields/email")
                .and_then(Value::as_str)
                .and_then(normal_email);
            Some(Person { id, name, email })
        })
        .collect()
}

/// The people an attendee could be: by address when people have one, else
/// by display name, else by the address's local part against the name.
fn matches<'p>(a: &Attendee, people: &'p [Person]) -> Vec<&'p Person> {
    if let Some(e) = &a.email {
        let by_mail: Vec<&Person> = people
            .iter()
            .filter(|p| p.email.as_ref() == Some(e))
            .collect();
        if !by_mail.is_empty() {
            return by_mail;
        }
    }
    if let Some(n) = a.name.as_deref().map(words).filter(|n| !n.is_empty()) {
        let by_name: Vec<&Person> = people.iter().filter(|p| words(&p.name) == n).collect();
        if !by_name.is_empty() {
            return by_name;
        }
    }
    if let Some(local) = a
        .email
        .as_ref()
        .and_then(|e| e.split_once('@'))
        .map(|(l, _)| squash(l))
        && local.len() >= 3
    {
        return people
            .iter()
            .filter(|p| p.email.is_none() && squash(&p.name) == local)
            .collect();
    }
    Vec::new()
}

/// The one person an event is a 1:1 with, if exactly one other participant
/// matches exactly one person.
fn one_on_one_with<'p>(
    e: &Event,
    s: &CalendarSettings,
    people: &'p [Person],
) -> Option<&'p Person> {
    if e.all_day || e.private || e.hint != Hint::None {
        return None;
    }
    let mut parts: Vec<Attendee> = e
        .attendees
        .iter()
        .filter(|a| !a.resource)
        .cloned()
        .collect();
    if let Some(o) = &e.organizer
        && !parts.iter().any(|a| a.email.as_ref() == Some(o))
    {
        parts.push(Attendee {
            email: Some(o.clone()),
            name: None,
            declined: false,
            resource: false,
            is_self: e.organizer_is_self,
        });
    }
    let me_known = parts.iter().any(|a| s.is_me(a));
    let candidates: Vec<&Attendee> = if me_known {
        let others: Vec<&Attendee> = parts.iter().filter(|a| !s.is_me(a)).collect();
        if others.len() != 1 || parts.len() != 2 {
            return None;
        }
        others
    } else {
        if parts.len() != 2 {
            return None;
        }
        parts.iter().collect()
    };
    let mut found: Vec<&Person> = Vec::new();
    for a in candidates {
        let m = matches(a, people);
        match m.as_slice() {
            [p] => found.push(p),
            [] => {}
            _ => return None,
        }
    }
    match found.as_slice() {
        [p] => Some(p),
        _ => None,
    }
}

fn open(v: &Value) -> bool {
    let st = v.pointer("/fields/status").and_then(Value::as_str);
    matches!(st, None | Some("scheduled"))
        && v.pointer("/fields/started_at").is_none()
        && v.pointer("/fields/ended_at").is_none()
        && !archived(v)
}

/// What linking 1:1 meetings asks for.
#[derive(Debug, Default, PartialEq)]
pub struct LinkPlan {
    /// `em.one_on_one` items to create (for people with no open 1:1).
    pub create: Vec<Value>,
    /// `scheduled_for` moves for open 1:1 items this integration created
    /// (through the core's `ext.patch_fields`; see docs/INTEGRATIONS.md).
    pub patches: Vec<FieldPatch>,
}

/// Links each person's next 1:1 meeting to their open `em.one_on_one`
/// item: moves its `scheduled_for`, or creates one when there is none.
/// Existing items are never re-sent, so their agenda is never touched.
/// Only items whose `origin` is `origin` (this integration) are moved;
/// when the person's next open 1:1 was made by someone else (the user,
/// an agent), it is left alone and no second one is created.
pub fn plan_links(
    events: &[Event],
    person_items: &[Value],
    one_on_ones: &[Value],
    s: &CalendarSettings,
    prefix: &str,
    origin: &str,
    now: DateTime<Utc>,
) -> LinkPlan {
    let people = persons(person_items);
    let mut plan = LinkPlan::default();
    if people.is_empty() {
        return plan;
    }
    let mut next: BTreeMap<&str, (&Person, &Event)> = BTreeMap::new();
    let mut sorted: Vec<&Event> = events
        .iter()
        .filter(|e| e.end_or_start() > now && !s.declined(e))
        .collect();
    sorted.sort_by(|a, b| a.start.cmp(&b.start).then_with(|| a.key.cmp(&b.key)));
    for e in sorted {
        if let Some(p) = one_on_one_with(e, s, &people) {
            next.entry(p.id.as_str()).or_insert((p, e));
        }
    }
    for (pid, (p, e)) in next {
        let when = rfc3339(e.start);
        let mut mine: Vec<&Value> = one_on_ones
            .iter()
            .filter(|v| {
                v.get("type_id").and_then(Value::as_str) == Some(ONE_ON_ONE)
                    && v.pointer("/fields/person_id").and_then(Value::as_str) == Some(pid)
                    && open(v)
            })
            .collect();
        mine.sort_by_key(|v| {
            v.pointer("/fields/scheduled_for")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        });
        match mine.first() {
            Some(v) if v.get("origin").and_then(Value::as_str) != Some(origin) => {}
            Some(v) => {
                let cur = v
                    .pointer("/fields/scheduled_for")
                    .and_then(Value::as_str)
                    .and_then(|t| DateTime::parse_from_rfc3339(t).ok())
                    .map(|t| rfc3339(t.with_timezone(&Utc)));
                if cur.as_deref() != Some(when.as_str())
                    && let Some(id) = v.get("id").and_then(Value::as_str)
                {
                    let mut f = Map::new();
                    f.insert("scheduled_for".into(), json!(when));
                    plan.patches.push(FieldPatch {
                        item_id: id.to_string(),
                        type_id: ONE_ON_ONE.into(),
                        if_version: v.get("version").and_then(Value::as_u64),
                        fields: f,
                    });
                }
            }
            None => {
                let name = if p.name.is_empty() { "?" } else { &p.name };
                plan.create.push(json!({
                    "external_id": cap(&format!("1on1:{pid}:{prefix}:{}", e.key), 300),
                    "title": cap(&format!("1:1 with {name}"), 300),
                    "kind": "task",
                    "type_id": ONE_ON_ONE,
                    "tags": ["calendar"],
                    "fields": {
                        "person_id": pid,
                        "scheduled_for": when,
                        "status": "scheduled",
                    },
                }));
            }
        }
    }
    plan
}

// --------------------------------------------------------------- connector

#[derive(Default)]
pub struct Calendar;

fn feed_error(e: crate::http::HttpError) -> SyncError {
    match e.kind {
        HttpErrorKind::TooLarge => {
            SyncError::Config("the calendar feed is larger than 5 MiB".into())
        }
        HttpErrorKind::Policy => {
            SyncError::Config("the feed URL is not allowed (https only); set it again".into())
        }
        _ => SyncError::from(e),
    }
}

impl Calendar {
    async fn fetch(&self, cx: &SyncCx<'_>, zone: Option<Tz>) -> Result<ics::Parsed, SyncError> {
        let secret = cx
            .token
            .ok_or_else(|| SyncError::Auth("no feed URL stored; add it in Settings".into()))?;
        let url = Url::parse(secret).map_err(|_| {
            SyncError::Auth("the stored feed URL is not valid; set it again".into())
        })?;
        let headers = [(
            ACCEPT,
            HeaderValue::from_static("text/calendar, text/plain;q=0.5"),
        )];
        let resp = cx
            .http
            .send_limited(
                Method::GET,
                &url,
                &headers,
                None,
                ics::MAX_FEED_BYTES,
                cx.cancel,
            )
            .await
            .map_err(feed_error)?;
        let code = resp.status.as_u16();
        match code {
            200..=299 => {}
            401 | 403 | 404 | 410 => {
                return Err(SyncError::Auth(format!(
                    "the calendar feed URL was refused (HTTP {code}); set it again"
                )));
            }
            429 | 500..=599 => {
                let retry_after = resp
                    .header("retry-after")
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(|s| std::time::Duration::from_secs(s.min(3600)));
                return Err(SyncError::Transient {
                    message: format!("the calendar server answered HTTP {code}"),
                    retry_after,
                });
            }
            _ => {
                return Err(SyncError::transient(format!(
                    "the calendar server answered HTTP {code}"
                )));
            }
        }
        let now = cx.now;
        let body = resp.body;
        tokio::task::spawn_blocking(move || ics::parse(&body, zone, now))
            .await
            .map_err(|_| SyncError::transient("the calendar parser failed"))?
    }

    async fn run(&self, cx: SyncCx<'_>) -> Result<Vec<Batch>, SyncError> {
        let s = CalendarSettings::parse(cx.settings)
            .and_then(|s| s.check(cx.http))
            .map_err(SyncError::Config)?;
        let zone = s.zone();
        let (events, complete, prefix) = match (s.source.as_str(), &s.mcp) {
            ("mcp", Some(m)) => {
                let answer = mcp::call(m, &cx).await?;
                let (recs, all) = mcp::records_limited(&answer, ics::MAX_EVENTS);
                let z = zone.unwrap_or(Tz::LOCAL);
                let evs = remote::events(&recs, z, cx.now);
                // An empty list is an empty calendar; no list at all is
                // not an answer to trust.
                (evs, all, "mcp")
            }
            _ => {
                let p = self.fetch(&cx, zone).await?;
                (p.events, p.complete, "ics")
            }
        };
        let mut out = batches(&events, &s, prefix, complete, cx.now);
        // Item types of a role this integration is not connected for are
        // not written (nor archived): no batch for them.
        out.retain(|b| type_allowed(&b.scope, cx.roles));
        for b in &mut out {
            b.items.retain_mut(|it| set_roles(it, cx.roles));
        }
        if s.link_one_on_ones && type_allowed(ONE_ON_ONE, cx.roles) {
            // A failed read is an error, not "no people" or "no 1:1s".
            let people = cx
                .items
                .list(json!({ "type_id": PERSON, "limit": LIST_LIMIT }))
                .await?;
            if !people.is_empty() {
                let ones = cx
                    .items
                    .list(json!({ "type_id": ONE_ON_ONE, "limit": LIST_LIMIT }))
                    .await?;
                let origin = format!("integration:{}", cx.id);
                let mut plan = plan_links(&events, &people, &ones, &s, prefix, &origin, cx.now);
                // `tasks.list` has no paging: at the cap some 1:1s may be
                // missing, and creating one could duplicate it. Moves of
                // the ones found are still safe.
                if ones.len() >= LIST_LIMIT as usize {
                    plan.create.clear();
                }
                plan.create.retain_mut(|it| set_roles(it, cx.roles));
                if !plan.create.is_empty() || !plan.patches.is_empty() {
                    out.push(Batch {
                        scope: "one_on_ones".into(),
                        complete: false,
                        items: plan.create,
                        patches: plan.patches,
                    });
                }
            }
        }
        Ok(out)
    }
}

impl Connector for Calendar {
    fn kind(&self) -> &'static str {
        "calendar"
    }

    fn validate(&self, settings: &Value, http: &Http) -> Result<Value, String> {
        let s = CalendarSettings::parse(settings)?.check(http)?;
        serde_json::to_value(&s).map_err(|e| e.to_string())
    }

    fn needs_token(&self, settings: &Value) -> bool {
        match CalendarSettings::parse(settings) {
            Ok(s) => match (s.source.as_str(), &s.mcp) {
                ("mcp", Some(m)) => m.needs_token(),
                _ => true,
            },
            Err(_) => true,
        }
    }

    fn check_secret(
        &self,
        settings: &Value,
        secret: &str,
        http: &Http,
    ) -> Result<String, &'static str> {
        match CalendarSettings::parse(settings) {
            Ok(s) if s.source == "mcp" => {
                check_user_token(secret)?;
                Ok(secret.to_string())
            }
            _ => check_feed_url(secret, http),
        }
    }

    fn secret_hint(&self, settings: &Value, secret: &str) -> Option<String> {
        match CalendarSettings::parse(settings) {
            Ok(s) if s.source == "mcp" => None,
            _ => url_hint(secret),
        }
    }

    fn secret_audience(&self, settings: &Value) -> String {
        match CalendarSettings::parse(settings) {
            Ok(s) => match (s.source.as_str(), &s.mcp) {
                ("mcp", Some(m)) => format!("mcp:{}", m.audience()),
                (src, _) => src.to_string(),
            },
            Err(_) => "invalid".into(),
        }
    }

    fn private_dir(&self, settings: &Value) -> bool {
        CalendarSettings::parse(settings).is_ok_and(|s| s.source == "mcp")
    }

    fn sync<'a>(&'a self, cx: SyncCx<'a>) -> BoxFuture<'a, Result<Vec<Batch>, SyncError>> {
        Box::pin(self.run(cx))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn http() -> Http {
        Http::new(crate::http::HttpOptions::default()).unwrap()
    }

    fn settings(v: Value) -> CalendarSettings {
        CalendarSettings::parse(&v).unwrap().check(&http()).unwrap()
    }

    fn ev(title: &str, attendees: &[(&str, &str)], organizer: Option<&str>) -> Event {
        let t = Utc::now() + Duration::hours(2);
        Event {
            key: "k".into(),
            title: title.into(),
            start: t,
            end: t + Duration::minutes(30),
            all_day: false,
            location: None,
            url: None,
            description: None,
            busy: true,
            recurring: false,
            private: false,
            hint: Hint::None,
            organizer: organizer.map(str::to_string),
            organizer_is_self: false,
            attendees: attendees
                .iter()
                .map(|(e, n)| Attendee {
                    email: Some(e.to_string()),
                    name: (!n.is_empty()).then(|| n.to_string()),
                    declined: false,
                    resource: false,
                    is_self: false,
                })
                .collect(),
            attendee_count: attendees.len() as u32,
        }
    }

    #[test]
    fn text_is_cleaned() {
        assert_eq!(clean("a\u{202E}b\u{200B}c\x07  d", 100, false), "abc d");
        assert_eq!(clean("one \n\n two", 100, true), "one\n\ntwo");
        assert_eq!(clean("abcdef", 3, false), "abc");
        assert_eq!(
            normal_email("MAILTO:Ana@Example.COM").unwrap(),
            "ana@example.com"
        );
        assert!(normal_email("mailto:nobody").is_none());
        assert_eq!(
            conference_url("Join: https://acme.zoom.us/j/123?pwd=x. Thanks").unwrap(),
            "https://acme.zoom.us/j/123?pwd=x"
        );
    }

    #[test]
    fn settings_are_checked() {
        let h = http();
        assert!(CalendarSettings::parse(&json!({"source": "ics", "url": "https://x"})).is_err());
        let s = settings(json!({"source": "ics", "me": ["Me@Acme.com"]}));
        assert_eq!(s.me, vec!["me@acme.com"]);
        assert_eq!(s.item_types, vec![DEV_MEETING, EM_MEETING]);
        for bad in [
            json!({"source": "caldav"}),
            json!({"source": "ics", "item_types": ["dev.ask"]}),
            json!({"source": "ics", "item_types": []}),
            json!({"source": "ics", "timezone": "Mars/Base"}),
            json!({"source": "ics", "me": ["nope"]}),
            json!({"source": "mcp"}),
        ] {
            assert!(
                CalendarSettings::parse(&bad)
                    .and_then(|s| s.check(&h))
                    .is_err(),
                "{bad}"
            );
        }
    }

    #[test]
    fn feed_urls_and_hints() {
        let h = http();
        assert_eq!(
            check_feed_url("webcal://example.com/cal.ics", &h).unwrap(),
            "https://example.com/cal.ics"
        );
        assert_eq!(
            check_feed_url("http://example.com/x", &h),
            Err("invalid_url")
        );
        assert_eq!(
            check_feed_url("https://u:p@example.com/x", &h),
            Err("invalid_url")
        );
        assert_eq!(check_feed_url("not a url", &h), Err("invalid_url"));
        assert_eq!(
            url_hint(
                "https://calendar.google.com/calendar/ical/me%40x.com/private-0123abcd/basic.ics"
            )
            .unwrap(),
            "calendar.google.com"
        );
        assert_eq!(
            url_hint("https://outlook.office365.com/owa/calendar/a/b/reachcalendar.ics").unwrap(),
            "outlook.office365.com"
        );
        assert_eq!(url_hint("https://x.com/a").unwrap(), "x.com");
    }

    #[test]
    fn categories() {
        let s = settings(json!({"source": "ics", "me": ["me@acme.com"]}));
        let me = ("me@acme.com", "Me");
        assert_eq!(
            category(&ev("Ana / me", &[me, ("ana@acme.com", "Ana")], None), &s),
            "1on1"
        );
        assert_eq!(
            category(
                &ev(
                    "Sync",
                    &[me, ("a@acme.com", ""), ("b@vendor.io", "")],
                    Some("b@vendor.io")
                ),
                &s
            ),
            "external"
        );
        assert_eq!(
            category(
                &ev(
                    "Daily standup",
                    &[me, ("a@acme.com", ""), ("b@acme.com", "")],
                    Some("a@acme.com")
                ),
                &s
            ),
            "team"
        );
        assert_eq!(category(&ev("Focus time", &[], None), &s), "focus");
        assert_eq!(category(&ev("Dentist", &[], None), &s), "focus");
        assert_eq!(
            category(
                &ev(
                    "Arch review",
                    &[me, ("a@acme.com", ""), ("b@acme.com", "")],
                    Some("me@acme.com")
                ),
                &s
            ),
            "cross-team"
        );
    }

    #[test]
    fn items_narrow_to_their_pack_role_when_connected_for_it() {
        let both = vec!["developer".to_string(), "eng-manager".to_string()];
        let mut a = json!({"type_id": "dev.meeting"});
        assert!(set_roles(&mut a, &both));
        assert_eq!(a["roles"], json!(["developer"]));
        let mut b = json!({"type_id": "em.meeting"});
        assert!(set_roles(&mut b, &both));
        assert_eq!(b["roles"], json!(["eng-manager"]));
        // No roles means every role: still the pack's role only.
        let mut e = json!({"type_id": "em.one_on_one"});
        assert!(set_roles(&mut e, &[]));
        assert_eq!(e["roles"], json!(["eng-manager"]));
        // Not connected for the pack's role: not written at all.
        let mut c = json!({"type_id": "em.meeting"});
        assert!(!set_roles(&mut c, &["developer".to_string()]));
        assert!(!type_allowed("em.one_on_one", &["developer".to_string()]));
        assert!(type_allowed("dev.meeting", &["developer".to_string()]));
        let mut d = json!({"type_id": "other"});
        assert!(set_roles(&mut d, &both));
        assert!(d.get("roles").is_none());
    }

    #[test]
    fn links_plan_patches_and_creates() {
        let s = settings(json!({"source": "ics", "me": ["me@acme.com"]}));
        let e = ev(
            "Ana / me",
            &[("me@acme.com", "Me"), ("ana.lopez@acme.com", "Ana López")],
            None,
        );
        let people = vec![
            json!({"id": "P1", "type_id": PERSON, "title": "Ana López", "tags": [], "fields": {"name": "Ana López"}}),
            json!({"id": "P2", "type_id": PERSON, "title": "Bo", "tags": [], "fields": {"name": "Bo"}}),
        ];
        let p = plan_links(
            std::slice::from_ref(&e),
            &people,
            &[],
            &s,
            "ics",
            "integration:cal",
            Utc::now(),
        );
        assert_eq!(p.create.len(), 1);
        assert_eq!(p.create[0]["fields"]["person_id"], "P1");
        assert!(p.patches.is_empty());

        let open = json!({"id": "O1", "type_id": ONE_ON_ONE, "version": 4, "tags": [],
            "origin": "integration:cal",
            "fields": {"person_id": "P1", "scheduled_for": "2020-01-01T00:00:00Z",
                       "agenda": [{"text": "keep me"}]}});
        let p = plan_links(
            std::slice::from_ref(&e),
            &people,
            std::slice::from_ref(&open),
            &s,
            "ics",
            "integration:cal",
            Utc::now(),
        );
        assert!(p.create.is_empty());
        assert_eq!(p.patches.len(), 1);
        assert_eq!(p.patches[0].item_id, "O1");
        assert_eq!(p.patches[0].if_version, Some(4));
        assert_eq!(
            p.patches[0].fields.keys().collect::<Vec<_>>(),
            vec!["scheduled_for"]
        );

        // Already right: nothing to do.
        let mut same = open;
        same["fields"]["scheduled_for"] = json!(rfc3339(e.start));
        let p = plan_links(
            std::slice::from_ref(&e),
            &people,
            std::slice::from_ref(&same),
            &s,
            "ics",
            "integration:cal",
            Utc::now(),
        );
        assert_eq!(p, LinkPlan::default());

        // Someone else's open 1:1: neither moved nor duplicated.
        let mut theirs = same.clone();
        theirs["fields"]["scheduled_for"] = json!("2020-01-01T00:00:00Z");
        theirs["origin"] = json!("user");
        let p = plan_links(
            std::slice::from_ref(&e),
            &people,
            &[theirs],
            &s,
            "ics",
            "integration:cal",
            Utc::now(),
        );
        assert_eq!(p, LinkPlan::default());

        // Ambiguous names: no link.
        let twins = vec![
            people[0].clone(),
            json!({"id": "P3", "type_id": PERSON, "tags": [], "fields": {"name": "Ana López"}}),
        ];
        assert_eq!(
            plan_links(
                std::slice::from_ref(&e),
                &twins,
                &[],
                &s,
                "ics",
                "integration:cal",
                Utc::now()
            ),
            LinkPlan::default()
        );

        // Three people: not a 1:1.
        let group = ev(
            "x",
            &[
                ("me@acme.com", ""),
                ("ana.lopez@acme.com", "Ana López"),
                ("bo@acme.com", "Bo"),
            ],
            None,
        );
        assert_eq!(
            plan_links(
                &[group],
                &people,
                &[],
                &s,
                "ics",
                "integration:cal",
                Utc::now()
            ),
            LinkPlan::default()
        );

        // An address on the person wins over names.
        let by_mail = vec![
            json!({"id": "P9", "type_id": PERSON, "tags": [], "fields": {"name": "Someone Else", "email": "Ana.Lopez@acme.com"}}),
        ];
        let p = plan_links(
            &[e],
            &by_mail,
            &[],
            &s,
            "ics",
            "integration:cal",
            Utc::now(),
        );
        assert_eq!(p.create[0]["fields"]["person_id"], "P9");
    }
}
