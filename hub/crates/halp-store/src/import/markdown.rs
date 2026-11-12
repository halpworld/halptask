//! The TUI's legacy Markdown format (`ParseMarkdown` in `model/storage.go`).

use chrono::NaiveDate;
use halp_domain::dates::extract_due_token;

use super::LegacyNode;
use crate::error::{Result, StoreError};

fn take_comment(line: &mut String, marker: &str) -> Option<String> {
    let idx = line.find(marker)?;
    let end_rel = line[idx..].find("-->")?;
    let inner = line[idx + marker.len()..idx + end_rel].trim().to_string();
    let rest = format!("{}{}", &line[..idx], &line[idx + end_rel + 3..]);
    *line = rest.trim().to_string();
    Some(inner)
}

fn take_flag(line: &mut String, a: &str, b: &str) -> bool {
    if line.contains(a) || line.contains(b) {
        *line = line.replace(a, "").replace(b, "").trim().to_string();
        true
    } else {
        false
    }
}

/// Splits `#tags` from text the way the TUI does (tag must start with an
/// ASCII letter; lowercased; de-duplicated).
fn split_tags(text: &str) -> (String, Vec<String>) {
    let mut words = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    for w in text.split_whitespace() {
        if let Some(rest) = w.strip_prefix('#')
            && rest.starts_with(|c: char| c.is_ascii_alphabetic())
        {
            let t = rest.to_lowercase();
            if !tags.contains(&t) && tags.len() <= halp_domain::limits::MAX_TAGS_PER_ITEM {
                tags.push(t);
            }
            continue;
        }
        words.push(w);
    }
    (words.join(" "), tags)
}

pub(super) fn parse(
    content: &str,
    today: NaiveDate,
    max_depth: usize,
    max_items: usize,
    warnings: &mut Vec<String>,
) -> Result<Vec<LegacyNode>> {
    let mut flattened = 0usize;
    // Stack of (depth, path of child indices from the roots).
    let mut roots: Vec<LegacyNode> = Vec::new();
    let mut stack: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut count = 0usize;

    for raw in content.lines() {
        let line = raw.trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        count += 1;
        if count > max_items {
            return Err(StoreError::Import(format!(
                "the file holds more than {max_items} items"
            )));
        }
        let mut indent = 0usize;
        for c in line.chars() {
            match c {
                ' ' => indent += 1,
                '\t' => indent += 2,
                _ => break,
            }
        }
        let depth = indent / 2;

        let mut text = trimmed.to_string();
        let legacy_id = take_comment(&mut text, "<!-- id:").unwrap_or_default();
        let folded = take_flag(&mut text, "<!-- fold -->", "<!-- folded -->");
        let focused = take_flag(&mut text, "<!-- focus -->", "<!-- focused -->");

        let (body, is_task, status) = if let Some(r) = text
            .strip_prefix("- [ ] ")
            .or_else(|| text.strip_prefix("* [ ] "))
        {
            (r, true, 2)
        } else if let Some(r) = text
            .strip_prefix("- [~] ")
            .or_else(|| text.strip_prefix("* [~] "))
        {
            (r, true, 3)
        } else if let Some(r) = ["- [x] ", "- [X] ", "* [x] ", "* [X] "]
            .iter()
            .find_map(|p| text.strip_prefix(p))
        {
            (r, true, 4)
        } else if let Some(r) = ["- ", "* ", "• "].iter().find_map(|p| text.strip_prefix(p)) {
            (r, false, 1)
        } else {
            (text.as_str(), false, 1)
        };

        let mut node = LegacyNode {
            legacy_id,
            is_task,
            status,
            folded,
            focused,
            ..LegacyNode::default()
        };
        let mut body = body.to_string();
        if let Some(ex) = extract_due_token(&body, today) {
            node.due = Some(ex.due.date);
            node.due_text = ex.due.raw;
            body = ex.clean_text;
        }
        let (title, tags) = split_tags(&body);
        node.text = title;
        node.tags = tags;

        // Attach under the nearest shallower ancestor (or as a root).
        while stack.last().is_some_and(|(d, _)| *d >= depth) {
            stack.pop();
        }
        if depth == 0 || stack.is_empty() {
            roots.push(node);
            stack.clear();
            stack.push((0, vec![roots.len() - 1]));
            continue;
        }
        if stack.len() >= max_depth {
            // Too deep: keep the line, attached to the deepest ancestor the
            // limit allows (the TUI accepted any depth).
            flattened += 1;
            stack.truncate(max_depth.saturating_sub(1).max(1));
        }
        let Some((_, path)) = stack.last() else {
            continue;
        };
        let path = path.clone();
        let parent = node_at(&mut roots, &path)
            .ok_or_else(|| StoreError::Import("markdown structure error".into()))?;
        parent.children.push(node);
        let mut child_path = path;
        child_path.push(parent.children.len() - 1);
        stack.push((depth, child_path));
    }
    if flattened > 0 {
        warnings.push(format!(
            "{flattened} line(s) nested deeper than {max_depth} levels were attached to their deepest allowed ancestor"
        ));
    }
    Ok(roots)
}

fn node_at<'a>(roots: &'a mut [LegacyNode], path: &[usize]) -> Option<&'a mut LegacyNode> {
    let (first, rest) = path.split_first()?;
    let mut cur = roots.get_mut(*first)?;
    for i in rest {
        cur = cur.children.get_mut(*i)?;
    }
    Some(cur)
}
