//! Role layouts: a 12-column widget grid, and roles that own a layout.

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{DomainError, Result};
use crate::limits;

pub const GRID_COLUMNS: u32 = 12;
pub const MAX_WIDGETS: usize = 48;
pub const MAX_WIDGET_H: u32 = 24;
pub const MAX_GRID_ROWS: u32 = 200;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Widget {
    pub id: String,
    /// Widget kind, e.g. `task-list`, `agenda`, `pr-queue`.
    pub kind: String,
    pub title: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    #[serde(default)]
    pub config: Map<String, Value>,
}

impl Widget {
    fn overlaps(&self, o: &Widget) -> bool {
        self.x < o.x + o.w && o.x < self.x + self.w && self.y < o.y + o.h && o.y < self.y + self.h
    }

    /// Validates the widget on its own (ids, labels, geometry, config bounds).
    pub fn validate(&self) -> Result<()> {
        limits::simple_id("widget id", &self.id)?;
        limits::simple_id("widget kind", &self.kind)?;
        limits::label("widget title", &self.title, 80)?;
        if !(1..=GRID_COLUMNS).contains(&self.w) {
            return Err(DomainError::invalid(format!(
                "widget {}: w must be 1..=12",
                self.id
            )));
        }
        if self.x + self.w > GRID_COLUMNS {
            return Err(DomainError::invalid(format!(
                "widget {}: x + w must be at most 12",
                self.id
            )));
        }
        if !(1..=MAX_WIDGET_H).contains(&self.h) {
            return Err(DomainError::invalid(format!(
                "widget {}: h must be 1..={MAX_WIDGET_H}",
                self.id
            )));
        }
        if self.y.saturating_add(self.h) > MAX_GRID_ROWS {
            return Err(DomainError::invalid(format!(
                "widget {}: y + h must be at most {MAX_GRID_ROWS}",
                self.id
            )));
        }
        limits::json_bounds(
            "widget config",
            &Value::Object(self.config.clone()),
            limits::MAX_WIDGET_CONFIG_BYTES,
            limits::MAX_WIDGET_CONFIG_DEPTH,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub id: String,
    pub role_id: String,
    pub columns: u32,
    pub widgets: Vec<Widget>,
}

impl Layout {
    pub fn validate(&self) -> Result<()> {
        limits::simple_id("layout id", &self.id)?;
        validate_role_id(&self.role_id)?;
        if self.columns != GRID_COLUMNS {
            return Err(DomainError::invalid("layout columns must be 12"));
        }
        if self.widgets.len() > MAX_WIDGETS {
            return Err(DomainError::invalid(format!(
                "at most {MAX_WIDGETS} widgets per layout"
            )));
        }
        let mut ids = HashSet::new();
        for (i, w) in self.widgets.iter().enumerate() {
            w.validate()?;
            if !ids.insert(w.id.as_str()) {
                return Err(DomainError::invalid(format!(
                    "duplicate widget id {}",
                    w.id
                )));
            }
            if let Some(o) = self.widgets[..i].iter().find(|o| o.overlaps(w)) {
                return Err(DomainError::invalid(format!(
                    "widget {} overlaps widget {}",
                    w.id, o.id
                )));
            }
        }
        Ok(())
    }

    pub fn widget(&self, id: &str) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.id == id)
    }

    /// First free top-left position (row-major) for a `w` x `h` widget,
    /// ignoring the widget `except` (when moving it).
    pub fn find_free_slot(&self, w: u32, h: u32, except: Option<&str>) -> Option<(u32, u32)> {
        if !(1..=GRID_COLUMNS).contains(&w) || !(1..=MAX_WIDGET_H).contains(&h) {
            return None;
        }
        for y in 0..=MAX_GRID_ROWS.saturating_sub(h) {
            for x in 0..=GRID_COLUMNS - w {
                let probe = Widget {
                    id: String::new(),
                    kind: String::new(),
                    title: String::new(),
                    x,
                    y,
                    w,
                    h,
                    config: Map::new(),
                };
                if !self
                    .widgets
                    .iter()
                    .filter(|o| Some(o.id.as_str()) != except)
                    .any(|o| o.overlaps(&probe))
                {
                    return Some((x, y));
                }
            }
        }
        None
    }
}

static ROLE_ID: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9-]{0,47}$").ok());
static ACCENT: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^#[0-9a-fA-F]{6}$").ok());

pub fn validate_role_id(id: &str) -> Result<()> {
    if ROLE_ID.as_ref().is_some_and(|r| r.is_match(id)) {
        Ok(())
    } else {
        Err(DomainError::invalid(format!(
            "role id {id:?} must be lowercase letters, digits or '-' (max 48)"
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Role {
    pub id: String,
    pub title: String,
    /// Accent colour, `#rrggbb`.
    pub accent: String,
    pub layout_id: String,
    /// Task type ids this role offers.
    pub task_types: Vec<String>,
    /// Widget kinds this role offers.
    pub widgets: Vec<String>,
}

impl Role {
    pub fn validate(&self) -> Result<()> {
        validate_role_id(&self.id)?;
        if limits::label("role title", &self.title, 80)?.is_empty() {
            return Err(DomainError::invalid("role title must not be empty"));
        }
        if !ACCENT.as_ref().is_some_and(|r| r.is_match(&self.accent)) {
            return Err(DomainError::invalid("role accent must be #rrggbb"));
        }
        limits::simple_id("layout id", &self.layout_id)?;
        if self.task_types.len() > 64 || self.widgets.len() > 64 {
            return Err(DomainError::invalid(
                "too many task types or widgets in role",
            ));
        }
        for t in &self.task_types {
            limits::simple_id("task type id", t)?;
        }
        for w in &self.widgets {
            limits::simple_id("widget kind", w)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn w(id: &str, x: u32, y: u32, ww: u32, h: u32) -> Widget {
        Widget {
            id: id.into(),
            kind: "task-list".into(),
            title: "Tasks".into(),
            x,
            y,
            w: ww,
            h,
            config: Map::new(),
        }
    }

    fn layout(widgets: Vec<Widget>) -> Layout {
        Layout {
            id: "developer-default".into(),
            role_id: "developer".into(),
            columns: 12,
            widgets,
        }
    }

    #[test]
    fn valid_layout_and_slots() {
        let l = layout(vec![w("a", 0, 0, 8, 4), w("b", 8, 0, 4, 4)]);
        l.validate().unwrap();
        assert_eq!(l.find_free_slot(12, 2, None), Some((0, 4)));
        assert_eq!(l.find_free_slot(4, 4, Some("b")), Some((8, 0)));
        assert_eq!(l.find_free_slot(13, 1, None), None);
    }

    #[test]
    fn invalid_layouts() {
        let cases = vec![
            layout(vec![w("a", 0, 0, 8, 4), w("b", 7, 0, 4, 4)]), // overlap
            layout(vec![w("a", 0, 0, 8, 4), w("a", 8, 0, 4, 4)]), // dup id
            layout(vec![w("a", 6, 0, 7, 4)]),                     // x+w > 12
            layout(vec![w("a", 0, 0, 0, 4)]),                     // w = 0
            layout(vec![w("a", 0, 0, 4, 0)]),                     // h = 0
            layout(vec![w("a", 0, 0, 4, 25)]),                    // too tall
            layout(vec![w("a", 0, 199, 4, 2)]),                   // off grid
            layout(vec![w("bad id", 0, 0, 4, 2)]),
            layout(
                (0..49)
                    .map(|i| w(&format!("w{i}"), 0, i * 2, 1, 1))
                    .collect(),
            ),
        ];
        for l in cases {
            assert!(
                l.validate().is_err(),
                "{:?}",
                l.widgets.first().map(|w| &w.id)
            );
        }
        let mut l = layout(vec![]);
        l.columns = 10;
        assert!(l.validate().is_err());
        let mut big = w("a", 0, 0, 4, 2);
        big.config
            .insert("x".into(), Value::String("y".repeat(20_000)));
        assert!(layout(vec![big]).validate().is_err());
    }

    #[test]
    fn role_validation() {
        let mut r = Role {
            id: "developer".into(),
            title: "Developer".into(),
            accent: "#3b82f6".into(),
            layout_id: "developer-default".into(),
            task_types: vec!["dev.pull_request".into()],
            widgets: vec!["pr-queue".into()],
        };
        r.validate().unwrap();
        r.accent = "blue".into();
        assert!(r.validate().is_err());
        r.accent = "#3b82f6".into();
        r.id = "Dev".into();
        assert!(r.validate().is_err());
    }
}
