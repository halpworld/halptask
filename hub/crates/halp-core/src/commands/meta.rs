use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::history::NoArgs;
use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::registry::CommandDescriptor;
use crate::state::State;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct CommandsResult {
    pub commands: Vec<CommandDescriptor>,
}

pub fn list(st: &mut State, _ctx: &Ctx, _a: NoArgs) -> Result<CommandsResult, CmdError> {
    Ok(CommandsResult {
        commands: st.descriptors.clone(),
    })
}
