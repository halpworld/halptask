use chrono::NaiveDate;
use halp_domain::capture::parse_capture_text;
use halp_domain::{Item, ItemKind, ItemView, Status, limits};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{clean_tags, parse_due};
use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::state::{Done, State, entity, short};

pub const INBOX_TITLE: &str = "Inbox";
pub const INBOX_TAG: &str = "inbox";
const MAX_CAPTURE_CHARS: usize = 4_000;

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QuickArgs {
    /// One line, e.g. `Fix login #bug due:friday` or `- [~] Draft memo`.
    pub text: String,
    /// Where to put it. Default: the Inbox (a root bullet, created on first use).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Extra tags on top of the parsed `#tags`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// `task` (default) or `bullet`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
    /// Insert at the top instead of the bottom.
    #[serde(default)]
    pub top: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Parsed {
    pub title: String,
    pub tags: Vec<String>,
    /// The parsed due date; `null` if none was given or it did not parse
    /// (an unparseable `due:` token is kept in the title).
    pub due: Option<NaiveDate>,
    /// The item kind capture.quick would create.
    pub kind: ItemKind,
    /// The status it would get (`- [~] x` is in progress; bullets are `todo`).
    pub status: Status,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParseArgs {
    /// The same text capture.quick takes.
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
}

/// The parsing half of capture.quick, shared with capture.parse so the live
/// preview and the saved item can never disagree.
fn resolve(
    st: &State,
    text: &str,
    extra_tags: Option<&[String]>,
    kind: Option<ItemKind>,
) -> Result<Parsed, CmdError> {
    if text.chars().count() > MAX_CAPTURE_CHARS {
        return Err(CmdError::invalid(format!(
            "capture text exceeds {MAX_CAPTURE_CHARS} characters"
        )));
    }
    let cap = parse_capture_text(text);
    let mut title = cap.title.clone();
    let mut due = None;
    if let Some(raw) = &cap.raw_due {
        match parse_due(raw, st.today()) {
            Ok(d) => due = Some(d),
            // Keep what the user typed rather than losing it.
            Err(_) => title = format!("{title} due:{raw}").trim().to_string(),
        }
    }
    // An empty title means there was nothing to capture; any other problem
    // (hidden characters, too long) is reported as it is.
    if title.trim().is_empty() {
        return Err(CmdError::invalid("nothing to capture"));
    }
    let title = limits::title(&title)?;
    let mut tags = cap.tags.clone();
    tags.extend(extra_tags.unwrap_or_default().iter().cloned());
    let tags = clean_tags(&tags)?;
    let kind = kind.unwrap_or(ItemKind::Task);
    let status = if kind == ItemKind::Task {
        cap.status
    } else {
        Status::Todo
    };
    Ok(Parsed {
        title,
        tags,
        due,
        kind,
        status,
    })
}

/// `capture.parse`: what capture.quick would create, without writing.
pub fn parse(st: &mut State, _ctx: &Ctx, a: ParseArgs) -> Result<Parsed, CmdError> {
    resolve(st, &a.text, a.tags.as_deref(), a.kind)
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
pub struct QuickResult {
    pub item: ItemView,
    pub parsed: Parsed,
}

/// The Inbox `capture.quick` files into without a `parent_id` (the core's
/// data-class check resolves it too, so a private Inbox needs the grant).
pub(crate) fn find_inbox(st: &State) -> Option<String> {
    st.outline.roots().iter().find_map(|id| {
        let it = st.outline.get(id)?;
        (it.kind == ItemKind::Bullet && it.title.eq_ignore_ascii_case(INBOX_TITLE))
            .then(|| id.clone())
    })
}

pub fn quick(st: &mut State, ctx: &Ctx, a: QuickArgs) -> Result<QuickResult, CmdError> {
    let parsed = resolve(st, &a.text, a.tags.as_deref(), a.kind)?;
    let title = parsed.title.clone();
    let mut item = Item::new(parsed.kind, title.clone(), st.now());
    item.tags = parsed.tags.clone();
    item.due = parsed.due;
    item.status = parsed.status;
    item.origin = ctx.actor.to_string();
    // Captured while in a role: it belongs to that role.
    item.roles = vec![st.current_role.clone()];
    if let Some(p) = &a.parent_id {
        st.outline.require(p)?;
    }
    let id = item.id.clone();
    let summary = format!("Captured \"{}\"", short(&title));
    st.write(ctx, "capture.quick", &a, true, |st, _| {
        let parent = match &a.parent_id {
            Some(p) => p.clone(),
            None => match find_inbox(st) {
                Some(i) => i,
                None => {
                    let now = st.now();
                    let mut inbox = Item::new(ItemKind::Bullet, INBOX_TITLE.into(), now);
                    inbox.tags = vec![INBOX_TAG.into()];
                    let iid = inbox.id.clone();
                    st.outline.insert(inbox, None, Some(0))?;
                    iid
                }
            },
        };
        st.outline
            .insert(item, Some(&parent), if a.top { Some(0) } else { None })?;
        Ok(Done {
            result: QuickResult {
                item: st.view(&id)?,
                parsed,
            },
            entity: entity("item", &id),
            summary,
        })
    })
}
