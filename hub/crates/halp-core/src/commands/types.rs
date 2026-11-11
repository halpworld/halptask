use halp_domain::{TaskType, TaskTypeOrigin};
use halp_store::EntityKind;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::history::NoArgs;
use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::state::{Done, State, entity};

/// A task type plus where it came from: `origin` is `builtin`, `pack`,
/// `user` or `retired`; pack and retired types also carry `trust` and
/// `pack` (v1.10).
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TypeInfo {
    #[serde(flatten)]
    pub def: TaskType,
    #[serde(flatten)]
    pub origin: TaskTypeOrigin,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct TypesResult {
    pub types: Vec<TypeInfo>,
}

pub fn list(st: &mut State, _ctx: &Ctx, _a: NoArgs) -> Result<TypesResult, CmdError> {
    Ok(TypesResult {
        types: st
            .types
            .list()
            .into_iter()
            .map(|(d, o)| TypeInfo {
                def: d.clone(),
                origin: o.clone(),
            })
            .collect(),
    })
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetArgs {
    pub id: String,
}

fn info(st: &State, id: &str) -> Result<TypeInfo, CmdError> {
    st.types
        .get(id)
        .map(|(d, o)| TypeInfo {
            def: d.clone(),
            origin: o.clone(),
        })
        .ok_or_else(|| CmdError::not_found(format!("task type {id} not found")))
}

pub fn get(st: &mut State, _ctx: &Ctx, a: GetArgs) -> Result<TypeInfo, CmdError> {
    info(st, &a.id)
}

/// Args are the task type itself: `{ id, title, icon?, schema, display? }`.
pub fn define(st: &mut State, ctx: &Ctx, def: TaskType) -> Result<TypeInfo, CmdError> {
    halp_domain::limits::check_json_text("schema", &def.schema)?;
    let id = def.id.clone();
    let summary = format!("Defined task type {id}");
    st.write(ctx, "types.define", &def, true, |st, tx| {
        crate::state::check_type_class(&def)?;
        // Validates the definition and refuses to shadow built-in or pack types.
        st.types.insert(def.clone(), TaskTypeOrigin::User)?;
        // A redefinition may not leave existing items of the type invalid
        // (a refused write reloads the registry).
        let bad: Vec<String> = st
            .outline
            .preorder()
            .into_iter()
            .filter(|i| {
                st.outline.get(i).is_some_and(|it| {
                    it.type_id.as_deref() == Some(id.as_str())
                        && st.types.validate_fields(Some(&id), &it.fields).is_err()
                })
            })
            .collect();
        if !bad.is_empty() {
            let ids: Vec<&str> = bad.iter().take(50).map(String::as_str).collect();
            return Err(CmdError::conflict(format!(
                "{} items of {id} would no longer validate ({}); nothing was changed",
                bad.len(),
                ids.join(", ")
            )));
        }
        let before = st.store.get(EntityKind::TaskType, &id)?;
        tx.record(
            EntityKind::TaskType,
            &id,
            before,
            Some(serde_json::to_value(&def)?),
        );
        Ok(Done {
            result: info(st, &id)?,
            entity: entity("task_type", &id),
            summary,
        })
    })
}
