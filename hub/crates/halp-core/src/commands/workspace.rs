use chrono::{Days, NaiveDate};
use halp_domain::{ItemKind, ItemView, Layout, Role, Status};
use halp_store::EntityKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::history::NoArgs;
use crate::envelope::CmdError;
use crate::policy::{Actor, Ctx};
use crate::state::{Done, SETTING_ROLE, State, entity, short};

const AGENDA_DAYS: u64 = 7;
const AGENDA_MAX: usize = 100;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RolesResult {
    pub roles: Vec<Role>,
    /// The active role id.
    pub current: String,
}

pub fn role_list(st: &mut State, _ctx: &Ctx, _a: NoArgs) -> Result<RolesResult, CmdError> {
    Ok(RolesResult {
        roles: st.roles.clone(),
        current: st.current_role.clone(),
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleGetArgs {
    /// Default: the active role.
    #[serde(default, alias = "role_id")]
    pub id: Option<String>,
}

pub fn role_get(st: &mut State, _ctx: &Ctx, a: RoleGetArgs) -> Result<Role, CmdError> {
    st.role(a.id.as_deref()).cloned()
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RoleSwitchArgs {
    /// Role id (`role_id` is accepted too).
    #[serde(alias = "role_id")]
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct RoleSwitchResult {
    pub role: Role,
    pub layout: Layout,
}

pub fn role_switch(
    st: &mut State,
    ctx: &Ctx,
    a: RoleSwitchArgs,
) -> Result<RoleSwitchResult, CmdError> {
    let role = st.role(Some(&a.id))?.clone();
    let summary = format!("Switched to {}", short(&role.title));
    let before = json!(st.current_role);
    // Recorded for the activity log but not undoable: undo is for content.
    st.write(ctx, "role.switch", &a, false, |st, tx| {
        tx.record(
            EntityKind::Setting,
            SETTING_ROLE,
            Some(before),
            Some(json!(role.id)),
        );
        st.current_role = role.id.clone();
        Ok(Done {
            result: RoleSwitchResult {
                layout: st.layout_for(None)?.clone(),
                role: role.clone(),
            },
            entity: entity("role", &role.id),
            summary,
        })
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct FocusResult {
    /// The active role's focused item.
    pub focus: Option<ItemView>,
}

pub fn focus_get(st: &mut State, _ctx: &Ctx, _a: NoArgs) -> Result<FocusResult, CmdError> {
    Ok(FocusResult {
        focus: st.focus_view(),
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FocusSetArgs {
    /// Item id (`item_id` is accepted too).
    #[serde(alias = "item_id")]
    pub id: String,
}

pub fn focus_set(st: &mut State, ctx: &Ctx, a: FocusSetArgs) -> Result<FocusResult, CmdError> {
    let item = st.view(&a.id)?;
    let role = st.current_role.clone();
    // The event's entity is the role, not the item, so the write-time
    // redaction (which looks at item changes) never sees this title.
    let summary = if st
        .item_class(&item.id)
        .is_some_and(|c| c != crate::policy::DEFAULT_DATA_CLASS)
    {
        "Focused a private item".to_string()
    } else {
        format!("Focus: \"{}\"", short(&item.title))
    };
    st.write(ctx, "focus.set", &a, false, |st, tx| {
        st.set_focus(tx, &role, Some(item.id.clone()));
        Ok(Done {
            result: FocusResult { focus: Some(item) },
            entity: entity("focus", &role),
            summary,
        })
    })
}

pub fn focus_clear(st: &mut State, ctx: &Ctx, a: NoArgs) -> Result<FocusResult, CmdError> {
    let role = st.current_role.clone();
    st.write(ctx, "focus.clear", &a, false, |st, tx| {
        if st.focus.contains_key(&role) {
            st.set_focus(tx, &role, None);
        }
        Ok(Done {
            result: FocusResult { focus: None },
            entity: entity("focus", &role),
            summary: "Cleared focus".into(),
        })
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DayArgs {
    /// Default: today.
    #[serde(default)]
    pub date: Option<NaiveDate>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DayCounts {
    pub overdue: u32,
    pub today: u32,
    pub in_progress: u32,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct DayState {
    pub date: NaiveDate,
    /// The active role id.
    pub role: String,
    pub focus: Option<ItemView>,
    pub counts: DayCounts,
    /// Open tasks of this role due within 7 days (overdue included), by due date.
    pub agenda: Vec<ItemView>,
    pub layout: Layout,
}

pub fn day_state(st: &mut State, ctx: &Ctx, a: DayArgs) -> Result<DayState, CmdError> {
    // Counts, like the agenda, cover only items the caller may see: a
    // count must not reveal that a hidden item is due today.
    let trusted = matches!(ctx.actor, Actor::User | Actor::System);
    let visible = |st: &State, id: &str| {
        trusted
            || st
                .item_class(id)
                .is_some_and(|c| st.policy.allows_class(ctx, &c))
    };
    let date = a.date.unwrap_or_else(|| st.today());
    let horizon = date
        .checked_add_days(Days::new(AGENDA_DAYS))
        .unwrap_or(date);
    let role = st.current_role.clone();
    let statuses = st.outline.effective_statuses();
    let mut counts = DayCounts {
        overdue: 0,
        today: 0,
        in_progress: 0,
    };
    let mut agenda = Vec::new();
    for id in st.outline.preorder() {
        let Some(it) = st.outline.get(&id) else {
            continue;
        };
        if it.kind != ItemKind::Task || !st.outline.in_role(&id, &role) || !visible(st, &id) {
            continue;
        }
        let status = statuses.get(&id).copied().unwrap_or(it.status);
        if status == Status::Done {
            continue;
        }
        if status == Status::InProgress {
            counts.in_progress += 1;
        }
        if let Some(due) = it.due {
            if due < date {
                counts.overdue += 1;
            } else if due == date {
                counts.today += 1;
            }
            if due <= horizon
                && let Some(v) = st.outline.view_with_status(&id, status)
            {
                agenda.push(v);
            }
        }
    }
    // Stable sort keeps outline order within a day.
    agenda.sort_by_key(|v| v.due);
    agenda.truncate(AGENDA_MAX);
    Ok(DayState {
        date,
        role,
        focus: st.focus_view(),
        counts,
        agenda,
        layout: st.layout_for(None)?.clone(),
    })
}
