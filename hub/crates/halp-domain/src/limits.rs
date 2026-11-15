//! Input hardening. Every value that enters the domain from a command, a pack or
//! an imported file passes through these checks.

use crate::error::{DomainError, Result};
use serde_json::Value;

pub const MAX_TITLE_CHARS: usize = 2_000;
pub const MAX_NOTES_BYTES: usize = 256 * 1024;
pub const MAX_TAGS_PER_ITEM: usize = 32;
pub const MAX_TAG_CHARS: usize = 64;
pub const MAX_FIELDS_BYTES: usize = 32 * 1024;
pub const MAX_FIELDS_DEPTH: usize = 8;
pub const MAX_TREE_DEPTH: usize = 64;
pub const MAX_ITEMS: usize = 200_000;
pub const MAX_SCHEMA_BYTES: usize = 64 * 1024;
pub const MAX_SCHEMA_DEPTH: usize = 32;
pub const MAX_WIDGET_CONFIG_BYTES: usize = 16 * 1024;
pub const MAX_WIDGET_CONFIG_DEPTH: usize = 8;
pub const MAX_TEXT_QUERY_CHARS: usize = 200;
pub const MAX_LIST_LIMIT: usize = 1_000;
pub const DEFAULT_LIST_LIMIT: usize = 200;

/// Invisible Unicode format characters (general category Cf) that can
/// reorder or hide text: bidi marks, embeddings, overrides and isolates
/// (U+200E/F, U+202A-E, U+2066-9, U+061C), zero-width spaces, invisible
/// operators, the BOM and the tag characters (U+E0000 block, a way to
/// smuggle hidden text to a model). ZWNJ (U+200C) and ZWJ (U+200D) stay
/// allowed: scripts and emoji need them.
pub fn is_format_control(c: char) -> bool {
    matches!(
        c,
        '\u{AD}'
            | '\u{600}'..='\u{605}'
            | '\u{61C}'
            | '\u{6DD}'
            | '\u{70F}'
            | '\u{890}'..='\u{891}'
            | '\u{8E2}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200E}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{110BD}'
            | '\u{110CD}'
            | '\u{13430}'..='\u{1343F}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0001}'
            | '\u{E0020}'..='\u{E007F}'
            // Variation selectors supplement: invisible, and a known way to
            // smuggle bytes into text a model reads.
            | '\u{E0100}'..='\u{E01EF}'
    )
}

/// Characters no single-line text may hold: controls, format characters,
/// and the Unicode line and paragraph separators (U+2028, U+2029).
fn bad_line_char(c: char) -> bool {
    c.is_control() || is_format_control(c) || matches!(c, '\u{2028}' | '\u{2029}')
}

/// Whether `c` is one of the BMP variation selectors (U+FE00-FE0F).
fn is_variation_selector(c: char) -> bool {
    matches!(c, '\u{FE00}'..='\u{FE0F}')
}

/// The first misused BMP variation selector in single-line text, if any.
/// U+FE00-FE0D (standardized variation sequences for CJK and math, never
/// needed in titles) are refused outright. U+FE0E (text style) and U+FE0F
/// (emoji style) are allowed once, directly after a visible base character
/// (`❤\u{FE0F}`, `1\u{FE0F}\u{20E3}`); at the start, after white space or
/// after another selector they are refused, so runs of selectors cannot
/// smuggle hidden bytes into text a model reads.
pub fn misused_variation_selector(t: &str) -> Option<char> {
    let mut prev: Option<char> = None;
    for c in t.chars() {
        if matches!(c, '\u{FE00}'..='\u{FE0D}') {
            return Some(c);
        }
        if matches!(c, '\u{FE0E}' | '\u{FE0F}')
            && prev.is_none_or(|p| is_variation_selector(p) || p.is_whitespace() || p.is_control())
        {
            return Some(c);
        }
        prev = Some(c);
    }
    None
}

/// `t` without the variation selectors [`misused_variation_selector`]
/// refuses (for untrusted imported text).
fn strip_misused_variation_selectors(t: &str) -> String {
    let mut out = String::with_capacity(t.len());
    let mut prev: Option<char> = None;
    for c in t.chars() {
        let bad = matches!(c, '\u{FE00}'..='\u{FE0D}')
            || (matches!(c, '\u{FE0E}' | '\u{FE0F}')
                && prev.is_none_or(|p| {
                    is_variation_selector(p) || p.is_whitespace() || p.is_control()
                }));
        if !bad {
            out.push(c);
            prev = Some(c);
        }
    }
    out
}

/// Checks every string in a JSON document (object keys included): no
/// control characters other than line feed and tab, and no invisible
/// format characters ([`is_format_control`]). Applied to typed fields,
/// widget configs and type schemas, whose strings reach the UI and agents
/// like titles and notes do.
pub fn check_json_text(what: &str, v: &Value) -> Result<()> {
    fn bad(s: &str) -> bool {
        s.chars()
            .any(|c| (c.is_control() && !matches!(c, '\n' | '\t')) || is_format_control(c))
    }
    // Iterative to stay safe on hostile input.
    let mut stack: Vec<&Value> = vec![v];
    while let Some(v) = stack.pop() {
        let found = match v {
            Value::String(s) => bad(s),
            Value::Array(a) => {
                stack.extend(a.iter());
                false
            }
            Value::Object(o) => {
                stack.extend(o.values());
                o.keys().any(|k| bad(k))
            }
            _ => false,
        };
        if found {
            return Err(DomainError::invalid(format!(
                "{what} must not contain control characters or invisible format characters (such as bidi overrides)"
            )));
        }
    }
    Ok(())
}

/// Validates and trims a single-line title.
pub fn title(raw: &str) -> Result<String> {
    let t = raw.trim();
    if t.is_empty() {
        return Err(DomainError::invalid("title must not be empty"));
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(DomainError::invalid(format!(
            "title exceeds {MAX_TITLE_CHARS} characters"
        )));
    }
    if t.chars().any(bad_line_char) {
        return Err(DomainError::invalid(
            "title must not contain control characters, line breaks or invisible format characters (such as bidi overrides)",
        ));
    }
    if let Some(c) = misused_variation_selector(t) {
        return Err(DomainError::invalid(format!(
            "title must not contain the variation selector U+{:04X} here (one U+FE0E or U+FE0F directly after a symbol is allowed)",
            c as u32
        )));
    }
    Ok(t.to_string())
}

/// Replaces control characters (including line breaks) with spaces and trims,
/// truncating to the title limit. Used for untrusted imported text.
pub fn sanitize_title(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| !is_format_control(*c))
        .map(|c| if bad_line_char(c) { ' ' } else { c })
        .collect();
    strip_misused_variation_selectors(cleaned.trim())
        .trim()
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect()
}

/// Validates multi-line notes: newlines and tabs allowed, other control
/// characters rejected.
pub fn notes(raw: &str) -> Result<String> {
    if raw.len() > MAX_NOTES_BYTES {
        return Err(DomainError::invalid(format!(
            "notes exceed {MAX_NOTES_BYTES} bytes"
        )));
    }
    if raw
        .chars()
        .any(|c| (c.is_control() && !matches!(c, '\n' | '\t' | '\r')) || is_format_control(c))
    {
        return Err(DomainError::invalid(
            "notes must not contain control characters or invisible format characters (such as bidi overrides)",
        ));
    }
    Ok(raw.to_string())
}

/// Removes disallowed control characters from untrusted notes and truncates
/// them on a character boundary.
pub fn sanitize_notes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len().min(MAX_NOTES_BYTES));
    for c in raw.chars() {
        if (c.is_control() && !matches!(c, '\n' | '\t' | '\r')) || is_format_control(c) {
            continue;
        }
        if out.len() + c.len_utf8() > MAX_NOTES_BYTES {
            break;
        }
        out.push(c);
    }
    out
}

fn tag_char_ok(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '-' | '/')
}

/// Normalises a tag: strips one leading `#`, lowercases, then checks the
/// charset. A tag starts with an ASCII letter (as in the TUI, so `#123` issue
/// numbers are not tags) and continues with letters, digits, `_`, `-` or `/`.
pub fn tag(raw: &str) -> Result<String> {
    let t = raw.trim();
    let t = t.strip_prefix('#').unwrap_or(t).to_lowercase();
    let mut chars = t.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => {
            return Err(DomainError::invalid(format!(
                "tag {raw:?} must start with a letter"
            )));
        }
    }
    if t.chars().count() > MAX_TAG_CHARS {
        return Err(DomainError::invalid(format!(
            "tag exceeds {MAX_TAG_CHARS} characters"
        )));
    }
    if !chars.all(tag_char_ok) {
        return Err(DomainError::invalid(format!(
            "tag {raw:?} may only contain letters, digits, '_', '-' and '/'"
        )));
    }
    Ok(t)
}

/// Best-effort repair of an untrusted tag: invalid characters become `-`.
/// Returns `None` when nothing usable remains.
pub fn sanitize_tag(raw: &str) -> Option<String> {
    let t = raw.trim();
    let t = t.strip_prefix('#').unwrap_or(t).to_lowercase();
    let repaired: String = t
        .chars()
        .map(|c| if tag_char_ok(c) { c } else { '-' })
        .take(MAX_TAG_CHARS)
        .collect();
    let repaired = repaired.trim_matches('-').to_string();
    tag(&repaired).ok()
}

/// Validates and de-duplicates a tag list, keeping first-seen order.
pub fn tags(raw: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in raw {
        let t = tag(t)?;
        if !out.contains(&t) {
            out.push(t);
        }
    }
    if out.len() > MAX_TAGS_PER_ITEM {
        return Err(DomainError::invalid(format!(
            "at most {MAX_TAGS_PER_ITEM} tags per item"
        )));
    }
    Ok(out)
}

/// Nesting depth of a JSON value (scalars are depth 0).
pub fn json_depth(v: &Value) -> usize {
    // Iterative to stay safe on hostile input.
    let mut max = 0;
    let mut stack: Vec<(&Value, usize)> = vec![(v, 0)];
    while let Some((v, d)) = stack.pop() {
        max = max.max(d);
        match v {
            Value::Array(a) => stack.extend(a.iter().map(|x| (x, d + 1))),
            Value::Object(o) => stack.extend(o.values().map(|x| (x, d + 1))),
            _ => {}
        }
    }
    max
}

/// Serialized size in bytes.
pub fn json_size(v: &Value) -> usize {
    serde_json::to_vec(v).map(|b| b.len()).unwrap_or(usize::MAX)
}

/// Checks size and depth of a JSON document.
pub fn json_bounds(what: &str, v: &Value, max_bytes: usize, max_depth: usize) -> Result<()> {
    if json_depth(v) > max_depth {
        return Err(DomainError::invalid(format!(
            "{what} nests deeper than {max_depth} levels"
        )));
    }
    if json_size(v) > max_bytes {
        return Err(DomainError::invalid(format!(
            "{what} exceeds {max_bytes} bytes"
        )));
    }
    Ok(())
}

/// Identifier charset used for widget ids and similar client-chosen ids.
pub fn simple_id(what: &str, raw: &str) -> Result<()> {
    if raw.is_empty()
        || raw.len() > 64
        || !raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(DomainError::invalid(format!(
            "{what} must be 1-64 characters of [A-Za-z0-9_.-]"
        )));
    }
    Ok(())
}

/// Short single-line label (widget title, task type title, icon).
pub fn label(what: &str, raw: &str, max_chars: usize) -> Result<String> {
    let t = raw.trim();
    if t.chars().count() > max_chars {
        return Err(DomainError::invalid(format!(
            "{what} exceeds {max_chars} characters"
        )));
    }
    if t.chars().any(bad_line_char) {
        return Err(DomainError::invalid(format!(
            "{what} must not contain control characters or invisible format characters"
        )));
    }
    if let Some(c) = misused_variation_selector(t) {
        return Err(DomainError::invalid(format!(
            "{what} must not contain the variation selector U+{:04X} here (one U+FE0E or U+FE0F directly after a symbol is allowed)",
            c as u32
        )));
    }
    Ok(t.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn titles() {
        assert_eq!(title("  Fix login ").ok().as_deref(), Some("Fix login"));
        assert!(title("").is_err());
        assert!(title("   ").is_err());
        assert!(title("a\nb").is_err());
        assert!(title("a\u{1b}[31m").is_err());
        assert!(title(&"x".repeat(MAX_TITLE_CHARS + 1)).is_err());
        assert!(title(&"é".repeat(MAX_TITLE_CHARS)).is_ok());
        assert_eq!(sanitize_title(" a\nb\u{7}c "), "a b c");
    }

    #[test]
    fn notes_rules() {
        assert!(notes("line1\nline2\ttab").is_ok());
        assert!(notes("bell\u{7}").is_err());
        assert!(notes(&"x".repeat(MAX_NOTES_BYTES + 1)).is_err());
        assert_eq!(sanitize_notes("a\u{0}b\nc"), "ab\nc");
        assert!(sanitize_notes(&"é".repeat(MAX_NOTES_BYTES)).len() <= MAX_NOTES_BYTES);
    }

    #[test]
    fn tag_rules() {
        assert_eq!(tag("#Bug").ok().as_deref(), Some("bug"));
        assert_eq!(tag("team/backend").ok().as_deref(), Some("team/backend"));
        assert_eq!(tag("überall").ok(), None, "must start with ASCII letter");
        assert_eq!(tag("a-ü_1").ok().as_deref(), Some("a-ü_1"));
        assert!(tag("123").is_err());
        assert!(tag("has space").is_err());
        assert!(tag("semi;colon").is_err());
        assert!(tag("").is_err());
        assert!(tag(&"a".repeat(65)).is_err());
        assert_eq!(sanitize_tag("bad tag!").as_deref(), Some("bad-tag"));
        assert_eq!(sanitize_tag("!!!"), None);
        let many: Vec<String> = (0..40).map(|i| format!("t{i}")).collect();
        assert!(tags(&many).is_err());
        assert_eq!(
            tags(&["A".into(), "a".into(), "b".into()]).ok(),
            Some(vec!["a".to_string(), "b".to_string()])
        );
    }

    /// Finding: bidi overrides and other invisible format characters let a
    /// title read differently from what it holds.
    #[test]
    fn format_controls_are_rejected_and_stripped_on_import() {
        for c in [
            '\u{200E}',
            '\u{200F}',
            '\u{202A}',
            '\u{202E}',
            '\u{2066}',
            '\u{2069}',
            '\u{061C}',
            '\u{200B}',
            '\u{FEFF}',
            '\u{2060}',
            '\u{E0041}',
            '\u{AD}',
        ] {
            let s = format!("pay {c}evil");
            assert!(title(&s).is_err(), "{c:?} title");
            assert!(notes(&s).is_err(), "{c:?} notes");
            assert!(label("widget title", &s, 80).is_err(), "{c:?} label");
            assert!(tag(&format!("a{c}b")).is_err(), "{c:?} tag");
            assert_eq!(sanitize_title(&s), "pay evil");
            assert_eq!(sanitize_notes(&s), "pay evil");
            assert_eq!(sanitize_tag(&format!("ab{c}")).as_deref(), Some("ab"));
        }
        // ZWJ and ZWNJ are part of real text and emoji.
        let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
        assert!(title(family).is_ok());
        assert!(title("\u{645}\u{200C}\u{6CC}").is_ok());
        assert!(notes("a\u{200D}b").is_ok());
    }

    #[test]
    fn json_limits() {
        assert_eq!(json_depth(&json!(1)), 0);
        assert_eq!(json_depth(&json!({"a": [1, {"b": 2}]})), 3);
        let mut deep = json!(0);
        for _ in 0..20 {
            deep = json!([deep]);
        }
        assert!(json_bounds("fields", &deep, 1024, 8).is_err());
        assert!(json_bounds("fields", &json!({"a": "x".repeat(2000)}), 1024, 8).is_err());
        assert!(json_bounds("fields", &json!({"a": 1}), 1024, 8).is_ok());
    }

    #[test]
    fn json_text_rejects_hidden_characters_in_every_string() {
        assert!(
            check_json_text("fields", &json!({"a": "line\nnext\tcol", "b": [1, "ok"]})).is_ok()
        );
        for bad in [
            "\u{202E}",
            "\u{200B}",
            "\u{7}",
            "\r",
            "\u{E0041}",
            "\u{E0100}",
            "\u{FEFF}",
        ] {
            let deep = json!({"a": [{"b": format!("x{bad}y")}]});
            assert!(
                check_json_text("fields", &deep).is_err(),
                "{bad:?} in a value"
            );
            let mut m = serde_json::Map::new();
            m.insert(format!("k{bad}"), json!(1));
            assert!(
                check_json_text("fields", &Value::Object(m)).is_err(),
                "{bad:?} in a key"
            );
        }
        // ZWJ stays allowed (emoji sequences).
        assert!(check_json_text("fields", &json!({"a": "a\u{200D}b"})).is_ok());
    }

    #[test]
    fn line_separators_and_variation_selectors_are_refused() {
        assert!(title("a\u{2028}b").is_err());
        assert!(title("a\u{2029}b").is_err());
        assert!(label("x", "a\u{2028}b", 10).is_err());
        assert!(title("a\u{E0100}b").is_err());
        assert!(notes("a\u{E01EF}b").is_err());
        assert!(notes("para\u{2029}ok").is_ok());
        assert_eq!(sanitize_title("a\u{E0101}b"), "ab");
    }

    #[test]
    fn bmp_variation_selectors_only_after_a_symbol() {
        // One emoji or text selector after a visible base stays allowed.
        for ok in [
            "Ship it \u{2764}\u{FE0F}",
            "Keycap 1\u{FE0F}\u{20E3}",
            "Text style \u{2709}\u{FE0E}",
            "\u{1F441}\u{FE0F}\u{200D}\u{1F5E8}\u{FE0F} witness",
        ] {
            assert!(title(ok).is_ok(), "{ok:?}");
            assert!(label("x", ok, 100).is_ok(), "{ok:?}");
            assert_eq!(sanitize_title(ok), ok);
        }
        for bad in [
            "a\u{FE0F}\u{FE0F}b",
            "a\u{FE0E}\u{FE0F}b",
            "\u{2764}\u{FE0F}\u{FE0F}",
            "\u{FE0F}start",
            "a \u{FE0F}b",
            "a\u{FE00}b",
            "a\u{FE0D}b",
        ] {
            assert!(title(bad).is_err(), "{bad:?}");
            assert!(label("x", bad, 100).is_err(), "{bad:?}");
        }
        assert_eq!(sanitize_title("a\u{FE0F}\u{FE0F}\u{FE0F}b"), "a\u{FE0F}b");
        assert_eq!(sanitize_title("a\u{FE01}b"), "ab");
        assert!(title("a\u{FE0F}\u{FE0F}").is_err_and(|e| e.to_string().contains("U+FE0F")));
    }
}
