//! Widget grid placement, a port of `hub/app/src/lib/layout/grid.ts` so the
//! UI's drag preview and the core agree on the result:
//!
//! 1. Positions are clamped into the grid: `x + w <= columns`, `y >= 0`.
//! 2. The moved or resized widget wins; widgets it overlaps are pushed down,
//!    recursively.
//! 3. Everything is then compacted upward (gravity) in reading order; the
//!    moved widget floats up only over space free in its own columns.

use std::collections::{HashMap, VecDeque};

use crate::error::{DomainError, Result};
use crate::layout::{MAX_GRID_ROWS, MAX_WIDGET_H, Widget};

pub const MIN_W: u32 = 2;
pub const MIN_H: u32 = 2;

/// Refuses a requested position outside the grid, before any grid math:
/// `x < columns`, `y <= MAX_GRID_ROWS`. (Negative or fractional values never
/// get this far: they fail to deserialize as `u32`.) The TS port must refuse
/// the same inputs rather than clamp them.
pub fn check_position(x: u32, y: u32, columns: u32) -> Result<()> {
    if x >= columns {
        return Err(DomainError::invalid(format!(
            "x must be below {columns} (the column count)"
        )));
    }
    if y > MAX_GRID_ROWS {
        return Err(DomainError::invalid(format!(
            "y must be at most {MAX_GRID_ROWS}"
        )));
    }
    Ok(())
}

/// Refuses a requested size outside `1..=columns` x `1..=MAX_WIDGET_H`.
pub fn check_size(w: u32, h: u32, columns: u32) -> Result<()> {
    if !(1..=columns).contains(&w) {
        return Err(DomainError::invalid(format!("w must be 1..={columns}")));
    }
    if !(1..=MAX_WIDGET_H).contains(&h) {
        return Err(DomainError::invalid(format!(
            "h must be 1..={MAX_WIDGET_H}"
        )));
    }
    Ok(())
}

fn collides(a: &Widget, b: &Widget) -> bool {
    a.id != b.id
        && a.x < b.x.saturating_add(b.w)
        && a.x.saturating_add(a.w) > b.x
        && a.y < b.y.saturating_add(b.h)
        && a.y.saturating_add(a.h) > b.y
}

/// Lowest occupied row + 1.
pub fn bottom(ws: &[Widget]) -> u32 {
    ws.iter()
        .map(|w| w.y.saturating_add(w.h))
        .max()
        .unwrap_or(0)
}

/// Clamps size and position into the grid.
pub fn clamp(mut w: Widget, columns: u32) -> Widget {
    w.w = w.w.clamp(1, columns.max(1));
    w.h = w.h.clamp(1, MAX_WIDGET_H);
    w.x = w.x.min(columns.saturating_sub(w.w));
    w
}

fn reading_order(ws: &[Widget]) -> Vec<Widget> {
    let mut v = ws.to_vec();
    v.sort_by(|a, b| a.y.cmp(&b.y).then(a.x.cmp(&b.x)).then(a.id.cmp(&b.id)));
    v
}

fn restore_order(original: &[Widget], placed: Vec<Widget>) -> Vec<Widget> {
    let mut by_id: HashMap<String, Widget> =
        placed.into_iter().map(|w| (w.id.clone(), w)).collect();
    original
        .iter()
        .map(|w| by_id.remove(&w.id).unwrap_or_else(|| w.clone()))
        .collect()
}

/// Moves each widget as far up as it can go without overlapping earlier ones.
/// A `pinned` widget keeps its row.
pub fn compact(ws: &[Widget], pinned: Option<&str>) -> Vec<Widget> {
    let order = reading_order(ws);
    let mut placed: Vec<Widget> = Vec::with_capacity(ws.len());
    if let Some(p) = pinned.and_then(|id| order.iter().find(|w| w.id == id)) {
        placed.push(p.clone());
    }
    for w in order {
        if Some(w.id.as_str()) == pinned {
            continue;
        }
        let mut next = w;
        while next.y > 0 && {
            let mut up = next.clone();
            up.y -= 1;
            !placed.iter().any(|p| collides(&up, p))
        } {
            next.y -= 1;
        }
        while next.y < u32::MAX && placed.iter().any(|p| collides(&next, p)) {
            next.y += 1;
        }
        placed.push(next);
    }
    restore_order(ws, placed)
}

/// Pushes anything overlapping `moved` downward, cascading.
fn push_down(ws: &[Widget], moved: &str) -> Vec<Widget> {
    let mut result = ws.to_vec();
    let mut queue = VecDeque::from([moved.to_string()]);
    let mut guard = 0;
    while let Some(id) = queue.pop_front() {
        guard += 1;
        if guard > 10_000 {
            break;
        }
        let Some(mover) = result.iter().find(|w| w.id == id).cloned() else {
            continue;
        };
        let order: Vec<String> = reading_order(&result).into_iter().map(|w| w.id).collect();
        for oid in order {
            if oid == moved || oid == id {
                continue;
            }
            if let Some(other) = result.iter_mut().find(|w| w.id == oid)
                && collides(&mover, other)
            {
                other.y = mover.y.saturating_add(mover.h);
                queue.push_back(oid);
            }
        }
    }
    result
}

fn compact_around(ws: &[Widget], moved: &str) -> Vec<Widget> {
    let Some(m) = ws.iter().find(|w| w.id == moved) else {
        return compact(ws, None);
    };
    let mut m = m.clone();
    let others: Vec<&Widget> = ws.iter().filter(|w| w.id != moved).collect();
    while m.y > 0 && {
        let mut up = m.clone();
        up.y -= 1;
        !others.iter().any(|o| collides(&up, o))
    } {
        m.y -= 1;
    }
    let with: Vec<Widget> = ws
        .iter()
        .map(|w| if w.id == moved { m.clone() } else { w.clone() })
        .collect();
    compact(&with, Some(moved))
}

fn settle(ws: &[Widget], id: &str, patch: impl FnOnce(&mut Widget), columns: u32) -> Vec<Widget> {
    let Some(target) = ws.iter().find(|w| w.id == id) else {
        return ws.to_vec();
    };
    let mut next = target.clone();
    patch(&mut next);
    let next = clamp(next, columns);
    let replaced: Vec<Widget> = ws
        .iter()
        .map(|w| {
            if w.id == id {
                next.clone()
            } else {
                clamp(w.clone(), columns)
            }
        })
        .collect();
    compact_around(&push_down(&replaced, id), id)
}

/// Moves widget `id` to `(x, y)` and resolves collisions.
pub fn move_widget(ws: &[Widget], id: &str, x: u32, y: u32, columns: u32) -> Vec<Widget> {
    settle(
        ws,
        id,
        |w| {
            w.x = x;
            w.y = y;
        },
        columns,
    )
}

/// Resizes widget `id` (at least `MIN_W` x `MIN_H`) and resolves collisions.
pub fn resize_widget(ws: &[Widget], id: &str, w: u32, h: u32, columns: u32) -> Vec<Widget> {
    settle(
        ws,
        id,
        |b| {
            b.w = w.max(MIN_W);
            b.h = h.max(MIN_H);
        },
        columns,
    )
}

/// First free spot in reading order that fits a `w` x `h` widget.
pub fn find_space(ws: &[Widget], w: u32, h: u32, columns: u32) -> (u32, u32) {
    let width = w.clamp(1, columns.max(1));
    let limit = bottom(ws).saturating_add(1);
    for y in 0..=limit {
        for x in 0..=columns.saturating_sub(width) {
            let probe = Widget {
                id: "\u{0}probe".into(),
                kind: String::new(),
                title: String::new(),
                x,
                y,
                w: width,
                h,
                config: Default::default(),
            };
            if !ws.iter().any(|b| collides(&probe, b)) {
                return (x, y);
            }
        }
    }
    (0, limit)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn w(id: &str, x: u32, y: u32, ww: u32, h: u32) -> Widget {
        Widget {
            id: id.into(),
            kind: "k".into(),
            title: id.into(),
            x,
            y,
            w: ww,
            h,
            config: Default::default(),
        }
    }

    fn pos(ws: &[Widget]) -> Vec<(&str, u32, u32, u32, u32)> {
        ws.iter()
            .map(|b| (b.id.as_str(), b.x, b.y, b.w, b.h))
            .collect()
    }

    #[test]
    fn move_pushes_down_and_compacts() {
        let ws = vec![w("a", 0, 0, 6, 2), w("b", 6, 0, 6, 2), w("c", 0, 2, 12, 2)];
        // Drop b onto a's spot: a is pushed below b, c follows, all compact.
        let out = move_widget(&ws, "b", 0, 0, 12);
        assert_eq!(
            pos(&out),
            [("a", 0, 2, 6, 2), ("b", 0, 0, 6, 2), ("c", 0, 4, 12, 2)]
        );
    }

    #[test]
    fn move_is_clamped_and_floats_up() {
        let ws = vec![w("a", 0, 0, 4, 2)];
        let out = move_widget(&ws, "a", 11, 9, 12);
        assert_eq!(pos(&out), [("a", 8, 0, 4, 2)]);
    }

    #[test]
    fn resize_respects_minimum_and_pushes() {
        let ws = vec![w("a", 0, 0, 4, 2), w("b", 0, 2, 4, 2)];
        let out = resize_widget(&ws, "a", 1, 4, 12);
        assert_eq!(pos(&out), [("a", 0, 0, 2, 4), ("b", 0, 4, 4, 2)]);
    }

    #[test]
    fn compaction_and_free_space() {
        let ws = vec![w("a", 0, 5, 4, 2), w("b", 4, 9, 4, 2)];
        assert_eq!(
            pos(&compact(&ws, None)),
            [("a", 0, 0, 4, 2), ("b", 4, 0, 4, 2)]
        );
        let full = vec![w("a", 0, 0, 12, 3)];
        assert_eq!(find_space(&full, 4, 2, 12), (0, 3));
        assert_eq!(find_space(&[w("a", 0, 0, 4, 3)], 4, 2, 12), (4, 0));
    }
}
