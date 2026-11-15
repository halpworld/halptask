//! The outline tree: ordered children, reparenting with cycle prevention,
//! derived parent status and tag inheritance.
//!
//! Mutations can be wrapped in a journal ([`Outline::begin`] /
//! [`Outline::commit`] / [`Outline::rollback`]) which records the before-state
//! of every touched item, so callers can persist exactly the changed rows and
//! build undo records.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::error::{DomainError, Result};
use crate::item::{Item, ItemKind, ItemView, Status};
use crate::limits::{MAX_ITEMS, MAX_TREE_DEPTH};

/// One changed item: `before` is `None` for an insert, `after` is `None` for a
/// delete.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemChange {
    pub id: String,
    pub before: Option<Item>,
    pub after: Option<Item>,
}

/// Aggregated leaf-task state of a subtree.
#[derive(Debug, Clone, Copy, Default)]
struct Rollup {
    contains_task: bool,
    has_subtasks: bool,
    todo: bool,
    done: bool,
    active: bool,
}

#[derive(Debug, Default)]
struct Journal {
    order: Vec<String>,
    before: HashMap<String, Option<Item>>,
}

#[derive(Debug, Default)]
pub struct Outline {
    items: HashMap<String, Item>,
    roots: Vec<String>,
    children: HashMap<String, Vec<String>>,
    journal: Option<Journal>,
}

impl Outline {
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds an outline from stored rows, validating structure: unique ids,
    /// existing parents, no cycles, bounded depth. Sibling `order` is
    /// normalised to 0..n.
    pub fn from_items(items: Vec<Item>) -> Result<Self> {
        if items.len() > MAX_ITEMS {
            return Err(DomainError::invalid(format!(
                "outline exceeds {MAX_ITEMS} items"
            )));
        }
        let mut o = Outline::new();
        for it in items {
            if o.items.contains_key(&it.id) {
                return Err(DomainError::invalid(format!("duplicate item id {}", it.id)));
            }
            o.items.insert(it.id.clone(), it);
        }
        for it in o.items.values() {
            if let Some(p) = &it.parent_id
                && !o.items.contains_key(p)
            {
                return Err(DomainError::invalid(format!(
                    "item {} references missing parent {p}",
                    it.id
                )));
            }
        }
        o.rebuild_index();
        // Every item must be reachable from a root within the depth limit.
        let mut reached = 0usize;
        let mut stack: Vec<(&str, usize)> = o.roots.iter().map(|r| (r.as_str(), 1)).collect();
        while let Some((id, depth)) = stack.pop() {
            if depth > MAX_TREE_DEPTH {
                return Err(DomainError::invalid(format!(
                    "outline nests deeper than {MAX_TREE_DEPTH} levels"
                )));
            }
            reached += 1;
            if let Some(kids) = o.children.get(id) {
                stack.extend(kids.iter().map(|k| (k.as_str(), depth + 1)));
            }
        }
        if reached != o.items.len() {
            return Err(DomainError::invalid("outline contains a parent cycle"));
        }
        Ok(o)
    }

    /// Repairs stored rows that [`Self::from_items`] would refuse, so a
    /// damaged database still opens: items whose parent is missing, that sit
    /// in a parent cycle or that nest deeper than the limit are moved to the
    /// root. Returns the repaired rows (only those that changed) and one
    /// warning per repair. Duplicate ids keep the first row.
    pub fn repair(items: Vec<Item>) -> (Vec<Item>, Vec<Item>, Vec<String>) {
        let mut warnings = Vec::new();
        let mut by_id: HashMap<String, Item> = HashMap::new();
        let mut order: Vec<String> = Vec::new();
        for it in items {
            if by_id.contains_key(&it.id) {
                warnings.push(format!("dropped duplicate item row {}", it.id));
                continue;
            }
            order.push(it.id.clone());
            by_id.insert(it.id.clone(), it);
        }
        let mut changed: Vec<String> = Vec::new();
        // Missing parents.
        for id in &order {
            let missing = by_id[id]
                .parent_id
                .as_ref()
                .is_some_and(|p| !by_id.contains_key(p));
            if missing && let Some(it) = by_id.get_mut(id) {
                warnings.push(format!(
                    "item {id} referenced missing parent {}; moved to the root",
                    it.parent_id.clone().unwrap_or_default()
                ));
                it.parent_id = None;
                changed.push(id.clone());
            }
        }
        // Cycles and depth: walk each item's ancestor chain.
        for id in &order {
            let mut chain: Vec<String> = vec![id.clone()];
            let mut cur = by_id[id].parent_id.clone();
            let mut cut: Option<&'static str> = None;
            while let Some(p) = cur {
                if chain.contains(&p) {
                    cut = Some("sat in a parent cycle");
                    break;
                }
                if chain.len() >= MAX_TREE_DEPTH {
                    cut = Some("nested too deep");
                    break;
                }
                chain.push(p.clone());
                cur = by_id.get(&p).and_then(|i| i.parent_id.clone());
            }
            if let Some(why) = cut
                && let Some(it) = by_id.get_mut(id)
            {
                warnings.push(format!("item {id} {why}; moved to the root"));
                it.parent_id = None;
                changed.push(id.clone());
            }
        }
        let fixed: Vec<Item> = changed
            .iter()
            .filter_map(|id| by_id.get(id).cloned())
            .collect();
        let all = order
            .into_iter()
            .filter_map(|id| by_id.remove(&id))
            .collect();
        (all, fixed, warnings)
    }

    fn rebuild_index(&mut self) {
        let mut roots: Vec<(u32, String)> = Vec::new();
        let mut children: HashMap<String, Vec<(u32, String)>> = HashMap::new();
        for it in self.items.values() {
            match &it.parent_id {
                None => roots.push((it.order, it.id.clone())),
                Some(p) => children
                    .entry(p.clone())
                    .or_default()
                    .push((it.order, it.id.clone())),
            }
        }
        roots.sort();
        self.roots = roots.into_iter().map(|(_, id)| id).collect();
        self.children = children
            .into_iter()
            .map(|(k, mut v)| {
                v.sort();
                (k, v.into_iter().map(|(_, id)| id).collect())
            })
            .collect();
        // Normalise order without journaling; this only repairs gaps.
        let fix: Vec<(String, u32)> = std::iter::once(&self.roots)
            .chain(self.children.values())
            .flat_map(|sibs| {
                sibs.iter()
                    .enumerate()
                    .map(|(i, id)| (id.clone(), i as u32))
            })
            .collect();
        for (id, i) in fix {
            if let Some(it) = self.items.get_mut(&id) {
                it.order = i;
            }
        }
    }

    // ----- journal -------------------------------------------------------

    /// Starts recording changes. Nested `begin` calls are not supported; a
    /// second call keeps the existing journal.
    pub fn begin(&mut self) {
        if self.journal.is_none() {
            self.journal = Some(Journal::default());
        }
    }

    /// Ends recording and returns the net changes.
    pub fn commit(&mut self) -> Vec<ItemChange> {
        let Some(j) = self.journal.take() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        let mut before = j.before;
        for id in j.order {
            let b = before.remove(&id).flatten();
            let a = self.items.get(&id).cloned();
            if b != a {
                out.push(ItemChange {
                    id,
                    before: b,
                    after: a,
                });
            }
        }
        out
    }

    /// Ends recording and restores every touched item.
    pub fn rollback(&mut self) {
        let Some(j) = self.journal.take() else {
            return;
        };
        for (id, before) in j.before {
            match before {
                Some(it) => {
                    self.items.insert(id, it);
                }
                None => {
                    self.items.remove(&id);
                }
            }
        }
        self.rebuild_index();
    }

    fn touch(&mut self, id: &str) {
        if let Some(j) = &mut self.journal
            && !j.before.contains_key(id)
        {
            j.before.insert(id.to_string(), self.items.get(id).cloned());
            j.order.push(id.to_string());
        }
    }

    // ----- queries -------------------------------------------------------

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.get(id)
    }

    pub fn require(&self, id: &str) -> Result<&Item> {
        self.items
            .get(id)
            .ok_or_else(|| DomainError::not_found(format!("item {id} not found")))
    }

    pub fn roots(&self) -> &[String] {
        &self.roots
    }

    pub fn children(&self, id: &str) -> &[String] {
        self.children.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    fn siblings(&self, parent: Option<&str>) -> &[String] {
        match parent {
            None => &self.roots,
            Some(p) => self.children(p),
        }
    }

    fn siblings_mut(&mut self, parent: Option<&str>) -> &mut Vec<String> {
        match parent {
            None => &mut self.roots,
            Some(p) => self.children.entry(p.to_string()).or_default(),
        }
    }

    /// Depth of an item: roots are depth 1.
    pub fn depth(&self, id: &str) -> usize {
        let mut d = 0;
        let mut cur = self.items.get(id);
        while let Some(it) = cur {
            d += 1;
            if d > MAX_TREE_DEPTH + 1 {
                break;
            }
            cur = it.parent_id.as_deref().and_then(|p| self.items.get(p));
        }
        d
    }

    /// Height of the subtree rooted at `id` (a leaf has height 1).
    pub fn subtree_height(&self, id: &str) -> usize {
        let mut max = 0;
        let mut stack = vec![(id, 1usize)];
        while let Some((n, h)) = stack.pop() {
            max = max.max(h);
            stack.extend(self.children(n).iter().map(|c| (c.as_str(), h + 1)));
        }
        max
    }

    /// Ancestors from the parent up to the root.
    pub fn ancestors(&self, id: &str) -> Vec<&Item> {
        let mut out = Vec::new();
        let mut cur = self
            .items
            .get(id)
            .and_then(|it| it.parent_id.as_deref())
            .and_then(|p| self.items.get(p));
        while let Some(it) = cur {
            if out.len() > MAX_TREE_DEPTH {
                break;
            }
            out.push(it);
            cur = it.parent_id.as_deref().and_then(|p| self.items.get(p));
        }
        out
    }

    /// True when `ancestor` is `id` itself or one of its ancestors.
    pub fn is_self_or_ancestor(&self, ancestor: &str, id: &str) -> bool {
        ancestor == id || self.ancestors(id).iter().any(|a| a.id == ancestor)
    }

    /// Ids of the subtree rooted at `id`, in pre-order, including `id`.
    pub fn subtree_ids(&self, id: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![id.to_string()];
        while let Some(n) = stack.pop() {
            stack.extend(self.children(&n).iter().rev().cloned());
            out.push(n);
        }
        out
    }

    /// All ids in outline (pre-order) order.
    pub fn preorder(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.items.len());
        let mut stack: Vec<String> = self.roots.iter().rev().cloned().collect();
        while let Some(n) = stack.pop() {
            stack.extend(self.children(&n).iter().rev().cloned());
            out.push(n);
        }
        out
    }

    /// Returns `(descendants_only, subtree_inclusive)` rollups for `id`.
    fn rollup_into(
        &self,
        id: &str,
        memo: &mut HashMap<String, (Rollup, Rollup)>,
    ) -> (Rollup, Rollup) {
        if let Some(r) = memo.get(id) {
            return *r;
        }
        let mut desc = Rollup::default();
        for c in self.children(id) {
            let (_, cr) = self.rollup_into(c, memo);
            desc.has_subtasks |= cr.contains_task;
            desc.contains_task |= cr.contains_task;
            desc.todo |= cr.todo;
            desc.done |= cr.done;
            desc.active |= cr.active;
        }
        let mut incl = desc;
        if let Some(it) = self.items.get(id)
            && it.is_task()
        {
            incl.contains_task = true;
            if !desc.has_subtasks {
                match it.status {
                    Status::Done => incl.done = true,
                    Status::InProgress => incl.active = true,
                    Status::Todo => incl.todo = true,
                }
            }
        }
        memo.insert(id.to_string(), (desc, incl));
        (desc, incl)
    }

    fn descendant_rollup(&self, id: &str, memo: &mut HashMap<String, (Rollup, Rollup)>) -> Rollup {
        self.rollup_into(id, memo).0
    }

    /// True when any descendant (through bullets too) is a task.
    pub fn has_subtasks(&self, id: &str) -> bool {
        let mut memo = HashMap::new();
        self.descendant_rollup(id, &mut memo).has_subtasks
    }

    fn effective_from(it: &Item, d: Rollup) -> Status {
        if !it.is_task() {
            return it.status;
        }
        if d.active || (d.todo && d.done) {
            Status::InProgress
        } else if d.done {
            Status::Done
        } else if d.todo {
            Status::Todo
        } else {
            it.status
        }
    }

    /// Status shown to clients. A task whose descendants include tasks is
    /// derived from its *leaf* tasks (bullets are not work, but tasks under
    /// bullets count): any in progress, or a mix of todo and done, is
    /// in progress; all done is done; all todo is todo. Mirrors the TUI's
    /// `EffectiveStatus`.
    pub fn effective_status(&self, id: &str) -> Option<Status> {
        let it = self.items.get(id)?;
        let mut memo = HashMap::new();
        let d = self.descendant_rollup(id, &mut memo);
        Some(Self::effective_from(it, d))
    }

    /// Effective status of every item, computed in one pass.
    pub fn effective_statuses(&self) -> HashMap<String, Status> {
        let mut memo = HashMap::new();
        let mut out = HashMap::with_capacity(self.items.len());
        for (id, it) in &self.items {
            let d = self.descendant_rollup(id, &mut memo);
            out.insert(id.clone(), Self::effective_from(it, d));
        }
        out
    }

    /// Direct tags and tags inherited from ancestors (deduplicated, without
    /// the direct ones), nearest ancestor first.
    pub fn effective_tags(&self, id: &str) -> (Vec<String>, Vec<String>) {
        let Some(it) = self.items.get(id) else {
            return (Vec::new(), Vec::new());
        };
        let direct = it.tags.clone();
        let mut seen: HashSet<&str> = direct.iter().map(String::as_str).collect();
        let mut inherited = Vec::new();
        for a in self.ancestors(id) {
            for t in &a.tags {
                if seen.insert(t.as_str()) {
                    inherited.push(t.clone());
                }
            }
        }
        (direct, inherited)
    }

    /// The roles an item belongs to: its own, else the nearest ancestor's
    /// non-empty list; empty means every role.
    pub fn effective_roles(&self, id: &str) -> &[String] {
        let mut cur = self.items.get(id);
        while let Some(it) = cur {
            if !it.roles.is_empty() {
                return &it.roles;
            }
            cur = it.parent_id.as_deref().and_then(|p| self.items.get(p));
        }
        &[]
    }

    /// Whether an item is visible in `role` (see [`Self::effective_roles`]).
    pub fn in_role(&self, id: &str, role: &str) -> bool {
        let r = self.effective_roles(id);
        r.is_empty() || r.iter().any(|x| x == role)
    }

    /// Client view of one item.
    pub fn view(&self, id: &str) -> Option<ItemView> {
        let status = self.effective_status(id)?;
        self.view_with_status(id, status)
    }

    /// Client view using a precomputed status (see [`Self::effective_statuses`]).
    pub fn view_with_status(&self, id: &str, status: Status) -> Option<ItemView> {
        let it = self.items.get(id)?;
        let (_, inherited_tags) = self.effective_tags(id);
        Some(ItemView {
            id: it.id.clone(),
            parent_id: it.parent_id.clone(),
            order: it.order,
            kind: it.kind,
            status,
            title: it.title.clone(),
            notes: it.notes.clone(),
            tags: it.tags.clone(),
            inherited_tags,
            due: it.due,
            type_id: it.type_id.clone(),
            fields: it.fields.clone(),
            roles: it.roles.clone(),
            origin: it.origin.clone(),
            created_at: it.created_at,
            updated_at: it.updated_at,
            version: it.version,
        })
    }

    // ----- mutations -----------------------------------------------------

    fn renumber(&mut self, parent: Option<&str>, from: usize) {
        let ids: Vec<(usize, String)> = self
            .siblings(parent)
            .iter()
            .enumerate()
            .skip(from)
            .map(|(i, id)| (i, id.clone()))
            .collect();
        for (i, id) in ids {
            let needs = self.items.get(&id).is_some_and(|it| it.order != i as u32);
            if needs {
                self.touch(&id);
                if let Some(it) = self.items.get_mut(&id) {
                    it.order = i as u32;
                }
            }
        }
    }

    fn check_parent(&self, parent: Option<&str>, extra_height: usize) -> Result<()> {
        if let Some(p) = parent {
            self.require(p)?;
            if self.depth(p) + extra_height > MAX_TREE_DEPTH {
                return Err(DomainError::invalid(format!(
                    "outline may not nest deeper than {MAX_TREE_DEPTH} levels"
                )));
            }
        }
        Ok(())
    }

    /// Inserts a new item under `parent` (root when `None`) at `index`
    /// (appends when `None` or past the end). Sets `parent_id` and `order`.
    pub fn insert(
        &mut self,
        mut item: Item,
        parent: Option<&str>,
        index: Option<usize>,
    ) -> Result<()> {
        if self.items.contains_key(&item.id) {
            return Err(DomainError::conflict(format!(
                "item {} already exists",
                item.id
            )));
        }
        if self.items.len() >= MAX_ITEMS {
            return Err(DomainError::invalid(format!(
                "outline is limited to {MAX_ITEMS} items"
            )));
        }
        self.check_parent(parent, 1)?;
        let len = self.siblings(parent).len();
        let idx = index.unwrap_or(len).min(len);
        item.parent_id = parent.map(str::to_string);
        item.order = idx as u32;
        let id = item.id.clone();
        self.touch(&id);
        self.items.insert(id.clone(), item);
        self.siblings_mut(parent).insert(idx, id);
        self.renumber(parent, idx);
        Ok(())
    }

    /// Moves `id` (with its subtree) under `new_parent` at `index`. Moving an
    /// item under itself or one of its descendants is rejected.
    pub fn move_item(
        &mut self,
        id: &str,
        new_parent: Option<&str>,
        index: Option<usize>,
        now: DateTime<Utc>,
    ) -> Result<()> {
        let old_parent = self.require(id)?.parent_id.clone();
        if let Some(p) = new_parent {
            self.require(p)?;
            if self.is_self_or_ancestor(id, p) {
                return Err(DomainError::invalid(
                    "cannot move an item under itself or its own descendant",
                ));
            }
        }
        self.check_parent(new_parent, self.subtree_height(id))?;

        let old_sibs = self.siblings_mut(old_parent.as_deref());
        let Some(pos) = old_sibs.iter().position(|s| s == id) else {
            return Err(DomainError::invalid("outline index out of sync"));
        };
        old_sibs.remove(pos);
        self.renumber(old_parent.as_deref(), pos);

        let len = self.siblings(new_parent).len();
        let idx = index.unwrap_or(len).min(len);
        self.siblings_mut(new_parent).insert(idx, id.to_string());
        self.touch(id);
        if let Some(it) = self.items.get_mut(id) {
            it.parent_id = new_parent.map(str::to_string);
            it.order = idx as u32;
            it.version += 1;
            it.updated_at = now;
        }
        self.renumber(new_parent, idx);
        Ok(())
    }

    /// Applies `f` to an item and bumps its version. Placement fields
    /// (`id`, `parent_id`, `order`) cannot be changed this way.
    pub fn update<F: FnOnce(&mut Item)>(
        &mut self,
        id: &str,
        now: DateTime<Utc>,
        f: F,
    ) -> Result<()> {
        self.require(id)?;
        self.touch(id);
        let Some(it) = self.items.get_mut(id) else {
            return Err(DomainError::not_found(format!("item {id} not found")));
        };
        let (pid, parent, order) = (it.id.clone(), it.parent_id.clone(), it.order);
        f(it);
        it.id = pid;
        it.parent_id = parent;
        it.order = order;
        it.version += 1;
        it.updated_at = now;
        Ok(())
    }

    /// Removes an item and its whole subtree; returns removed ids (pre-order).
    pub fn remove_subtree(&mut self, id: &str) -> Result<Vec<String>> {
        let parent = self.require(id)?.parent_id.clone();
        let ids = self.subtree_ids(id);
        let sibs = self.siblings_mut(parent.as_deref());
        if let Some(pos) = sibs.iter().position(|s| s == id) {
            sibs.remove(pos);
            self.renumber(parent.as_deref(), pos);
        }
        for n in &ids {
            self.touch(n);
            self.items.remove(n);
            self.children.remove(n);
        }
        Ok(ids)
    }

    /// Sets `status` on every task in the subtree (the item included), the
    /// explicit bulk operation behind "complete a group". Bullets stay
    /// bullets. Returns the ids that changed.
    pub fn set_subtree_status(
        &mut self,
        id: &str,
        status: Status,
        now: DateTime<Utc>,
    ) -> Result<Vec<String>> {
        self.require(id)?;
        let mut changed = Vec::new();
        for n in self.subtree_ids(id) {
            let needs = self
                .items
                .get(&n)
                .is_some_and(|it| it.kind == ItemKind::Task && it.status != status);
            if needs {
                self.update(&n, now, |it| it.status = status)?;
                changed.push(n);
            }
        }
        Ok(changed)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn now() -> DateTime<Utc> {
        DateTime::from_timestamp(1_790_000_000, 0).unwrap()
    }

    fn task(title: &str, st: Status) -> Item {
        let mut it = Item::new(ItemKind::Task, title.into(), now());
        it.status = st;
        it
    }

    fn bullet(title: &str) -> Item {
        Item::new(ItemKind::Bullet, title.into(), now())
    }

    fn add(o: &mut Outline, it: Item, parent: Option<&str>) -> String {
        let id = it.id.clone();
        o.insert(it, parent, None).unwrap();
        id
    }

    fn titles(o: &Outline, ids: &[String]) -> Vec<String> {
        ids.iter()
            .map(|i| o.get(i).unwrap().title.clone())
            .collect()
    }

    #[test]
    fn insert_orders_and_indexes() {
        let mut o = Outline::new();
        let a = add(&mut o, bullet("a"), None);
        let c = add(&mut o, bullet("c"), None);
        let b = bullet("b");
        let bid = b.id.clone();
        o.insert(b, None, Some(1)).unwrap();
        assert_eq!(titles(&o, o.roots()), ["a", "b", "c"]);
        assert_eq!(o.get(&a).unwrap().order, 0);
        assert_eq!(o.get(&bid).unwrap().order, 1);
        assert_eq!(o.get(&c).unwrap().order, 2);
        // index past end appends
        let d = bullet("d");
        o.insert(d, None, Some(99)).unwrap();
        assert_eq!(titles(&o, o.roots()), ["a", "b", "c", "d"]);
        // missing parent
        assert!(o.insert(bullet("x"), Some("nope"), None).is_err());
    }

    #[test]
    fn move_reparent_reorder_and_cycles() {
        let mut o = Outline::new();
        let a = add(&mut o, bullet("a"), None);
        let b = add(&mut o, bullet("b"), None);
        let a1 = add(&mut o, bullet("a1"), Some(&a));
        let a2 = add(&mut o, bullet("a2"), Some(&a));
        let a1x = add(&mut o, bullet("a1x"), Some(&a1));

        // reorder within parent
        o.move_item(&a2, Some(&a), Some(0), now()).unwrap();
        assert_eq!(titles(&o, o.children(&a)), ["a2", "a1"]);
        // reparent with subtree
        o.move_item(&a1, Some(&b), None, now()).unwrap();
        assert_eq!(titles(&o, o.children(&b)), ["a1"]);
        assert_eq!(o.get(&a1x).unwrap().parent_id.as_deref(), Some(a1.as_str()));
        assert_eq!(o.get(&a2).unwrap().order, 0);
        // cycles
        assert!(o.move_item(&b, Some(&a1x), None, now()).is_err());
        assert!(o.move_item(&b, Some(&b), None, now()).is_err());
        // to root
        o.move_item(&a1x, None, Some(0), now()).unwrap();
        assert_eq!(titles(&o, o.roots()), ["a1x", "a", "b"]);
        assert_eq!(o.get(&a1x).unwrap().version, 2);
    }

    #[test]
    fn depth_limit_enforced() {
        let mut o = Outline::new();
        let mut parent: Option<String> = None;
        for i in 0..MAX_TREE_DEPTH {
            let id = add(&mut o, bullet(&format!("n{i}")), parent.as_deref());
            parent = Some(id);
        }
        assert!(
            o.insert(bullet("too deep"), parent.as_deref(), None)
                .is_err()
        );
        // moving a 2-high subtree under a depth-63 node is also refused
        let mut o2 = Outline::new();
        let mut p: Option<String> = None;
        for i in 0..MAX_TREE_DEPTH - 1 {
            p = Some(add(&mut o2, bullet(&format!("n{i}")), p.as_deref()));
        }
        let top = add(&mut o2, bullet("top"), None);
        add(&mut o2, bullet("kid"), Some(&top));
        let err = o2.move_item(&top, p.as_deref(), None, now());
        assert!(err.is_err());
    }

    #[test]
    fn derived_status_matches_tui() {
        // Port of model/parent_status_test.go: parent with a nested task
        // parent and a bullet; leaf tasks a (under nested) and b (under bullet).
        let cases = [
            (Status::Todo, Status::Todo, Status::Todo),
            (Status::Done, Status::Done, Status::Done),
            (Status::Todo, Status::Done, Status::InProgress),
            (Status::InProgress, Status::Todo, Status::InProgress),
        ];
        for (sa, sb, want) in cases {
            let mut o = Outline::new();
            let parent = add(&mut o, task("parent", Status::Done), None);
            let nested = add(&mut o, task("nested parent", Status::Done), Some(&parent));
            let bul = add(&mut o, bullet("bullet"), Some(&parent));
            let a = add(&mut o, task("a", sa), Some(&nested));
            let b = add(&mut o, task("b", sb), Some(&bul));
            assert_eq!(o.effective_status(&parent), Some(want), "{sa:?}/{sb:?}");
            assert!(o.has_subtasks(&parent));
            assert!(o.has_subtasks(&bul));
            assert!(!o.has_subtasks(&a));
            // the bulk computation agrees
            assert_eq!(o.effective_statuses().get(&parent).copied(), Some(want));
            // direct mutation of the parent's own status has no effect
            o.update(&parent, now(), |it| it.status = Status::Todo)
                .unwrap();
            assert_eq!(o.effective_status(&parent), Some(want));
            // bulk completion
            o.set_subtree_status(&parent, Status::Done, now()).unwrap();
            assert_eq!(o.effective_status(&parent), Some(Status::Done));
            assert_eq!(o.get(&a).unwrap().status, Status::Done);
            assert_eq!(o.get(&b).unwrap().status, Status::Done);
            assert_eq!(o.get(&bul).unwrap().kind, ItemKind::Bullet);
            // childless task is editable
            o.remove_subtree(&nested).unwrap();
            o.remove_subtree(&bul).unwrap();
            o.update(&parent, now(), |it| it.status = Status::Todo)
                .unwrap();
            assert_eq!(o.effective_status(&parent), Some(Status::Todo));
        }
    }

    #[test]
    fn bullets_only_children_keep_own_status() {
        let mut o = Outline::new();
        let p = add(&mut o, task("p", Status::InProgress), None);
        add(&mut o, bullet("note"), Some(&p));
        assert_eq!(o.effective_status(&p), Some(Status::InProgress));
        assert!(!o.has_subtasks(&p));
    }

    #[test]
    fn tag_inheritance() {
        let mut o = Outline::new();
        let mut p = bullet("project");
        p.tags = vec!["work".into(), "q4".into()];
        let p = add(&mut o, p, None);
        let mut c = task("child", Status::Todo);
        c.tags = vec!["bug".into(), "work".into()];
        let c = add(&mut o, c, Some(&p));
        let g = add(&mut o, task("grandchild", Status::Todo), Some(&c));
        assert_eq!(
            o.effective_tags(&c),
            (vec!["bug".into(), "work".into()], vec!["q4".into()])
        );
        assert_eq!(
            o.effective_tags(&g),
            (vec![], vec!["bug".to_string(), "work".into(), "q4".into()])
        );
        assert_eq!(o.view(&g).unwrap().inherited_tags.len(), 3);
    }

    #[test]
    fn journal_commit_and_rollback() {
        let mut o = Outline::new();
        let a = add(&mut o, bullet("a"), None);
        let b = add(&mut o, bullet("b"), None);
        o.begin();
        let x = bullet("x");
        let xid = x.id.clone();
        o.insert(x, None, Some(0)).unwrap();
        let changes = o.commit();
        // x inserted, a and b renumbered
        assert_eq!(changes.len(), 3);
        assert!(changes.iter().any(|c| c.id == xid && c.before.is_none()));

        o.begin();
        o.remove_subtree(&a).unwrap();
        o.update(&b, now(), |it| it.title = "changed".into())
            .unwrap();
        o.rollback();
        assert_eq!(titles(&o, o.roots()), ["x", "a", "b"]);
        assert_eq!(o.get(&b).unwrap().title, "b");
    }

    #[test]
    fn from_items_validates_structure() {
        let mut o = Outline::new();
        let a = add(&mut o, bullet("a"), None);
        let b = add(&mut o, bullet("b"), Some(&a));
        let mut items: Vec<Item> = o
            .preorder()
            .iter()
            .map(|i| o.get(i).unwrap().clone())
            .collect();
        assert!(Outline::from_items(items.clone()).is_ok());
        // cycle a -> b -> a
        items[0].parent_id = Some(b.clone());
        assert!(Outline::from_items(items.clone()).is_err());
        // missing parent
        items[0].parent_id = Some("ghost".into());
        assert!(Outline::from_items(items.clone()).is_err());
        // duplicate id
        items[0].parent_id = None;
        let dup = items[0].clone();
        items.push(dup);
        assert!(Outline::from_items(items).is_err());
    }

    #[test]
    fn subtree_and_preorder() {
        let mut o = Outline::new();
        let a = add(&mut o, bullet("a"), None);
        let a1 = add(&mut o, bullet("a1"), Some(&a));
        add(&mut o, bullet("a1a"), Some(&a1));
        add(&mut o, bullet("a2"), Some(&a));
        add(&mut o, bullet("b"), None);
        assert_eq!(titles(&o, &o.preorder()), ["a", "a1", "a1a", "a2", "b"]);
        assert_eq!(titles(&o, &o.subtree_ids(&a)), ["a", "a1", "a1a", "a2"]);
        let removed = o.remove_subtree(&a1).unwrap();
        assert_eq!(removed.len(), 2);
        assert_eq!(titles(&o, &o.preorder()), ["a", "a2", "b"]);
        assert_eq!(o.subtree_height(&a), 2);
    }

    #[test]
    fn repair_reroots_orphans_cycles_and_deep_items() {
        let mut o = Outline::new();
        let a = add(&mut o, bullet("a"), None);
        let b = add(&mut o, bullet("b"), Some(&a));
        let c = add(&mut o, bullet("c"), Some(&b));
        let mut items: Vec<Item> = o
            .preorder()
            .iter()
            .map(|i| o.get(i).unwrap().clone())
            .collect();
        // a's parent is gone; b -> c -> b is a cycle once c is b's parent.
        items[0].parent_id = Some("ghost".into());
        items[1].parent_id = Some(c.clone());
        assert!(Outline::from_items(items.clone()).is_err());
        let (all, fixed, warnings) = Outline::repair(items);
        assert!(!fixed.is_empty());
        assert!(!warnings.is_empty());
        let o2 = Outline::from_items(all).unwrap();
        assert_eq!(o2.len(), 3);
        assert!(o2.get(&a).unwrap().parent_id.is_none());

        // Deeper than the limit: the deep part is re-rooted.
        let mut o = Outline::new();
        let mut parent: Option<String> = None;
        for i in 0..MAX_TREE_DEPTH {
            parent = Some(add(&mut o, bullet(&format!("n{i}")), parent.as_deref()));
        }
        let mut items: Vec<Item> = o
            .preorder()
            .iter()
            .map(|i| o.get(i).unwrap().clone())
            .collect();
        let mut extra = bullet("too deep");
        extra.parent_id = parent;
        items.push(extra);
        assert!(Outline::from_items(items.clone()).is_err());
        let (all, _, warnings) = Outline::repair(items);
        assert!(!warnings.is_empty());
        assert!(Outline::from_items(all).is_ok());
    }
}
