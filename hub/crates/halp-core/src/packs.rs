//! Built-in role packs (`hub/packs/<role>/pack.json`), embedded at build time
//! and validated on load.

use halp_domain::task_type::builtin_types;
use halp_domain::{DomainError, Layout, Role, TaskType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub id: String,
    pub title: String,
    pub accent: String,
    /// Widget kinds the role offers.
    pub widgets: Vec<String>,
    /// Task types the pack defines (in addition to the built-in base types).
    #[serde(default)]
    pub task_types: Vec<TaskType>,
    /// The role's default layout.
    pub layout: Layout,
    /// Ids of types the role inherits from the packs it extends (set by
    /// the resolver for installed packs; never in a pack file).
    #[serde(skip)]
    pub inherited: Vec<String>,
}

const BUILTIN: &[(&str, &str)] = &[
    (
        "developer",
        include_str!("../../../packs/developer/pack.json"),
    ),
    (
        "eng-manager",
        include_str!("../../../packs/eng-manager/pack.json"),
    ),
    ("cso", include_str!("../../../packs/cso/pack.json")),
];

pub const DEFAULT_ROLE: &str = "developer";

impl Pack {
    pub fn parse(raw: &str) -> Result<Pack, DomainError> {
        if raw.len() > 256 * 1024 {
            return Err(DomainError::invalid("pack file is too large"));
        }
        let p: Pack = serde_json::from_str(raw)
            .map_err(|e| DomainError::invalid(format!("pack is malformed: {e}")))?;
        p.validate()?;
        Ok(p)
    }

    pub fn validate(&self) -> Result<(), DomainError> {
        self.role().validate()?;
        self.layout.validate()?;
        // Pack text reaches the UI and agents like user text does: no
        // controls or invisible format characters in schemas or layouts.
        let layout = serde_json::to_value(&self.layout)
            .map_err(|e| DomainError::invalid(format!("pack {}: layout: {e}", self.id)))?;
        halp_domain::limits::check_json_text(&format!("pack {} layout", self.id), &layout)?;
        for t in &self.task_types {
            halp_domain::limits::check_json_text(&format!("pack type {} schema", t.id), &t.schema)?;
            halp_domain::limits::check_json_text(
                &format!("pack type {}", t.id),
                &serde_json::json!([t.title, t.icon, t.display]),
            )?;
        }
        if self.layout.role_id != self.id {
            return Err(DomainError::invalid(format!(
                "pack {}: layout belongs to role {}",
                self.id, self.layout.role_id
            )));
        }
        for w in &self.layout.widgets {
            if !self.widgets.contains(&w.kind) {
                return Err(DomainError::invalid(format!(
                    "pack {}: layout uses widget kind {} that the role does not offer",
                    self.id, w.kind
                )));
            }
        }
        for t in &self.task_types {
            t.validate()?;
            crate::state::check_type_class(t)?;
            crate::immutable::check_type(t)?;
            if t.id.starts_with("base.") {
                return Err(DomainError::invalid(format!(
                    "pack {}: type {} uses the reserved base namespace",
                    self.id, t.id
                )));
            }
        }
        Ok(())
    }

    pub fn role(&self) -> Role {
        let mut task_types: Vec<String> = builtin_types().into_iter().map(|t| t.id).collect();
        task_types.extend(self.task_types.iter().map(|t| t.id.clone()));
        for t in &self.inherited {
            if !task_types.contains(t) {
                task_types.push(t.clone());
            }
        }
        Role {
            id: self.id.clone(),
            title: self.title.clone(),
            accent: self.accent.clone(),
            layout_id: self.layout.id.clone(),
            task_types,
            widgets: self.widgets.clone(),
        }
    }
}

/// The embedded packs, validated.
pub fn builtin_packs() -> Result<Vec<Pack>, DomainError> {
    BUILTIN
        .iter()
        .map(|(dir, raw)| {
            let p = Pack::parse(raw)?;
            if p.id != *dir {
                return Err(DomainError::invalid(format!(
                    "pack in {dir}/ declares id {}",
                    p.id
                )));
            }
            Ok(p)
        })
        .collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    /// Checks what must hold for any pack, not a copy of today's lists:
    /// every embedded pack loads, its layout only uses widget kinds the
    /// role offers, ids are unique, every type schema compiles and the
    /// types load together with the built-ins.
    #[test]
    fn builtin_packs_are_valid() {
        use std::collections::BTreeSet;

        use halp_domain::task_type::{TaskTypeOrigin, TaskTypeRegistry};

        let packs = builtin_packs().unwrap();
        assert_eq!(packs.len(), BUILTIN.len());
        let ids: BTreeSet<_> = packs.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.len(), packs.len(), "pack ids are unique");
        let mut registry = TaskTypeRegistry::with_builtins().unwrap();
        for p in &packs {
            assert!(!p.layout.widgets.is_empty(), "{}: empty layout", p.id);
            let offered: BTreeSet<_> = p.widgets.iter().collect();
            assert_eq!(offered.len(), p.widgets.len(), "{}: duplicate kinds", p.id);
            let mut widget_ids = BTreeSet::new();
            for w in &p.layout.widgets {
                assert!(
                    offered.contains(&w.kind),
                    "{}: {} not offered",
                    p.id,
                    w.kind
                );
                assert!(
                    widget_ids.insert(&w.id),
                    "{}: widget id {} twice",
                    p.id,
                    w.id
                );
            }
            for t in &p.task_types {
                t.validate().unwrap();
                if let Some(class) = t.schema.get("x-halp-data-class") {
                    assert!(
                        class.as_str().is_some_and(|c| !c.is_empty()),
                        "{}: data class must be a non-empty string",
                        t.id
                    );
                }
                registry
                    .insert(
                        t.clone(),
                        TaskTypeOrigin::Pack {
                            trust: halp_domain::PackTrust::Builtin,
                            pack: format!("halp.{}", p.id),
                        },
                    )
                    .unwrap_or_else(|e| panic!("{}: {e:?}", t.id));
            }
        }
    }

    #[test]
    fn bad_packs_rejected() {
        let good = BUILTIN[0].1;
        let mut v: serde_json::Value = serde_json::from_str(good).unwrap();
        v["layout"]["widgets"][0]["kind"] = "unknown-widget".into();
        assert!(Pack::parse(&v.to_string()).is_err());
        let mut v: serde_json::Value = serde_json::from_str(good).unwrap();
        v["layout"]["widgets"][1]["x"] = 0.into();
        assert!(Pack::parse(&v.to_string()).is_err(), "overlap");
        let mut v: serde_json::Value = serde_json::from_str(good).unwrap();
        v["accent"] = "red".into();
        assert!(Pack::parse(&v.to_string()).is_err());
        let mut v: serde_json::Value = serde_json::from_str(good).unwrap();
        v["surprise"] = true.into();
        assert!(Pack::parse(&v.to_string()).is_err());
        // Hidden characters in pack text.
        let mut v: serde_json::Value = serde_json::from_str(good).unwrap();
        v["layout"]["widgets"][0]["config"] = serde_json::json!({"note": "a\u{202E}b"});
        assert!(
            Pack::parse(&v.to_string()).is_err(),
            "bidi in widget config"
        );
        let mut v: serde_json::Value = serde_json::from_str(good).unwrap();
        if let Some(t) = v["task_types"].get_mut(0) {
            t["schema"]["description"] = "x\u{E0041}".into();
            assert!(Pack::parse(&v.to_string()).is_err(), "tag char in schema");
        }
        assert!(Pack::parse("not json").is_err());
    }
}

/// A Tier C file served to the sandboxed UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtFile {
    pub bytes: Vec<u8>,
    pub mime: &'static str,
    pub sha256: String,
}

/// A pack's UI widget as the desktop shell needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtUiWidget {
    /// `x.<pack>.<name>`.
    pub kind: String,
    /// The commands the widget may call (its manifest allow-list).
    pub calls: Vec<String>,
    /// The entry document's full path in the manifest's file index
    /// (`<root>/<entry>`).
    pub entry: String,
    pub trust: halp_pack::PackTrust,
}

/// The loaded, enabled pack's UI widget declaration, when its trust lets
/// it run code.
fn ui_decl<'a>(
    st: &'a crate::state::State,
    pack_id: &str,
    widget: &str,
) -> Option<(
    &'a crate::installed::LoadedPack,
    &'a halp_pack::manifest::UiWidgetDecl,
)> {
    let m = st.pack_cache.meta.get(pack_id)?;
    if !m.is_live() || !m.enabled || !m.trust.may_run_code() {
        return None;
    }
    let d = m
        .manifest
        .as_deref()?
        .ui_widgets
        .iter()
        .find(|w| w.name == widget)?;
    Some((m, d))
}

/// A file of a Tier C widget: `path` is its full path in the pack's file
/// index and must lie under the widget's `root`. The bytes are the ones
/// verified at load, hashed again before they are served.
pub(crate) fn ext_ui_file(
    st: &crate::state::State,
    pack_id: &str,
    widget: &str,
    path: &str,
) -> Option<ExtFile> {
    let (m, d) = ui_decl(st, pack_id, widget)?;
    let under = path
        .strip_prefix(d.root.as_str())
        .and_then(|r| r.strip_prefix('/'))
        .is_some_and(|r| !r.is_empty());
    if !under || halp_pack::ids::check_path(path).is_err() {
        return None;
    }
    let entry = m.file_index.get(path)?;
    let mime = crate::installed::ui_mime(path)?;
    let bytes = m.files.get(path)?;
    let sha256 = halp_pack::ids::sha256_hex(bytes);
    if sha256 != entry.sha256 {
        return None;
    }
    Some(ExtFile {
        bytes: bytes.clone(),
        mime,
        sha256,
    })
}

/// A Tier C widget of a loaded, enabled pack that may run code. `entry` is
/// the entry document's full path in the file index (`<root>/<entry>`).
pub(crate) fn ext_ui_widget(
    st: &crate::state::State,
    pack_id: &str,
    widget: &str,
) -> Option<ExtUiWidget> {
    let (m, d) = ui_decl(st, pack_id, widget)?;
    Some(ExtUiWidget {
        kind: d.kind(pack_id),
        calls: d.calls.clone(),
        entry: format!("{}/{}", d.root, d.entry),
        trust: m.trust,
    })
}

/// Longest prompt file served over MCP.
pub const MAX_PROMPT_BYTES: usize = 32 * 1024;

/// A pack prompt as the MCP server serves it. `text` is untrusted: it is
/// the pack author's file, verified and cleaned of control and invisible
/// format characters, and a client must label it by `pack_id` and `trust`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackPrompt {
    /// The prompt name (the file stem of `prompts/<name>.md`).
    pub name: String,
    /// The pack whose file it is (an ancestor's prompt keeps its owner).
    pub pack_id: String,
    /// The trust of the pack that owns the file, never the child's.
    pub trust: halp_pack::PackTrust,
    pub text: String,
}

/// Prompts of enabled, loaded packs for MCP (docs/hub/P5-EXTENSIONS.md §7).
/// Only first-party and community owners count: unsigned and dev packs
/// never serve prompts (an agent could have written them), and the
/// compiled-in prompts of built-in packs are the MCP server's own. Each
/// file is hashed again against the manifest's file index; a file that
/// does not match, is not UTF-8, is over [`MAX_PROMPT_BYTES`] or carries
/// control or invisible format characters is left out.
pub(crate) fn pack_prompts(st: &crate::state::State) -> Vec<PackPrompt> {
    use halp_pack::PackTrust;
    let mut out: Vec<PackPrompt> = Vec::new();
    for m in st.pack_cache.meta.values() {
        if !m.is_live() || !m.enabled {
            continue;
        }
        for r in &m.prompts {
            let Some(owner) = st.pack_cache.meta.get(&r.pack_id) else {
                continue;
            };
            if !owner.is_live()
                || !matches!(owner.trust, PackTrust::FirstParty | PackTrust::Community)
            {
                continue;
            }
            if out
                .iter()
                .any(|p| p.pack_id == r.pack_id && p.name == r.name)
            {
                continue;
            }
            if halp_pack::manifest::prompt_name(&r.path) != Some(r.name.as_str())
                || halp_pack::ids::check_path(&r.path).is_err()
            {
                continue;
            }
            let (Some(entry), Some(bytes)) =
                (owner.file_index.get(&r.path), owner.files.get(&r.path))
            else {
                continue;
            };
            if bytes.len() > MAX_PROMPT_BYTES || halp_pack::ids::sha256_hex(bytes) != entry.sha256 {
                continue;
            }
            let Ok(text) = String::from_utf8(bytes.clone()) else {
                continue;
            };
            if halp_domain::limits::check_json_text(
                "prompt",
                &serde_json::Value::String(text.clone()),
            )
            .is_err()
            {
                continue;
            }
            out.push(PackPrompt {
                name: r.name.clone(),
                pack_id: r.pack_id.clone(),
                trust: owner.trust,
                text,
            });
        }
    }
    out.sort_by(|a, b| (&a.pack_id, &a.name).cmp(&(&b.pack_id, &b.name)));
    out
}

/// A Tier B module (§5) as the extension host needs it: the declaration
/// from the installed manifest and the verified wasm bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtModule {
    pub pack_id: String,
    /// The manifest's module declaration (name, inputs, output schema).
    pub decl: halp_pack::manifest::ModuleDecl,
    /// The `.wasm` bytes, hashed again against the file index just now.
    pub wasm: Vec<u8>,
    pub trust: halp_pack::PackTrust,
    /// Untrusted text from the manifest (for the widget chrome).
    pub title: String,
    /// Untrusted text from the manifest (for the widget chrome).
    pub publisher: String,
}

/// Module `name` of pack `pack_id`, when the pack is loaded, enabled and
/// trusted to run code (§2: first party, community or dev; never unsigned
/// or built-in) and its wasm still matches the hash in its file index.
pub(crate) fn ext_module(st: &crate::state::State, pack_id: &str, name: &str) -> Option<ExtModule> {
    let m = st.pack_cache.meta.get(pack_id)?;
    if !m.is_live() || !m.enabled || !m.trust.may_run_code() {
        return None;
    }
    let manifest = m.manifest.as_deref()?;
    let d = manifest.modules.iter().find(|d| d.name == name)?;
    if halp_pack::ids::check_path(&d.wasm).is_err() {
        return None;
    }
    let entry = m.file_index.get(&d.wasm)?;
    let bytes = m.files.get(&d.wasm)?;
    if halp_pack::ids::sha256_hex(bytes) != entry.sha256 {
        return None;
    }
    Some(ExtModule {
        pack_id: m.id.clone(),
        decl: d.clone(),
        wasm: bytes.clone(),
        trust: m.trust,
        title: m.title.clone(),
        publisher: manifest.publisher.name.clone(),
    })
}
