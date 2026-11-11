//! HalpTask Hub desktop shell.
//!
//! One in-process [`Core`] behind one `dispatch` command (always as the
//! user), its change events forwarded to the webview as `halp://event`, a
//! tray, a quick-capture window on a global shortcut, and the MCP host that
//! lets agents reach the same core over a local socket.

mod bridge;
mod ext;
mod integrations;
mod keys;
mod mcp;
mod nav;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant};

use halp_core::Core;
use keys::{KeyStatus, McpInfo, Paths};
use serde_json::Value;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::webview::NewWindowResponse;
use tauri::{
    AppHandle, Emitter, Manager, RunEvent, State, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tokio::sync::broadcast::error::RecvError;
use zeroize::Zeroizing;

/// Change events for the webview (see `src/lib/api/tauri.ts`).
const EVENT_CHANNEL: &str = "halp://event";
/// Key status changes (locked -> ready ...).
const KEY_CHANNEL: &str = "halp://key";
const MAIN: &str = "main";
const CAPTURE: &str = "capture";

struct AppState {
    paths: Paths,
    core: RwLock<Option<Arc<Core>>>,
    status: Mutex<KeyStatus>,
    mcp: tokio::sync::Mutex<Option<mcp::Host>>,
    attention: Attention,
}

impl AppState {
    fn core(&self) -> Option<Arc<Core>> {
        self.core
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn status(&self) -> KeyStatus {
        self.status
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
    fn set_status(&self, app: &AppHandle, s: KeyStatus) {
        *self.status.lock().unwrap_or_else(PoisonError::into_inner) = s.clone();
        let _ = app.emit(KEY_CHANNEL, s);
    }
    /// Records how the MCP host start went in the `Ready` status and
    /// re-sends it on `halp://key`.
    fn set_mcp(&self, app: &AppHandle, result: Result<McpInfo, String>) {
        let s = {
            let mut st = self.status.lock().unwrap_or_else(PoisonError::into_inner);
            let KeyStatus::Ready { mcp, mcp_error, .. } = &mut *st else {
                return;
            };
            (*mcp, *mcp_error) = match result {
                Ok(info) => (Some(info), None),
                Err(e) => (None, Some(e)),
            };
            st.clone()
        };
        let _ = app.emit(KEY_CHANNEL, s);
    }
}

fn data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("HALP_HUB_DATA_DIR").filter(|d| !d.is_empty()) {
        return Ok(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("", "", "halptask-hub")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| "no home directory".to_string())
}

fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// Makes `core` live: stores it, forwards its events, starts the MCP host.
fn activate(app: &AppHandle, core: Arc<Core>, kind: &'static str) {
    let state = app.state::<AppState>();
    *state.core.write().unwrap_or_else(PoisonError::into_inner) = Some(core.clone());

    let mut rx = core.subscribe();
    let handle = app.clone();
    let events_core = core.clone();
    update_pending_badge(&handle, &events_core);
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let _ = handle.emit(EVENT_CHANNEL, &event);
                    if wants_attention(&event.command) {
                        let raise = handle
                            .state::<AppState>()
                            .attention
                            .should_raise(&event.actor, Instant::now());
                        call_for_attention(&handle, raise);
                    }
                    if changes_pending(&event.command) {
                        update_pending_badge(&handle, &events_core);
                    }
                }
                // The UI refetches on the next event; missed ones only cost
                // freshness, never correctness (all state lives in the core).
                Err(RecvError::Lagged(n)) => eprintln!("halp: webview missed {n} change events"),
                Err(RecvError::Closed) => break,
            }
        }
    });

    integrations::start(app, &core, &state.paths.data_dir);

    // Tier B: the wasm transform host for `computed` widgets. Without it a
    // computed widget shows an error; nothing else depends on it.
    if let Err(e) = halp_ext::wiring::attach(&core) {
        eprintln!("halp: extension host did not start: {e}");
    }

    let handle = app.clone();
    let data_dir = state.paths.data_dir.clone();
    // Ready first (without MCP news), so `set_mcp` always finds it.
    state.set_status(
        app,
        KeyStatus::Ready {
            kind,
            mcp: None,
            mcp_error: None,
        },
    );
    tauri::async_runtime::spawn(async move {
        let state = handle.state::<AppState>();
        match mcp::start(core, &data_dir).await {
            Ok(host) => {
                let socket = host.socket_path().display().to_string();
                eprintln!("halp: MCP host listening at {socket}");
                *state.mcp.lock().await = Some(host);
                state.set_mcp(&handle, Ok(McpInfo { socket }));
            }
            Err(e) => {
                eprintln!("halp: {e}");
                state.set_mcp(&handle, Err(e));
            }
        }
    });
}

// ---- commands ---------------------------------------------------------------

/// The only way the webview reaches the core. Always the user; see `bridge`.
#[tauri::command]
async fn dispatch(
    state: State<'_, AppState>,
    request: bridge::WebRequest,
) -> Result<Value, String> {
    let Some(core) = state.core() else {
        return Ok(bridge::locked_response());
    };
    tauri::async_runtime::spawn_blocking(move || bridge::dispatch_as_user(&core, request))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn key_status(state: State<'_, AppState>) -> KeyStatus {
    state.status()
}

/// Unlocks (or, with `create`, sets up) a passphrase database. The
/// passphrase is moved into a zeroizing buffer at once and dropped as soon
/// as the key is derived.
#[tauri::command]
async fn unlock(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
    create: bool,
) -> Result<KeyStatus, String> {
    let passphrase = Zeroizing::new(passphrase);
    match (state.status(), create) {
        (KeyStatus::Locked, false) | (KeyStatus::Setup { .. }, true) => {}
        (s, _) => return Err(format!("cannot unlock in state {s:?}")),
    }
    let paths = state.paths.clone();
    let core = tauri::async_runtime::spawn_blocking(move || {
        keys::open_with_passphrase(&paths, passphrase, create)
    })
    .await
    .map_err(|e| e.to_string())??;
    activate(&app, core, "passphrase");
    Ok(state.status())
}

#[tauri::command]
fn hide_capture(app: AppHandle) {
    if let Some(w) = app.get_webview_window(CAPTURE) {
        let _ = w.hide();
    }
}

// ---- windows ----------------------------------------------------------------

/// Navigation, new-window and page-load guards for the Hub's windows.
/// `ext_frames`: the window may show Tier C widget frames (main only).
fn guarded<'a, R: tauri::Runtime, M: Manager<R>>(
    b: WebviewWindowBuilder<'a, R, M>,
    ext_frames: bool,
) -> WebviewWindowBuilder<'a, R, M> {
    let dev = tauri::is_dev();
    b.on_navigation(move |url| {
        nav::is_app_url(url, dev) || (ext_frames && nav::is_ext_frame_url(url))
    })
    .on_new_window(|_url, _features| NewWindowResponse::Deny)
    // `on_navigation` cannot tell a frame from the top level, so a
    // halpext page could in principle replace the app itself, where
    // Tauri's main-frame scripts (and the invoke key) would reach it.
    // The page-load hook fires for the top level only (wry: WKNavigation
    // delegate / WebKitGTK load-changed): fail closed if it ever happens.
    .on_page_load(|w, p| {
        if p.url().scheme() == ext::SCHEME {
            eprintln!("halp: a widget page replaced a Hub window; quitting");
            let _ = w.hide();
            w.app_handle().exit(1);
        }
    })
    .devtools(cfg!(debug_assertions))
}

fn build_main(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = guarded(
        WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into())),
        true,
    )
    .title("HalpTask Hub")
    .inner_size(1440.0, 900.0)
    .min_inner_size(960.0, 600.0)
    .center()
    .build()?;
    // Closing hides: the tray, the shortcut and the MCP host keep running.
    let hide = w.clone();
    w.on_window_event(move |e| {
        if let WindowEvent::CloseRequested { api, .. } = e {
            api.prevent_close();
            let _ = hide.hide();
        }
    });
    Ok(w)
}

fn build_capture(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = guarded(
        WebviewWindowBuilder::new(app, CAPTURE, WebviewUrl::App("index.html".into())),
        false,
    )
    .title("Quick capture")
    .inner_size(640.0, 220.0)
    .resizable(false)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .center()
    .build()?;
    let hide = w.clone();
    w.on_window_event(move |e| match e {
        WindowEvent::Focused(false) => {
            let _ = hide.hide();
        }
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = hide.hide();
        }
        _ => {}
    });
    Ok(w)
}

fn show(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if label == CAPTURE {
            let _ = w.center();
            let _ = w.emit_to(CAPTURE, "halp://capture-shown", ());
        }
    }
}

// ---- consent prompts -----------------------------------------------------------

/// An agent is waiting for an approval that can only be given in this
/// window (the socket's user channel can only list and deny).
fn wants_attention(command: &str) -> bool {
    command == "consent.requested"
}

/// Events after which the number of pending requests may differ.
fn changes_pending(command: &str) -> bool {
    command.starts_with("consent.")
}

fn pending_tooltip(n: usize) -> String {
    match n {
        0 => "HalpTask Hub".into(),
        1 => "HalpTask Hub: 1 request waiting for your approval".into(),
        n => format!("HalpTask Hub: {n} requests waiting for your approval"),
    }
}

/// How often one actor's requests may bring the main window to the front.
const RAISE_EVERY: Duration = Duration::from_secs(10);

/// Remembers when each actor last raised the window, so an agent that
/// parks request after request cannot keep stealing focus (and catch a
/// stray click or key press).
#[derive(Default)]
struct Attention {
    last: Mutex<HashMap<String, Instant>>,
}

impl Attention {
    fn should_raise(&self, actor: &str, now: Instant) -> bool {
        let mut last = self.last.lock().unwrap_or_else(PoisonError::into_inner);
        last.retain(|_, at| now.saturating_duration_since(*at) < RAISE_EVERY);
        if last.contains_key(actor) {
            return false;
        }
        last.insert(actor.to_string(), now);
        true
    }
}

/// Asks the OS to flag the main window (dock bounce, taskbar flash) and,
/// when `raise`, brings it up (it may be hidden in the tray). The tray
/// count is updated separately, on every consent event.
fn call_for_attention(app: &AppHandle, raise: bool) {
    if raise {
        show(app, MAIN);
    }
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.request_user_attention(Some(tauri::UserAttentionType::Informational));
    }
}

/// Shows the number of pending requests on the tray icon.
fn update_pending_badge(app: &AppHandle, core: &Arc<Core>) {
    let app = app.clone();
    let core = core.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let r = core.dispatch(
            &halp_core::Ctx::user(),
            halp_core::Request::new("consent.list", serde_json::json!({})),
        );
        let n = match r {
            halp_core::Response::Ok(v) => v["pending"].as_array().map_or(0, Vec::len),
            halp_core::Response::Err(_) => 0,
        };
        if let Some(tray) = app.tray_by_id("halp") {
            let _ = tray.set_tooltip(Some(pending_tooltip(n)));
            #[cfg(target_os = "macos")]
            let _ = tray.set_title(if n == 0 { None } else { Some(n.to_string()) });
        }
    });
}

fn capture_shortcut() -> Shortcut {
    let primary = if cfg!(target_os = "macos") {
        Modifiers::SUPER
    } else {
        Modifiers::CONTROL
    };
    Shortcut::new(Some(primary | Modifiers::SHIFT), Code::Space)
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let capture = MenuItem::with_id(
        app,
        "capture",
        "Quick capture",
        true,
        Some("CmdOrCtrl+Shift+Space"),
    )?;
    let hub = MenuItem::with_id(app, "show", "Show hub", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit HalpTask Hub", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&capture, &hub, &sep, &quit])?;
    let mut tray = TrayIconBuilder::with_id("halp")
        .tooltip("HalpTask Hub")
        .menu(&menu)
        .on_menu_event(|app, e| match e.id().as_ref() {
            "capture" => show(app, CAPTURE),
            "show" => show(app, MAIN),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

// ---- entry ------------------------------------------------------------------

pub fn run() {
    let paths = match data_dir() {
        Ok(d) => Paths::new(d),
        Err(e) => {
            eprintln!("halp: {e}");
            std::process::exit(1);
        }
    };

    let shortcut = capture_shortcut();
    let builder = tauri::Builder::default();
    // Tier C widget files (§6.2). Not registered on Windows, where Tier C is off.
    #[cfg(not(target_os = "windows"))]
    let builder =
        builder.register_asynchronous_uri_scheme_protocol(ext::SCHEME, |ctx, req, responder| {
            let label = ctx.webview_label().to_string();
            let app = ctx.app_handle().clone();
            let method = req.method().clone();
            let uri = req.uri().to_string();
            tauri::async_runtime::spawn_blocking(move || {
                let core = app.state::<AppState>().core();
                let ext = app.state::<ext::ExtState>();
                let source = core.as_deref().map(|c| c as &dyn ext::ExtSource);
                responder.respond(ext::scheme::serve(
                    source,
                    &ext.tickets,
                    &label,
                    &method,
                    &uri,
                ));
            });
        });
    let app = builder
        // First, so a second launch only focuses the running Hub.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            show(app, MAIN)
        }))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_denylist(&[CAPTURE])
                .build(),
        )
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, s, e| {
                    if *s == shortcut && e.state() == ShortcutState::Pressed {
                        show(app, CAPTURE);
                    }
                })
                .build(),
        )
        .manage(AppState {
            paths,
            core: RwLock::new(None),
            status: Mutex::new(KeyStatus::Starting),
            mcp: tokio::sync::Mutex::new(None),
            attention: Attention::default(),
        })
        .manage(integrations::Slot::default())
        .manage(ext::ExtState::default())
        // New webview commands go here, in build.rs and in a capability file.
        .invoke_handler(tauri::generate_handler![
            dispatch,
            key_status,
            unlock,
            hide_capture,
            integrations::integrations_set_token,
            integrations::integrations_secret_hint,
            // Tier C (main window only; never reachable from a frame).
            ext::ext_widget,
            ext::ext_call,
            ext::transform::ext_run_transform
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            build_main(&handle)?;
            build_capture(&handle)?;
            build_tray(&handle)?;
            if let Err(e) = handle.global_shortcut().register(shortcut) {
                // Another app may own it; the tray item still works.
                eprintln!("halp: quick-capture shortcut unavailable: {e}");
            }
            // The keychain prompt can block; keep it off the main thread.
            tauri::async_runtime::spawn_blocking(move || {
                let state = handle.state::<AppState>();
                if let Err(e) = create_private_dir(&state.paths.data_dir) {
                    state.set_status(
                        &handle,
                        KeyStatus::Error {
                            reason: format!("data directory: {e}"),
                        },
                    );
                    return;
                }
                match keys::open_at_startup(&state.paths) {
                    Ok((core, kind)) => activate(&handle, core, kind),
                    Err(s) => state.set_status(&handle, s),
                }
            });
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(a) => a,
        Err(e) => {
            eprintln!("halp: cannot start: {e}");
            std::process::exit(1);
        }
    };
    app.run(|app, event| match event {
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => show(app, MAIN),
        RunEvent::Exit => {
            integrations::stop(app);
            let state = app.state::<AppState>();
            tauri::async_runtime::block_on(async {
                if let Some(host) = state.mcp.lock().await.take() {
                    host.shutdown().await;
                }
            });
        }
        _ => {}
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_consent_request_calls_the_user_and_updates_the_count() {
        assert!(wants_attention("consent.requested"));
        assert!(!wants_attention("consent.approve"));
        assert!(!wants_attention("tasks.create"));
        for c in [
            "consent.requested",
            "consent.approve",
            "consent.deny",
            "consent.expired",
            "consent.superseded",
        ] {
            assert!(changes_pending(c), "{c}");
        }
        assert!(!changes_pending("tasks.delete"));
    }

    /// Finding: every parked request brought the window to the front.
    #[test]
    fn one_actor_raises_the_window_at_most_every_ten_seconds() {
        let a = Attention::default();
        let t0 = Instant::now();
        assert!(a.should_raise("agent:bot", t0));
        assert!(!a.should_raise("agent:bot", t0 + Duration::from_secs(1)));
        assert!(!a.should_raise("agent:bot", t0 + Duration::from_millis(9_999)));
        assert!(
            a.should_raise("agent:other", t0 + Duration::from_secs(2)),
            "per actor"
        );
        assert!(a.should_raise("agent:bot", t0 + RAISE_EVERY));
        // Old entries do not pile up.
        assert!(a.should_raise("user", t0 + Duration::from_secs(60)));
        assert_eq!(
            a.last.lock().unwrap_or_else(PoisonError::into_inner).len(),
            1
        );
    }

    #[test]
    fn the_ready_status_reports_the_mcp_host() {
        let ready = KeyStatus::Ready {
            kind: "keychain",
            mcp: None,
            mcp_error: None,
        };
        assert_eq!(
            serde_json::to_value(&ready).unwrap_or_default(),
            serde_json::json!({"state": "ready", "kind": "keychain"})
        );
        let failed = KeyStatus::Ready {
            kind: "keychain",
            mcp: None,
            mcp_error: Some("socket in use".into()),
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap_or_default(),
            serde_json::json!({"state": "ready", "kind": "keychain", "mcp_error": "socket in use"})
        );
        let ok = KeyStatus::Ready {
            kind: "passphrase",
            mcp: Some(McpInfo {
                socket: "/d/halp.sock".into(),
            }),
            mcp_error: None,
        };
        assert_eq!(
            serde_json::to_value(&ok).unwrap_or_default()["mcp"]["socket"],
            "/d/halp.sock"
        );
    }

    #[test]
    fn the_tooltip_counts_pending_requests() {
        assert_eq!(pending_tooltip(0), "HalpTask Hub");
        assert!(pending_tooltip(1).contains("1 request waiting"));
        assert!(pending_tooltip(3).contains("3 requests waiting"));
    }
}
