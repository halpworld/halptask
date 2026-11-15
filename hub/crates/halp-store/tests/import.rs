//! Importer tests against files written by the real Go TUI
//! (`tests/fixtures`, generated from the TUI's `model` package).
#![allow(clippy::unwrap_used)]

use std::collections::HashMap;

use chrono::{FixedOffset, NaiveDate, TimeZone, Utc};
use halp_domain::{ItemKind, Outline, Status};
use halp_store::import::{ImportOptions, ImportedTree, SourceFormat, encrypt_tui_file, parse_tui};
use serde_json::Value;
use zeroize::Zeroizing;

const PLAIN: &[u8] = include_bytes!("fixtures/tui_plain.pb");
const ENCRYPTED: &[u8] = include_bytes!("fixtures/tui_encrypted.pb");
const LEGACY_MD: &[u8] = include_bytes!("fixtures/tui_legacy.md");
const EXPECTED: &str = include_str!("fixtures/tui_expected.json");
const PASS: &str = "correct horse battery staple";

fn opts(pass: Option<&str>, name: Option<&str>) -> ImportOptions {
    ImportOptions {
        passphrase: pass.map(|p| Zeroizing::new(p.to_string())),
        utc_offset: FixedOffset::east_opt(0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
        now: Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap(),
        file_name: name.map(str::to_string),
    }
}

/// Flattens an import into the same shape as `tui_expected.json`.
fn flatten(t: &ImportedTree) -> Vec<HashMap<&'static str, Value>> {
    let outline = Outline::from_items(t.items.clone()).unwrap();
    let legacy: HashMap<_, _> = t.legacy.iter().map(|l| (l.item_id.clone(), l)).collect();
    let mut paths: HashMap<String, String> = HashMap::new();
    let mut out = Vec::new();
    for id in outline.preorder() {
        let it = outline.get(&id).unwrap();
        let path = match &it.parent_id {
            Some(p) => format!("{}.{}", paths[p], it.order),
            None => it.order.to_string(),
        };
        paths.insert(id.clone(), path.clone());
        let l = legacy[&id];
        let status = if it.kind == ItemKind::Task {
            outline.effective_status(&id).unwrap().as_str().to_string()
        } else {
            String::new()
        };
        let mut m = HashMap::new();
        m.insert("path", Value::from(path));
        m.insert("id", Value::from(l.legacy_id.clone()));
        m.insert("text", Value::from(it.title.clone()));
        m.insert("is_task", Value::from(it.kind == ItemKind::Task));
        m.insert("status", Value::from(status));
        m.insert("tags", serde_json::to_value(&it.tags).unwrap());
        m.insert("note", Value::from(it.notes.clone()));
        m.insert(
            "due",
            Value::from(it.due.map(|d| d.to_string()).unwrap_or_default()),
        );
        m.insert("due_text", Value::from(l.due_text.clone()));
        m.insert(
            "created_at",
            Value::from(it.created_at.timestamp_nanos_opt().unwrap()),
        );
        m.insert(
            "updated_at",
            Value::from(it.updated_at.timestamp_nanos_opt().unwrap()),
        );
        m.insert("version", Value::from(it.version));
        m.insert("folded", Value::from(l.folded));
        m.insert("focused", Value::from(l.focused));
        out.push(m);
    }
    out
}

fn expected() -> Vec<serde_json::Map<String, Value>> {
    serde_json::from_str::<Vec<Value>>(EXPECTED)
        .unwrap()
        .into_iter()
        .map(|v| v.as_object().cloned().unwrap())
        .collect()
}

fn assert_matches(t: &ImportedTree, keys: &[&str]) {
    let got = flatten(t);
    let want = expected();
    assert_eq!(got.len(), want.len(), "item count");
    for (g, w) in got.iter().zip(&want) {
        for k in keys {
            assert_eq!(&g[k], &w[*k], "field {k} of {}", w["path"]);
        }
    }
}

const ALL: &[&str] = &[
    "path",
    "id",
    "text",
    "is_task",
    "status",
    "tags",
    "note",
    "due",
    "due_text",
    "created_at",
    "updated_at",
    "version",
    "folded",
    "focused",
];

#[test]
fn plain_protobuf_is_lossless() {
    let t = parse_tui(PLAIN, &opts(None, Some("halptask.pb"))).unwrap();
    assert_eq!(t.format, SourceFormat::Protobuf);
    assert_matches(&t, ALL);
    assert!(t.warnings.is_empty(), "{:?}", t.warnings);
    let focused = t.focused.as_deref().unwrap();
    assert_eq!(
        t.items.iter().find(|i| i.id == focused).unwrap().title,
        "Focused item"
    );
    // ids are fresh ULIDs, distinct
    let mut ids: Vec<_> = t.items.iter().map(|i| i.id.clone()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), t.items.len());
    assert!(ids.iter().all(|i| i.len() == 26));
    // detection also works without the file name hint
    let t2 = parse_tui(PLAIN, &opts(None, None)).unwrap();
    assert_eq!(t2.format, SourceFormat::Protobuf);
}

#[test]
fn encrypted_protobuf_is_lossless() {
    let t = parse_tui(ENCRYPTED, &opts(Some(PASS), Some("halptask.pb"))).unwrap();
    assert_eq!(t.format, SourceFormat::EncryptedProtobuf);
    assert_matches(&t, ALL);
}

#[test]
fn encrypted_needs_the_right_passphrase() {
    let e = parse_tui(ENCRYPTED, &opts(None, None)).unwrap_err();
    assert!(e.to_string().contains("passphrase is required"), "{e}");
    let e = parse_tui(ENCRYPTED, &opts(Some("wrong"), None)).unwrap_err();
    assert!(e.to_string().contains("incorrect passphrase"), "{e}");
}

#[test]
fn legacy_markdown() {
    let t = parse_tui(LEGACY_MD, &opts(None, Some("halptask.md"))).unwrap();
    assert_eq!(t.format, SourceFormat::Markdown);
    // Markdown never carried notes, timestamps, versions or the raw due text.
    assert_matches(
        &t,
        &[
            "path", "id", "text", "is_task", "status", "tags", "due", "folded", "focused",
        ],
    );
    let parent = t.items.iter().find(|i| i.title == "Parent task").unwrap();
    // The TUI wrote the derived status; the effective status still matches.
    assert_eq!(parent.status, Status::InProgress);
}

#[test]
fn encrypted_markdown() {
    let file = encrypt_tui_file(LEGACY_MD, PASS, [3; 16], [9; 12]).unwrap();
    let t = parse_tui(file.as_bytes(), &opts(Some(PASS), None)).unwrap();
    assert_eq!(t.format, SourceFormat::EncryptedMarkdown);
    assert_eq!(t.items.len(), 14);
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

fn field_bytes(out: &mut Vec<u8>, field: u64, b: &[u8]) {
    varint(out, field << 3 | 2);
    varint(out, b.len() as u64);
    out.extend_from_slice(b);
}

fn field_varint(out: &mut Vec<u8>, field: u64, v: u64) {
    varint(out, field << 3);
    varint(out, v);
}

#[test]
fn tombstones_and_hostile_text() {
    let mut child = Vec::new();
    field_bytes(&mut child, 1, b"c");
    field_bytes(&mut child, 2, b"child of deleted");
    let mut deleted = Vec::new();
    field_bytes(&mut deleted, 1, b"d");
    field_bytes(&mut deleted, 2, b"gone");
    field_varint(&mut deleted, 11, 1);
    field_bytes(&mut deleted, 7, &child);
    let mut kept = Vec::new();
    field_bytes(&mut kept, 1, b"k");
    field_bytes(
        &mut kept,
        2,
        "]11;rgb:2828/2c2c/3434\\Line\none\u{7}".as_bytes(),
    );
    field_varint(&mut kept, 3, 1);
    field_varint(&mut kept, 4, 1); // STATUS_NONE on a task
    field_bytes(&mut kept, 6, b"Good");
    field_bytes(&mut kept, 6, b"bad tag!");
    field_bytes(&mut kept, 6, b"123");
    field_bytes(&mut kept, 13, "note\u{0}with nul".as_bytes());
    field_varint(&mut kept, 99, 5); // unknown field
    let mut empty = Vec::new();
    field_bytes(&mut empty, 1, b"e");
    let mut tree = Vec::new();
    field_varint(&mut tree, 1, 1);
    field_bytes(&mut tree, 2, &deleted);
    field_bytes(&mut tree, 2, &kept);
    field_bytes(&mut tree, 2, &empty);

    let t = parse_tui(&tree, &opts(None, None)).unwrap();
    assert_eq!(t.skipped_deleted, 2);
    assert_eq!(t.items.len(), 2);
    // Deleted nodes are kept losslessly as tombstones.
    assert_eq!(t.tombstones.len(), 2);
    assert_eq!(t.tombstones[0].legacy_id, "d");
    assert_eq!(t.tombstones[0].text, "gone");
    assert_eq!(t.tombstones[1].parent_legacy_id.as_deref(), Some("d"));
    let k = &t.items[0];
    assert_eq!(k.order, 0, "roots renumbered after skipping");
    assert_eq!(k.title, "Line one");
    assert_eq!(k.status, Status::Todo);
    assert_eq!(k.tags, ["good", "bad-tag"]);
    assert_eq!(k.notes, "notewith nul");
    assert_eq!(t.items[1].title, "(untitled)");
    assert!(t.warnings.iter().any(|w| w.contains("deleted")));
    assert!(t.warnings.iter().any(|w| w.contains("\"123\"")));
}

/// Truncated, bit-flipped and random inputs never panic.
#[test]
fn garbage_never_panics() {
    let o = opts(Some(PASS), None);
    for input in [PLAIN, LEGACY_MD] {
        for len in 0..input.len() {
            let _ = parse_tui(&input[..len], &o);
        }
        let mut x = input.to_vec();
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let i = (seed as usize) % x.len();
            x[i] ^= (seed >> 32) as u8;
            let _ = parse_tui(&x, &o);
        }
    }
    // Encrypted files: truncation must fail cleanly (GCM authenticates).
    // (A prefix shorter than the header is just a Markdown comment line.)
    for len in [24, 40, ENCRYPTED.len() / 2, ENCRYPTED.len() - 3] {
        assert!(parse_tui(&ENCRYPTED[..len], &o).is_err(), "len {len}");
    }
    let mut seed = 42u64;
    for n in 0..500 {
        let mut buf = vec![0u8; n];
        for b in &mut buf {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *b = (seed >> 56) as u8;
        }
        let _ = parse_tui(&buf, &o);
        let _ = parse_tui(&buf, &opts(None, Some("x.pb")));
    }
    // pathological markdown nesting fails cleanly
    let mut deep = String::new();
    for d in 0..100 {
        deep.push_str(&"  ".repeat(d));
        deep.push_str("- x\n");
    }
    // Deeper than the limit is flattened with a warning, not refused.
    let t = parse_tui(deep.as_bytes(), &o).unwrap();
    assert_eq!(t.items.len(), 100);
    assert!(t.warnings.iter().any(|w| w.contains("deeper than")));
    let ids: std::collections::HashMap<_, _> = t
        .items
        .iter()
        .map(|i| (i.id.clone(), i.parent_id.clone()))
        .collect();
    for it in &t.items {
        let mut d = 1;
        let mut cur = it.parent_id.clone();
        while let Some(p) = cur {
            d += 1;
            cur = ids[&p].clone();
        }
        assert!(d <= 64);
    }
}

/// Tags per node are bounded at decode and warnings are capped.
#[test]
fn tags_and_warnings_are_bounded() {
    let mut tree = Vec::new();
    field_varint(&mut tree, 1, 1);
    for i in 0..300 {
        let mut node = Vec::new();
        field_bytes(&mut node, 1, format!("n{i}").as_bytes());
        field_bytes(&mut node, 2, b"t");
        for j in 0..5_000 {
            field_bytes(&mut node, 6, format!("tag{j}").as_bytes());
        }
        // An invalid tag per node produces one warning each.
        field_bytes(&mut node, 6, b"123");
        field_bytes(&mut tree, 2, &node);
    }
    let t = parse_tui(&tree, &opts(None, None)).unwrap();
    assert_eq!(t.items.len(), 300);
    assert!(t.items.iter().all(|i| i.tags.len() <= 32));
    assert_eq!(
        t.warnings.len(),
        halp_store::import::MAX_WARNINGS + 1,
        "capped plus a summary line"
    );
    assert!(t.warnings.last().unwrap().starts_with("and "));
}

/// Without an extension hint, protobuf is tried first and Markdown is the
/// fallback (like the TUI), so a Markdown file still imports.
#[test]
fn detection_tries_protobuf_then_markdown() {
    let t = parse_tui(PLAIN, &opts(None, None)).unwrap();
    assert!(matches!(
        t.format,
        halp_store::import::SourceFormat::Protobuf
    ));
    let t = parse_tui(LEGACY_MD, &opts(None, None)).unwrap();
    assert!(matches!(
        t.format,
        halp_store::import::SourceFormat::Markdown
    ));
    // A `.pb` name means protobuf only.
    assert!(parse_tui(b"- just text\n", &opts(None, Some("x.pb"))).is_err());
}
