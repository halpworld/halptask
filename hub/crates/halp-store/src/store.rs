//! The encrypted database: entity tables, the append-only event log and the
//! undo/redo stacks.
//!
//! Every write goes through [`Store::commit`], which applies a set of entity
//! [`Change`]s and appends the describing event in one SQLite transaction.
//! Undo and redo replay a recorded event's inverse (or forward) changes and
//! are themselves appended as `history.undo` / `history.redo` events.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use halp_domain::{EntityRef, Event, new_id};
use rusqlite::{Connection, ErrorCode, OptionalExtension, Transaction, params};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Result, StoreError};
use crate::fsutil;
use crate::keys::KeyProvider;

/// Kinds of stored entities. Each maps to one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Item,
    Layout,
    TaskType,
    Setting,
    /// Provenance of items imported from the TUI (legacy id, fold state, ...).
    Legacy,
    /// Paired MCP clients (token hash, grants). Schema v2.
    Client,
    /// Standing consent rules. Schema v2.
    Rule,
    /// Integration configurations (no secrets: tokens live in the OS
    /// keychain). Schema v3.
    Integration,
    /// External object id -> item id links of integration-synced items.
    /// Schema v3.
    ExternalLink,
    /// Installed extension packs (manifest, file index, trust, pin,
    /// enabled, previous version). Their file bytes live in the
    /// content-addressed blob table, outside events. Schema v4.
    Pack,
    /// Community signing keys the user trusted. Schema v4.
    PackKey,
    /// Snapshots of task types whose pack left (uninstalled, disabled, or
    /// the type dropped in an update). Schema v4.
    RetiredType,
}

impl EntityKind {
    fn table(self) -> &'static str {
        match self {
            EntityKind::Item => "items",
            EntityKind::Layout => "layouts",
            EntityKind::TaskType => "task_types",
            EntityKind::Setting => "settings",
            EntityKind::Legacy => "legacy_import",
            EntityKind::Client => "clients",
            EntityKind::Rule => "policy_rules",
            EntityKind::Integration => "integrations",
            EntityKind::ExternalLink => "external_links",
            EntityKind::Pack => "packs",
            EntityKind::PackKey => "pack_keys",
            EntityKind::RetiredType => "retired_types",
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::Item => "item",
            EntityKind::Layout => "layout",
            EntityKind::TaskType => "task_type",
            EntityKind::Setting => "setting",
            EntityKind::Legacy => "legacy",
            EntityKind::Client => "client",
            EntityKind::Rule => "rule",
            EntityKind::Integration => "integration",
            EntityKind::ExternalLink => "external_link",
            EntityKind::Pack => "pack",
            EntityKind::PackKey => "pack_key",
            EntityKind::RetiredType => "retired_type",
        }
    }
}

/// One entity's state before and after a command (`None` = absent).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub kind: EntityKind,
    pub id: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

/// A command's outcome, to be recorded.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub actor: String,
    pub command: String,
    pub entity: EntityRef,
    pub summary: String,
    /// The command's arguments (kept in the event payload for audit).
    pub args: Value,
    pub changes: Vec<Change>,
    /// Whether `history.undo` may revert it (seeding, for example, is not).
    pub undoable: bool,
}

/// An event as stored, with log-only metadata.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EventRecord {
    #[serde(flatten)]
    pub event: Event,
    pub seq: i64,
    pub causation_id: Option<String>,
    pub undoable: bool,
}

/// Result of an undo or redo.
#[derive(Debug, Clone)]
pub struct HistoryOutcome {
    /// The `history.undo` / `history.redo` event just recorded.
    pub event: Event,
    /// The event that was undone or redone.
    pub original: Event,
    /// The changes applied by this step.
    pub changes: Vec<Change>,
}

pub const SCHEMA_VERSION: i64 = 4;
/// Undo entries kept per actor; older ones fall off that actor's stack (the
/// events stay). Stacks are per actor, so another actor's writes can never
/// push the user's history out.
pub const MAX_UNDO_DEPTH: i64 = 500;
/// Payload and inverse of a purged event.
pub const REDACTED: &str = r#"{"redacted":true}"#;
/// Summary of a purged event.
pub const REDACTED_SUMMARY: &str = "[redacted]";

/// Forward-only migrations; entry `n` upgrades `user_version` n to n+1.
const MIGRATIONS: &[&str] = &[
    r#"
CREATE TABLE items (
    id TEXT PRIMARY KEY,
    parent_id TEXT,
    ord INTEGER NOT NULL,
    data TEXT NOT NULL
);
CREATE INDEX items_parent ON items(parent_id, ord);
CREATE TABLE task_types (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE layouts (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE settings (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE legacy_import (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE events (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    ts TEXT NOT NULL,
    actor TEXT NOT NULL,
    command TEXT NOT NULL,
    entity_kind TEXT NOT NULL,
    entity_id TEXT NOT NULL,
    summary TEXT NOT NULL,
    payload TEXT NOT NULL,
    inverse TEXT NOT NULL,
    causation_id TEXT,
    undoable INTEGER NOT NULL
);
CREATE INDEX events_entity ON events(entity_kind, entity_id);
CREATE TRIGGER events_no_update BEFORE UPDATE ON events
BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TRIGGER events_no_delete BEFORE DELETE ON events
BEGIN SELECT RAISE(ABORT, 'events are append-only'); END;
CREATE TABLE history_stack (
    stack TEXT NOT NULL CHECK (stack IN ('undo', 'redo')),
    pos INTEGER NOT NULL,
    event_id TEXT NOT NULL REFERENCES events(id),
    PRIMARY KEY (stack, pos)
);
"#,
    r#"
ALTER TABLE history_stack ADD COLUMN actor TEXT NOT NULL DEFAULT 'user';
UPDATE history_stack SET actor = coalesce(
    (SELECT actor FROM events WHERE events.id = history_stack.event_id), 'user');
CREATE INDEX history_stack_actor ON history_stack(stack, actor, pos);
CREATE TABLE clients (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE policy_rules (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE audit_log (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    ts TEXT NOT NULL,
    actor TEXT NOT NULL,
    command TEXT NOT NULL,
    tier TEXT NOT NULL,
    args_hash TEXT NOT NULL,
    decision TEXT NOT NULL,
    reason TEXT NOT NULL,
    approver TEXT,
    confirmation_id TEXT,
    tainted INTEGER NOT NULL
);
CREATE TRIGGER audit_no_update BEFORE UPDATE ON audit_log
BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
CREATE TRIGGER audit_no_delete BEFORE DELETE ON audit_log
BEGIN SELECT RAISE(ABORT, 'the audit log is append-only'); END;
DROP TRIGGER events_no_update;
CREATE TRIGGER events_redact_only BEFORE UPDATE ON events
WHEN NEW.seq IS NOT OLD.seq OR NEW.id IS NOT OLD.id OR NEW.ts IS NOT OLD.ts
  OR NEW.actor IS NOT OLD.actor OR NEW.command IS NOT OLD.command
  OR NEW.entity_kind IS NOT OLD.entity_kind OR NEW.entity_id IS NOT OLD.entity_id
  OR NEW.causation_id IS NOT OLD.causation_id
  OR NEW.payload IS NOT '{"redacted":true}' OR NEW.inverse IS NOT '{"redacted":true}'
  OR NEW.summary IS NOT '[redacted]' OR NEW.undoable IS NOT 0
BEGIN SELECT RAISE(ABORT, 'events are append-only (only redaction is allowed)'); END;
"#,
    r#"
CREATE TABLE integrations (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE external_links (id TEXT PRIMARY KEY, data TEXT NOT NULL);
"#,
    r#"
CREATE TABLE packs (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE pack_keys (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE retired_types (id TEXT PRIMARY KEY, data TEXT NOT NULL);
CREATE TABLE pack_blobs (sha256 TEXT PRIMARY KEY, bytes BLOB NOT NULL);
"#,
];

/// One policy decision as recorded in the append-only `audit_log`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuditRecord {
    pub id: String,
    pub ts: String,
    pub actor: String,
    pub command: String,
    pub tier: String,
    /// SHA-256 of the canonical JSON of command + args (args themselves are
    /// not stored here).
    pub args_hash: String,
    /// `allow`, `deny`, `confirm`, `approved`, `denied`, `expired`.
    pub decision: String,
    pub reason: String,
    /// Who decided a confirmation (`user`), when there was one.
    pub approver: Option<String>,
    pub confirmation_id: Option<String>,
    pub tainted: bool,
}

/// An undo or redo, worked out but not applied.
#[derive(Debug, Clone)]
pub struct StepPlan {
    /// The event that would be undone or redone.
    pub original: Event,
    /// The entity changes the step would apply (`before` = current state).
    pub changes: Vec<Change>,
    /// Actor whose history stack holds the entry.
    pub owner: String,
    pos: i64,
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
    /// The exclusive lock on the database's lock file, held as long as the
    /// store is open (released when the file closes).
    _lock: std::fs::File,
}

/// What a second process opening a database that is in use is told.
pub const ALREADY_RUNNING: &str =
    "HalpTask Hub is already running; use the agent socket (halp-mcp) instead";

/// The lock file of the database at `path`: `hub.db` -> `hub.lock`.
pub fn lock_path(path: &Path) -> PathBuf {
    path.with_extension("lock")
}

/// The real location of the database at `path`: symlinks and `..` resolved,
/// so every spelling of one database shares one lock file. A database that
/// does not exist yet resolves through its (existing) parent directory.
fn real_path(path: &Path) -> Result<PathBuf> {
    if let Ok(p) = std::fs::canonicalize(path) {
        return Ok(p);
    }
    let name = path.file_name().ok_or_else(|| {
        StoreError::Io(std::io::Error::other("the database path has no file name"))
    })?;
    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    Ok(std::fs::canonicalize(parent)?.join(name))
}

/// Takes the exclusive advisory lock on `path`'s lock file (next to the
/// real database file, see [`real_path`]). Two stores on one database
/// would each cache state and overwrite the other's changes, so the second
/// opener gets [`StoreError::Conflict`] ([`ALREADY_RUNNING`]), however the
/// path is spelled.
fn lock_db(path: &Path) -> Result<std::fs::File> {
    let lp = lock_path(&real_path(path)?);
    let mut o = std::fs::OpenOptions::new();
    o.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    let f = o.open(&lp)?;
    match f.try_lock() {
        Ok(()) => Ok(f),
        Err(std::fs::TryLockError::WouldBlock) => {
            Err(StoreError::Conflict(ALREADY_RUNNING.to_string()))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(StoreError::Io(e)),
    }
}

impl std::fmt::Debug for Store {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Store").field("path", &self.path).finish()
    }
}

fn is_not_a_db(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::NotADatabase)
}

fn ts(now: DateTime<Utc>) -> String {
    now.to_rfc3339_opts(SecondsFormat::Millis, true)
}

impl Store {
    /// Opens (or creates) the encrypted database at `path`.
    ///
    /// The parent directory is created owner-only (0700 on unix) and the
    /// file is created 0600 *before* SQLite opens it, so the WAL and shared
    /// memory files SQLite derives from it inherit the same mode. A wrong key
    /// fails with [`StoreError::WrongKey`] and leaves the file untouched.
    ///
    /// The store holds an exclusive lock on [`lock_path`] while it is open;
    /// opening a database another store (in this or another process) holds
    /// fails with `Conflict` ([`ALREADY_RUNNING`]) before the key is read.
    pub fn open(path: &Path, keys: &dyn KeyProvider) -> Result<Store> {
        if let Some(parent) = path.parent() {
            fsutil::ensure_private_dir(parent)?;
        }
        let lock = lock_db(path)?;
        let is_new = fsutil::ensure_private_file(path)?;
        let key = keys.db_key(path, is_new)?;
        let mut conn = Connection::open(path)?;
        conn.execute_batch(&key.pragma())?;
        drop(key);

        match conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| {
            r.get::<_, i64>(0)
        }) {
            Ok(_) => {}
            Err(e) if is_not_a_db(&e) => return Err(StoreError::WrongKey),
            Err(e) => return Err(e.into()),
        }
        let cipher: Option<String> = conn
            .query_row("PRAGMA cipher_version", [], |r| r.get(0))
            .optional()?;
        if cipher.as_deref().is_none_or(str::is_empty) {
            return Err(StoreError::Key(
                "SQLite was built without SQLCipher; refusing to store data unencrypted".into(),
            ));
        }
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |r| r.get(0))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA secure_delete = ON;")?;
        migrate(&mut conn)?;
        for suffix in ["-wal", "-shm"] {
            let mut p = path.as_os_str().to_owned();
            p.push(suffix);
            let p = PathBuf::from(p);
            if p.exists() {
                fsutil::restrict_file(&p)?;
            }
        }
        Ok(Store {
            conn,
            path: path.to_path_buf(),
            _lock: lock,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    /// All entities of `kind`, deserialized.
    pub fn load<T: DeserializeOwned>(&self, kind: EntityKind) -> Result<Vec<T>> {
        let sql = format!("SELECT data FROM {} ORDER BY id", kind.table());
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(serde_json::from_str(&row?)?);
        }
        Ok(out)
    }

    pub fn get(&self, kind: EntityKind, id: &str) -> Result<Option<Value>> {
        read_entity(&self.conn, kind, id)
    }

    /// Stores `bytes` in the content-addressed pack blob table and returns
    /// their SHA-256 (lowercase hex). Idempotent. Blobs are kept outside
    /// events: write them before the event that references them and drop
    /// unreferenced ones with [`Store::retain_blobs`].
    pub fn put_blob(&mut self, bytes: &[u8]) -> Result<String> {
        use sha2::Digest as _;
        let sha = hex::encode(sha2::Sha256::digest(bytes));
        self.conn.execute(
            "INSERT OR IGNORE INTO pack_blobs (sha256, bytes) VALUES (?1, ?2)",
            params![sha, bytes],
        )?;
        Ok(sha)
    }

    /// The bytes stored under `sha256`, as stored. Callers re-hash them:
    /// the table is not trusted to still match its keys.
    pub fn get_blob(&self, sha256: &str) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row(
                "SELECT bytes FROM pack_blobs WHERE sha256 = ?1",
                [sha256],
                |r| r.get::<_, Vec<u8>>(0),
            )
            .optional()?)
    }

    /// Deletes every blob whose hash is not in `keep`; returns how many.
    pub fn retain_blobs(&mut self, keep: &BTreeSet<String>) -> Result<usize> {
        let all: Vec<String> = {
            let mut stmt = self.conn.prepare("SELECT sha256 FROM pack_blobs")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let tx = self.conn.transaction()?;
        let mut n = 0;
        for sha in all.iter().filter(|s| !keep.contains(*s)) {
            n += tx.execute("DELETE FROM pack_blobs WHERE sha256 = ?1", [sha])?;
        }
        tx.commit()?;
        Ok(n)
    }

    /// Overwrites the bytes under `sha256` without checking them. Only for
    /// tests that simulate a tampered database.
    #[doc(hidden)]
    pub fn overwrite_blob_unchecked(&mut self, sha256: &str, bytes: &[u8]) -> Result<()> {
        self.conn.execute(
            "UPDATE pack_blobs SET bytes = ?2 WHERE sha256 = ?1",
            params![sha256, bytes],
        )?;
        Ok(())
    }

    /// Applies `ev.changes` and appends the event, atomically.
    ///
    /// Compare-and-swap: every entity must still hold its change's `before`
    /// (for items: the same `version` and `updated_at`), else nothing is
    /// written and the result is `Conflict`. The lock file already keeps a
    /// second store off the database; this catches a writer that got past it.
    pub fn commit(&mut self, ev: NewEvent, now: DateTime<Utc>) -> Result<Event> {
        let changes = merge_changes(ev.changes);
        let tx = self.conn.transaction()?;
        for c in &changes {
            let stored = read_entity(&tx, c.kind, &c.id)?;
            if !cas_same(stored.as_ref(), c.before.as_ref()) {
                return Err(StoreError::Conflict(format!(
                    "{} {} was changed by someone else since it was read; reload and try again",
                    c.kind.as_str(),
                    c.id
                )));
            }
        }
        for c in &changes {
            write_entity(&tx, c.kind, &c.id, c.after.as_ref())?;
        }
        let event = Event {
            id: new_id(),
            ts: ts(now),
            actor: ev.actor,
            command: ev.command,
            entity: ev.entity,
            summary: ev.summary,
        };
        let payload = json!({
            "args": ev.args,
            "changes": changes.iter().map(|c| json!({"kind": c.kind, "id": c.id, "after": c.after})).collect::<Vec<_>>(),
        });
        let inverse = json!({
            "changes": changes.iter().map(|c| json!({"kind": c.kind, "id": c.id, "before": c.before})).collect::<Vec<_>>(),
        });
        let undoable = ev.undoable && !changes.is_empty();
        insert_event(&tx, &event, &payload, &inverse, None, undoable)?;
        if undoable {
            push(&tx, "undo", &event.id, &event.actor)?;
            // A new write ends the redo history of its own actor; the user's
            // writes end everyone's. Another actor never clears the user's.
            if event.actor == "user" {
                tx.execute("DELETE FROM history_stack WHERE stack = 'redo'", [])?;
            } else {
                tx.execute(
                    "DELETE FROM history_stack WHERE stack = 'redo' AND actor = ?1",
                    [&event.actor],
                )?;
            }
            // Keep the newest MAX_UNDO_DEPTH entries of this actor only.
            tx.execute(
                "DELETE FROM history_stack WHERE stack = 'undo' AND actor = ?1 AND pos NOT IN
                   (SELECT pos FROM history_stack WHERE stack = 'undo' AND actor = ?1 ORDER BY pos DESC LIMIT ?2)",
                params![&event.actor, MAX_UNDO_DEPTH],
            )?;
        }
        tx.commit()?;
        Ok(event)
    }

    /// Reverts the most recent undoable event. `NotFound` when there is none,
    /// `Conflict` when an affected entity changed since (nothing is applied).
    ///
    /// Actor `user` steps through the shared history (the newest entry of
    /// any actor); any other actor only through its own entries.
    pub fn undo(&mut self, actor: &str, now: DateTime<Utc>) -> Result<HistoryOutcome> {
        self.step(true, None, actor, now, &mut |_| Ok(()))
    }

    /// Selectively reverts one event that is still in the undo history (not
    /// necessarily the latest). `NotFound` when it is not undoable or already
    /// undone, `Conflict` when an affected entity changed since, `Denied`
    /// when a non-user actor names another actor's event.
    pub fn undo_event(
        &mut self,
        event_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<HistoryOutcome> {
        self.step(true, Some(event_id), actor, now, &mut |_| Ok(()))
    }

    /// Re-applies the most recently undone event.
    pub fn redo(&mut self, actor: &str, now: DateTime<Utc>) -> Result<HistoryOutcome> {
        self.step(false, None, actor, now, &mut |_| Ok(()))
    }

    /// Re-applies one specific undone event.
    pub fn redo_event(
        &mut self,
        event_id: &str,
        actor: &str,
        now: DateTime<Utc>,
    ) -> Result<HistoryOutcome> {
        self.step(false, Some(event_id), actor, now, &mut |_| Ok(()))
    }

    /// Works out an undo (`undo = true`) or redo without applying it: the
    /// event it would revert and the changes. `Ok(None)` when there is
    /// nothing to step (plain step only).
    pub fn plan_step(
        &self,
        undo: bool,
        target: Option<&str>,
        actor: &str,
    ) -> Result<Option<StepPlan>> {
        match plan(&self.conn, undo, target, actor) {
            Ok(p) => Ok(Some(p)),
            Err(StoreError::NotFound(_)) if target.is_none() => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Undo or redo with a check of the resulting state: `validate` sees the
    /// planned changes inside the transaction and can refuse them (for
    /// example because the outline would become invalid); nothing is applied
    /// then.
    pub fn step(
        &mut self,
        undo: bool,
        target: Option<&str>,
        actor: &str,
        now: DateTime<Utc>,
        validate: &mut dyn FnMut(&StepPlan) -> Result<()>,
    ) -> Result<HistoryOutcome> {
        let (from, to, command, verb) = if undo {
            ("undo", "redo", "history.undo", "Undo")
        } else {
            ("redo", "undo", "history.redo", "Redo")
        };
        let tx = self.conn.transaction()?;
        let mut planned = plan(&tx, undo, target, actor)?;
        for c in &mut planned.changes {
            c.after = c.after.take().map(|t| restamp(t, c.before.as_ref(), now));
        }
        validate(&planned)?;
        let StepPlan {
            original,
            changes: applied,
            owner,
            pos,
        } = planned;
        for c in &applied {
            write_entity(&tx, c.kind, &c.id, c.after.as_ref())?;
        }
        let event = Event {
            id: new_id(),
            ts: ts(now),
            actor: actor.to_string(),
            command: command.to_string(),
            entity: original.entity.clone(),
            summary: format!("{verb}: {}", original.summary),
        };
        let payload = json!({
            "args": {"event_id": original.id},
            "changes": applied.iter().map(|c| json!({"kind": c.kind, "id": c.id, "after": c.after})).collect::<Vec<_>>(),
        });
        let inverse = json!({
            "changes": applied.iter().map(|c| json!({"kind": c.kind, "id": c.id, "before": c.before})).collect::<Vec<_>>(),
        });
        insert_event(&tx, &event, &payload, &inverse, Some(&original.id), false)?;
        tx.execute(
            "DELETE FROM history_stack WHERE stack = ?1 AND pos = ?2",
            params![from, pos],
        )?;
        push(&tx, to, &original.id, &owner)?;
        tx.commit()?;
        Ok(HistoryOutcome {
            event,
            original,
            changes: applied,
        })
    }

    /// Appends one policy decision to the audit log.
    pub fn audit_append(&mut self, r: &AuditRecord) -> Result<()> {
        self.conn.execute(
            "INSERT INTO audit_log (id, ts, actor, command, tier, args_hash, decision, reason, approver, confirmation_id, tainted)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                r.id,
                r.ts,
                r.actor,
                r.command,
                r.tier,
                r.args_hash,
                r.decision,
                r.reason,
                r.approver,
                r.confirmation_id,
                i64::from(r.tainted)
            ],
        )?;
        Ok(())
    }

    /// Audit records newest first, paged like [`Store::list_events`].
    pub fn audit_list(
        &self,
        limit: usize,
        before: Option<&str>,
        actor_prefix: Option<&str>,
    ) -> Result<Vec<AuditRecord>> {
        let before_seq: i64 = match before {
            Some(id) => self
                .conn
                .query_row("SELECT seq FROM audit_log WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?
                .ok_or_else(|| StoreError::NotFound(format!("audit record {id}")))?,
            None => i64::MAX,
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, actor, command, tier, args_hash, decision, reason, approver, confirmation_id, tainted
             FROM audit_log WHERE seq < ?1 AND (?3 IS NULL OR substr(actor, 1, length(?3)) = ?3)
             ORDER BY seq DESC LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![before_seq, limit, actor_prefix], |r| {
            Ok(AuditRecord {
                id: r.get(0)?,
                ts: r.get(1)?,
                actor: r.get(2)?,
                command: r.get(3)?,
                tier: r.get(4)?,
                args_hash: r.get(5)?,
                decision: r.get(6)?,
                reason: r.get(7)?,
                approver: r.get(8)?,
                confirmation_id: r.get(9)?,
                tainted: r.get::<_, i64>(10)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Events after `after_id` (exclusive), oldest first: the change feed
    /// behind `halp://events?since=<id>`.
    pub fn events_since(&self, after_id: Option<&str>, limit: usize) -> Result<Vec<EventRecord>> {
        let after_seq: i64 = match after_id {
            Some(id) => self
                .conn
                .query_row("SELECT seq FROM events WHERE id = ?1", [id], |r| r.get(0))
                .optional()?
                .ok_or_else(|| StoreError::NotFound(format!("event {id}")))?,
            None => 0,
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, actor, command, entity_kind, entity_id, summary, seq, causation_id, undoable
             FROM events WHERE seq > ?1 ORDER BY seq ASC LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![after_seq, limit], event_record)?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Redacts the payload, inverse and summary of events recorded before
    /// `older_than` (RFC 3339) and/or, with `deleted_items`, of every event
    /// about an item that no longer exists, of every other event whose
    /// payload names such an item (consent requests, focus changes), and
    /// of the whole consent trail (request and decision) of those. Redacted
    /// events leave the undo
    /// and redo stacks. The rows stay (the log is append-only), so sequence,
    /// actor, command and timestamps remain auditable. Returns how many
    /// events were redacted.
    pub fn purge_events(&mut self, older_than: Option<&str>, deleted_items: bool) -> Result<usize> {
        if older_than.is_none() && !deleted_items {
            return Ok(0);
        }
        let tx = self.conn.transaction()?;
        let ids = purge_targets(&tx, older_than, deleted_items)?;
        for id in &ids {
            tx.execute("DELETE FROM history_stack WHERE event_id = ?1", [id])?;
            tx.execute(
                "UPDATE events SET payload = ?2, inverse = ?2, summary = ?3, undoable = 0 WHERE id = ?1",
                params![id, REDACTED, REDACTED_SUMMARY],
            )?;
        }
        tx.commit()?;
        Ok(ids.len())
    }

    /// The ids of the events [`Store::purge_events`] would redact now,
    /// without changing anything (what a consent prompt is about).
    pub fn purge_candidates(
        &self,
        older_than: Option<&str>,
        deleted_items: bool,
    ) -> Result<BTreeSet<String>> {
        if older_than.is_none() && !deleted_items {
            return Ok(BTreeSet::new());
        }
        purge_targets(&self.conn, older_than, deleted_items)
    }

    /// `(undo depth, redo depth)`.
    pub fn history_depth(&self) -> Result<(u64, u64)> {
        let count = |stack: &str| -> Result<u64> {
            Ok(self.conn.query_row(
                "SELECT count(*) FROM history_stack WHERE stack = ?1",
                [stack],
                |r| r.get::<_, i64>(0),
            )? as u64)
        };
        Ok((count("undo")?, count("redo")?))
    }

    /// Events newest first. `before` is an event id to page from (exclusive);
    /// `actor_prefix` keeps only events whose actor starts with it (`agent:`).
    pub fn list_events(
        &self,
        limit: usize,
        before: Option<&str>,
        actor_prefix: Option<&str>,
    ) -> Result<Vec<EventRecord>> {
        let before_seq: i64 = match before {
            Some(id) => self
                .conn
                .query_row("SELECT seq FROM events WHERE id = ?1", [id], |r| r.get(0))
                .optional()?
                .ok_or_else(|| StoreError::NotFound(format!("event {id}")))?,
            None => i64::MAX,
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, ts, actor, command, entity_kind, entity_id, summary, seq, causation_id, undoable
             FROM events WHERE seq < ?1 AND (?3 IS NULL OR substr(actor, 1, length(?3)) = ?3)
             ORDER BY seq DESC LIMIT ?2",
        )?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(params![before_seq, limit, actor_prefix], |r| {
            Ok(EventRecord {
                event: Event {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    actor: r.get(2)?,
                    command: r.get(3)?,
                    entity: EntityRef {
                        kind: r.get(4)?,
                        id: r.get(5)?,
                    },
                    summary: r.get(6)?,
                },
                seq: r.get(7)?,
                causation_id: r.get(8)?,
                undoable: r.get::<_, i64>(9)? != 0,
            })
        })?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Raw payload and inverse JSON of an event (for audit tooling and tests).
    pub fn event_detail(&self, id: &str) -> Result<(Value, Value)> {
        let (p, i): (String, String) = self
            .conn
            .query_row(
                "SELECT payload, inverse FROM events WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
            .ok_or_else(|| StoreError::NotFound(format!("event {id}")))?;
        Ok((serde_json::from_str(&p)?, serde_json::from_str(&i)?))
    }

    #[cfg(test)]
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }
}

fn migrate(conn: &mut Connection) -> Result<()> {
    let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    let supported = MIGRATIONS.len() as i64;
    if found > supported {
        return Err(StoreError::SchemaTooNew { found, supported });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

fn read_entity(conn: &Connection, kind: EntityKind, id: &str) -> Result<Option<Value>> {
    let sql = format!("SELECT data FROM {} WHERE id = ?1", kind.table());
    let raw: Option<String> = conn.query_row(&sql, [id], |r| r.get(0)).optional()?;
    raw.map(|s| serde_json::from_str(&s).map_err(StoreError::from))
        .transpose()
}

fn write_entity(
    tx: &Transaction<'_>,
    kind: EntityKind,
    id: &str,
    value: Option<&Value>,
) -> Result<()> {
    let table = kind.table();
    match value {
        None => {
            tx.execute(&format!("DELETE FROM {table} WHERE id = ?1"), [id])?;
        }
        Some(v) if kind == EntityKind::Item => {
            let parent = v.get("parent_id").and_then(Value::as_str);
            let ord = v.get("order").and_then(Value::as_i64).unwrap_or(0);
            tx.execute(
                "INSERT OR REPLACE INTO items (id, parent_id, ord, data) VALUES (?1, ?2, ?3, ?4)",
                params![id, parent, ord, v.to_string()],
            )?;
        }
        Some(v) => {
            tx.execute(
                &format!("INSERT OR REPLACE INTO {table} (id, data) VALUES (?1, ?2)"),
                params![id, v.to_string()],
            )?;
        }
    }
    Ok(())
}

fn insert_event(
    tx: &Transaction<'_>,
    e: &Event,
    payload: &Value,
    inverse: &Value,
    causation: Option<&str>,
    undoable: bool,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events (id, ts, actor, command, entity_kind, entity_id, summary, payload, inverse, causation_id, undoable)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            e.id,
            e.ts,
            e.actor,
            e.command,
            e.entity.kind,
            e.entity.id,
            e.summary,
            payload.to_string(),
            inverse.to_string(),
            causation,
            i64::from(undoable)
        ],
    )?;
    Ok(())
}

/// Not yet redacted events that name a deleted item in their payload
/// without being about it (consent requests, focus changes), plus every
/// event of the same consent request (its decision repeats the summary).
/// The events a purge would redact: older than `older_than`, or (with
/// `deleted_items`) about or mentioning an item that no longer exists.
fn purge_targets(
    conn: &Connection,
    older_than: Option<&str>,
    deleted_items: bool,
) -> Result<BTreeSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT id FROM events WHERE payload IS NOT ?3 AND (
            (?1 IS NOT NULL AND ts < ?1)
            OR (?2 = 1 AND entity_kind = 'item' AND entity_id NOT IN (SELECT id FROM items)))",
    )?;
    let rows = stmt.query_map(
        params![older_than, i64::from(deleted_items), REDACTED],
        |r| r.get::<_, String>(0),
    )?;
    let mut ids = rows.collect::<std::result::Result<BTreeSet<_>, _>>()?;
    if deleted_items {
        ids.extend(mentioning_deleted_items(conn)?);
    }
    Ok(ids)
}

fn mentioning_deleted_items(tx: &Connection) -> Result<BTreeSet<String>> {
    let deleted: Vec<String> = {
        let mut stmt = tx.prepare(
            "SELECT DISTINCT entity_id FROM events
             WHERE entity_kind = 'item' AND entity_id NOT IN (SELECT id FROM items)",
        )?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    let mut out = BTreeSet::new();
    if deleted.is_empty() {
        return Ok(out);
    }
    let mut confirmations = BTreeSet::new();
    let mut by_confirmation: Vec<(String, String)> = Vec::new();
    {
        let mut stmt = tx.prepare(
            "SELECT id, entity_kind, entity_id, payload FROM events
             WHERE payload IS NOT ?1 AND entity_kind != 'item'",
        )?;
        let rows = stmt.query_map([REDACTED], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (id, kind, entity_id, payload) = row?;
            let names = deleted.iter().any(|d| payload.contains(d.as_str()));
            if kind == "confirmation" {
                if names {
                    confirmations.insert(entity_id.clone());
                }
                by_confirmation.push((id.clone(), entity_id));
            }
            if names {
                out.insert(id);
            }
        }
    }
    out.extend(
        by_confirmation
            .into_iter()
            .filter(|(_, c)| confirmations.contains(c))
            .map(|(id, _)| id),
    );
    Ok(out)
}

fn push(tx: &Transaction<'_>, stack: &str, event_id: &str, actor: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO history_stack (stack, pos, event_id, actor)
         VALUES (?1, coalesce((SELECT max(pos) FROM history_stack WHERE stack = ?1), 0) + 1, ?2, ?3)",
        params![stack, event_id, actor],
    )?;
    Ok(())
}

fn event_record(r: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
    Ok(EventRecord {
        event: Event {
            id: r.get(0)?,
            ts: r.get(1)?,
            actor: r.get(2)?,
            command: r.get(3)?,
            entity: EntityRef {
                kind: r.get(4)?,
                id: r.get(5)?,
            },
            summary: r.get(6)?,
        },
        seq: r.get(7)?,
        causation_id: r.get(8)?,
        undoable: r.get::<_, i64>(9)? != 0,
    })
}

/// Picks the stack entry for an undo/redo and works out its changes,
/// checking every affected entity is still as the event left it.
fn plan(conn: &Connection, undo: bool, target: Option<&str>, actor: &str) -> Result<StepPlan> {
    let from = if undo { "undo" } else { "redo" };
    let any = actor == "user";
    let top: Option<(i64, String, String)> = match target {
        None => conn
            .query_row(
                "SELECT pos, event_id, actor FROM history_stack WHERE stack = ?1 AND (?2 OR actor = ?3)
                 ORDER BY pos DESC LIMIT 1",
                params![from, any, actor],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?,
        Some(id) => conn
            .query_row(
                "SELECT pos, event_id, actor FROM history_stack WHERE stack = ?1 AND event_id = ?2",
                params![from, id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?,
    };
    let Some((pos, event_id, owner)) = top else {
        return Err(StoreError::NotFound(match target {
            None => format!("nothing to {from}"),
            Some(id) => format!("event {id} is not in the {from} history"),
        }));
    };
    if !any && owner != actor {
        return Err(StoreError::Denied(format!(
            "{actor} may only {from} its own changes"
        )));
    }
    let (original, payload, inverse) = conn.query_row(
        "SELECT id, ts, actor, command, entity_kind, entity_id, summary, payload, inverse FROM events WHERE id = ?1",
        [&event_id],
        |r| {
            Ok((
                Event {
                    id: r.get(0)?,
                    ts: r.get(1)?,
                    actor: r.get(2)?,
                    command: r.get(3)?,
                    entity: EntityRef {
                        kind: r.get(4)?,
                        id: r.get(5)?,
                    },
                    summary: r.get(6)?,
                },
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
            ))
        },
    )?;
    let recorded = recorded_changes(&payload, &inverse)?;
    let mut changes = Vec::with_capacity(recorded.len());
    for c in recorded {
        let (expected, target) = if undo {
            (c.after, c.before)
        } else {
            (c.before, c.after)
        };
        let current = read_entity(conn, c.kind, &c.id)?;
        if !same_state(current.as_ref(), expected.as_ref()) {
            return Err(StoreError::Conflict(format!(
                "{} {} changed after \"{}\"; it cannot be {}ne automatically",
                c.kind.as_str(),
                c.id,
                original.summary,
                from
            )));
        }
        changes.push(Change {
            kind: c.kind,
            id: c.id,
            before: current,
            after: target,
        });
    }
    Ok(StepPlan {
        original,
        changes,
        owner,
        pos,
    })
}

/// Collapses repeated changes to the same entity into one (first `before`,
/// last `after`) and drops no-ops, keeping first-seen order.
fn merge_changes(changes: Vec<Change>) -> Vec<Change> {
    let mut out: Vec<Change> = Vec::with_capacity(changes.len());
    for c in changes {
        if let Some(prev) = out.iter_mut().find(|p| p.kind == c.kind && p.id == c.id) {
            prev.after = c.after;
        } else {
            out.push(c);
        }
    }
    out.retain(|c| c.before != c.after);
    out
}

#[derive(Deserialize)]
struct RecordedAfter {
    kind: EntityKind,
    id: String,
    after: Option<Value>,
}

#[derive(Deserialize)]
struct RecordedBefore {
    kind: EntityKind,
    id: String,
    before: Option<Value>,
}

fn recorded_changes(payload: &str, inverse: &str) -> Result<Vec<Change>> {
    #[derive(Deserialize)]
    struct P {
        changes: Vec<RecordedAfter>,
    }
    #[derive(Deserialize)]
    struct I {
        changes: Vec<RecordedBefore>,
    }
    let p: P = serde_json::from_str(payload)?;
    let i: I = serde_json::from_str(inverse)?;
    if p.changes.len() != i.changes.len() {
        return Err(StoreError::Invalid(
            "event payload and inverse disagree".into(),
        ));
    }
    p.changes
        .into_iter()
        .zip(i.changes)
        .map(|(a, b)| {
            if a.kind != b.kind || a.id != b.id {
                return Err(StoreError::Invalid(
                    "event payload and inverse disagree".into(),
                ));
            }
            Ok(Change {
                kind: a.kind,
                id: a.id,
                before: b.before,
                after: a.after,
            })
        })
        .collect()
}

/// Compares two entity states ignoring bookkeeping (`version`, `updated_at`).
/// Whether the stored row is the one a change was computed from. Versioned
/// documents (items) compare `version` and `updated_at`; others compare
/// their content, ignoring keys that only hold a default (`null`, `false`,
/// `[]`, `{}`), so a row written before a field existed still matches the
/// same document read back with that field defaulted.
fn cas_same(stored: Option<&Value>, before: Option<&Value>) -> bool {
    fn norm(v: &Value) -> Value {
        match v {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(_, x)| !is_default(x))
                    .map(|(k, x)| (k.clone(), norm(x)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(norm).collect()),
            other => other.clone(),
        }
    }
    fn is_default(v: &Value) -> bool {
        match v {
            Value::Null | Value::Bool(false) => true,
            Value::Array(a) => a.is_empty(),
            Value::Object(m) => m.is_empty(),
            _ => false,
        }
    }
    match (stored, before) {
        (None, None) => true,
        (Some(a), Some(b)) => match (a.get("version"), b.get("version")) {
            (Some(va), Some(vb)) => va == vb && a.get("updated_at") == b.get("updated_at"),
            _ => norm(a) == norm(b),
        },
        _ => false,
    }
}

fn same_state(a: Option<&Value>, b: Option<&Value>) -> bool {
    fn strip(v: &Value) -> Value {
        match v {
            Value::Object(m) => {
                let mut m = m.clone();
                m.remove("version");
                m.remove("updated_at");
                Value::Object(m)
            }
            other => other.clone(),
        }
    }
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => strip(a) == strip(b),
        _ => false,
    }
}

/// A restored state gets a fresh `version` (one past the current one) and
/// `updated_at`, so optimistic-concurrency clients see it as a new write.
fn restamp(mut target: Value, current: Option<&Value>, now: DateTime<Utc>) -> Value {
    if let Value::Object(m) = &mut target {
        if let Some(v) = m.get("version").and_then(Value::as_u64) {
            let cur = current
                .and_then(|c| c.get("version"))
                .and_then(Value::as_u64)
                .unwrap_or(v);
            m.insert("version".into(), json!(cur.max(v) + 1));
        }
        if m.contains_key("updated_at") {
            m.insert("updated_at".into(), json!(now));
        }
    }
    target
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::keys::{PassphraseKeyProvider, RawKeyProvider};
    use chrono::TimeZone;
    use zeroize::Zeroizing;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 10, 12, 0, 0).unwrap()
    }

    fn ev(command: &str, changes: Vec<Change>) -> NewEvent {
        NewEvent {
            actor: "user".into(),
            command: command.into(),
            entity: EntityRef {
                kind: "item".into(),
                id: changes.first().map(|c| c.id.clone()).unwrap_or_default(),
            },
            summary: format!("{command} test"),
            args: json!({}),
            changes,
            undoable: true,
        }
    }

    fn item(id: &str, title: &str, version: u64) -> Value {
        json!({"id": id, "parent_id": null, "order": 0, "title": title, "version": version, "updated_at": "2026-01-01T00:00:00Z"})
    }

    fn open(dir: &Path) -> Store {
        Store::open(&dir.join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap()
    }

    /// Two stores on one database lost each other's updates; the second
    /// opener is now refused while the first is open.
    #[test]
    fn a_second_store_on_the_same_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let first = open(dir.path());
        let err =
            Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap_err();
        match err {
            StoreError::Conflict(m) => assert_eq!(m, ALREADY_RUNNING),
            other => panic!("expected conflict, got {other:?}"),
        }
        assert!(lock_path(&dir.path().join("hub.db")).ends_with("hub.lock"));
        drop(first);
        // Closing releases the lock.
        let _again = open(dir.path());
    }

    /// Review A4: the lock was keyed on the path as spelled, so the same
    /// database through `..` or a symlink got a second, separate lock.
    #[test]
    fn other_spellings_of_the_same_database_share_its_lock() {
        let dir = tempfile::tempdir().unwrap();
        let _first = open(dir.path());
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let keys = RawKeyProvider::new([7; 32]);
        let dotted = dir.path().join("sub").join("..").join("hub.db");
        assert!(matches!(
            Store::open(&dotted, &keys),
            Err(StoreError::Conflict(_))
        ));
        #[cfg(unix)]
        {
            let alias = dir.path().join("sub").join("alias.db");
            std::os::unix::fs::symlink(dir.path().join("hub.db"), &alias).unwrap();
            assert!(matches!(
                Store::open(&alias, &keys),
                Err(StoreError::Conflict(_))
            ));
        }
    }

    /// Defense in depth: a commit whose `before` no longer matches the row
    /// is refused, and nothing of it is written.
    #[test]
    fn commit_is_compare_and_swap() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let c = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: None,
            after: Some(item("a", "v1", 1)),
        };
        s.commit(ev("tasks.create", vec![c]), now()).unwrap();
        // A stale writer still thinks "a" does not exist.
        let stale = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: None,
            after: Some(item("a", "stale", 1)),
        };
        assert!(matches!(
            s.commit(ev("tasks.create", vec![stale]), now()),
            Err(StoreError::Conflict(_))
        ));
        // Another one read version 1, but the row is at version 2 by now.
        let v2 = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: Some(item("a", "v1", 1)),
            after: Some(item("a", "v2", 2)),
        };
        s.commit(ev("tasks.update", vec![v2]), now()).unwrap();
        let lost = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: Some(item("a", "v1", 1)),
            after: Some(item("a", "lost update", 2)),
        };
        let other = Change {
            kind: EntityKind::Setting,
            id: "x".into(),
            before: None,
            after: Some(json!("y")),
        };
        assert!(matches!(
            s.commit(ev("tasks.update", vec![other, lost]), now()),
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(
            s.get(EntityKind::Item, "a").unwrap().unwrap()["title"],
            "v2"
        );
        assert!(s.get(EntityKind::Setting, "x").unwrap().is_none());
        // Unversioned documents compare content; defaulted keys are ignored.
        let set = Change {
            kind: EntityKind::Setting,
            id: "l".into(),
            before: None,
            after: Some(json!({"id": "l", "widgets": [1]})),
        };
        s.commit(ev("layout.x", vec![set]), now()).unwrap();
        let upd = Change {
            kind: EntityKind::Setting,
            id: "l".into(),
            before: Some(json!({"id": "l", "widgets": [1], "extra": null, "tags": []})),
            after: Some(json!({"id": "l", "widgets": [2]})),
        };
        s.commit(ev("layout.x", vec![upd]), now()).unwrap();
    }

    #[test]
    fn encrypted_at_rest_and_wrong_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("hub.db");
        {
            let mut s = Store::open(&path, &RawKeyProvider::new([7; 32])).unwrap();
            let c = Change {
                kind: EntityKind::Item,
                id: "a".into(),
                before: None,
                after: Some(item("a", "TOP-SECRET-PLANNING-TITLE", 1)),
            };
            s.commit(ev("tasks.create", vec![c]), now()).unwrap();
            s.conn()
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .unwrap();
        }
        let mut bytes = std::fs::read(&path).unwrap();
        for suffix in ["-wal", "-shm"] {
            if let Ok(b) = std::fs::read(format!("{}{suffix}", path.display())) {
                bytes.extend(b);
            }
        }
        let needle = b"TOP-SECRET-PLANNING-TITLE";
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle),
            "plaintext leaked"
        );
        assert!(
            !bytes.starts_with(b"SQLite format 3"),
            "header must be encrypted"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&path), 0o600);
            assert_eq!(mode(path.parent().unwrap()), 0o700);
        }

        let err = Store::open(&path, &RawKeyProvider::new([8; 32])).unwrap_err();
        assert!(matches!(err, StoreError::WrongKey), "{err:?}");
        // the right key still works afterwards
        let s = Store::open(&path, &RawKeyProvider::new([7; 32])).unwrap();
        assert_eq!(
            s.get(EntityKind::Item, "a").unwrap().unwrap()["title"],
            "TOP-SECRET-PLANNING-TITLE"
        );
        assert_eq!(s.schema_version().unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn passphrase_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hub.db");
        let small = crate::keys::KdfParams {
            m_cost: 64,
            t_cost: 1,
            p_cost: 1,
        };
        let good =
            PassphraseKeyProvider::with_params(Zeroizing::new("hunter2".into()), small).unwrap();
        drop(Store::open(&path, &good).unwrap());
        drop(Store::open(&path, &good).unwrap());
        let bad =
            PassphraseKeyProvider::with_params(Zeroizing::new("hunter3".into()), small).unwrap();
        assert!(matches!(
            Store::open(&path, &bad),
            Err(StoreError::WrongKey)
        ));
    }

    #[test]
    fn newer_schema_refused() {
        let dir = tempfile::tempdir().unwrap();
        let s = open(dir.path());
        s.conn().pragma_update(None, "user_version", 99).unwrap();
        drop(s);
        let err =
            Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap_err();
        assert!(matches!(err, StoreError::SchemaTooNew { found: 99, .. }));
    }

    #[test]
    fn events_are_append_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let c = Change {
            kind: EntityKind::Setting,
            id: "k".into(),
            before: None,
            after: Some(json!(1)),
        };
        let e = s.commit(ev("x", vec![c]), now()).unwrap();
        assert!(
            s.conn()
                .execute("UPDATE events SET summary = 'x'", [])
                .is_err()
        );
        assert!(s.conn().execute("DELETE FROM events", []).is_err());
        let list = s.list_events(10, None, None).unwrap();
        assert_eq!(list[0].event, e);
        let (payload, inverse) = s.event_detail(&e.id).unwrap();
        assert_eq!(payload["changes"][0]["after"], json!(1));
        assert_eq!(inverse["changes"][0]["before"], Value::Null);
    }

    #[test]
    fn undo_redo_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let create = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: None,
            after: Some(item("a", "one", 1)),
        };
        s.commit(ev("tasks.create", vec![create]), now()).unwrap();
        let update = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: Some(item("a", "one", 1)),
            after: Some(item("a", "two", 2)),
        };
        let e2 = s.commit(ev("tasks.update", vec![update]), now()).unwrap();
        assert_eq!(s.history_depth().unwrap(), (2, 0));

        let out = s.undo("user", now()).unwrap();
        assert_eq!(out.original.id, e2.id);
        let a = s.get(EntityKind::Item, "a").unwrap().unwrap();
        assert_eq!(a["title"], "one");
        assert_eq!(a["version"], 3, "restored state gets a new version");
        assert_eq!(s.history_depth().unwrap(), (1, 1));

        s.undo("user", now()).unwrap();
        assert!(s.get(EntityKind::Item, "a").unwrap().is_none());
        assert!(matches!(
            s.undo("user", now()),
            Err(StoreError::NotFound(_))
        ));

        s.redo("user", now()).unwrap();
        s.redo("user", now()).unwrap();
        assert_eq!(
            s.get(EntityKind::Item, "a").unwrap().unwrap()["title"],
            "two"
        );
        assert!(matches!(
            s.redo("user", now()),
            Err(StoreError::NotFound(_))
        ));

        let log = s.list_events(10, None, None).unwrap();
        let cmds: Vec<_> = log.iter().map(|r| r.event.command.as_str()).collect();
        assert_eq!(
            cmds,
            [
                "history.redo",
                "history.redo",
                "history.undo",
                "history.undo",
                "tasks.update",
                "tasks.create"
            ]
        );
        assert_eq!(log[0].causation_id.as_deref(), Some(e2.id.as_str()));
        assert!(!log[0].undoable);
        // paging
        let page = s.list_events(2, Some(&log[1].event.id), None).unwrap();
        assert_eq!(page[0].event.id, log[2].event.id);
        assert_eq!(page.len(), 2);
    }

    #[test]
    fn new_write_clears_redo_and_conflicts_block_undo() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let mk = |before, after| Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before,
            after,
        };
        s.commit(
            ev("tasks.create", vec![mk(None, Some(item("a", "one", 1)))]),
            now(),
        )
        .unwrap();
        s.undo("user", now()).unwrap();
        assert_eq!(s.history_depth().unwrap(), (0, 1));
        s.commit(
            ev("tasks.create", vec![mk(None, Some(item("a", "uno", 1)))]),
            now(),
        )
        .unwrap();
        assert_eq!(s.history_depth().unwrap(), (1, 0));

        // a non-undoable write changes the entity behind the stack's back
        let mut sneaky = ev(
            "sync",
            vec![mk(Some(item("a", "uno", 1)), Some(item("a", "other", 2)))],
        );
        sneaky.undoable = false;
        s.commit(sneaky, now()).unwrap();
        assert!(matches!(
            s.undo("user", now()),
            Err(StoreError::Conflict(_))
        ));
        assert_eq!(
            s.get(EntityKind::Item, "a").unwrap().unwrap()["title"],
            "other"
        );
        assert_eq!(
            s.history_depth().unwrap(),
            (1, 0),
            "failed undo leaves stacks alone"
        );
    }

    #[test]
    fn merge_and_noop_changes() {
        let a = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: None,
            after: Some(json!(1)),
        };
        let b = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: Some(json!(1)),
            after: Some(json!(2)),
        };
        let noop = Change {
            kind: EntityKind::Item,
            id: "b".into(),
            before: Some(json!(3)),
            after: Some(json!(3)),
        };
        let m = merge_changes(vec![a, noop, b]);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].before, None);
        assert_eq!(m[0].after, Some(json!(2)));
    }

    #[test]
    fn selective_undo_and_actor_filter() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let mk = |id: &str| Change {
            kind: EntityKind::Item,
            id: id.into(),
            before: None,
            after: Some(item(id, id, 1)),
        };
        let first = s.commit(ev("tasks.create", vec![mk("a")]), now()).unwrap();
        let mut agent = ev("tasks.create", vec![mk("b")]);
        agent.actor = "agent:claude".into();
        let second = s.commit(agent, now()).unwrap();
        // Undo the older event while a newer one sits on top.
        let out = s.undo_event(&first.id, "user", now()).unwrap();
        assert_eq!(out.original.id, first.id);
        assert!(s.get(EntityKind::Item, "a").unwrap().is_none());
        assert!(s.get(EntityKind::Item, "b").unwrap().is_some());
        // Already undone: not in the undo history any more.
        assert!(matches!(
            s.undo_event(&first.id, "user", now()),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(
            s.undo_event("nope", "user", now()),
            Err(StoreError::NotFound(_))
        ));
        // A conflicting selective undo applies nothing.
        s.commit(
            ev(
                "tasks.update",
                vec![Change {
                    kind: EntityKind::Item,
                    id: "b".into(),
                    before: Some(item("b", "b", 1)),
                    after: Some(item("b", "changed", 2)),
                }],
            ),
            now(),
        )
        .unwrap();
        assert!(matches!(
            s.undo_event(&second.id, "user", now()),
            Err(StoreError::Conflict(_))
        ));
        let agents = s.list_events(10, None, Some("agent:")).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].event.id, second.id);
    }

    #[test]
    fn per_actor_history_stacks() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let mk = |id: &str| Change {
            kind: EntityKind::Item,
            id: id.into(),
            before: None,
            after: Some(item(id, id, 1)),
        };
        let user = s.commit(ev("tasks.create", vec![mk("u")]), now()).unwrap();
        // The user undoes and keeps a redo entry.
        s.undo("user", now()).unwrap();
        assert_eq!(s.history_depth().unwrap(), (0, 1));
        let user2 = s.commit(ev("tasks.create", vec![mk("u2")]), now()).unwrap();
        s.undo("user", now()).unwrap();
        assert_eq!(s.history_depth().unwrap(), (0, 1));
        // An agent flood neither clears the user's redo nor evicts history.
        for i in 0..(MAX_UNDO_DEPTH + 5) {
            let mut e = ev("tasks.create", vec![mk(&format!("a{i}"))]);
            e.actor = "agent:x".into();
            s.commit(e, now()).unwrap();
        }
        let (u, r) = s.history_depth().unwrap();
        assert_eq!(r, 1, "the user's redo survives agent writes");
        assert_eq!(u, MAX_UNDO_DEPTH as u64, "agent stack is capped on its own");
        // The agent may not undo or redo the user's events.
        assert!(matches!(
            s.redo_event(&user2.id, "agent:x", now()),
            Err(StoreError::Denied(_))
        ));
        // A plain agent undo takes the agent's own newest entry.
        let out = s.undo("agent:x", now()).unwrap();
        assert_eq!(out.original.actor, "agent:x");
        // The user can still redo their change.
        let out = s.redo_event(&user2.id, "user", now()).unwrap();
        assert_eq!(out.original.id, user2.id);
        assert!(s.get(EntityKind::Item, "u2").unwrap().is_some());
        let _ = user;
    }

    #[test]
    fn step_validation_refuses_without_applying() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let c = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: None,
            after: Some(item("a", "a", 1)),
        };
        s.commit(ev("tasks.create", vec![c]), now()).unwrap();
        let plan = s.plan_step(true, None, "user").unwrap().unwrap();
        assert_eq!(plan.changes[0].after, None);
        let err = s
            .step(true, None, "user", now(), &mut |_| {
                Err(StoreError::Conflict("no".into()))
            })
            .unwrap_err();
        assert!(matches!(err, StoreError::Conflict(_)));
        assert!(s.get(EntityKind::Item, "a").unwrap().is_some());
        assert_eq!(s.history_depth().unwrap(), (1, 0));
    }

    #[test]
    fn purge_redacts_and_audit_is_append_only() {
        let dir = tempfile::tempdir().unwrap();
        let mut s = open(dir.path());
        let c = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: None,
            after: Some(item("a", "SECRET-TITLE", 1)),
        };
        let e = s.commit(ev("tasks.create", vec![c]), now()).unwrap();
        let del = Change {
            kind: EntityKind::Item,
            id: "a".into(),
            before: Some(item("a", "SECRET-TITLE", 1)),
            after: None,
        };
        s.commit(ev("tasks.delete", vec![del]), now()).unwrap();
        // Only redaction passes the trigger.
        assert!(
            s.conn()
                .execute("UPDATE events SET payload = '{}'", [])
                .is_err()
        );
        let n = s.purge_events(None, true).unwrap();
        assert_eq!(n, 2);
        let (p, i) = s.event_detail(&e.id).unwrap();
        assert_eq!(p, json!({"redacted": true}));
        assert_eq!(i, json!({"redacted": true}));
        assert_eq!(s.history_depth().unwrap(), (0, 0));
        let log = s.list_events(10, None, None).unwrap();
        assert!(log.iter().all(|r| r.event.summary == REDACTED_SUMMARY));
        assert_eq!(s.purge_events(None, true).unwrap(), 0);

        let rec = AuditRecord {
            id: "r1".into(),
            ts: "2026-11-10T12:00:00.000Z".into(),
            actor: "agent:x".into(),
            command: "tasks.delete".into(),
            tier: "T3".into(),
            args_hash: "00".into(),
            decision: "confirm".into(),
            reason: "T3".into(),
            approver: None,
            confirmation_id: None,
            tainted: false,
        };
        s.audit_append(&rec).unwrap();
        assert_eq!(s.audit_list(10, None, Some("agent:")).unwrap(), vec![rec]);
        assert!(s.conn().execute("DELETE FROM audit_log", []).is_err());
        assert!(
            s.conn()
                .execute("UPDATE audit_log SET decision = 'allow'", [])
                .is_err()
        );
    }
}
