use halp_domain::{Layout, Widget, grid, new_id};
use halp_store::EntityKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::state::{Done, State, entity, short};

const DEFAULT_W: u32 = 4;
const DEFAULT_H: u32 = 3;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetArgs {
    /// Default: the current role.
    #[serde(default)]
    pub role_id: Option<String>,
}

pub fn get(st: &mut State, _ctx: &Ctx, a: GetArgs) -> Result<Layout, CmdError> {
    st.layout_for(a.role_id.as_deref()).cloned()
}

/// Applies `f` to a copy of the role's layout, validates it and records it.
fn edit<A: Serialize>(
    st: &mut State,
    ctx: &Ctx,
    command: &str,
    args: &A,
    role_id: Option<&str>,
    summary: String,
    f: impl FnOnce(&State, &mut Layout) -> Result<(), CmdError>,
) -> Result<Layout, CmdError> {
    let before = st.layout_for(role_id)?.clone();
    let mut after = before.clone();
    f(st, &mut after)?;
    after.validate()?;
    if after == before {
        return Ok(after);
    }
    st.write(ctx, command, args, true, |st, tx| {
        tx.record(
            EntityKind::Layout,
            &after.id,
            Some(serde_json::to_value(&before)?),
            Some(serde_json::to_value(&after)?),
        );
        st.layouts.insert(after.id.clone(), after.clone());
        Ok(Done {
            entity: entity("layout", &after.id),
            result: after,
            summary,
        })
    })
}

fn widget_mut<'a>(l: &'a mut Layout, id: &str) -> Result<&'a mut Widget, CmdError> {
    let lid = l.id.clone();
    l.widgets
        .iter_mut()
        .find(|w| w.id == id)
        .ok_or_else(|| CmdError::not_found(format!("widget {id} not found in layout {lid}")))
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AddArgs {
    /// Default: the current role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    /// One of the role's widget kinds.
    pub kind: String,
    /// Default: the kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Both x and y, or neither (first free slot in reading order).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u32>,
    /// Width in grid columns (default 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    /// Height in grid rows (default 3).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Map<String, Value>>,
}

/// Returns the updated layout; the new widget is the last one.
pub fn add_widget(st: &mut State, ctx: &Ctx, a: AddArgs) -> Result<Layout, CmdError> {
    let role = st.role(a.role_id.as_deref())?;
    if !role.widgets.contains(&a.kind) {
        return Err(CmdError::invalid(format!(
            "role {} does not offer widget kind {}",
            role.id, a.kind
        )));
    }
    if let Some(c) = &a.config {
        halp_domain::limits::check_json_text("widget config", &Value::Object(c.clone()))?;
    }
    let w = a.w.unwrap_or(DEFAULT_W);
    let h = a.h.unwrap_or(DEFAULT_H);
    let cols = st.layout_for(a.role_id.as_deref())?.columns;
    grid::check_size(w, h, cols)?;
    if let (Some(x), Some(y)) = (a.x, a.y) {
        grid::check_position(x, y, cols)?;
    }
    let ulid = new_id().to_lowercase();
    let id = format!(
        "w-{}",
        ulid.get(ulid.len().saturating_sub(10)..).unwrap_or(&ulid)
    );
    let summary = format!("Added {} widget", short(&a.kind));
    edit(
        st,
        ctx,
        "layout.add_widget",
        &a,
        a.role_id.as_deref(),
        summary,
        |_, l| {
            let (x, y) = match (a.x, a.y) {
                (Some(x), Some(y)) => (x, y),
                (None, None) => grid::find_space(&l.widgets, w, h, l.columns),
                _ => return Err(CmdError::invalid("give both x and y, or neither")),
            };
            let widget = grid::clamp(
                Widget {
                    id: id.clone(),
                    kind: a.kind.clone(),
                    title: a.title.clone().unwrap_or_else(|| a.kind.clone()),
                    x,
                    y,
                    w,
                    h,
                    config: a.config.clone().unwrap_or_default(),
                },
                l.columns,
            );
            let (wx, wy) = (widget.x, widget.y);
            l.widgets.push(widget);
            l.widgets = grid::move_widget(&l.widgets, &id, wx, wy, l.columns);
            Ok(())
        },
    )
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    pub widget_id: String,
    pub patch: WidgetPatch,
}

/// Widget fields to change; size and position changes follow the grid rules.
#[derive(Debug, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct WidgetPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Replaces the widget's config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config: Option<Map<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub y: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub w: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub h: Option<u32>,
}

pub fn update_widget(st: &mut State, ctx: &Ctx, a: UpdateArgs) -> Result<Layout, CmdError> {
    let cols = st.layout_for(a.role_id.as_deref())?.columns;
    let p = &a.patch;
    if let Some(c) = &p.config {
        halp_domain::limits::check_json_text("widget config", &Value::Object(c.clone()))?;
    }
    if p.x.is_some_and(|x| x >= cols) || p.y.is_some_and(|y| y > halp_domain::layout::MAX_GRID_ROWS)
    {
        grid::check_position(p.x.unwrap_or(0), p.y.unwrap_or(0), cols)?;
    }
    if p.w.is_some() || p.h.is_some() {
        grid::check_size(p.w.unwrap_or(1), p.h.unwrap_or(1), cols)?;
    }
    let summary = format!("Updated widget {}", short(&a.widget_id));
    edit(
        st,
        ctx,
        "layout.update_widget",
        &a,
        a.role_id.as_deref(),
        summary,
        |_, l| {
            let p = &a.patch;
            let (cw, ch, cx, cy) = {
                let w = widget_mut(l, &a.widget_id)?;
                if let Some(t) = &p.title {
                    w.title = t.clone();
                }
                if let Some(c) = &p.config {
                    w.config = c.clone();
                }
                (w.w, w.h, w.x, w.y)
            };
            let cols = l.columns;
            if p.w.is_some() || p.h.is_some() {
                l.widgets = grid::resize_widget(
                    &l.widgets,
                    &a.widget_id,
                    p.w.unwrap_or(cw),
                    p.h.unwrap_or(ch),
                    cols,
                );
            }
            if p.x.is_some() || p.y.is_some() {
                l.widgets = grid::move_widget(
                    &l.widgets,
                    &a.widget_id,
                    p.x.unwrap_or(cx),
                    p.y.unwrap_or(cy),
                    cols,
                );
            }
            Ok(())
        },
    )
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MoveArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    pub widget_id: String,
    pub x: u32,
    pub y: u32,
}

pub fn move_widget(st: &mut State, ctx: &Ctx, a: MoveArgs) -> Result<Layout, CmdError> {
    let cols = st.layout_for(a.role_id.as_deref())?.columns;
    grid::check_position(a.x, a.y, cols)?;
    let summary = format!("Moved widget {}", short(&a.widget_id));
    edit(
        st,
        ctx,
        "layout.move_widget",
        &a,
        a.role_id.as_deref(),
        summary,
        |_, l| {
            widget_mut(l, &a.widget_id)?;
            l.widgets = grid::move_widget(&l.widgets, &a.widget_id, a.x, a.y, l.columns);
            Ok(())
        },
    )
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_id: Option<String>,
    pub widget_id: String,
}

pub fn remove_widget(st: &mut State, ctx: &Ctx, a: RemoveArgs) -> Result<Layout, CmdError> {
    let summary = format!("Removed widget {}", short(&a.widget_id));
    edit(
        st,
        ctx,
        "layout.remove_widget",
        &a,
        a.role_id.as_deref(),
        summary,
        |_, l| {
            let n = l.widgets.len();
            l.widgets.retain(|w| w.id != a.widget_id);
            if l.widgets.len() == n {
                return Err(CmdError::not_found(format!(
                    "widget {} not found",
                    a.widget_id
                )));
            }
            l.widgets = grid::compact(&l.widgets, None);
            Ok(())
        },
    )
}
