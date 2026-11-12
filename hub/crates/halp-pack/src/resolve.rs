//! The inheritance resolver (§3.3): a chain of packs, root first, becomes
//! one effective pack in the shape of halp-core's `Pack`.

use std::collections::{BTreeMap, BTreeSet};

use halp_domain::task_type::builtin_types;
use halp_domain::{Layout, Role, TaskType, limits};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{PackError, Result};
use crate::manifest::{LayoutOp, Manifest, req};
use crate::overrides::apply_override;
use crate::trust::PackTrust;

/// Most packs in a chain, the root (built-in or not) and the child
/// included.
pub const MAX_CHAIN: usize = 4;

/// The effective pack. **Mirror of `halp_core::packs::Pack`** (same
/// fields, same serde shape); this crate does not depend on halp-core, so
/// P5-B maps it field by field (or round-trips it through JSON) and then
/// runs `Pack::validate` on it. `id` is the role id, as in halp-core.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectivePack {
    pub id: String,
    pub title: String,
    pub accent: String,
    /// Widget kinds the role offers.
    pub widgets: Vec<String>,
    /// Types the pack itself defines (inherited types are referenced, not
    /// copied: see [`Resolved::inherited_type_ids`]).
    #[serde(default)]
    pub task_types: Vec<TaskType>,
    pub layout: Layout,
}

/// A built-in pack as a chain root.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltinBase {
    /// Its package id, e.g. `halp.developer`.
    pub package_id: String,
    /// The version `extends.version` is matched against (P5-B: the Hub's
    /// version).
    pub version: semver::Version,
    pub pack: EffectivePack,
}

/// An installed (or dev) pack in a chain.
#[derive(Debug, Clone, PartialEq)]
pub struct ChainLink {
    pub manifest: Manifest,
    pub trust: PackTrust,
}

/// A chain, root first; the last link is the pack being resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Chain {
    pub base: Option<BuiltinBase>,
    pub links: Vec<ChainLink>,
}

/// What a parent id refers to, for [`collect_chain`].
#[derive(Debug, Clone, PartialEq)]
pub enum Parent {
    Builtin(Box<BuiltinBase>),
    Installed(Box<ChainLink>),
}

/// A prompt of the effective role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptRef {
    pub name: String,
    /// The pack whose file it is.
    pub pack_id: String,
    /// The file's path in that pack.
    pub path: String,
}

/// The resolver's output.
#[derive(Debug, Clone, PartialEq)]
pub struct Resolved {
    pub pack: EffectivePack,
    pub package_id: String,
    pub version: semver::Version,
    /// The child's own trust; a parent's trust is never raised or lowered.
    pub trust: PackTrust,
    /// Package ids, root first, ending with the child.
    pub chain: Vec<String>,
    /// Ids of the ancestors' types, which the role also offers.
    pub inherited_type_ids: Vec<String>,
    /// Ancestor types after the child's `type_overrides` (the new global
    /// definitions while the child is enabled).
    pub overridden_types: Vec<TaskType>,
    /// The role's pack prompts, by name.
    pub prompts: Vec<PromptRef>,
    /// Names removed that no pack in the chain defines (a built-in
    /// root's compiled-in prompts).
    pub removed_base_prompts: Vec<String>,
}

/// Walks `extends` from `child` up to a root, refusing cycles, missing
/// parents and chains longer than [`MAX_CHAIN`].
pub fn collect_chain(child: ChainLink, lookup: impl Fn(&str) -> Option<Parent>) -> Result<Chain> {
    let mut seen = BTreeSet::from([child.manifest.id.clone()]);
    let mut links = vec![child];
    let mut base = None;
    loop {
        let next = links
            .last()
            .and_then(|l| l.manifest.extends.as_ref())
            .map(|e| e.id.clone());
        let Some(pid) = next else { break };
        if !seen.insert(pid.clone()) {
            return Err(PackError::invalid(format!(
                "pack inheritance has a cycle through {pid}"
            )));
        }
        if links.len() >= MAX_CHAIN {
            return Err(PackError::invalid(format!(
                "pack inheritance is deeper than {MAX_CHAIN} packs"
            )));
        }
        match lookup(&pid) {
            Some(Parent::Builtin(b)) => {
                base = Some(*b);
                break;
            }
            Some(Parent::Installed(l)) => links.push(*l),
            None => {
                return Err(PackError::not_found(format!(
                    "parent pack {pid} is not installed"
                )));
            }
        }
    }
    links.reverse();
    Ok(Chain { base, links })
}

/// RFC 7386 JSON merge patch.
pub fn merge_patch(target: &mut Value, patch: &Value) {
    match patch {
        Value::Object(p) => {
            if !target.is_object() {
                *target = Value::Object(Map::new());
            }
            if let Value::Object(t) = target {
                for (k, v) in p {
                    if v.is_null() {
                        t.remove(k);
                    } else {
                        merge_patch(t.entry(k.clone()).or_insert(Value::Null), v);
                    }
                }
            }
        }
        _ => *target = patch.clone(),
    }
}

fn apply_layout_ops(layout: &mut Layout, ops: &[LayoutOp]) -> Result<()> {
    for op in ops {
        match op {
            LayoutOp::Set { id, patch } => {
                let w = layout
                    .widgets
                    .iter_mut()
                    .find(|w| w.id == *id)
                    .ok_or_else(|| {
                        PackError::invalid(format!("layout_patch: no widget {id} to set"))
                    })?;
                if let Some(v) = patch.x {
                    w.x = v;
                }
                if let Some(v) = patch.y {
                    w.y = v;
                }
                if let Some(v) = patch.w {
                    w.w = v;
                }
                if let Some(v) = patch.h {
                    w.h = v;
                }
                if let Some(v) = &patch.title {
                    w.title = v.clone();
                }
                if let Some(c) = &patch.config {
                    if !c.is_object() {
                        return Err(PackError::invalid(format!(
                            "layout_patch: config of {id} must be an object (a merge patch)"
                        )));
                    }
                    let mut cur = Value::Object(std::mem::take(&mut w.config));
                    merge_patch(&mut cur, c);
                    if let Value::Object(m) = cur {
                        w.config = m;
                    }
                }
            }
            LayoutOp::Remove { id } => {
                let before = layout.widgets.len();
                layout.widgets.retain(|w| w.id != *id);
                if layout.widgets.len() == before {
                    return Err(PackError::invalid(format!(
                        "layout_patch: no widget {id} to remove"
                    )));
                }
            }
            LayoutOp::Add { widget } => {
                if layout.widget(&widget.id).is_some() {
                    return Err(PackError::invalid(format!(
                        "layout_patch: widget {} already exists",
                        widget.id
                    )));
                }
                layout.widgets.push(widget.clone());
            }
        }
    }
    Ok(())
}

/// Resolves a chain into the child's effective pack and validates it.
pub fn resolve(chain: &Chain) -> Result<Resolved> {
    let child = chain
        .links
        .last()
        .ok_or_else(|| PackError::invalid("an empty pack chain"))?;
    let total = chain.links.len() + usize::from(chain.base.is_some());
    if total > MAX_CHAIN {
        return Err(PackError::invalid(format!(
            "pack inheritance is deeper than {MAX_CHAIN} packs"
        )));
    }
    // The links must form the chain they claim.
    let mut ids: Vec<String> = Vec::new();
    let mut prev: Option<(String, semver::Version)> = chain
        .base
        .as_ref()
        .map(|b| (b.package_id.clone(), b.version.clone()));
    if let Some((id, _)) = &prev {
        ids.push(id.clone());
    }
    for l in &chain.links {
        let m = &l.manifest;
        if ids.contains(&m.id) {
            return Err(PackError::invalid(format!(
                "pack inheritance has a cycle through {}",
                m.id
            )));
        }
        match (&m.extends, &prev) {
            (None, None) => {}
            (Some(e), Some((pid, pver))) => {
                if e.id != *pid {
                    return Err(PackError::invalid(format!(
                        "pack {} extends {}, not {pid}",
                        m.id, e.id
                    )));
                }
                if !req(&e.version, "extends.version")?.matches(pver) {
                    return Err(PackError::conflict(format!(
                        "pack {} needs {} {}, but {pver} is installed",
                        m.id, e.id, e.version
                    )));
                }
            }
            (Some(e), None) => {
                return Err(PackError::not_found(format!(
                    "parent pack {} of {} is not installed",
                    e.id, m.id
                )));
            }
            (None, Some((pid, _))) => {
                return Err(PackError::invalid(format!(
                    "pack {} does not extend {pid}",
                    m.id
                )));
            }
        }
        prev = Some((m.id.clone(), m.semver()?));
        ids.push(m.id.clone());
    }

    // Fold the chain.
    let (mut title, mut accent, mut widgets, mut layout, mut types_by_id) = match &chain.base {
        Some(b) => (
            b.pack.title.clone(),
            b.pack.accent.clone(),
            b.pack.widgets.clone(),
            Some(b.pack.layout.clone()),
            b.pack
                .task_types
                .iter()
                .map(|t| (t.id.clone(), t.clone()))
                .collect::<BTreeMap<_, _>>(),
        ),
        None => (
            String::new(),
            String::new(),
            Vec::new(),
            None,
            BTreeMap::new(),
        ),
    };
    let mut overridden_by: BTreeMap<String, String> = BTreeMap::new();
    // Who defined each type in `types_by_id`: an unsigned or dev pack may
    // only override types of packs as little trusted as itself, never a
    // built-in or signed pack's (titles and descriptions reach agents).
    let mut defined_by: BTreeMap<String, PackTrust> = types_by_id
        .keys()
        .map(|k| (k.clone(), PackTrust::Builtin))
        .collect();
    let mut prompts: Vec<PromptRef> = Vec::new();
    let mut removed_base_prompts: Vec<String> = Vec::new();
    let mut overridden_types = Vec::new();
    let last = chain.links.len() - 1;
    for (i, l) in chain.links.iter().enumerate() {
        let m = &l.manifest;
        title = m.role.title.clone();
        accent = m.role.accent.clone();
        for k in &m.widgets.add {
            if !widgets.contains(k) {
                widgets.push(k.clone());
            }
        }
        widgets.retain(|k| !m.widgets.remove.contains(k));
        layout = match (&m.layout, &m.layout_patch, layout) {
            (Some(own), _, _) => Some(own.clone()),
            (None, ops, Some(mut parent)) => {
                if let Some(ops) = ops {
                    apply_layout_ops(&mut parent, ops)?;
                }
                parent.id = format!("{}-default", m.role.id);
                parent.role_id = m.role.id.clone();
                Some(parent)
            }
            (None, _, None) => {
                return Err(PackError::invalid(format!(
                    "pack {}: no layout to inherit",
                    m.id
                )));
            }
        };
        for o in &m.type_overrides {
            if let Some(other) = overridden_by.insert(o.type_id.clone(), m.id.clone()) {
                return Err(PackError::conflict(format!(
                    "type {} is overridden by both {other} and {}",
                    o.type_id, m.id
                )));
            }
            let base = types_by_id.get(&o.type_id).ok_or_else(|| {
                PackError::invalid(format!(
                    "pack {}: {} is not a type of a parent pack",
                    m.id, o.type_id
                ))
            })?;
            let owner = defined_by
                .get(&o.type_id)
                .copied()
                .unwrap_or(PackTrust::Builtin);
            if !l.trust.may_annotate() && owner.may_annotate() {
                return Err(PackError::denied(format!(
                    "pack {} ({}) may not override {}, a type of a {} pack: an unsigned or dev pack may only override types of unsigned or dev packs",
                    m.id,
                    l.trust.as_str(),
                    o.type_id,
                    owner.as_str()
                )));
            }
            let after = apply_override(base, o)?;
            if i == last {
                overridden_types.push(after);
            }
        }
        if i < last {
            for t in &m.task_types {
                types_by_id.insert(t.id.clone(), t.clone());
                defined_by.insert(t.id.clone(), l.trust);
            }
        }
        for name in &m.prompts.remove {
            let before = prompts.len();
            prompts.retain(|p| p.name != *name);
            if prompts.len() == before && !removed_base_prompts.contains(name) {
                removed_base_prompts.push(name.clone());
            }
        }
        for (name, path) in m.prompt_paths() {
            prompts.retain(|p| p.name != name);
            prompts.push(PromptRef {
                name: name.to_string(),
                pack_id: m.id.clone(),
                path: path.to_string(),
            });
        }
    }
    let layout = layout.ok_or_else(|| PackError::invalid("no layout"))?;
    let inherited_type_ids: Vec<String> = types_by_id.keys().cloned().collect();
    for t in &child.manifest.task_types {
        if types_by_id.contains_key(&t.id) {
            return Err(PackError::conflict(format!(
                "pack {}: type {} is already defined by a parent",
                child.manifest.id, t.id
            )));
        }
    }
    let pack = EffectivePack {
        id: child.manifest.role.id.clone(),
        title,
        accent,
        widgets,
        task_types: child.manifest.task_types.clone(),
        layout,
    };
    validate_effective(&pack, &inherited_type_ids)?;
    Ok(Resolved {
        pack,
        package_id: child.manifest.id.clone(),
        version: child.manifest.semver()?,
        trust: child.trust,
        chain: ids,
        inherited_type_ids,
        overridden_types,
        prompts,
        removed_base_prompts,
    })
}

/// The checks halp-core's `Pack::validate` makes that need nothing from
/// the core: role, layout, offered kinds, text. (P5-B still runs
/// `Pack::validate`, which adds the type class and immutability checks.)
fn validate_effective(p: &EffectivePack, inherited: &[String]) -> Result<()> {
    let mut task_types: Vec<String> = builtin_types().into_iter().map(|t| t.id).collect();
    task_types.extend(inherited.iter().cloned());
    task_types.extend(p.task_types.iter().map(|t| t.id.clone()));
    Role {
        id: p.id.clone(),
        title: p.title.clone(),
        accent: p.accent.clone(),
        layout_id: p.layout.id.clone(),
        task_types,
        widgets: p.widgets.clone(),
    }
    .validate()?;
    let offered: BTreeSet<&String> = p.widgets.iter().collect();
    if offered.len() != p.widgets.len() {
        return Err(PackError::invalid(format!(
            "pack {}: a widget kind is offered twice",
            p.id
        )));
    }
    p.layout.validate()?;
    if p.layout.role_id != p.id {
        return Err(PackError::invalid(format!(
            "pack {}: layout belongs to role {}",
            p.id, p.layout.role_id
        )));
    }
    for w in &p.layout.widgets {
        if !offered.contains(&w.kind) {
            return Err(PackError::invalid(format!(
                "pack {}: layout uses widget kind {} that the role does not offer",
                p.id, w.kind
            )));
        }
    }
    let layout = serde_json::to_value(&p.layout).map_err(|e| PackError::invalid(e.to_string()))?;
    limits::check_json_text(&format!("pack {} layout", p.id), &layout)?;
    Ok(())
}
