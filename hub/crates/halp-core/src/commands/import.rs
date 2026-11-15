//! `import.tui`: brings a TUI storage file into the outline as one undoable
//! event. User only, T3 (it can add up to 200k items at once).

use base64::Engine as _;
use chrono::FixedOffset;
use halp_store::EntityKind;
use halp_store::import::{self, ImportOptions, SourceFormat};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use zeroize::Zeroizing;

use crate::envelope::CmdError;
use crate::policy::Ctx;
use crate::state::{Done, State, entity, short};

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportArgs {
    /// The file's bytes, standard base64.
    pub content_base64: String,
    /// Used like the TUI does to recognise `.pb` / `.halp` files.
    #[serde(default)]
    pub file_name: Option<String>,
    /// For encrypted files. Never shown or logged.
    #[serde(default)]
    pub passphrase: Option<String>,
    /// Offset of the TUI's time zone in minutes (default: this machine's).
    #[serde(default)]
    pub utc_offset_minutes: Option<i32>,
}

/// Summary of a TUI import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ImportReport {
    /// `protobuf` or `markdown`.
    pub format: String,
    pub items: usize,
    pub roots: usize,
    /// Deleted TUI items kept as tombstones (not in the outline).
    pub skipped_deleted: usize,
    pub warnings: Vec<String>,
}

fn format_name(f: SourceFormat) -> String {
    serde_json::to_value(f)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".into())
}

pub fn tui(st: &mut State, ctx: &Ctx, a: ImportArgs) -> Result<ImportReport, CmdError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(a.content_base64.trim())
        .map_err(|e| CmdError::invalid(format!("content_base64 is not base64: {e}")))?;
    let offset_min = match a.utc_offset_minutes {
        Some(m) => m,
        None => chrono::Local::now().offset().local_minus_utc() / 60,
    };
    let utc_offset = FixedOffset::east_opt(offset_min.saturating_mul(60))
        .ok_or_else(|| CmdError::invalid("utc_offset_minutes is out of range"))?;
    let opts = ImportOptions {
        passphrase: a.passphrase.clone().map(Zeroizing::new),
        utc_offset,
        today: st.today(),
        now: st.now(),
        file_name: a.file_name.clone(),
    };
    let tree = import::parse_tui(&bytes, &opts)?;
    let roots = tree.items.iter().filter(|i| i.parent_id.is_none()).count();
    let report = ImportReport {
        format: format_name(tree.format),
        items: tree.items.len(),
        roots,
        skipped_deleted: tree.skipped_deleted,
        warnings: tree.warnings.clone(),
    };
    let file = a.file_name.clone().unwrap_or_default();
    // The event keeps what was imported, never the content or passphrase.
    let args = json!({ "file_name": file, "format": tree.format, "items": tree.items.len(), "bytes": bytes.len() });
    let summary = format!(
        "Imported {} items from {}",
        tree.items.len(),
        short(if file.is_empty() { "the TUI" } else { &file })
    );
    st.write(ctx, "import.tui", &args, true, |st, tx| {
        for item in tree.items {
            let parent = item.parent_id.clone();
            st.outline.insert(item, parent.as_deref(), None)?;
        }
        for rec in &tree.legacy {
            tx.record(
                EntityKind::Legacy,
                &rec.item_id,
                None,
                Some(serde_json::to_value(rec)?),
            );
        }
        for t in &tree.tombstones {
            tx.record(
                EntityKind::Legacy,
                &t.id,
                None,
                Some(serde_json::to_value(t)?),
            );
        }
        Ok(Done {
            result: report,
            entity: entity("import", "tui"),
            summary,
        })
    })
}
