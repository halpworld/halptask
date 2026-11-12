//! Installed and dev packs (P5, docs/hub/P5-EXTENSIONS.md §3.5-§3.8): the
//! stored records, verification on every load, chain resolution, and the
//! pack-derived part of the type registry (live and retired types).
//!
//! [`load`] is pure over its [`Inputs`]: the commands run it on a candidate
//! set of records to see what a change would do before committing it, and
//! the state runs it on the stored records after every pack change.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use halp_domain::task_type::builtin_types;
use halp_domain::{TaskType, TaskTypeOrigin};
use halp_pack::{self as hp, FileIndex, Manifest, PackTrust};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::envelope::CmdError;
use crate::packs::Pack;

/// The Hub version packs are matched against (`min_hub`, and the version
/// of the built-in packs for `extends`).
pub(crate) const HUB_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn hub_version() -> semver::Version {
    semver::Version::parse(HUB_VERSION).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
}

/// The package id of a built-in role pack (`developer` → `halp.developer`).
pub(crate) fn builtin_package_id(role: &str) -> String {
    format!("halp.{role}")
}

/// halp-pack's trust as the domain's mirror (the orphan rule rules out a
/// `From` impl).
pub(crate) fn domain_trust(t: PackTrust) -> halp_domain::PackTrust {
    use halp_domain::PackTrust as D;
    match t {
        PackTrust::Builtin => D::Builtin,
        PackTrust::FirstParty => D::FirstParty,
        PackTrust::Community => D::Community,
        PackTrust::Unsigned => D::Unsigned,
        PackTrust::Dev => D::Dev,
    }
}

// ---- stored records -------------------------------------------------------

/// One version of an installed pack. The bundle bytes live in
/// `pack_blobs` under `bundle_sha256` (outside the event payload).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackVersion {
    pub version: String,
    pub trust: PackTrust,
    /// The key pinned at first install (first-party or community), if any.
    pub key_id: Option<String>,
    /// That key's full fingerprint (64 hex): the pin, compared in full
    /// (v1.10). Absent on rows written earlier, whose 64-bit `key_id` pin
    /// still counts until their next update records it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_fp: Option<String>,
    pub bundle_sha256: String,
    pub manifest_sha256: String,
    pub title: String,
    pub role_id: String,
    /// The pack's own types at install: what is retired when the bundle no
    /// longer verifies, so class and lock rules still hold.
    pub types: Vec<TaskType>,
}

impl PackVersion {
    /// The pin [`hp::check_pin`] compares: the full fingerprint, or a
    /// legacy key id.
    pub fn pin(&self) -> Option<&str> {
        self.key_fp.as_deref().or(self.key_id.as_deref())
    }

    /// Whether this version is pinned to `k` (by full fingerprint when
    /// recorded).
    pub fn pinned_to(&self, k: &KeyRecord) -> bool {
        match &self.key_fp {
            Some(fp) => hp::crypto::decode_pubkey(&k.pubkey_b64)
                .is_ok_and(|p| fp_hex(&hp::fingerprint(&p)) == *fp),
            None => self.key_id.as_deref() == Some(k.key_id.as_str()),
        }
    }
}

/// An item an update migrated, with what it was before (for rollback).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MigratedItem {
    pub id: String,
    pub type_id: Option<String>,
    pub fields: Map<String, Value>,
    /// The item's version right after the migration; rollback refuses
    /// when it changed since.
    pub version: u64,
}

/// A `packs` row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PackRecord {
    pub id: String,
    pub enabled: bool,
    pub installed_at: String,
    pub current: PackVersion,
    pub previous: Option<PackVersion>,
    /// Items the update to `current` migrated.
    #[serde(default)]
    pub migrated: Vec<MigratedItem>,
}

/// A `pack_keys` row: a community key the user trusted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct KeyRecord {
    pub key_id: String,
    pub pubkey_b64: String,
    pub label: String,
    pub added_at: String,
    pub revoked_at: Option<String>,
}

impl KeyRecord {
    pub fn community(&self) -> Option<hp::CommunityKey> {
        Some(hp::CommunityKey {
            key_id: self.key_id.clone(),
            pubkey: hp::crypto::decode_pubkey(&self.pubkey_b64).ok()?,
            revoked: self.revoked_at.is_some(),
        })
    }
}

/// A `retired_types` row: the last definition of a type whose pack left.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RetiredRecord {
    pub id: String,
    pub def: TaskType,
    pub from_pack: String,
    pub trust: PackTrust,
    pub retired_at: String,
}

// ---- dev folder -----------------------------------------------------------

/// Most files read from the dev folder (all packs together).
pub(crate) const DEV_MAX_FILES: usize = 512;
/// Most bytes read from the dev folder (all packs together).
pub(crate) const DEV_MAX_BYTES: u64 = 8 * 1024 * 1024;

/// A pack folder read from the dev folder, before verification.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DevPack {
    pub dir: String,
    pub manifest_text: String,
    pub files: BTreeMap<String, Vec<u8>>,
}

/// Dev mode (§3.8): the folder (never persisted) and what was read from it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DevState {
    pub path: PathBuf,
    pub packs: Vec<DevPack>,
    /// Folders that could not be read: `(dir, message)`.
    pub errors: Vec<(String, String)>,
}

/// Reads the dev folder: each subfolder (or the folder itself) holding a
/// `halp-pack.json`. Symlinks are never followed; dotfiles are skipped.
pub(crate) fn read_dev(path: &Path) -> Result<DevState, CmdError> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| CmdError::invalid(format!("dev folder: {e}")))?;
    if !meta.is_dir() {
        return Err(CmdError::invalid("dev folder must be a directory"));
    }
    let mut dirs: Vec<(String, PathBuf)> = Vec::new();
    if is_file(&path.join(hp::ids::MANIFEST_FILE)) {
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("pack")
            .to_string();
        dirs.push((name, path.to_path_buf()));
    } else {
        let rd =
            std::fs::read_dir(path).map_err(|e| CmdError::invalid(format!("dev folder: {e}")))?;
        for e in rd.flatten() {
            let Ok(ft) = e.file_type() else { continue };
            let name = e.file_name().to_string_lossy().to_string();
            if ft.is_dir()
                && !name.starts_with('.')
                && is_file(&e.path().join(hp::ids::MANIFEST_FILE))
            {
                dirs.push((name, e.path()));
            }
        }
    }
    dirs.sort();
    let mut budget = (DEV_MAX_FILES, DEV_MAX_BYTES);
    let mut packs = Vec::new();
    let mut errors = Vec::new();
    for (name, dir) in dirs {
        match read_dev_pack(&dir, &mut budget) {
            Ok((manifest_text, files)) => packs.push(DevPack {
                dir: name,
                manifest_text,
                files,
            }),
            Err(m) => errors.push((name, m)),
        }
    }
    Ok(DevState {
        path: path.to_path_buf(),
        packs,
        errors,
    })
}

fn is_file(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_file())
}

type DevFiles = (String, BTreeMap<String, Vec<u8>>);

fn read_dev_pack(dir: &Path, budget: &mut (usize, u64)) -> Result<DevFiles, String> {
    let manifest_path = dir.join(hp::ids::MANIFEST_FILE);
    let len = std::fs::symlink_metadata(&manifest_path)
        .map_err(|e| e.to_string())?
        .len();
    if len > hp::manifest::MAX_MANIFEST_BYTES as u64 {
        return Err("halp-pack.json is too large".into());
    }
    let manifest_text = std::fs::read_to_string(&manifest_path).map_err(|e| e.to_string())?;
    let mut files = BTreeMap::new();
    let mut stack: Vec<(PathBuf, String)> = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, prefix)) = stack.pop() {
        let rd = std::fs::read_dir(&d).map_err(|e| e.to_string())?;
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let rel = format!("{prefix}{name}");
            if rel == hp::ids::MANIFEST_FILE {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_symlink() {
                return Err(format!(
                    "{rel} is a symlink, which dev mode does not follow"
                ));
            }
            if ft.is_dir() {
                if rel.split('/').count() >= hp::ids::MAX_PATH_DEPTH {
                    return Err(format!("{rel} nests too deep"));
                }
                stack.push((e.path(), format!("{rel}/")));
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let size = e.metadata().map_err(|e| e.to_string())?.len();
            if budget.0 == 0 || size > budget.1 {
                return Err("the dev folder holds too many files or bytes".into());
            }
            budget.0 -= 1;
            budget.1 -= size;
            hp::ids::check_path(&rel).map_err(|e| e.message)?;
            let bytes = std::fs::read(e.path()).map_err(|e| e.to_string())?;
            files.insert(rel, bytes);
        }
    }
    Ok((manifest_text, files))
}

/// The dev manifest as loaded: id `dev.<id>`, role id `dev-<role>`, its
/// layout moved to that role, and `files` computed from the folder.
fn dev_manifest_text(dp: &DevPack) -> Result<String, CmdError> {
    let mut v: Value = serde_json::from_str(&dp.manifest_text)
        .map_err(|e| CmdError::invalid(format!("halp-pack.json is not JSON: {e}")))?;
    let Some(m) = v.as_object_mut() else {
        return Err(CmdError::invalid("halp-pack.json must be an object"));
    };
    let id = m.get("id").and_then(Value::as_str).unwrap_or_default();
    let id = format!("{}{id}", hp::ids::DEV_PREFIX);
    m.insert("id".into(), Value::String(id));
    let role = m
        .get("role")
        .and_then(|r| r.get("id"))
        .and_then(Value::as_str)
        .map(|r| format!("dev-{r}"));
    if let Some(role) = role {
        if let Some(r) = m.get_mut("role").and_then(Value::as_object_mut) {
            r.insert("id".into(), Value::String(role.clone()));
        }
        if let Some(l) = m.get_mut("layout").and_then(Value::as_object_mut) {
            l.insert("role_id".into(), Value::String(role.clone()));
            l.insert("id".into(), Value::String(format!("{role}-default")));
        }
    }
    m.insert(
        "files".into(),
        serde_json::to_value(hp::file_index(&dp.files))?,
    );
    Ok(serde_json::to_string(&v)?)
}

// ---- the loaded view ------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Ok,
    Disabled,
    /// Did not verify (tampered, revoked, malformed).
    Error,
    /// Verified, but could not be loaded with the others (a missing or
    /// incompatible parent, a clash).
    Blocked,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Disabled => "disabled",
            Status::Error => "error",
            Status::Blocked => "blocked",
        }
    }
}

/// A pack as loaded (built-in, installed or dev).
#[derive(Debug, Clone)]
pub(crate) struct LoadedPack {
    pub id: String,
    pub version: String,
    pub title: String,
    pub role_id: String,
    pub trust: PackTrust,
    pub enabled: bool,
    pub status: Status,
    pub error: Option<CmdError>,
    /// Package ids, root first, ending with this one (when resolved).
    pub chain: Vec<String>,
    pub key_fp: Option<String>,
    pub manifest: Option<Arc<Manifest>>,
    /// The verified bytes (empty for built-in packs).
    pub files: Arc<BTreeMap<String, Vec<u8>>>,
    pub file_index: FileIndex,
    pub prompts: Vec<hp::PromptRef>,
    pub previous_version: Option<String>,
    pub warnings: Vec<String>,
    /// The privileged annotations of its own types.
    pub annotations: usize,
}

impl LoadedPack {
    pub fn is_dev(&self) -> bool {
        self.trust == PackTrust::Dev
    }

    pub fn is_live(&self) -> bool {
        self.status == Status::Ok
    }
}

/// Where a pack-derived registry entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeSource {
    /// Defined by a loaded pack (or a pack a loaded pack extends).
    Live,
    /// Retired in memory: its pack is disabled or failed to load.
    Inactive,
    /// Retired by a `retired_types` row.
    Table,
}

#[derive(Debug, Clone)]
pub(crate) struct PackType {
    pub def: TaskType,
    pub origin: TaskTypeOrigin,
    pub source: TypeSource,
}

/// Everything packs contribute to the state.
#[derive(Debug, Clone, Default)]
pub(crate) struct PackCache {
    /// Loaded roles: built-in first, then installed, then dev.
    pub packs: Vec<Pack>,
    /// Every pack by package id, loaded or not.
    pub meta: BTreeMap<String, LoadedPack>,
    /// Pack-derived types (built-in pack types included, base types not).
    pub types: BTreeMap<String, PackType>,
}

impl PackCache {
    pub fn live_type_ids(&self) -> BTreeSet<&str> {
        self.types
            .iter()
            .filter(|(_, t)| t.source == TypeSource::Live)
            .map(|(k, _)| k.as_str())
            .collect()
    }
}

/// What [`load`] reads.
pub(crate) struct Inputs<'a> {
    pub builtin: &'a [Pack],
    pub records: &'a [PackRecord],
    pub blob: &'a dyn Fn(&str) -> Option<Vec<u8>>,
    pub keys: &'a [KeyRecord],
    pub retired: &'a [RetiredRecord],
    pub dev: Option<&'a DevState>,
    /// Ids of the user's own types: a pack may not take them over.
    pub user_types: &'a BTreeSet<String>,
}

fn class_of(t: &TaskType) -> Option<&str> {
    t.schema
        .get(crate::state::DATA_CLASS_KEY)
        .and_then(Value::as_str)
}

/// Whether a pack of `trust` may define `new`, an id an earlier
/// definition `old` (of trust `old_trust`) left behind: it must keep the
/// data class, and a pack that may not carry privileged annotations may
/// not take over a type whose source could (its locks would stop counting).
pub(crate) fn may_claim(
    old: &TaskType,
    old_trust: PackTrust,
    new: &TaskType,
    trust: PackTrust,
) -> Result<(), CmdError> {
    if class_of(old) != class_of(new) {
        return Err(CmdError::conflict(format!(
            "task type {} would change its data class from {} to {}",
            new.id,
            class_of(old).unwrap_or(crate::policy::DEFAULT_DATA_CLASS),
            class_of(new).unwrap_or(crate::policy::DEFAULT_DATA_CLASS)
        )));
    }
    if old_trust.may_annotate()
        && !trust.may_annotate()
        && !hp::trust::privileged_in(&old.schema).is_empty()
    {
        return Err(CmdError::conflict(format!(
            "task type {} carries locks or integration rules that a {} pack cannot take over",
            new.id,
            trust.as_str()
        )));
    }
    Ok(())
}

/// A pack that verified.
struct Verified {
    manifest: Manifest,
    trust: PackTrust,
    files: BTreeMap<String, Vec<u8>>,
    index: FileIndex,
    key_fp: Option<String>,
    warnings: Vec<String>,
}

fn annotations_of(types: &[TaskType]) -> usize {
    types
        .iter()
        .map(|t| hp::trust::privileged_in(&t.schema).len())
        .sum()
}

/// Re-verifies a stored version: the bundle bytes against their hash and
/// the manifest's file index, the signature against the keys trusted
/// now, the pin, and the manifest against the stored trust.
pub(crate) fn verify_stored(
    id: &str,
    v: &PackVersion,
    blob: &dyn Fn(&str) -> Option<Vec<u8>>,
    keys: &[KeyRecord],
) -> Result<hp::VerifiedBundle, CmdError> {
    let bytes = blob(&v.bundle_sha256)
        .ok_or_else(|| CmdError::invalid(format!("pack {id}: its stored bundle is missing")))?;
    if hp::ids::sha256_hex(&bytes) != v.bundle_sha256 {
        return Err(CmdError::invalid(format!(
            "pack {id}: its stored bundle does not match its hash"
        )));
    }
    let community: Vec<hp::CommunityKey> = keys.iter().filter_map(KeyRecord::community).collect();
    let mut vb = hp::verify(&bytes, &community)?;
    if vb.manifest_sha256 != v.manifest_sha256
        || vb.manifest.id != id
        || vb.manifest.version != v.version
    {
        return Err(CmdError::invalid(format!(
            "pack {id}: its stored manifest does not match its record"
        )));
    }
    hp::check_pin(v.pin(), &vb)?;
    if vb.trust != v.trust {
        // Installed unsigned, and its key was trusted later: it keeps the
        // trust the user accepted at install.
        if v.trust != PackTrust::Unsigned {
            return Err(CmdError::invalid(format!(
                "pack {id}: its trust changed from {} to {}",
                v.trust.as_str(),
                vb.trust.as_str()
            )));
        }
        let (m, w) = Manifest::parse(&vb.manifest_text, PackTrust::Unsigned)?;
        vb.manifest = m;
        vb.warnings = w;
        vb.trust = PackTrust::Unsigned;
    }
    Ok(vb)
}

fn verify_dev(dp: &DevPack) -> Result<Verified, CmdError> {
    let text = dev_manifest_text(dp)?;
    let (manifest, warnings) = Manifest::parse(&text, PackTrust::Dev)?;
    let index = manifest.files.clone();
    hp::bundle::check_files(&index, &dp.files)?;
    Ok(Verified {
        manifest,
        trust: PackTrust::Dev,
        files: dp.files.clone(),
        index,
        key_fp: None,
        warnings,
    })
}

fn effective(p: &Pack) -> hp::EffectivePack {
    hp::EffectivePack {
        id: p.id.clone(),
        title: p.title.clone(),
        accent: p.accent.clone(),
        widgets: p.widgets.clone(),
        task_types: p.task_types.clone(),
        layout: p.layout.clone(),
    }
}

fn failed(meta: &mut LoadedPack, e: CmdError, status: Status) {
    meta.status = status;
    meta.error = Some(e);
}

/// Builds the pack view from the inputs. Never fails: a pack that does not
/// verify or load is recorded with its error, and its types are retired.
pub(crate) fn load(inp: &Inputs) -> PackCache {
    let hub = hub_version();
    let mut meta: BTreeMap<String, LoadedPack> = BTreeMap::new();
    let mut builtin_types_map: BTreeMap<String, PackType> = BTreeMap::new();
    let mut bases: BTreeMap<String, hp::BuiltinBase> = BTreeMap::new();
    for p in inp.builtin {
        let pid = builtin_package_id(&p.id);
        for t in &p.task_types {
            builtin_types_map.insert(
                t.id.clone(),
                PackType {
                    def: t.clone(),
                    origin: TaskTypeOrigin::Pack {
                        trust: halp_domain::PackTrust::Builtin,
                        pack: pid.clone(),
                    },
                    source: TypeSource::Live,
                },
            );
        }
        bases.insert(
            pid.clone(),
            hp::BuiltinBase {
                package_id: pid.clone(),
                version: hub.clone(),
                pack: effective(p),
            },
        );
        meta.insert(
            pid.clone(),
            LoadedPack {
                id: pid.clone(),
                version: HUB_VERSION.to_string(),
                title: p.title.clone(),
                role_id: p.id.clone(),
                trust: PackTrust::Builtin,
                enabled: true,
                status: Status::Ok,
                error: None,
                chain: vec![pid],
                key_fp: None,
                manifest: None,
                files: Arc::default(),
                file_index: FileIndex::new(),
                prompts: Vec::new(),
                previous_version: None,
                warnings: Vec::new(),
                annotations: annotations_of(&p.task_types),
            },
        );
    }

    // 1. Verify. Installed packs first, then dev packs: that order also
    // decides who wins a clash.
    let mut order: Vec<String> = Vec::new();
    let mut verified: BTreeMap<String, Verified> = BTreeMap::new();
    let mut snapshots: BTreeMap<String, (PackTrust, Vec<TaskType>)> = BTreeMap::new();
    for rec in inp.records {
        if meta.contains_key(&rec.id) {
            continue;
        }
        let mut m = LoadedPack {
            id: rec.id.clone(),
            version: rec.current.version.clone(),
            title: rec.current.title.clone(),
            role_id: rec.current.role_id.clone(),
            trust: rec.current.trust,
            enabled: rec.enabled,
            status: if rec.enabled {
                Status::Ok
            } else {
                Status::Disabled
            },
            error: None,
            chain: Vec::new(),
            key_fp: None,
            manifest: None,
            files: Arc::default(),
            file_index: FileIndex::new(),
            prompts: Vec::new(),
            previous_version: rec.previous.as_ref().map(|p| p.version.clone()),
            warnings: Vec::new(),
            annotations: annotations_of(&rec.current.types),
        };
        snapshots.insert(
            rec.id.clone(),
            (rec.current.trust, rec.current.types.clone()),
        );
        match verify_stored(&rec.id, &rec.current, inp.blob, inp.keys) {
            Ok(vb) => {
                m.key_fp = vb
                    .signer
                    .as_ref()
                    .filter(|s| rec.current.key_id.as_deref() == Some(s.key_id.as_str()))
                    .filter(|s| {
                        rec.current
                            .key_fp
                            .as_ref()
                            .is_none_or(|fp| *fp == fp_hex(&s.fingerprint))
                    })
                    .map(|s| fp_hex(&s.fingerprint));
                m.warnings = vb.warnings.clone();
                m.manifest = Some(Arc::new(vb.manifest.clone()));
                verified.insert(
                    rec.id.clone(),
                    Verified {
                        manifest: vb.manifest,
                        trust: vb.trust,
                        files: vb.files,
                        index: vb.file_index,
                        key_fp: m.key_fp.clone(),
                        warnings: vb.warnings,
                    },
                );
                order.push(rec.id.clone());
            }
            Err(e) => failed(&mut m, e, Status::Error),
        }
        meta.insert(rec.id.clone(), m);
    }
    if let Some(dev) = inp.dev {
        for dp in &dev.packs {
            // The folder name is not checked text: keep a safe ASCII form
            // for the id and title an errored dev pack shows (to agents too).
            let dir: String = dp
                .dir
                .chars()
                .map(|c| {
                    if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                        c.to_ascii_lowercase()
                    } else {
                        '_'
                    }
                })
                .take(64)
                .collect();
            let fallback = format!("{}{}", hp::ids::DEV_PREFIX, dir);
            match verify_dev(dp) {
                Ok(v) => {
                    let id = v.manifest.id.clone();
                    if meta.contains_key(&id) {
                        continue;
                    }
                    let types = v.manifest.task_types.clone();
                    snapshots.insert(id.clone(), (PackTrust::Dev, types.clone()));
                    meta.insert(
                        id.clone(),
                        LoadedPack {
                            id: id.clone(),
                            version: v.manifest.version.clone(),
                            title: v.manifest.title.clone(),
                            role_id: v.manifest.role.id.clone(),
                            trust: PackTrust::Dev,
                            enabled: true,
                            status: Status::Ok,
                            error: None,
                            chain: Vec::new(),
                            key_fp: None,
                            manifest: Some(Arc::new(v.manifest.clone())),
                            files: Arc::default(),
                            file_index: FileIndex::new(),
                            prompts: Vec::new(),
                            previous_version: None,
                            warnings: v.warnings.clone(),
                            annotations: annotations_of(&types),
                        },
                    );
                    verified.insert(id.clone(), v);
                    order.push(id);
                }
                Err(e) => {
                    if meta.contains_key(&fallback) {
                        continue;
                    }
                    meta.insert(
                        fallback.clone(),
                        LoadedPack {
                            id: fallback.clone(),
                            version: String::new(),
                            title: dir.clone(),
                            role_id: String::new(),
                            trust: PackTrust::Dev,
                            enabled: true,
                            status: Status::Error,
                            error: Some(e),
                            chain: Vec::new(),
                            key_fp: None,
                            manifest: None,
                            files: Arc::default(),
                            file_index: FileIndex::new(),
                            prompts: Vec::new(),
                            previous_version: None,
                            warnings: Vec::new(),
                            annotations: 0,
                        },
                    );
                }
            }
        }
    }

    // 2. Resolve every enabled pack's chain.
    let lookup = |child_dev: bool| {
        let bases = &bases;
        let verified = &verified;
        move |pid: &str| -> Option<hp::Parent> {
            if let Some(b) = bases.get(pid) {
                return Some(hp::Parent::Builtin(Box::new(b.clone())));
            }
            let v = verified.get(pid)?;
            if v.trust == PackTrust::Dev && !child_dev {
                return None;
            }
            Some(hp::Parent::Installed(Box::new(hp::ChainLink {
                manifest: v.manifest.clone(),
                trust: v.trust,
            })))
        }
    };
    let mut live: Vec<(String, hp::Resolved, Pack)> = Vec::new();
    for id in &order {
        let Some(v) = verified.get(id) else { continue };
        let Some(m) = meta.get_mut(id) else { continue };
        if !m.enabled {
            continue;
        }
        if !v.manifest.supports_hub(&hub).unwrap_or(false) {
            failed(
                m,
                CmdError::conflict(format!(
                    "pack {id} needs Hub {}, this is {HUB_VERSION}",
                    v.manifest.min_hub
                )),
                Status::Blocked,
            );
            continue;
        }
        let child = hp::ChainLink {
            manifest: v.manifest.clone(),
            trust: v.trust,
        };
        let r = hp::collect_chain(child, lookup(v.trust == PackTrust::Dev))
            .and_then(|c| hp::resolve(&c))
            .map_err(CmdError::from)
            .and_then(|r| {
                let ep = r.pack.clone();
                let p = Pack {
                    id: ep.id,
                    title: ep.title,
                    accent: ep.accent,
                    widgets: ep.widgets,
                    task_types: ep.task_types,
                    layout: ep.layout,
                    inherited: r.inherited_type_ids.clone(),
                };
                p.validate()?;
                Ok((r, p))
            });
        match r {
            Ok((r, p)) => {
                m.chain = r.chain.clone();
                m.prompts = r.prompts.clone();
                live.push((id.clone(), r, p));
            }
            Err(e) => failed(m, e, Status::Blocked),
        }
    }

    // 3. Load them together; a pack that clashes drops out, and the rest
    // are checked again without it.
    let base_ids: BTreeSet<String> = builtin_types().into_iter().map(|t| t.id).collect();
    let mut types;
    loop {
        types = builtin_types_map.clone();
        let mut drop: Vec<(String, CmdError)> = Vec::new();
        // Providers: every pack in a live pack's chain, in load order.
        let in_chains: BTreeSet<&str> = live
            .iter()
            .flat_map(|(_, r, _)| r.chain.iter().map(String::as_str))
            .collect();
        'providers: for pid in order.iter().filter(|p| in_chains.contains(p.as_str())) {
            let Some(v) = verified.get(pid) else { continue };
            for t in &v.manifest.task_types {
                let owner = snapshots
                    .iter()
                    .find(|(o, (_, defs))| *o != pid && defs.iter().any(|d| d.id == t.id));
                let prior = inp.retired.iter().find(|r| r.id == t.id);
                let clash = if base_ids.contains(&t.id)
                    || types.contains_key(&t.id)
                    || inp.user_types.contains(&t.id)
                {
                    Some(CmdError::conflict(format!(
                        "pack {pid}: task type {} is already defined",
                        t.id
                    )))
                } else if let Some((o, _)) = owner {
                    Some(CmdError::conflict(format!(
                        "pack {pid}: task type {} belongs to pack {o}",
                        t.id
                    )))
                } else if let Some(r) =
                    prior.filter(|r| v.trust == PackTrust::Dev && r.trust != PackTrust::Dev)
                {
                    // A dev pack never takes over a retired type of a real
                    // pack: its items are the user's, not test data.
                    Some(CmdError::conflict(format!(
                        "pack {pid}: a dev pack cannot take over retired task type {} of pack {}",
                        t.id, r.from_pack
                    )))
                } else if let Some(r) = prior {
                    may_claim(&r.def, r.trust, t, v.trust)
                        .err()
                        .map(|e| CmdError::conflict(format!("pack {pid}: {}", e.message)))
                } else {
                    None
                };
                if let Some(e) = clash {
                    for (lid, r, _) in &live {
                        if r.chain.iter().any(|c| c == pid) {
                            drop.push((lid.clone(), e.clone()));
                        }
                    }
                    break 'providers;
                }
                types.insert(
                    t.id.clone(),
                    PackType {
                        def: t.clone(),
                        origin: TaskTypeOrigin::Pack {
                            trust: domain_trust(v.trust),
                            pack: pid.clone(),
                        },
                        source: TypeSource::Live,
                    },
                );
            }
        }
        if drop.is_empty() {
            let mut overridden: BTreeMap<String, (String, TaskType)> = BTreeMap::new();
            let mut roles: BTreeSet<String> = inp.builtin.iter().map(|p| p.id.clone()).collect();
            let mut layouts: BTreeSet<String> =
                inp.builtin.iter().map(|p| p.layout.id.clone()).collect();
            for (lid, r, p) in &live {
                let mut bad = None;
                for t in &r.overridden_types {
                    match overridden.get(&t.id) {
                        Some((other, d)) if d != t => {
                            bad = Some(CmdError::conflict(format!(
                                "pack {lid}: task type {} is also overridden by {other}",
                                t.id
                            )));
                        }
                        _ if !types.contains_key(&t.id) => {
                            bad = Some(CmdError::conflict(format!(
                                "pack {lid}: overrides task type {}, which is not loaded",
                                t.id
                            )));
                        }
                        _ => {}
                    }
                }
                if bad.is_none() && !roles.insert(p.id.clone()) {
                    bad = Some(CmdError::conflict(format!(
                        "pack {lid}: role id {} is already used by another pack",
                        p.id
                    )));
                }
                if bad.is_none() && !layouts.insert(p.layout.id.clone()) {
                    bad = Some(CmdError::conflict(format!(
                        "pack {lid}: layout id {} is already used by another pack",
                        p.layout.id
                    )));
                }
                if let Some(e) = bad {
                    drop.push((lid.clone(), e));
                    break;
                }
                for t in &r.overridden_types {
                    overridden.insert(t.id.clone(), (lid.clone(), t.clone()));
                    if let Some(e) = types.get_mut(&t.id) {
                        e.def = t.clone();
                    }
                }
            }
        }
        if drop.is_empty() {
            break;
        }
        for (id, e) in drop {
            live.retain(|(l, _, _)| *l != id);
            if let Some(m) = meta.get_mut(&id) {
                m.chain.clear();
                m.prompts.clear();
                failed(m, e, Status::Blocked);
            }
        }
    }

    // 4. Retired: types of packs that are not loaded (in memory), then the
    // `retired_types` rows, each only where no live pack defines the id.
    for (id, (trust, defs)) in &snapshots {
        for t in defs {
            if types.contains_key(&t.id) || base_ids.contains(&t.id) {
                continue;
            }
            types.insert(
                t.id.clone(),
                PackType {
                    def: t.clone(),
                    origin: TaskTypeOrigin::Retired {
                        trust: domain_trust(*trust),
                        pack: id.clone(),
                    },
                    source: TypeSource::Inactive,
                },
            );
        }
    }
    for r in inp.retired {
        if types.contains_key(&r.id) || base_ids.contains(&r.id) {
            continue;
        }
        types.insert(
            r.id.clone(),
            PackType {
                def: r.def.clone(),
                origin: TaskTypeOrigin::Retired {
                    trust: domain_trust(r.trust),
                    pack: r.from_pack.clone(),
                },
                source: TypeSource::Table,
            },
        );
    }

    let mut packs: Vec<Pack> = inp.builtin.to_vec();
    for (id, _, p) in live {
        if let Some(v) = verified.remove(&id)
            && let Some(m) = meta.get_mut(&id)
        {
            m.files = Arc::new(v.files);
            m.file_index = v.index;
            m.key_fp = v.key_fp;
            m.warnings = v.warnings;
        }
        packs.push(p);
    }
    PackCache { packs, meta, types }
}

/// The `retired_types` rows a change from `before` to `after` adds and
/// removes: a pack type that is no longer live is retired with its last
/// definition; a retired id that is live again is un-retired. Types of dev
/// packs are kept in the table even while live: dev mode is not
/// persisted, and their items must keep a definition after a restart.
pub(crate) fn retire_diff(
    before: &PackCache,
    after: &PackCache,
    table: &[RetiredRecord],
    now: &str,
) -> (Vec<RetiredRecord>, Vec<String>) {
    let in_table: BTreeMap<&str, &RetiredRecord> =
        table.iter().map(|r| (r.id.as_str(), r)).collect();
    let live = after.live_type_ids();
    let is_dev = |o: &TaskTypeOrigin| {
        matches!(
            o,
            TaskTypeOrigin::Pack {
                trust: halp_domain::PackTrust::Dev,
                ..
            }
        )
    };
    let row = |id: &str, t: &PackType| -> Option<RetiredRecord> {
        let (trust, pack) = match &t.origin {
            TaskTypeOrigin::Pack { trust, pack } | TaskTypeOrigin::Retired { trust, pack } => {
                (*trust, pack.clone())
            }
            _ => return None,
        };
        Some(RetiredRecord {
            id: id.to_string(),
            def: t.def.clone(),
            from_pack: pack,
            trust: pack_trust(trust),
            retired_at: now.to_string(),
        })
    };
    let mut add: BTreeMap<String, RetiredRecord> = BTreeMap::new();
    for (id, t) in &before.types {
        if t.source == TypeSource::Table
            || live.contains(id.as_str())
            || in_table.contains_key(id.as_str())
        {
            continue;
        }
        if let Some(r) = row(id, t) {
            add.insert(id.clone(), r);
        }
    }
    for (id, t) in &after.types {
        if t.source != TypeSource::Live || !is_dev(&t.origin) {
            continue;
        }
        let Some(r) = row(id, t) else { continue };
        // Never replace a real pack's retired row with a dev one.
        if in_table
            .get(id.as_str())
            .is_some_and(|old| old.trust != PackTrust::Dev)
        {
            continue;
        }
        let same = in_table
            .get(id.as_str())
            .is_some_and(|old| old.def == r.def && old.from_pack == r.from_pack);
        if !same {
            add.insert(id.clone(), r);
        }
    }
    let remove = table
        .iter()
        .filter(|r| live.contains(r.id.as_str()) && !add.contains_key(&r.id))
        .filter(|r| {
            !after
                .types
                .get(&r.id)
                .is_some_and(|t| t.source == TypeSource::Live && is_dev(&t.origin))
        })
        .map(|r| r.id.clone())
        .collect();
    (add.into_values().collect(), remove)
}

/// The domain trust back as halp-pack's.
pub(crate) fn pack_trust(t: halp_domain::PackTrust) -> PackTrust {
    use halp_domain::PackTrust as D;
    match t {
        D::Builtin => PackTrust::Builtin,
        D::FirstParty => PackTrust::FirstParty,
        D::Community => PackTrust::Community,
        D::Unsigned => PackTrust::Unsigned,
        D::Dev => PackTrust::Dev,
    }
}

/// Content types a Tier C file may be served as, by extension. No SVG: it
/// can run script as a document, and only `.html` documents are ticketed.
pub(crate) fn ui_mime(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1;
    Some(match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "png" => "image/png",
        "woff2" => "font/woff2",
        _ => return None,
    })
}

/// A `packs.preview` held for `packs.install` (in memory only).
#[derive(Debug, Clone)]
pub(crate) struct Preview {
    pub bytes: Arc<Vec<u8>>,
    pub bundle_sha256: String,
    pub expires: chrono::DateTime<chrono::Utc>,
}

/// How long a preview stays installable.
pub(crate) const PREVIEW_TTL_MINUTES: i64 = 10;
/// Most previews held at once (the oldest is dropped).
pub(crate) const MAX_PREVIEWS: usize = 4;

/// A fingerprint as clients take it: 64 hex, without the grouping spaces
/// `halp_pack::fingerprint` adds for display.
pub(crate) fn fp_hex(fp: &str) -> String {
    fp.chars().filter(|c| !c.is_whitespace()).collect()
}
