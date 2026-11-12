//! Task list filters (`tasks.list`).

use chrono::NaiveDate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{DomainError, Result};
use crate::item::{ItemKind, ItemView, Status};
use crate::limits;
use crate::outline::Outline;

/// Filter for `tasks.list`. All conditions are ANDed. Results come in outline
/// pre-order (parents before children, siblings by `order`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskFilter {
    /// Only items of this task type.
    #[serde(default)]
    pub type_id: Option<String>,
    /// Only tasks with this (effective) status, or any of these statuses.
    /// Implies `kind: task`.
    #[serde(default)]
    pub status: Option<StatusSet>,
    /// Only items of this kind.
    #[serde(default)]
    pub kind: Option<ItemKind>,
    /// Only items carrying this tag, directly or inherited from an ancestor.
    #[serde(default)]
    pub tag: Option<String>,
    /// Only items due strictly before this date. Items without a due date are excluded.
    #[serde(default)]
    pub due_before: Option<NaiveDate>,
    /// Only items due strictly after this date. Items without a due date are excluded.
    #[serde(default)]
    pub due_after: Option<NaiveDate>,
    /// Case-insensitive substring match on title and notes.
    #[serde(default)]
    pub text: Option<String>,
    /// Only direct children of this item; `null` = only root items.
    #[serde(default, deserialize_with = "nullable")]
    pub parent_id: Option<Option<String>>,
    /// Only items that belong to this role (items with no roles belong to every role).
    #[serde(default)]
    pub role: Option<String>,
    /// Maximum number of results (default 200, max 1000).
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One status or a list of statuses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum StatusSet {
    One(Status),
    Many(Vec<Status>),
}

impl StatusSet {
    pub fn contains(&self, s: Status) -> bool {
        match self {
            StatusSet::One(x) => *x == s,
            StatusSet::Many(v) => v.contains(&s),
        }
    }
}

impl From<Status> for StatusSet {
    fn from(s: Status) -> Self {
        StatusSet::One(s)
    }
}

fn nullable<'de, D, T>(d: D) -> std::result::Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

impl TaskFilter {
    fn normalized(&self) -> Result<(Option<String>, Option<String>, usize)> {
        let tag = match &self.tag {
            Some(t) => Some(limits::tag(t)?),
            None => None,
        };
        let text = match &self.text {
            Some(t) if t.chars().count() > limits::MAX_TEXT_QUERY_CHARS => {
                return Err(DomainError::invalid(format!(
                    "text filter is longer than {} characters",
                    limits::MAX_TEXT_QUERY_CHARS
                )));
            }
            Some(t) if !t.trim().is_empty() => Some(t.trim().to_lowercase()),
            _ => None,
        };
        let limit = match self.limit {
            None => limits::DEFAULT_LIST_LIMIT,
            Some(0) => return Err(DomainError::invalid("limit must be at least 1")),
            Some(n) => (n as usize).min(limits::MAX_LIST_LIMIT),
        };
        if let (Some(a), Some(b)) = (self.due_after, self.due_before)
            && a > b
        {
            return Err(DomainError::invalid("due_after is later than due_before"));
        }
        Ok((tag, text, limit))
    }

    /// Applies the filter. An unknown `parent_id` is `not_found`.
    pub fn apply(&self, outline: &Outline) -> Result<Vec<ItemView>> {
        let (tag, text, limit) = self.normalized()?;
        if let Some(r) = &self.role {
            crate::layout::validate_role_id(r)?;
        }
        let candidates: Vec<String> = match &self.parent_id {
            Some(Some(p)) => {
                outline.require(p)?;
                outline.children(p).to_vec()
            }
            Some(None) => outline.roots().to_vec(),
            None => outline.preorder(),
        };
        let statuses = outline.effective_statuses();
        let mut out = Vec::new();
        for id in candidates {
            if out.len() >= limit {
                break;
            }
            let Some(item) = outline.get(&id) else {
                continue;
            };
            let kind = if self.status.is_some() {
                Some(ItemKind::Task)
            } else {
                self.kind
            };
            if kind.is_some_and(|k| k != item.kind) {
                continue;
            }
            let status = statuses.get(&id).copied().unwrap_or(item.status);
            if self.status.as_ref().is_some_and(|s| !s.contains(status)) {
                continue;
            }
            if let Some(t) = &self.type_id
                && item.type_id.as_deref() != Some(t.as_str())
            {
                continue;
            }
            if let Some(d) = self.due_before
                && item.due.is_none_or(|due| due >= d)
            {
                continue;
            }
            if let Some(d) = self.due_after
                && item.due.is_none_or(|due| due <= d)
            {
                continue;
            }
            if let Some(r) = &self.role
                && !outline.in_role(&id, r)
            {
                continue;
            }
            if let Some(q) = &text
                && !item.title.to_lowercase().contains(q)
                && !item.notes.to_lowercase().contains(q)
            {
                continue;
            }
            let Some(view) = outline.view_with_status(&id, status) else {
                continue;
            };
            if let Some(t) = &tag
                && !view.tags.contains(t)
                && !view.inherited_tags.contains(t)
            {
                continue;
            }
            out.push(view);
        }
        Ok(out)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::item::Item;
    use chrono::{TimeZone, Utc};

    fn outline() -> (Outline, Vec<String>) {
        let now = Utc.with_ymd_and_hms(2026, 8, 25, 9, 0, 0).unwrap();
        let mut o = Outline::new();
        let mut ids = Vec::new();
        let add =
            |o: &mut Outline, parent: Option<&str>, kind, title: &str, f: &dyn Fn(&mut Item)| {
                let mut it = Item::new(kind, title.into(), now);
                f(&mut it);
                let id = it.id.clone();
                o.insert(it, parent, None).unwrap();
                id
            };
        let work = add(&mut o, None, ItemKind::Bullet, "Work", &|i| {
            i.tags = vec!["work".into()];
            i.roles = vec!["developer".into()];
        });
        let a = add(&mut o, Some(&work), ItemKind::Task, "Fix login", &|i| {
            i.due = NaiveDate::from_ymd_opt(2026, 8, 26);
            i.notes = "Repro steps".into();
        });
        let b = add(&mut o, Some(&work), ItemKind::Task, "Ship release", &|i| {
            i.status = Status::Done;
            i.type_id = Some("base.task".into());
        });
        let c = add(&mut o, None, ItemKind::Task, "Groceries", &|i| {
            i.due = NaiveDate::from_ymd_opt(2026, 9, 1);
        });
        ids.extend([work, a, b, c]);
        (o, ids)
    }

    #[test]
    fn filters_combine() {
        let (o, ids) = outline();
        let all = TaskFilter::default().apply(&o).unwrap();
        assert_eq!(all.len(), 4);
        assert_eq!(all[0].id, ids[0]); // pre-order

        let f = TaskFilter {
            tag: Some("#Work".into()),
            ..Default::default()
        };
        let r = f.apply(&o).unwrap();
        assert_eq!(r.len(), 3, "inherited tag matches children");

        let f = TaskFilter {
            status: Some(Status::Todo.into()),
            ..Default::default()
        };
        let r: Vec<_> = f.apply(&o).unwrap().into_iter().map(|v| v.title).collect();
        assert_eq!(r, ["Fix login", "Groceries"], "status implies tasks only");

        let f = TaskFilter {
            due_before: NaiveDate::from_ymd_opt(2026, 8, 26),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 0, "due_before is exclusive");
        let f = TaskFilter {
            due_before: NaiveDate::from_ymd_opt(2026, 8, 27),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 1);
        let f = TaskFilter {
            due_after: NaiveDate::from_ymd_opt(2026, 8, 26),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 1, "due_after is exclusive");

        let f = TaskFilter {
            parent_id: Some(None),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 2, "null parent_id = roots");

        let f: TaskFilter =
            serde_json::from_value(serde_json::json!({"status": ["todo", "done"]})).unwrap();
        assert_eq!(f.apply(&o).unwrap().len(), 3, "status list");

        let f = TaskFilter {
            role: Some("cso".into()),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 1, "roles inherited from Work");
        let f = TaskFilter {
            role: Some("developer".into()),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 4);

        let f = TaskFilter {
            text: Some("REPRO".into()),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap()[0].id, ids[1], "text searches notes");

        let f = TaskFilter {
            type_id: Some("base.task".into()),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap()[0].id, ids[2]);

        let f = TaskFilter {
            parent_id: Some(Some(ids[0].clone())),
            limit: Some(1),
            ..Default::default()
        };
        let r = f.apply(&o).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, ids[1]);
    }

    #[test]
    fn invalid_filters() {
        let (o, _) = outline();
        let bad = [
            TaskFilter {
                limit: Some(0),
                ..Default::default()
            },
            TaskFilter {
                tag: Some("1bad".into()),
                ..Default::default()
            },
            TaskFilter {
                text: Some("x".repeat(500)),
                ..Default::default()
            },
            TaskFilter {
                due_after: NaiveDate::from_ymd_opt(2026, 9, 2),
                due_before: NaiveDate::from_ymd_opt(2026, 9, 1),
                ..Default::default()
            },
        ];
        for f in bad {
            assert_eq!(
                f.apply(&o).unwrap_err().kind,
                crate::ErrorKind::Invalid,
                "{f:?}"
            );
        }
        let f = TaskFilter {
            parent_id: Some(Some("nope".into())),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap_err().kind, crate::ErrorKind::NotFound);
        let f = TaskFilter {
            limit: Some(5000),
            ..Default::default()
        };
        assert_eq!(f.apply(&o).unwrap().len(), 4);
    }
}
