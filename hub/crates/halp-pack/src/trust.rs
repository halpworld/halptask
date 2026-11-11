//! Trust levels (§2) and what each one allows.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{PackError, Result};
use crate::manifest::Manifest;

/// The two schema annotations that grant power over items:
/// `x-halp-immutable-when` (locks items) and `x-halp-ext-writable` (lets
/// integrations write fields). `x-halp-data-class` only restricts, so it
/// is not here.
pub const PRIVILEGED_ANNOTATIONS: &[&str] = &["x-halp-immutable-when", "x-halp-ext-writable"];

/// Where a pack came from and how far it is trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackTrust {
    /// Compiled into the Hub.
    Builtin,
    /// Signed by a compiled-in HalpTask key.
    FirstParty,
    /// Signed by a key the user trusted.
    Community,
    /// No signature, or a key nobody trusted; the user accepted it.
    Unsigned,
    /// Loaded from the dev folder; never stored, never signed.
    Dev,
}

impl PackTrust {
    /// Whether this pack's types may carry the privileged annotations
    /// (P5-B's `TaskTypeOrigin::may_annotate` for pack types).
    pub fn may_annotate(self) -> bool {
        matches!(self, Self::Builtin | Self::FirstParty | Self::Community)
    }

    /// Whether Tier B modules and Tier C widgets may run (dev: with the
    /// limits of §2; built-in packs carry none).
    pub fn may_run_code(self) -> bool {
        matches!(self, Self::FirstParty | Self::Community | Self::Dev)
    }

    /// Whether the pack's prompts are served over MCP (never for dev: an
    /// agent could have written them).
    pub fn serves_prompts(self) -> bool {
        !matches!(self, Self::Dev)
    }

    /// Whether the pack may use a reserved namespace (§3.1).
    pub fn may_use_reserved_namespace(self) -> bool {
        matches!(self, Self::Builtin | Self::FirstParty)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::FirstParty => "first_party",
            Self::Community => "community",
            Self::Unsigned => "unsigned",
            Self::Dev => "dev",
        }
    }
}

/// Every privileged annotation in a schema, at any depth, as
/// `(json pointer of the holder, key)`.
pub fn privileged_in(schema: &Value) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    let mut stack: Vec<(String, &Value)> = vec![(String::new(), schema)];
    while let Some((path, v)) = stack.pop() {
        match v {
            Value::Object(o) => {
                for (k, child) in o {
                    if let Some(p) = PRIVILEGED_ANNOTATIONS.iter().find(|p| **p == k) {
                        out.push((path.clone(), *p));
                    }
                    stack.push((format!("{path}/{k}"), child));
                }
            }
            Value::Array(a) => {
                for (i, child) in a.iter().enumerate() {
                    stack.push((format!("{path}/{i}"), child));
                }
            }
            _ => {}
        }
    }
    out
}

/// The trust rules a manifest must meet beyond its shape:
/// * `unsigned` may not carry a privileged annotation, a module or a UI
///   widget (`denied`);
/// * `dev` may carry annotations, which the core ignores (a warning per
///   annotation is returned);
/// * `builtin` packs are not manifests and are refused here.
pub fn check_trust(m: &Manifest, trust: PackTrust) -> Result<Vec<String>> {
    let mut warnings = Vec::new();
    let found: Vec<(String, &str)> = m
        .task_types
        .iter()
        .flat_map(|t| {
            privileged_in(&t.schema)
                .into_iter()
                .map(move |(_, k)| (t.id.clone(), k))
        })
        .collect();
    match trust {
        PackTrust::Builtin => {
            return Err(PackError::invalid(format!(
                "pack {}: built-in packs are compiled in, not installed",
                m.id
            )));
        }
        PackTrust::Unsigned => {
            if let Some((t, k)) = found.first() {
                return Err(PackError::denied(format!(
                    "pack {}: type {t} uses {k}, which needs a signed pack from a trusted key",
                    m.id
                )));
            }
            if !m.modules.is_empty() || !m.ui_widgets.is_empty() {
                return Err(PackError::denied(format!(
                    "pack {}: modules and UI widgets need a signed pack from a trusted key",
                    m.id
                )));
            }
        }
        PackTrust::Dev => {
            for (t, k) in &found {
                warnings.push(format!(
                    "dev pack {}: {k} on type {t} is ignored in dev mode",
                    m.id
                ));
            }
        }
        PackTrust::FirstParty | PackTrust::Community => {}
    }
    Ok(warnings)
}
