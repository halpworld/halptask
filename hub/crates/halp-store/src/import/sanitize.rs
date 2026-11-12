//! Port of the TUI's `SanitizeTerminalEscapeArtifacts` (`model/sanitize.go`):
//! strips leaked terminal query responses (OSC colour reports, cursor
//! position reports) that older TUI builds could write into item text.

use std::sync::LazyLock;

use regex::Regex;

static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        r"(?i)(?:\x1b\]|\])\d+;rgb:[0-9a-f/]+(?:\\|\x07|\x1b\\)?",
        r"(?i)(?:\x1b\]|\])\d+;rgb:[0-9a-f/]*",
        r"(?:\x1b\[|\[)\??\d+;\d+[cR]",
    ]
    .iter()
    .filter_map(|p| Regex::new(p).ok())
    .collect()
});

pub fn terminal_artifacts(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    let mut out = text.to_string();
    for re in PATTERNS.iter() {
        out = re.replace_all(&out, "").into_owned();
    }
    if out.starts_with('\n') && !text.starts_with('\n') {
        out.remove(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_go_cases() {
        assert_eq!(PATTERNS.len(), 3);
        assert_eq!(
            terminal_artifacts("]11;rgb:2828/2c2c/3434\\Buy milk"),
            "Buy milk"
        );
        assert_eq!(
            terminal_artifacts("\x1b]10;rgb:1a1a/1b1b/2626\x07Task"),
            "Task"
        );
        assert_eq!(terminal_artifacts("Task[53;1R"), "Task");
        assert_eq!(terminal_artifacts("\x1b[?1;2cHello"), "Hello");
        assert_eq!(terminal_artifacts("Plain [text] ok"), "Plain [text] ok");
        assert_eq!(terminal_artifacts(""), "");
    }
}
