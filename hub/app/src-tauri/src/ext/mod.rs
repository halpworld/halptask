//! Tier C custom UI (docs/hub/P5-EXTENSIONS.md §6): community widgets in
//! sandboxed iframes served from the `halpext://` scheme.
//!
//! - [`scheme`] serves a widget's listed files, with a strict CSP on every
//!   response (§6.2, §6.4 condition 2).
//! - [`call`] is `ext_call`: the trusted host component relays a frame's
//!   `tools/call` here, and it runs as `module:<pack_id>` (§6.3, §7).
//!
//! Frames never get Tauri IPC: no capability names a remote or `halpext`
//! origin, Tauri's init scripts are main-frame-only on macOS and Linux, and
//! the frame talks only to the host component over a `MessagePort`
//! (`src/lib/widgets/ext/bridge.ts`).
//!
//! **Windows: off** (§6.4 lead decision). WebView2 injects
//! `__TAURI_INTERNALS__` and the invoke key into subframes and does not
//! report frame navigations, so on Windows the scheme is not registered,
//! navigation never allows it, `ext_widget` answers `unsupported` and
//! `ext_call` refuses. The UI shows a placeholder card instead.
//!
//! Tier B (§5): [`transform`] is `ext_run_transform`, which runs the wasm
//! transform behind a placed `computed` widget through `halp-ext` and
//! returns its checked output. It has no frame or web content, so it is
//! **not** off on Windows.

pub mod call;
#[cfg_attr(target_os = "windows", allow(dead_code))]
pub mod scheme;
pub mod tickets;
pub mod transform;

use halp_core::Core;
use serde::Serialize;
use serde_json::Value;

/// The custom scheme widgets are served from.
pub const SCHEME: &str = "halpext";

/// Whether Tier C runs on this platform (§6.4: not on Windows).
pub const SUPPORTED: bool = cfg!(not(target_os = "windows"));

/// What the shell needs to know about one installed Tier C widget. Built
/// from the core's installed manifest, never from a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WidgetInfo {
    /// `x.<pack_id>.<name>`.
    pub kind: String,
    /// MCP tool names from the manifest's `calls`.
    pub calls: Vec<String>,
    /// The entry document, relative to the widget root.
    pub entry: String,
    /// `first_party`, `community`, `dev`, ...
    pub trust: &'static str,
    /// The pack is enabled and its trust allows Tier C (§2).
    pub may_run: bool,
}

/// One served file: verified bytes and the hash they must match.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedFile {
    pub bytes: Vec<u8>,
    /// Lowercase hex SHA-256 from the installed file index.
    pub sha256: String,
}

/// The shell's view of the core for Tier C. [`Core`] implements it; tests
/// use a fake.
pub trait ExtSource: Send + Sync {
    /// The widget, or `None` when the pack is unknown, disabled or
    /// uninstalled, or has no such widget.
    fn widget(&self, pack_id: &str, widget: &str) -> Option<WidgetInfo>;
    /// A file under the widget's root, or `None` when it is not listed.
    fn file(&self, pack_id: &str, widget: &str, path: &str) -> Option<ServedFile>;
    /// Runs `command` as `module:<pack_id>` and returns the contract
    /// envelope.
    fn dispatch_as_module(&self, pack_id: &str, command: &str, args: Value) -> Value;
}

impl ExtSource for Core {
    fn widget(&self, pack_id: &str, widget: &str) -> Option<WidgetInfo> {
        let w = self.ext_ui_widget(pack_id, widget)?;
        Some(WidgetInfo {
            kind: w.kind,
            calls: w.calls,
            entry: w.entry,
            trust: w.trust.as_str(),
            may_run: w.trust.may_run_code(),
        })
    }

    fn file(&self, pack_id: &str, widget: &str, path: &str) -> Option<ServedFile> {
        let f = self.ext_ui_file(pack_id, widget, path)?;
        Some(ServedFile {
            bytes: f.bytes,
            sha256: f.sha256,
        })
    }

    fn dispatch_as_module(&self, pack_id: &str, command: &str, args: Value) -> Value {
        let ctx = halp_core::Ctx::new(halp_core::Actor::Module(pack_id.to_string()));
        match self.dispatch(&ctx, halp_core::Request::new(command, args)) {
            // Second lock, as for Tier B inputs: drop any item the module
            // may not see before the result reaches the frame.
            halp_core::Response::Ok(v) => match halp_ext::wiring::only_visible(self, &ctx, v) {
                Ok(v) => serde_json::json!({"ok": true, "result": v}),
                Err(e) => serde_json::json!({
                    "ok": false,
                    "error": {"code": e.code, "message": e.message}
                }),
            },
            r => r.to_value(),
        }
    }
}

// ---- identifiers ------------------------------------------------------------

/// One id segment: a lowercase letter, then up to 31 lowercase letters,
/// digits or `-` (`halp-pack` `ids.rs` and `check_name`).
fn is_segment(s: &str) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= 32
        && b[0].is_ascii_lowercase()
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// A pack id as `halp-pack` defines it, plus the `dev.` prefix of a
/// dev-folder pack: 2 to 5 dot-separated segments, at most 64 bytes.
pub fn is_pack_id(s: &str) -> bool {
    let n = s.split('.').count();
    s.len() <= 64 && (2..=5).contains(&n) && s.split('.').all(is_segment)
}

/// A widget name (`halp-pack` `check_name`).
pub fn is_widget_name(s: &str) -> bool {
    is_segment(s)
}

/// The widget kind for `pack_id` and `widget`.
pub fn kind_of(pack_id: &str, widget: &str) -> String {
    format!("x.{pack_id}.{widget}")
}

// ---- commands ---------------------------------------------------------------

/// What `ext_widget` tells the host component.
#[derive(Debug, Serialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum WidgetAnswer {
    /// Mount `src` (it carries a one-time ticket for the entry document).
    Ready {
        kind: String,
        src: String,
        calls: Vec<String>,
        trust: &'static str,
    },
    /// Tier C is off on this platform (Windows).
    Unsupported,
    /// The Hub is locked.
    Locked,
    /// No such widget, or its pack is disabled, uninstalled or not trusted
    /// to run code.
    Unavailable,
}

/// Looks up a widget for the host component and issues a ticket for its
/// entry document.
pub fn widget_answer(
    src: Option<&dyn ExtSource>,
    tickets: &tickets::Tickets,
    pack_id: &str,
    widget: &str,
) -> WidgetAnswer {
    if !SUPPORTED {
        return WidgetAnswer::Unsupported;
    }
    let Some(src) = src else {
        return WidgetAnswer::Locked;
    };
    if !is_pack_id(pack_id) || !is_widget_name(widget) {
        return WidgetAnswer::Unavailable;
    }
    let Some(info) = src.widget(pack_id, widget) else {
        return WidgetAnswer::Unavailable;
    };
    if !info.may_run || info.kind != kind_of(pack_id, widget) || !scheme::is_rel_path(&info.entry) {
        return WidgetAnswer::Unavailable;
    }
    let ticket = tickets.issue(pack_id, widget);
    WidgetAnswer::Ready {
        kind: info.kind,
        src: format!(
            "{SCHEME}://{pack_id}/{widget}/{}?ticket={ticket}",
            info.entry
        ),
        calls: call::effective_calls(&info.calls),
        trust: info.trust,
    }
}

// ---- Tauri commands (main window only) ----------------------------------------

/// Tier B and C state kept by the app.
pub struct ExtState {
    pub tickets: tickets::Tickets,
    /// `ext_call`, per pack.
    pub rate: call::RateLimit,
    /// `ext_run_transform`, per pack.
    pub transform_rate: call::RateLimit,
}

impl Default for ExtState {
    fn default() -> Self {
        Self {
            tickets: tickets::Tickets::default(),
            rate: call::RateLimit::default(),
            transform_rate: transform::rate_limit(),
        }
    }
}

/// Tier B and C commands answer the main window only (the capability grants them
/// to `main` alone; this is the second lock). A frame never reaches these:
/// it has no IPC, and the host component relays for it.
fn main_only(label: &str) -> Result<(), String> {
    if label == crate::MAIN {
        Ok(())
    } else {
        Err("forbidden".into())
    }
}

/// Where to mount widget `pack_id`/`widget`, or why not.
#[tauri::command]
pub async fn ext_widget(
    window: tauri::WebviewWindow,
    app: tauri::State<'_, crate::AppState>,
    ext: tauri::State<'_, ExtState>,
    pack_id: String,
    widget: String,
) -> Result<WidgetAnswer, String> {
    main_only(window.label())?;
    let core = app.core();
    Ok(widget_answer(
        core.as_deref().map(|c| c as &dyn ExtSource),
        &ext.tickets,
        &pack_id,
        &widget,
    ))
}

/// Runs one widget call as `module:<pack_id>`; returns the contract
/// envelope (`denied` for anything outside the widget's allow-list).
#[tauri::command]
pub async fn ext_call(
    window: tauri::WebviewWindow,
    app: tauri::State<'_, crate::AppState>,
    pack_id: String,
    widget: String,
    name: String,
    arguments: Value,
) -> Result<Value, String> {
    main_only(window.label())?;
    let core = app.core();
    let handle = tauri::Manager::app_handle(&window).clone();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let ext = handle.state::<ExtState>();
        call::ext_call(
            core.as_deref().map(|c| c as &dyn ExtSource),
            &ext.rate,
            std::time::Instant::now(),
            &pack_id,
            &widget,
            &name,
            arguments,
        )
    })
    .await
    .map_err(|_| "internal".to_string())
}

#[cfg(test)]
mod tests {
    use super::scheme::tests::fake;
    use super::*;

    #[test]
    fn ids() {
        for ok in [
            "acme.sre",
            "a.b.c.d",
            "dev.acme.sre",
            "dev.a.b.c.d",
            "a1-b.c2",
        ] {
            assert!(is_pack_id(ok), "{ok}");
        }
        let long = format!("{}.{}", "a".repeat(32), "b".repeat(33));
        for bad in [
            "acme",
            "Acme.sre",
            "1a.b",
            "a..b",
            ".a.b",
            "a.b.",
            "a.b.c.d.e.f",
            "a_b.c",
            "a.-b",
            &long,
            "",
        ] {
            assert!(!is_pack_id(bad), "{bad}");
        }
        assert!(is_widget_name("oncall-2"));
        for bad in ["", "On", "a.b", "a/b", "-a", &"a".repeat(33)] {
            assert!(!is_widget_name(bad), "{bad}");
        }
    }

    #[test]
    fn tier_c_commands_are_for_the_main_window_only() {
        assert!(main_only(crate::MAIN).is_ok());
        assert_eq!(main_only(crate::CAPTURE), Err("forbidden".into()));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn widget_answers() {
        let (src, t) = (fake(), tickets::Tickets::default());
        let WidgetAnswer::Ready {
            kind,
            src: url,
            calls,
            trust,
        } = widget_answer(Some(&src), &t, "acme.sre", "oncall")
        else {
            panic!("ready expected");
        };
        assert_eq!(kind, "x.acme.sre.oncall");
        assert_eq!(trust, "community");
        assert_eq!(
            calls,
            vec!["tasks_list".to_string()],
            "write calls are never offered"
        );
        let tk = url
            .strip_prefix("halpext://acme.sre/oncall/index.html?ticket=")
            .unwrap_or_default();
        assert!(tickets::is_ticket(tk), "{url}");
        let parsed = scheme::parse_target(&url).expect("its own URL parses");
        assert_eq!(parsed.ticket.as_deref(), Some(tk));
        assert_eq!(
            widget_answer(Some(&src), &t, "acme.off", "oncall"),
            WidgetAnswer::Unavailable
        );
        assert_eq!(
            widget_answer(Some(&src), &t, "nope.x", "oncall"),
            WidgetAnswer::Unavailable
        );
        assert_eq!(
            widget_answer(Some(&src), &t, "../x", "oncall"),
            WidgetAnswer::Unavailable
        );
        assert_eq!(
            widget_answer(None, &t, "acme.sre", "oncall"),
            WidgetAnswer::Locked
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn windows_is_unsupported() {
        assert_eq!(
            widget_answer(
                Some(&fake()),
                &tickets::Tickets::default(),
                "acme.sre",
                "oncall"
            ),
            WidgetAnswer::Unsupported
        );
    }
}
