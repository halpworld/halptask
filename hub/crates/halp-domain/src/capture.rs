//! Quick capture, ported from the TUI's `ParseCaptureText`
//! (`model/quickcapture.go`): `"Fix login #bug due:friday"` becomes a title,
//! tags and a raw due string.

use crate::item::Status;
use crate::limits;

/// Parsed quick-capture text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capture {
    pub title: String,
    pub tags: Vec<String>,
    /// Raw due text (`friday`, `2026-09-01`), unparsed.
    pub raw_due: Option<String>,
    /// Status from a `- [ ]`, `- [~]` or `- [x]` prefix; `Todo` otherwise.
    pub status: Status,
    /// True when the text started with a plain bullet marker (`- `, `* `, `• `).
    /// The TUI still captured those as tasks; callers decide.
    pub bullet_prefix: bool,
}

fn strip_status_prefix(text: &str) -> (&str, Status, bool) {
    for (prefixes, st) in [
        (&["- [ ] ", "* [ ] "][..], Status::Todo),
        (&["- [~] ", "* [~] "][..], Status::InProgress),
        (&["- [x] ", "- [X] ", "* [x] ", "* [X] "][..], Status::Done),
    ] {
        for p in prefixes {
            if let Some(rest) = text.strip_prefix(p) {
                return (rest.trim(), st, false);
            }
        }
    }
    for p in ["- ", "* ", "• "] {
        if let Some(rest) = text.strip_prefix(p) {
            return (rest.trim(), Status::Todo, true);
        }
    }
    (text, Status::Todo, false)
}

/// Splits capture text into title, `#tags`, due and status.
///
/// Tags must start with an ASCII letter (`#123` stays in the title) and pass
/// the tag charset; trailing punctuation such as `#bug,` is dropped. A word
/// that looks like a tag but is not a valid one stays in the title.
pub fn parse_capture_text(raw: &str) -> Capture {
    let (text, status, bullet_prefix) = strip_status_prefix(raw.trim());
    let mut title_words: Vec<&str> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut raw_due: Option<String> = None;

    for w in text.split_whitespace() {
        let lower = w.to_lowercase();
        if lower.starts_with("@due(")
            && w.ends_with(')')
            && w.len() > 6
            && let Some(inner) = w.get(5..w.len() - 1)
        {
            raw_due = Some(inner.to_string());
            continue;
        }
        if (lower.starts_with("due:") || lower.starts_with("due=")) && w.len() > 4 {
            let val = w
                .get(4..)
                .unwrap_or("")
                .trim_matches(|c| c == '"' || c == '\'');
            if !val.is_empty() {
                raw_due = Some(val.to_string());
                continue;
            }
        }
        if let Some(rest) = w.strip_prefix('#')
            && rest.starts_with(|c: char| c.is_ascii_alphabetic())
        {
            let trimmed = rest.trim_end_matches(|c: char| ",.;:!?)".contains(c));
            if let Ok(t) = limits::tag(trimmed) {
                if !tags.contains(&t) {
                    tags.push(t);
                }
                continue;
            }
        }
        title_words.push(w);
    }

    Capture {
        title: title_words.join(" "),
        tags,
        raw_due,
        status,
        bullet_prefix,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tags(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn go_parse_capture_text_cases() {
        // (input, title, tags, due, status, bullet_prefix)
        let cases = [
            (
                "Fix Redis connection timeout",
                "Fix Redis connection timeout",
                tags(&[]),
                None,
                Status::Todo,
                false,
            ),
            (
                "Fix Redis timeout #ops #backend due:tomorrow",
                "Fix Redis timeout",
                tags(&["ops", "backend"]),
                Some("tomorrow"),
                Status::Todo,
                false,
            ),
            (
                "- [~] Deploy staging hotfix #dev",
                "Deploy staging hotfix",
                tags(&["dev"]),
                None,
                Status::InProgress,
                false,
            ),
            (
                "- [x] Submit expense report",
                "Submit expense report",
                tags(&[]),
                None,
                Status::Done,
                false,
            ),
            (
                "- General system note",
                "General system note",
                tags(&[]),
                None,
                Status::Todo,
                true,
            ),
            (
                "Quarterly review #planning due=2026-09-01",
                "Quarterly review",
                tags(&["planning"]),
                Some("2026-09-01"),
                Status::Todo,
                false,
            ),
        ];
        for (input, title, tg, due, st, bp) in cases {
            let c = parse_capture_text(input);
            assert_eq!(c.title, title, "{input}");
            assert_eq!(c.tags, tg, "{input}");
            assert_eq!(c.raw_due.as_deref(), due, "{input}");
            assert_eq!(c.status, st, "{input}");
            assert_eq!(c.bullet_prefix, bp, "{input}");
        }
    }

    #[test]
    fn contract_example_and_edges() {
        let c = parse_capture_text("Fix login #bug due:friday");
        assert_eq!(c.title, "Fix login");
        assert_eq!(c.tags, tags(&["bug"]));
        assert_eq!(c.raw_due.as_deref(), Some("friday"));

        let c = parse_capture_text("Close #123 and #Bug, @due(2026-09-15) #bug");
        assert_eq!(c.title, "Close #123 and");
        assert_eq!(c.tags, tags(&["bug"]));
        assert_eq!(c.raw_due.as_deref(), Some("2026-09-15"));

        let c = parse_capture_text("Weird #tag;drop me");
        assert_eq!(c.title, "Weird #tag;drop me");

        let c = parse_capture_text("• Unicode bullet 日本語 #i18n");
        assert_eq!(c.title, "Unicode bullet 日本語");
        assert!(c.bullet_prefix);

        let c = parse_capture_text("   ");
        assert_eq!(c.title, "");
        let c = parse_capture_text("due: alone");
        assert_eq!(c.title, "due: alone");
    }
}
