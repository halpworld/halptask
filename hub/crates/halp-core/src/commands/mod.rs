//! The P0 command set (see `docs/CONTRACT.md`).

use chrono::NaiveDate;
use halp_domain::dates;
use serde::{Deserialize, Deserializer};

use crate::envelope::CmdError;
use crate::policy::Tier;
use crate::registry::Registry;

mod admin;
pub(crate) mod capture;
mod consent;
mod history;
mod import;
mod integrations;
mod layout;
mod meta;
mod packs;
mod tasks;
mod types;
mod workspace;

pub(crate) use consent::ConsentArgs;
pub(crate) use history::{PurgeArgs, SETTING_RETENTION, StepArgs, cutoff};
pub use import::ImportReport;

pub(crate) fn register_all(r: &mut Registry) {
    use Tier::*;
    integrations::register(r);
    r.add(
        "tasks.create",
        "Create an item (task or bullet) in the outline.",
        T1,
        tasks::create,
    );
    r.add("tasks.get", "Get one item by id.", T0, tasks::get);
    r.add(
        "tasks.list",
        "List items matching a filter, in outline order.",
        T0,
        tasks::list,
    );
    r.add(
        "tasks.update",
        "Change an item's fields. Nullable fields take null to clear.",
        T1,
        tasks::update,
    );
    r.add(
        "tasks.move",
        "Move an item (with its subtree) to a new parent and/or position.",
        T1,
        tasks::moved,
    );
    r.add(
        "tasks.complete",
        "Mark a task done. A parent or bullet needs cascade=true to complete its whole subtree.",
        T1,
        tasks::complete,
    );
    r.add(
        "tasks.delete",
        "Delete an item and its whole subtree.",
        T3,
        tasks::delete,
    );
    r.add(
        "history.undo",
        "Undo the most recent undoable change (an agent: its own most recent change). Undoing something that deletes items, or redoing a T3 change, needs the user's confirmation.",
        T1,
        history::undo,
    );
    r.add(
        "history.redo",
        "Redo the most recently undone change (an agent: its own).",
        T1,
        history::redo,
    );
    r.add(
        "history.list",
        "List recorded events, newest first (or, with `after`, the events since an event id, oldest first).",
        T0,
        history::list,
    );
    r.add(
        "types.list",
        "List task types (built-in, role pack and user defined).",
        T0,
        types::list,
    );
    r.add("types.get", "Get one task type.", T0, types::get);
    r.add(
        "types.define",
        "Define or redefine a user task type (JSON Schema 2020-12 for its fields).",
        T1,
        types::define,
    );
    r.add(
        "layout.get",
        "Get a role's layout (default: the current role).",
        T0,
        layout::get,
    );
    r.add(
        "layout.add_widget",
        "Add a widget; placed at the first free slot unless x/y are given.",
        T1,
        layout::add_widget,
    );
    r.add(
        "layout.update_widget",
        "Change a widget's title, config or size.",
        T1,
        layout::update_widget,
    );
    r.add(
        "layout.move_widget",
        "Move (and optionally resize) a widget.",
        T1,
        layout::move_widget,
    );
    r.add(
        "layout.remove_widget",
        "Remove a widget from a layout.",
        T1,
        layout::remove_widget,
    );
    r.add(
        "role.list",
        "List roles and the current role.",
        T0,
        workspace::role_list,
    );
    r.add(
        "role.get",
        "Get a role (default: the current role).",
        T0,
        workspace::role_get,
    );
    r.add(
        "role.switch",
        "Switch the current role.",
        T1,
        workspace::role_switch,
    );
    r.add(
        "focus.get",
        "Get the focused item.",
        T0,
        workspace::focus_get,
    );
    r.add("focus.set", "Focus an item.", T1, workspace::focus_set);
    r.add(
        "focus.clear",
        "Clear the focus.",
        T1,
        workspace::focus_clear,
    );
    r.add(
        "day.state",
        "Call this first. Today's overview for the current role: the focused item, open/overdue/due-today counts, the agenda for the next days and the dashboard layout. Cheap; use it to orient before reading or changing tasks.",
        T0,
        workspace::day_state,
    );
    r.add(
        "capture.quick",
        "Capture a task from one line of text, e.g. \"Fix login #bug due:friday\". Goes to the Inbox unless parent_id is given.",
        T1,
        capture::quick,
    );
    r.add(
        "capture.parse",
        "Parse capture text exactly as capture.quick would (title, tags, due, kind, status) without writing anything. For live previews.",
        T0,
        capture::parse,
    );
    r.add(
        "consent.list",
        "List requests waiting for the user's approval.",
        T0,
        consent::list,
    );
    r.add(
        "consent.get",
        "Describe one pending request.",
        T0,
        consent::get,
    );
    r.add(
        "consent.approve",
        "Approve a pending request (user only). It then runs as the original actor and this returns its result.",
        T0,
        consent::approve,
    );
    r.add(
        "consent.deny",
        "Deny a pending request (user only).",
        T0,
        consent::deny,
    );
    r.add(
        "commands.list",
        "List all commands with their tiers and JSON Schemas.",
        T0,
        meta::list,
    );
    r.add(
        "audit.list",
        "List policy decisions (allowed, denied, confirmed, approved, expired), newest first. Agents need the `audit` grant.",
        T0,
        admin::audit_list,
    );
    r.set_class("audit.list", "audit");
    r.add(
        "clients.pair",
        "Pair an MCP client: returns its token once (only a hash is stored). User only.",
        T2,
        admin::pair,
    );
    r.add(
        "clients.list",
        "List paired MCP clients. User only.",
        T0,
        admin::list,
    );
    r.add(
        "policy.clear_taint",
        "Clear an MCP client's taint (it read content from someone other than the user or itself), so standing rules apply to it again. User only.",
        T1,
        admin::clear_taint,
    );
    r.add(
        "clients.revoke",
        "Revoke an MCP client's token and delete its standing rules. User only.",
        T2,
        admin::revoke,
    );
    r.add(
        "grants.set",
        "Change a client's grants (T1 writes without asking; data classes). User only.",
        T2,
        admin::grants_set,
    );
    r.add(
        "grants.list",
        "List client grants. User only.",
        T0,
        admin::grants_list,
    );
    r.add(
        "rules.create",
        "Create a standing rule letting one client run one T2 command without asking when the given arguments match exactly. User only.",
        T2,
        admin::rules_create,
    );
    r.add(
        "rules.list",
        "List standing rules. User only.",
        T0,
        admin::rules_list,
    );
    r.add(
        "rules.delete",
        "Delete a standing rule. User only.",
        T2,
        admin::rules_delete,
    );
    r.add(
        "history.purge",
        "Redact old events (older_than_days) and/or events about deleted items; optionally remember the retention. User only.",
        T3,
        history::purge,
    );
    r.add(
        "import.tui",
        "Import a TUI storage file (protobuf or Markdown, plain or encrypted) as one undoable change. User only.",
        T3,
        import::tui,
    );
    r.add(
        "packs.list",
        "List extension packs (built-in, installed and dev) with their status.",
        T0,
        packs::list,
    );
    r.add("packs.get", "Get one extension pack.", T0, packs::get);
    r.add(
        "packs.preview",
        "Verify a .halppack bundle (base64) and report what installing it would do. Installs nothing. User only.",
        T0,
        packs::preview,
    );
    r.add(
        "packs.install",
        "Install or update exactly the previewed bundle. User only.",
        T3,
        packs::install,
    );
    r.add(
        "packs.enable",
        "Enable an installed pack. User only.",
        T3,
        packs::enable,
    );
    r.add(
        "packs.disable",
        "Disable an installed pack: its role is hidden and its types retired. User only.",
        T1,
        packs::disable,
    );
    r.add(
        "packs.rollback",
        "Go back to the version before the last update, undoing its migrations. User only.",
        T3,
        packs::rollback,
    );
    r.add(
        "packs.uninstall",
        "Uninstall a pack; its types are retired. User only.",
        T3,
        packs::uninstall,
    );
    r.add(
        "packs.reset_layout",
        "Reset a pack's role layout to the pack default. User only.",
        T1,
        packs::reset_layout,
    );
    r.add(
        "packs.keys_list",
        "List trusted publisher keys. User only.",
        T0,
        packs::keys_list,
    );
    r.add(
        "packs.keys_add",
        "Trust a publisher key (raw Ed25519 public key, base64). User only.",
        T3,
        packs::keys_add,
    );
    r.add(
        "packs.keys_revoke",
        "Revoke a publisher key: every pack pinned to it is disabled. User only.",
        T3,
        packs::keys_revoke,
    );
    r.add(
        "packs.dev_set",
        "Turn dev mode on for a folder of pack sources, or off with null. Not persisted. User only.",
        T3,
        packs::dev_set,
    );
    r.add(
        "packs.dev_get",
        "Dev mode: the folder, its packs and the folders that could not be read. User only.",
        T0,
        packs::dev_get,
    );
    r.add(
        "packs.dev_reload",
        "Read the dev folder again. User only.",
        T1,
        packs::dev_reload,
    );
    for name in [
        "clients.pair",
        "clients.list",
        "clients.revoke",
        "policy.clear_taint",
        "grants.set",
        "grants.list",
        "rules.create",
        "rules.list",
        "rules.delete",
        "history.purge",
        "import.tui",
        "consent.approve",
        "consent.deny",
        "packs.preview",
        "packs.install",
        "packs.enable",
        "packs.disable",
        "packs.rollback",
        "packs.uninstall",
        "packs.reset_layout",
        "packs.keys_list",
        "packs.keys_add",
        "packs.keys_revoke",
        "packs.dev_set",
        "packs.dev_get",
        "packs.dev_reload",
    ] {
        r.user_only(name);
    }
}

/// For patch fields: absent = `None`, `null` = `Some(None)`, value = `Some(Some(v))`.
pub(crate) fn nullable<'de, D, T>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

/// Parses a due value: ISO `YYYY-MM-DD` or natural language (`friday`, `+3d`).
pub(crate) fn parse_due(raw: &str, today: NaiveDate) -> Result<NaiveDate, CmdError> {
    Ok(dates::parse_due(raw, today)?.date)
}

/// Validates and de-duplicates tags, keeping first-seen order.
pub(crate) fn clean_tags(raw: &[String]) -> Result<Vec<String>, CmdError> {
    let mut out: Vec<String> = Vec::new();
    for t in halp_domain::limits::tags(raw)? {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    Ok(out)
}
