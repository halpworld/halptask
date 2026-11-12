//! The pack manifest, `halp-pack.json` (§3.2). Every struct refuses
//! unknown fields, and maps with author-chosen keys refuse duplicate keys.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::marker::PhantomData;
use std::sync::LazyLock;

use halp_domain::layout::validate_role_id;
use halp_domain::{Layout, TaskType, TaskTypeDisplay, Widget, limits};
use regex::Regex;
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::error::{PackError, Result};
use crate::ids::{self, check_path, is_sha256_hex};
use crate::migrate::{Migration, MigrationOp};
use crate::trust::{self, PackTrust};

/// The only manifest format this crate reads.
pub const MANIFEST_FORMAT: u32 = 1;
/// Most bytes in a manifest.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Most files in a pack.
pub const MAX_FILES: usize = 256;
/// Most bytes in one file.
pub const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;

const MAX_TYPES: usize = 32;
const MAX_OVERRIDES: usize = 32;
const MAX_MIGRATIONS: usize = 64;
const MAX_OPS: usize = 64;
const MAX_PROMPTS: usize = 32;
const MAX_MODULES: usize = 16;
const MAX_UI_WIDGETS: usize = 16;
const MAX_KINDS: usize = 64;
const MAX_LAYOUT_OPS: usize = 96;
const MAX_CALLS: usize = 16;
const MAX_INPUTS: usize = 16;

/// T0 commands a Tier B module may take as input (§5.1).
pub const MODULE_INPUT_COMMANDS: &[&str] = &[
    "tasks.list",
    "tasks.get",
    "types.list",
    "types.get",
    "day.state",
    "focus.get",
    "layout.get",
];
/// MCP tool names a Tier C widget may call (§6.3).
pub const UI_WIDGET_CALLS: &[&str] = &[
    "tasks_list",
    "tasks_get",
    "types_list",
    "types_get",
    "day_state",
    "focus_get",
];

static NAME: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9-]{0,31}$").ok());

fn check_name(what: &str, name: &str) -> Result<()> {
    if NAME.as_ref().is_some_and(|r| r.is_match(name)) {
        Ok(())
    } else {
        Err(PackError::invalid(format!(
            "{what} {name:?} must be 1-32 lowercase letters, digits or '-', starting with a letter"
        )))
    }
}

/// A `BTreeMap` that refuses a key given twice (serde_json keeps the last
/// one otherwise, so `files` could list a path twice unnoticed).
pub(crate) fn unique_map<'de, D, V>(d: D) -> std::result::Result<BTreeMap<String, V>, D::Error>
where
    D: Deserializer<'de>,
    V: Deserialize<'de>,
{
    struct V2<V>(PhantomData<V>);
    impl<'de, V: Deserialize<'de>> Visitor<'de> for V2<V> {
        type Value = BTreeMap<String, V>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("an object")
        }
        fn visit_map<A: MapAccess<'de>>(
            self,
            mut m: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut out = BTreeMap::new();
            while let Some((k, v)) = m.next_entry::<String, V>()? {
                if out.contains_key(&k) {
                    return Err(serde::de::Error::custom(format!("duplicate key {k:?}")));
                }
                out.insert(k, v);
            }
            Ok(out)
        }
    }
    d.deserialize_map(V2(PhantomData))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Publisher {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Extends {
    /// The parent's pack id (a built-in's package id such as
    /// `halp.developer`, or an installed pack).
    pub id: String,
    /// A semver requirement on the parent's version.
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDecl {
    pub id: String,
    pub title: String,
    pub accent: String,
}

/// Offered widget kinds: `parent ∪ add − remove`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetsDecl {
    #[serde(default)]
    pub add: Vec<String>,
    #[serde(default)]
    pub remove: Vec<String>,
}

/// The fields a `set` layout op may change. `config` is an RFC 7386 merge
/// patch onto the widget's config.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WidgetPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Value>,
}

/// One `layout_patch` op, applied in order to the parent's layout.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum LayoutOp {
    Set { id: String, patch: WidgetPatch },
    Remove { id: String },
    Add { widget: Widget },
}

/// An additive change to a parent type (§3.3). Nothing here can remove,
/// narrow or touch an `x-halp-*` key, so every item valid before stays
/// valid.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeOverride {
    /// The parent type's id.
    #[serde(rename = "type")]
    pub type_id: String,
    /// New optional properties (never added to `required`). Only allowed on
    /// a schema with `"additionalProperties": false` and no
    /// `patternProperties`, so no existing item can already hold the name.
    #[serde(
        default,
        deserialize_with = "unique_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub add_properties: BTreeMap<String, Value>,
    /// Values appended to a property's own `enum`.
    #[serde(
        default,
        deserialize_with = "unique_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub add_enum: BTreeMap<String, Vec<Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<TaskTypeDisplay>,
    /// A property's `description`, set or replaced.
    #[serde(
        default,
        deserialize_with = "unique_map",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub property_descriptions: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptsDecl {
    /// Prompt files, `prompts/<name>.md`; the name is the file stem.
    #[serde(default)]
    pub add: Vec<String>,
    /// Names of inherited prompts to drop.
    #[serde(default)]
    pub remove: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleInput {
    pub command: String,
    #[serde(default)]
    pub args: Value,
}

/// A Tier B transform (§5.1). Its id is `<pack_id>/<name>`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleDecl {
    pub name: String,
    /// Path of the module's `.wasm` file in the pack.
    pub wasm: String,
    #[serde(default, deserialize_with = "unique_map")]
    pub inputs: BTreeMap<String, ModuleInput>,
    pub output_schema: Value,
}

/// A Tier C widget (§6). Its kind is `x.<pack_id>.<name>`; it is served
/// from the files under `root`, starting at `root/entry`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiWidgetDecl {
    pub name: String,
    pub title: String,
    /// A directory in the pack (no trailing `/`).
    pub root: String,
    #[serde(default = "default_entry")]
    pub entry: String,
    /// MCP tool names the widget may call (within [`UI_WIDGET_CALLS`]).
    #[serde(default)]
    pub calls: Vec<String>,
}

fn default_entry() -> String {
    "index.html".into()
}

impl UiWidgetDecl {
    /// The widget kind, `x.<pack_id>.<name>`.
    pub fn kind(&self, pack_id: &str) -> String {
        format!("x.{pack_id}.{}", self.name)
    }
}

/// A file's entry in the manifest's index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEntry {
    pub sha256: String,
    pub size: u64,
}

/// Path to entry; every file in the pack, exactly once.
pub type FileIndex = BTreeMap<String, FileEntry>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub id: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub license: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub publisher: Publisher,
    /// A semver requirement on the Hub version, e.g. `0.x`.
    pub min_hub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<Extends>,
    pub role: RoleDecl,
    #[serde(default)]
    pub widgets: WidgetsDecl,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout_patch: Option<Vec<LayoutOp>>,
    #[serde(default)]
    pub task_types: Vec<TaskType>,
    #[serde(default)]
    pub type_overrides: Vec<TypeOverride>,
    #[serde(default)]
    pub migrations: Vec<Migration>,
    #[serde(default)]
    pub prompts: PromptsDecl,
    #[serde(default)]
    pub modules: Vec<ModuleDecl>,
    #[serde(default)]
    pub ui_widgets: Vec<UiWidgetDecl>,
    #[serde(default, deserialize_with = "unique_map")]
    pub files: FileIndex,
}

/// The prompt name of a `prompts/<name>.md` path.
pub fn prompt_name(path: &str) -> Option<&str> {
    path.strip_prefix("prompts/")?.strip_suffix(".md")
}

impl Manifest {
    /// Parses and validates manifest text for a pack of `trust`. Returns
    /// the manifest and warnings (privileged annotations ignored in a dev
    /// pack).
    pub fn parse(text: &str, trust: PackTrust) -> Result<(Manifest, Vec<String>)> {
        if text.len() > MAX_MANIFEST_BYTES {
            return Err(PackError::invalid(format!(
                "manifest exceeds {MAX_MANIFEST_BYTES} bytes"
            )));
        }
        let raw: Value = serde_json::from_str(text)
            .map_err(|e| PackError::invalid(format!("manifest is not JSON: {e}")))?;
        limits::check_json_text("manifest", &raw)?;
        let m: Manifest = serde_json::from_str(text)
            .map_err(|e| PackError::invalid(format!("manifest is malformed: {e}")))?;
        let warnings = m.validate(trust)?;
        Ok((m, warnings))
    }

    /// The parsed version.
    pub fn semver(&self) -> Result<semver::Version> {
        semver::Version::parse(&self.version).map_err(|e| {
            PackError::invalid(format!("pack {}: version {:?}: {e}", self.id, self.version))
        })
    }

    /// Whether this pack accepts Hub version `hub`.
    pub fn supports_hub(&self, hub: &semver::Version) -> Result<bool> {
        Ok(req(&self.min_hub, "min_hub")?.matches(hub))
    }

    /// The namespace types and data classes must use.
    pub fn namespace(&self, trust: PackTrust) -> &str {
        ids::namespace(&self.id, trust)
    }

    /// Prompt names this pack adds, with their paths.
    pub fn prompt_paths(&self) -> Vec<(&str, &str)> {
        self.prompts
            .add
            .iter()
            .filter_map(|p| prompt_name(p).map(|n| (n, p.as_str())))
            .collect()
    }

    /// Checks everything that does not need the parent: shape, limits,
    /// names, namespaces, references into `files`, and the trust rules.
    pub fn validate(&self, trust: PackTrust) -> Result<Vec<String>> {
        let id = &self.id;
        if self.format != MANIFEST_FORMAT {
            return Err(PackError::invalid(format!(
                "pack {id}: manifest format {} is not supported (expected {MANIFEST_FORMAT})",
                self.format
            )));
        }
        ids::check_pack_identity(id, trust)?;
        self.semver()?;
        req(&self.min_hub, "min_hub")?;
        nonempty_label("pack title", &self.title, 80)?;
        limits::label("pack description", &self.description, 2000)?;
        nonempty_label("pack license", &self.license, 100)?;
        nonempty_label("publisher name", &self.publisher.name, 80)?;
        if let Some(h) = &self.homepage {
            limits::label("homepage", h, 2048)?;
            if !h.starts_with("https://") || h.contains(char::is_whitespace) {
                return Err(PackError::invalid("homepage must be an https:// URL"));
            }
        }
        if let Some(e) = &self.extends {
            ids::check_pack_id(e.id.strip_prefix(ids::DEV_PREFIX).unwrap_or(&e.id))?;
            req(&e.version, "extends.version")?;
            if e.id == self.id {
                return Err(PackError::invalid(format!("pack {id} extends itself")));
            }
        }
        // Role.
        validate_role_id(&self.role.id)?;
        if ids::BUILTIN_ROLE_IDS.contains(&self.role.id.as_str()) {
            return Err(PackError::conflict(format!(
                "pack {id}: role id {} belongs to a built-in pack",
                self.role.id
            )));
        }
        nonempty_label("role title", &self.role.title, 80)?;
        let accent_ok = self.role.accent.len() == 7
            && self.role.accent.starts_with('#')
            && self.role.accent[1..].bytes().all(|b| b.is_ascii_hexdigit());
        if !accent_ok {
            return Err(PackError::invalid("role accent must be #rrggbb"));
        }
        // Widgets and layout.
        if self.widgets.add.len() + self.widgets.remove.len() > MAX_KINDS {
            return Err(PackError::invalid(format!(
                "pack {id}: too many widget kinds"
            )));
        }
        for k in self.widgets.add.iter().chain(&self.widgets.remove) {
            limits::simple_id("widget kind", k)?;
        }
        match (&self.layout, &self.layout_patch, &self.extends) {
            (Some(_), Some(_), _) => {
                return Err(PackError::invalid(format!(
                    "pack {id}: give either layout or layout_patch, not both"
                )));
            }
            (None, _, None) => {
                return Err(PackError::invalid(format!(
                    "pack {id}: a pack that does not extend another needs a layout"
                )));
            }
            (None, Some(ops), Some(_)) if ops.len() > MAX_LAYOUT_OPS => {
                return Err(PackError::invalid(format!(
                    "pack {id}: too many layout_patch ops"
                )));
            }
            _ => {}
        }
        if let Some(l) = &self.layout
            && l.role_id != self.role.id
        {
            return Err(PackError::invalid(format!(
                "pack {id}: layout belongs to role {}, not {}",
                l.role_id, self.role.id
            )));
        }
        // Types.
        let ns = self.namespace(trust).to_string();
        if self.task_types.len() > MAX_TYPES {
            return Err(PackError::invalid(format!(
                "pack {id}: at most {MAX_TYPES} task types"
            )));
        }
        let mut own = BTreeSet::new();
        for t in &self.task_types {
            t.validate()?;
            if !t.id.starts_with(&format!("{ns}.")) {
                return Err(PackError::invalid(format!(
                    "pack {id}: type {} must be in the pack's namespace {ns}.",
                    t.id
                )));
            }
            if !own.insert(t.id.as_str()) {
                return Err(PackError::invalid(format!(
                    "pack {id}: type {} twice",
                    t.id
                )));
            }
            limits::check_json_text(&format!("pack type {} schema", t.id), &t.schema)?;
            limits::check_json_text(
                &format!("pack type {}", t.id),
                &serde_json::json!([t.title, t.icon, t.display]),
            )?;
            check_data_class(t, &ns)?;
        }
        if self.type_overrides.len() > MAX_OVERRIDES {
            return Err(PackError::invalid(format!(
                "pack {id}: too many type_overrides"
            )));
        }
        if !self.type_overrides.is_empty() && self.extends.is_none() {
            return Err(PackError::invalid(format!(
                "pack {id}: type_overrides need a parent (extends)"
            )));
        }
        let mut overridden = BTreeSet::new();
        for o in &self.type_overrides {
            if own.contains(o.type_id.as_str()) {
                return Err(PackError::invalid(format!(
                    "pack {id}: type {} is the pack's own; change it directly",
                    o.type_id
                )));
            }
            if !overridden.insert(o.type_id.as_str()) {
                return Err(PackError::invalid(format!(
                    "pack {id}: type {} is overridden twice",
                    o.type_id
                )));
            }
            crate::overrides::check_override_shape(o)?;
        }
        // Migrations: the pack's own types only.
        if self.migrations.len() > MAX_MIGRATIONS {
            return Err(PackError::invalid(format!(
                "pack {id}: too many migrations"
            )));
        }
        for mg in &self.migrations {
            if !own.contains(mg.type_id.as_str()) {
                return Err(PackError::invalid(format!(
                    "pack {id}: migration for {}, which is not one of the pack's types",
                    mg.type_id
                )));
            }
            req(&mg.from, "migration from")?;
            if mg.ops.is_empty() || mg.ops.len() > MAX_OPS {
                return Err(PackError::invalid(format!(
                    "pack {id}: a migration needs 1 to {MAX_OPS} ops"
                )));
            }
            for op in &mg.ops {
                op.validate()?;
                if let MigrationOp::Retype { to } = op
                    && (!own.contains(to.as_str()) || *to == mg.type_id)
                {
                    return Err(PackError::invalid(format!(
                        "pack {id}: retype to {to}, which is not another type of this pack"
                    )));
                }
            }
        }
        crate::migrate::check_retype_classes(self)?;
        // Files.
        if self.files.len() > MAX_FILES {
            return Err(PackError::invalid(format!(
                "pack {id}: at most {MAX_FILES} files"
            )));
        }
        for (p, f) in &self.files {
            check_path(p)?;
            if !is_sha256_hex(&f.sha256) {
                return Err(PackError::invalid(format!(
                    "file {p}: sha256 must be 64 lowercase hex characters"
                )));
            }
            if f.size > MAX_FILE_BYTES {
                return Err(PackError::invalid(format!(
                    "file {p} exceeds {MAX_FILE_BYTES} bytes"
                )));
            }
        }
        // Prompts.
        if self.prompts.add.len() + self.prompts.remove.len() > MAX_PROMPTS {
            return Err(PackError::invalid(format!("pack {id}: too many prompts")));
        }
        let mut names = BTreeSet::new();
        for p in &self.prompts.add {
            let name = prompt_name(p).ok_or_else(|| {
                PackError::invalid(format!("prompt {p:?} must be a file prompts/<name>.md"))
            })?;
            check_name("prompt name", name)?;
            self.listed(p)?;
            if !names.insert(name) {
                return Err(PackError::invalid(format!(
                    "pack {id}: prompt {name} twice"
                )));
            }
        }
        for n in &self.prompts.remove {
            check_name("prompt name", n)?;
        }
        // Tier B modules.
        if self.modules.len() > MAX_MODULES {
            return Err(PackError::invalid(format!("pack {id}: too many modules")));
        }
        let mut names = BTreeSet::new();
        for m in &self.modules {
            check_name("module name", &m.name)?;
            if !names.insert(m.name.as_str()) {
                return Err(PackError::invalid(format!(
                    "pack {id}: module {} twice",
                    m.name
                )));
            }
            if !m.wasm.ends_with(".wasm") {
                return Err(PackError::invalid(format!(
                    "module {}: wasm must be a .wasm file",
                    m.name
                )));
            }
            self.listed(&m.wasm)?;
            if m.inputs.len() > MAX_INPUTS {
                return Err(PackError::invalid(format!(
                    "module {}: too many inputs",
                    m.name
                )));
            }
            for (k, input) in &m.inputs {
                check_name("module input", k)?;
                if !MODULE_INPUT_COMMANDS.contains(&input.command.as_str()) {
                    return Err(PackError::invalid(format!(
                        "module {}: input {k} uses {}, which modules may not call",
                        m.name, input.command
                    )));
                }
                if !(input.args.is_null() || input.args.is_object()) {
                    return Err(PackError::invalid(format!(
                        "module {}: input {k} args must be an object",
                        m.name
                    )));
                }
            }
            if !m.output_schema.is_object() {
                return Err(PackError::invalid(format!(
                    "module {}: output_schema must be a JSON object",
                    m.name
                )));
            }
            limits::json_bounds(
                "module output_schema",
                &m.output_schema,
                limits::MAX_SCHEMA_BYTES,
                limits::MAX_SCHEMA_DEPTH,
            )?;
        }
        // Tier C widgets.
        if self.ui_widgets.len() > MAX_UI_WIDGETS {
            return Err(PackError::invalid(format!(
                "pack {id}: too many ui_widgets"
            )));
        }
        let mut names = BTreeSet::new();
        for w in &self.ui_widgets {
            check_name("ui widget name", &w.name)?;
            if !names.insert(w.name.as_str()) {
                return Err(PackError::invalid(format!(
                    "pack {id}: ui widget {} twice",
                    w.name
                )));
            }
            let kind = w.kind(id);
            limits::simple_id("ui widget kind", &kind)?;
            nonempty_label("ui widget title", &w.title, 80)?;
            check_path(&w.root)?;
            if !w.entry.ends_with(".html") {
                return Err(PackError::invalid(format!(
                    "ui widget {}: entry must be an .html file",
                    w.name
                )));
            }
            let entry = format!("{}/{}", w.root, w.entry);
            check_path(&entry)?;
            self.listed(&entry)?;
            if w.calls.len() > MAX_CALLS {
                return Err(PackError::invalid(format!(
                    "ui widget {}: too many calls",
                    w.name
                )));
            }
            for c in &w.calls {
                if !UI_WIDGET_CALLS.contains(&c.as_str()) {
                    return Err(PackError::invalid(format!(
                        "ui widget {}: call {c} is not allowed for widgets",
                        w.name
                    )));
                }
            }
        }
        trust::check_trust(self, trust)
    }

    fn listed(&self, path: &str) -> Result<()> {
        if self.files.contains_key(path) {
            Ok(())
        } else {
            Err(PackError::invalid(format!(
                "pack {}: {path} is not listed in files",
                self.id
            )))
        }
    }
}

fn nonempty_label(what: &str, raw: &str, max: usize) -> Result<()> {
    if limits::label(what, raw, max)?.is_empty() {
        return Err(PackError::invalid(format!("{what} must not be empty")));
    }
    Ok(())
}

pub(crate) fn req(raw: &str, what: &str) -> Result<semver::VersionReq> {
    semver::VersionReq::parse(raw)
        .map_err(|e| PackError::invalid(format!("{what} {raw:?} is not a semver requirement: {e}")))
}

/// `x-halp-data-class` must name a built-in class or one in the pack's
/// namespace (§2 note 1).
fn check_data_class(t: &TaskType, ns: &str) -> Result<()> {
    let Some(v) = t.schema.get("x-halp-data-class") else {
        return Ok(());
    };
    let c = v.as_str().unwrap_or("");
    let own = c
        .strip_prefix(ns)
        .and_then(|r| r.strip_prefix('.'))
        .is_some_and(|r| !r.is_empty());
    if (halp_policy::is_data_class(c) || own) && limits::simple_id("data class", c).is_ok() {
        Ok(())
    } else {
        Err(PackError::invalid(format!(
            "type {}: x-halp-data-class {c:?} must be a built-in class or start with {ns}.",
            t.id
        )))
    }
}
