//! Natural-language due dates, ported from the TUI's `model/dateparser.go`.
//!
//! Everything is relative to an injected `today` so behaviour is testable.
//! Due dates are calendar dates (the Hub stores `YYYY-MM-DD`), so where the
//! TUI produced "23:59:59 local time" we produce the date itself.
//!
//! Deliberate differences from the TUI:
//! - Month offsets clamp to the last day of the month (Jan 31 + 1m = Feb 28)
//!   instead of overflowing into the next month.
//! - Offsets are bounded (|days| <= 36500, |weeks| <= 5200, |months| <= 1200).
//! - Month-day dates without a year must exist in the current year.

use chrono::{Datelike, Days, Months, NaiveDate, Weekday};

use crate::error::{DomainError, Result};

const MAX_DAYS: i64 = 36_500;
const MAX_WEEKS: i64 = 5_200;
const MAX_MONTHS: i64 = 1_200;

/// A parsed due date and the normalised raw text it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDue {
    pub date: NaiveDate,
    pub raw: String,
}

/// Cleans raw due tokens such as `due:tomorrow` or `@due(2026-08-25)`.
pub fn normalize_due_input(raw: &str) -> String {
    let mut s = raw.trim();
    if let Some(rest) = s.strip_prefix("@due(") {
        s = rest;
        if raw.starts_with("@due(") {
            s = s.strip_suffix(')').unwrap_or(s);
        }
    }
    s = s.strip_prefix("due:").unwrap_or(s);
    s = s.strip_prefix("due=").unwrap_or(s);
    s.trim_matches(|c| c == '"' || c == '\'' || c == '`')
        .trim()
        .to_string()
}

fn invalid(input: &str) -> DomainError {
    DomainError::invalid(format!("invalid date format: {input:?}"))
}

fn weekday(name: &str) -> Option<Weekday> {
    Some(match name {
        "sunday" | "sun" => Weekday::Sun,
        "monday" | "mon" => Weekday::Mon,
        "tuesday" | "tue" => Weekday::Tue,
        "wednesday" | "wed" => Weekday::Wed,
        "thursday" | "thu" => Weekday::Thu,
        "friday" | "fri" => Weekday::Fri,
        "saturday" | "sat" => Weekday::Sat,
        _ => return None,
    })
}

fn month_name(name: &str) -> Option<u32> {
    const FULL: [&str; 12] = [
        "january",
        "february",
        "march",
        "april",
        "may",
        "june",
        "july",
        "august",
        "september",
        "october",
        "november",
        "december",
    ];
    let lower = name.to_ascii_lowercase();
    FULL.iter()
        .position(|m| *m == lower || (lower.len() == 3 && m.starts_with(&lower)))
        .map(|i| i as u32 + 1)
}

fn num(s: &str, min_len: usize, max_len: usize) -> Option<u32> {
    if s.len() < min_len || s.len() > max_len || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

fn offset(lower: &str, suffix: char) -> Option<i64> {
    let body = lower.strip_suffix(suffix)?;
    let body = body.strip_prefix('+').unwrap_or(body);
    body.parse::<i64>().ok().filter(|n| *n != 0)
}

fn add_days(today: NaiveDate, n: i64) -> Option<NaiveDate> {
    if n >= 0 {
        today.checked_add_days(Days::new(n.unsigned_abs()))
    } else {
        today.checked_sub_days(Days::new(n.unsigned_abs()))
    }
}

/// Parses a numeric or named calendar date (the TUI's layout list).
fn parse_calendar(val: &str, today: NaiveDate) -> Option<NaiveDate> {
    // Y-M-D / Y/M/D with a 4-digit year; month and day 1 or 2 digits.
    for sep in ['-', '/'] {
        let parts: Vec<&str> = val.split(sep).collect();
        if parts.len() == 3 {
            let (y, m, d) = (
                num(parts[0], 4, 4)?,
                num(parts[1], 1, 2)?,
                num(parts[2], 1, 2)?,
            );
            return NaiveDate::from_ymd_opt(y as i32, m, d);
        }
        if parts.len() == 2 {
            let (m, d) = (num(parts[0], 1, 2)?, num(parts[1], 1, 2)?);
            return NaiveDate::from_ymd_opt(today.year(), m, d);
        }
    }
    // "Jan 2", "January 02", "2 Jan", "02 January".
    let parts: Vec<&str> = val.split(' ').collect();
    if parts.len() == 2 {
        if let (Some(m), Some(d)) = (month_name(parts[0]), num(parts[1], 1, 2)) {
            return NaiveDate::from_ymd_opt(today.year(), m, d);
        }
        if let (Some(d), Some(m)) = (num(parts[0], 1, 2), month_name(parts[1])) {
            return NaiveDate::from_ymd_opt(today.year(), m, d);
        }
    }
    None
}

/// Parses a due string relative to `today`.
///
/// Accepts `today`/`tdy`, `tomorrow`/`tmrw`, `yesterday`/`ydy`, weekday names
/// (next occurrence, a week out when it is today), offsets `+3d`, `2w`, `+1m`
/// (negative allowed), ISO-ish dates `2026-09-15`, `2026/9/15`, month-day
/// `09/15`, `9-15`, and `Jan 2` / `2 January`. `due:` / `due=` / `@due(...)`
/// wrappers are stripped first.
pub fn parse_due(input: &str, today: NaiveDate) -> Result<ParsedDue> {
    let val = normalize_due_input(input);
    if val.is_empty() || val.len() > 64 {
        return Err(invalid(input));
    }
    let lower = val.to_lowercase();
    let ok = |date: NaiveDate| {
        Ok(ParsedDue {
            date,
            raw: val.clone(),
        })
    };

    match lower.as_str() {
        "today" | "tdy" => return ok(today),
        "tomorrow" | "tmrw" => return add_days(today, 1).map_or_else(|| Err(invalid(input)), ok),
        "yesterday" | "ydy" => return add_days(today, -1).map_or_else(|| Err(invalid(input)), ok),
        _ => {}
    }

    if let Some(target) = weekday(&lower) {
        let cur = today.weekday().num_days_from_sunday() as i64;
        let tgt = target.num_days_from_sunday() as i64;
        let mut days = (tgt - cur + 7) % 7;
        if days == 0 {
            days = 7;
        }
        return add_days(today, days).map_or_else(|| Err(invalid(input)), ok);
    }

    if let Some(n) = offset(&lower, 'd') {
        if n.abs() > MAX_DAYS {
            return Err(invalid(input));
        }
        return add_days(today, n).map_or_else(|| Err(invalid(input)), ok);
    }
    if let Some(n) = offset(&lower, 'w') {
        if n.abs() > MAX_WEEKS {
            return Err(invalid(input));
        }
        return add_days(today, n * 7).map_or_else(|| Err(invalid(input)), ok);
    }
    if let Some(n) = offset(&lower, 'm') {
        if n.abs() > MAX_MONTHS {
            return Err(invalid(input));
        }
        let months = Months::new(n.unsigned_abs() as u32);
        let d = if n > 0 {
            today.checked_add_months(months)
        } else {
            today.checked_sub_months(months)
        };
        return d.map_or_else(|| Err(invalid(input)), ok);
    }

    parse_calendar(&val, today).map_or_else(|| Err(invalid(input)), ok)
}

/// Result of [`extract_due_token`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DueExtract {
    pub clean_text: String,
    pub due: ParsedDue,
}

/// Finds an inline `@due(...)` or `due:xxx` / `due=xxx` token in `text`,
/// returning the text without it. Only the first parseable token is taken.
pub fn extract_due_token(text: &str, today: NaiveDate) -> Option<DueExtract> {
    if let Some(idx) = text.find("@due(")
        && let Some(end_rel) = text[idx..].find(')')
    {
        let inside = &text[idx + 5..idx + end_rel];
        if let Ok(due) = parse_due(inside, today) {
            let left = text[..idx].trim_end();
            let right = text[idx + end_rel + 1..].trim_start();
            let clean_text = if left.is_empty() || right.is_empty() {
                format!("{left}{right}")
            } else {
                format!("{left} {right}")
            };
            return Some(DueExtract {
                clean_text: clean_text.trim().to_string(),
                due,
            });
        }
    }

    let mut found: Option<ParsedDue> = None;
    let mut remaining: Vec<&str> = Vec::new();
    for w in text.split_whitespace() {
        let lower = w.to_lowercase();
        if found.is_none()
            && (lower.starts_with("due:") || lower.starts_with("due="))
            && w.len() > 4
        {
            let val = w
                .get(4..)
                .unwrap_or("")
                .trim_matches(|c| c == '"' || c == '\'' || c == '`');
            if let Ok(p) = parse_due(val, today) {
                found = Some(p);
                continue;
            }
        }
        remaining.push(w);
    }
    found.map(|due| DueExtract {
        clean_text: remaining.join(" "),
        due,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    // Reference date of the Go tests: Tuesday 2026-08-25.
    fn today() -> NaiveDate {
        d(2026, 8, 25)
    }

    #[test]
    fn go_parse_due_string_cases() {
        let cases: &[(&str, Option<NaiveDate>)] = &[
            ("today", Some(d(2026, 8, 25))),
            ("tdy", Some(d(2026, 8, 25))),
            ("tomorrow", Some(d(2026, 8, 26))),
            ("tmrw", Some(d(2026, 8, 26))),
            ("yesterday", Some(d(2026, 8, 24))),
            ("due:today", Some(d(2026, 8, 25))),
            ("@due(tomorrow)", Some(d(2026, 8, 26))),
            ("+1d", Some(d(2026, 8, 26))),
            ("+3d", Some(d(2026, 8, 28))),
            ("+1w", Some(d(2026, 9, 1))),
            ("2026-09-15", Some(d(2026, 9, 15))),
            ("09/15", Some(d(2026, 9, 15))),
            ("friday", Some(d(2026, 8, 28))),
            ("fri", Some(d(2026, 8, 28))),
            ("tuesday", Some(d(2026, 9, 1))), // today is Tuesday: next week
            ("invalid-date-xyz", None),
        ];
        for (input, want) in cases {
            let got = parse_due(input, today()).ok().map(|p| p.date);
            assert_eq!(got, *want, "input {input:?}");
        }
    }

    #[test]
    fn more_formats() {
        let t = today();
        let p = |s: &str| parse_due(s, t).ok().map(|p| p.date);
        assert_eq!(p("3d"), Some(d(2026, 8, 28)));
        assert_eq!(p("-2d"), Some(d(2026, 8, 23)));
        assert_eq!(p("2w"), Some(d(2026, 9, 8)));
        assert_eq!(p("+1m"), Some(d(2026, 9, 25)));
        assert_eq!(p("2026/9/5"), Some(d(2026, 9, 5)));
        assert_eq!(p("2026-9-5"), Some(d(2026, 9, 5)));
        assert_eq!(p("9-5"), Some(d(2026, 9, 5)));
        assert_eq!(p("Jan 2"), Some(d(2026, 1, 2)));
        assert_eq!(p("jan 02"), Some(d(2026, 1, 2)));
        assert_eq!(p("2 January"), Some(d(2026, 1, 2)));
        assert_eq!(p("02 Jan"), Some(d(2026, 1, 2)));
        assert_eq!(p("September 30"), Some(d(2026, 9, 30)));
        assert_eq!(p("WED"), Some(d(2026, 8, 26)));
        assert_eq!(p("Sunday"), Some(d(2026, 8, 30)));
        assert_eq!(p("due=\"2026-12-24\""), Some(d(2026, 12, 24)));
        // clamping and bounds
        assert_eq!(
            parse_due("+1m", d(2026, 1, 31)).ok().map(|p| p.date),
            Some(d(2026, 2, 28))
        );
        assert_eq!(p("0d"), None);
        assert_eq!(p("99999999999d"), None);
        assert_eq!(p("2026-02-30"), None);
        assert_eq!(p("02/30"), None);
        assert_eq!(p("26-09-15"), None);
        assert_eq!(p(""), None);
        assert_eq!(p("Sept 3"), None);
        assert_eq!(p("13/45"), None);
        // raw text is preserved without wrapper
        assert_eq!(parse_due("@due(Friday)", t).unwrap().raw, "Friday");
    }

    #[test]
    fn go_extract_due_token_cases() {
        let t = today();
        let e = extract_due_token("Fix login bug due:tomorrow #urgent", t).unwrap();
        assert_eq!(e.clean_text, "Fix login bug #urgent");
        assert_eq!(e.due.date, d(2026, 8, 26));
        assert_eq!(e.due.raw, "tomorrow");

        let e = extract_due_token("Write unit tests @due(2026-08-30)", t).unwrap();
        assert_eq!(e.clean_text, "Write unit tests");
        assert_eq!(e.due.date, d(2026, 8, 30));
        assert_eq!(e.due.raw, "2026-08-30");

        assert!(extract_due_token("Normal bullet text without due", t).is_none());
    }

    #[test]
    fn extract_edge_cases() {
        let t = today();
        let e = extract_due_token("a @due(fri) b", t).unwrap();
        assert_eq!(e.clean_text, "a b");
        // unparseable due: token stays in the text
        assert!(extract_due_token("due:someday later", t).is_none());
        // only the first parseable token is consumed
        let e = extract_due_token("x due:today due:tomorrow", t).unwrap();
        assert_eq!(e.clean_text, "x due:tomorrow");
        // unterminated @due( falls through to word scan
        assert!(extract_due_token("x @due(fri", t).is_none());
        // multibyte text around tokens does not panic
        let e = extract_due_token("日本語 due:today ü", t).unwrap();
        assert_eq!(e.clean_text, "日本語 ü");
    }
}
