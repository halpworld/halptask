//! Error text that may be shown, stored (status events) or logged: tokens,
//! credentials in URLs, query strings and control characters removed.

use std::sync::LazyLock;

use halp_domain::integration::strip_controls;
use regex::Regex;

/// Longest error text kept.
pub const MAX_ERROR_CHARS: usize = 300;

static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // GitHub, Slack, Linear, GitLab, OpenAI-style and JWT tokens.
        r"gh[pousr]_[A-Za-z0-9]{10,}",
        r"github_pat_[A-Za-z0-9_]{10,}",
        r"xox[abprs]-[A-Za-z0-9-]{6,}",
        r"lin_(api|oauth)_[A-Za-z0-9]{10,}",
        r"glpat-[A-Za-z0-9_-]{10,}",
        r"sk-[A-Za-z0-9_-]{16,}",
        r"eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        // `Bearer x`, `token x`, `access_token=x`, `code=x`, ...
        r"(?i)\b(bearer|token|basic)\s+[A-Za-z0-9._~+/=-]{8,}",
        r#"(?i)\b(access_token|refresh_token|id_token|client_secret|code|code_verifier|password|api_key|apikey|secret)\b["']?\s*[:=]\s*["']?[^\s&"',}]+"#,
        // Anything else that looks like a long random credential.
        r"\b[A-Za-z0-9_-]{40,}\b",
    ]
    .iter()
    .filter_map(|p| Regex::new(p).ok())
    .collect()
});

static URL_QUERY: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(https?://)([^/\s@]+@)?([^\s?#]*)[?#][^\s]*").ok());

/// Redacts `text`, also removing every exact occurrence of `known` secrets.
pub fn redact(text: &str, known: &[&str]) -> String {
    let mut s = strip_controls(text);
    for k in known {
        if k.len() >= 4 {
            s = s.replace(k, "[redacted]");
        }
    }
    if let Some(re) = URL_QUERY.as_ref() {
        s = re.replace_all(&s, "$1$3?…").into_owned();
    }
    for re in PATTERNS.iter() {
        s = re.replace_all(&s, "[redacted]").into_owned();
    }
    let mut out: String = s.chars().take(MAX_ERROR_CHARS).collect();
    if s.chars().count() > MAX_ERROR_CHARS {
        out.push('…');
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_url_credentials_are_removed() {
        let r = redact(
            "GET https://user:pw@api.github.com/x?access_token=abc failed: Bearer ghp_abcdefghijklmnop1234 \u{1b}[0m",
            &[],
        );
        assert!(!r.contains("ghp_"), "{r}");
        assert!(!r.contains("pw@"), "{r}");
        assert!(
            !r.contains("abc failed") || !r.contains("access_token=abc"),
            "{r}"
        );
        assert!(!r.contains('\u{1b}'));
        assert!(r.contains("api.github.com/x"), "{r}");
    }

    #[test]
    fn known_secrets_are_removed_anywhere() {
        let r = redact("server said: s3cr3t-value is wrong", &["s3cr3t-value"]);
        assert_eq!(r, "server said: [redacted] is wrong");
    }

    #[test]
    fn long_text_is_cut() {
        assert!(redact(&"word ".repeat(200), &[]).chars().count() <= MAX_ERROR_CHARS + 1);
    }
}
