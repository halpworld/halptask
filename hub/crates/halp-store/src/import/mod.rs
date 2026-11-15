//! Importer for the HalpTask TUI's storage file.
//!
//! Reads all three formats the TUI has written: protobuf (`TreeProto`),
//! legacy Markdown, and either of those wrapped in `HALPTASK-ENCRYPTED-v1`.
//! The input is untrusted: size, depth and item count are bounded, text is
//! sanitised, and no input makes the parser panic.
//!
//! Items get fresh ULIDs. Everything the Hub item model has no field for
//! (the TUI id, `node_id`, fold and focus state, the raw due text) is kept in
//! a [`LegacyRecord`] per item, so nothing is lost.

mod encrypted;
mod markdown;
mod proto;
mod sanitize;

pub use encrypted::{HEADER as ENCRYPTED_HEADER, encrypt as encrypt_tui_file, is_encrypted};
pub use sanitize::terminal_artifacts as sanitize_terminal_artifacts;

use chrono::{DateTime, FixedOffset, NaiveDate, Utc};
use halp_domain::{Item, ItemKind, Status, limits};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::{Result, StoreError};

/// Largest file accepted.
pub const MAX_IMPORT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Default, Clone)]
pub(crate) struct LegacyNode {
    pub legacy_id: String,
    pub text: String,
    pub is_task: bool,
    /// Proto enum: 0 unspecified, 1 none, 2 todo, 3 in progress, 4 done.
    pub status: u64,
    pub folded: bool,
    pub tags: Vec<String>,
    pub children: Vec<LegacyNode>,
    pub created_at_ns: i64,
    pub updated_at_ns: i64,
    pub node_id: String,
    pub deleted: bool,
    pub version: u64,
    pub note: String,
    pub focused: bool,
    pub due_unix: i64,
    pub due_text: String,
    /// Due date already resolved (Markdown input).
    pub due: Option<NaiveDate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Protobuf,
    Markdown,
    EncryptedProtobuf,
    EncryptedMarkdown,
}

/// Provenance of one imported item (stored as a `legacy` entity).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyRecord {
    pub item_id: String,
    pub source: String,
    pub legacy_id: String,
    pub node_id: String,
    pub folded: bool,
    pub focused: bool,
    pub due_text: String,
}

/// A TUI item that was marked deleted. The TUI keeps such tombstones in its
/// file; the hub keeps them too, losslessly, as `legacy` entities (id
/// `tomb:<ulid>`) outside the outline, so they are never shown but nothing
/// from the source file is lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyTombstone {
    pub id: String,
    pub source: String,
    pub legacy_id: String,
    pub node_id: String,
    /// New id of the imported parent item, when the parent was kept.
    pub parent_item_id: Option<String>,
    /// Legacy id of the parent node, when the parent is itself a tombstone.
    pub parent_legacy_id: Option<String>,
    pub text: String,
    pub note: String,
    pub is_task: bool,
    pub status: u64,
    pub tags: Vec<String>,
    pub folded: bool,
    pub created_at_ns: i64,
    pub updated_at_ns: i64,
    pub version: u64,
    pub due_unix: i64,
    pub due_text: String,
}

pub struct ImportOptions {
    /// Needed for encrypted files.
    pub passphrase: Option<Zeroizing<String>>,
    /// Time zone used to turn the TUI's due timestamps into dates (the TUI
    /// stored local midnight as unix seconds).
    pub utc_offset: FixedOffset,
    /// "Today", for relative due tokens in Markdown files.
    pub today: NaiveDate,
    /// Timestamp for items that carry none.
    pub now: DateTime<Utc>,
    /// File name, used like the TUI does to recognise `.pb` / `.halp` files.
    pub file_name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ImportedTree {
    pub format: SourceFormat,
    pub schema_version: u32,
    /// Items in pre-order; roots have `parent_id: None` and `order` 0..n.
    pub items: Vec<Item>,
    pub legacy: Vec<LegacyRecord>,
    /// Deleted TUI items (and their subtrees), kept as tombstones.
    pub tombstones: Vec<LegacyTombstone>,
    /// The new id of the item the TUI had focused, if any.
    pub focused: Option<String>,
    pub warnings: Vec<String>,
    /// Deleted (tombstoned) TUI items kept out of the outline, subtrees
    /// included (see [`ImportedTree::tombstones`]).
    pub skipped_deleted: usize,
}

fn looks_like_proto(bytes: &[u8], file_name: Option<&str>) -> bool {
    let ext_hint = file_name.is_some_and(|n| {
        let n = n.to_ascii_lowercase();
        n.ends_with(".pb") || n.ends_with(".halp")
    });
    ext_hint || bytes.starts_with(&[0x08, 0x01])
}

/// Warnings kept per import; the rest are counted ("and N more").
pub const MAX_WARNINGS: usize = 100;

/// Parses a TUI storage file.
pub fn parse_tui(bytes: &[u8], opts: &ImportOptions) -> Result<ImportedTree> {
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(StoreError::Import(format!(
            "file is larger than {} MiB",
            MAX_IMPORT_BYTES / (1024 * 1024)
        )));
    }
    let mut warnings = Vec::new();
    let (plain, encrypted): (Zeroizing<Vec<u8>>, bool) = if encrypted::is_encrypted(bytes) {
        let pass = opts.passphrase.as_ref().ok_or_else(|| {
            StoreError::Import("the file is encrypted; a passphrase is required".into())
        })?;
        (encrypted::decrypt(bytes, pass)?, true)
    } else {
        (Zeroizing::new(bytes.to_vec()), false)
    };

    let limits = proto::Limits {
        max_depth: limits::MAX_TREE_DEPTH,
        max_items: limits::MAX_ITEMS,
    };
    let file_name = opts.file_name.as_deref();
    // Like the TUI: protobuf first, Markdown as the fallback (a `.pb` /
    // `.halp` name means protobuf only).
    let (roots, schema_version, is_proto) = {
        let mut proto_warnings = Vec::new();
        match proto::decode_tree(&plain, &limits, &mut proto_warnings) {
            Ok(d) => {
                warnings.extend(proto_warnings);
                (d.roots, d.schema_version, true)
            }
            Err(e) if looks_like_proto(&plain, file_name) => return Err(e),
            Err(e) => match std::str::from_utf8(&plain) {
                Ok(text) => {
                    let roots = markdown::parse(
                        text,
                        opts.today,
                        limits.max_depth,
                        limits.max_items,
                        &mut warnings,
                    )?;
                    (roots, 0, false)
                }
                Err(_) => return Err(e),
            },
        }
    };
    let format = match (encrypted, is_proto) {
        (false, true) => SourceFormat::Protobuf,
        (false, false) => SourceFormat::Markdown,
        (true, true) => SourceFormat::EncryptedProtobuf,
        (true, false) => SourceFormat::EncryptedMarkdown,
    };

    let mut out = ImportedTree {
        format,
        schema_version,
        items: Vec::new(),
        legacy: Vec::new(),
        tombstones: Vec::new(),
        focused: None,
        warnings,
        skipped_deleted: 0,
    };
    let mut order = 0u32;
    for root in &roots {
        if convert(root, None, order, opts, &mut out) {
            order += 1;
        }
    }
    if out.skipped_deleted > 0 {
        out.warnings.push(format!(
            "kept {} deleted item(s) as hidden tombstones",
            out.skipped_deleted
        ));
    }
    if out.warnings.len() > MAX_WARNINGS {
        let more = out.warnings.len() - MAX_WARNINGS;
        out.warnings.truncate(MAX_WARNINGS);
        out.warnings.push(format!("and {more} more"));
    }
    Ok(out)
}

/// Keeps a deleted node and its whole subtree as tombstones.
fn tombstone(
    n: &LegacyNode,
    parent_item: Option<&str>,
    parent_legacy: Option<&str>,
    out: &mut ImportedTree,
) {
    out.skipped_deleted += 1;
    let ms =
        u64::try_from(ns_to_utc(n.created_at_ns).map_or(0, |d| d.timestamp_millis())).unwrap_or(0);
    out.tombstones.push(LegacyTombstone {
        id: format!("tomb:{}", ulid::Ulid::from_parts(ms, rand_u128())),
        source: "tui".into(),
        legacy_id: n.legacy_id.clone(),
        node_id: n.node_id.clone(),
        parent_item_id: parent_item.map(str::to_string),
        parent_legacy_id: parent_legacy.map(str::to_string),
        text: n.text.clone(),
        note: n.note.clone(),
        is_task: n.is_task,
        status: n.status,
        tags: n.tags.clone(),
        folded: n.folded,
        created_at_ns: n.created_at_ns,
        updated_at_ns: n.updated_at_ns,
        version: n.version,
        due_unix: n.due_unix,
        due_text: n.due_text.clone(),
    });
    for c in &n.children {
        tombstone(c, None, Some(&n.legacy_id), out);
    }
}

fn ns_to_utc(ns: i64) -> Option<DateTime<Utc>> {
    if ns <= 0 {
        return None;
    }
    let dt = DateTime::from_timestamp_nanos(ns);
    // Reject nonsense far in the future (bad data), keep anything after 1971.
    (dt.timestamp() > 31_536_000 && dt.timestamp() < 32_503_680_000).then_some(dt)
}

/// Converts one node and its subtree; returns false when it was skipped.
fn convert(
    n: &LegacyNode,
    parent: Option<&str>,
    order: u32,
    opts: &ImportOptions,
    out: &mut ImportedTree,
) -> bool {
    if n.deleted {
        tombstone(n, parent, None, out);
        return false;
    }
    let created = ns_to_utc(n.created_at_ns).unwrap_or(opts.now);
    let updated = ns_to_utc(n.updated_at_ns).unwrap_or(created);
    let label = if n.legacy_id.is_empty() {
        "an item".to_string()
    } else {
        format!("item {}", n.legacy_id)
    };

    let cleaned = sanitize::terminal_artifacts(&n.text);
    let mut title = limits::sanitize_title(&cleaned);
    if title != n.text.trim() {
        out.warnings.push(format!("{label}: title was cleaned up"));
    }
    if title.is_empty() {
        title = "(untitled)".into();
        out.warnings
            .push(format!("{label}: empty title replaced with \"(untitled)\""));
    }
    let cleaned_note = sanitize::terminal_artifacts(&n.note);
    let notes = limits::sanitize_notes(&cleaned_note);
    if notes != n.note {
        out.warnings.push(format!("{label}: notes were cleaned up"));
    }
    let mut tags: Vec<String> = Vec::new();
    for t in &n.tags {
        match limits::sanitize_tag(t) {
            Some(t) if !tags.contains(&t) => {
                if tags.len() < limits::MAX_TAGS_PER_ITEM {
                    tags.push(t);
                } else {
                    out.warnings.push(format!(
                        "{label}: more than {} tags, extra dropped",
                        limits::MAX_TAGS_PER_ITEM
                    ));
                }
            }
            Some(_) => {}
            None => out
                .warnings
                .push(format!("{label}: dropped unusable tag {t:?}")),
        }
    }

    let kind = if n.is_task {
        ItemKind::Task
    } else {
        ItemKind::Bullet
    };
    let status = match (n.is_task, n.status) {
        (true, 3) => Status::InProgress,
        (true, 4) => Status::Done,
        _ => Status::Todo,
    };
    let due = n.due.or_else(|| {
        (n.due_unix > 0)
            .then(|| DateTime::from_timestamp(n.due_unix, 0))
            .flatten()
            .map(|d| d.with_timezone(&opts.utc_offset).date_naive())
    });

    // Fresh, time-ordered ids derived from the original creation time.
    let ms = u64::try_from(created.timestamp_millis()).unwrap_or(0);
    let id = ulid::Ulid::from_parts(ms, rand_u128()).to_string();

    let item = Item {
        id: id.clone(),
        parent_id: parent.map(str::to_string),
        order,
        kind,
        status,
        title,
        notes,
        tags,
        due,
        type_id: None,
        fields: serde_json::Map::new(),
        roles: Vec::new(),
        origin: halp_domain::item::user_origin(),
        created_at: created,
        updated_at: updated.max(created),
        version: n.version.max(1),
    };
    out.items.push(item);
    out.legacy.push(LegacyRecord {
        item_id: id.clone(),
        source: "tui".into(),
        legacy_id: n.legacy_id.clone(),
        node_id: n.node_id.clone(),
        folded: n.folded,
        focused: n.focused,
        due_text: n.due_text.clone(),
    });
    if n.focused && out.focused.is_none() {
        out.focused = Some(id.clone());
    }
    let mut child_order = 0u32;
    for c in &n.children {
        if convert(c, Some(&id), child_order, opts, out) {
            child_order += 1;
        }
    }
    true
}

fn rand_u128() -> u128 {
    let mut b = [0u8; 16];
    // Uniqueness, not secrecy: fall back to a counter if the OS RNG fails.
    if getrandom::fill(&mut b).is_err() {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        b[..8].copy_from_slice(&n.to_le_bytes());
    }
    u128::from_le_bytes(b) & ((1u128 << 80) - 1)
}
