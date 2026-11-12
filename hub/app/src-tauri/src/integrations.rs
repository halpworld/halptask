//! The integration runtime (`halp-integrations`) inside the app process:
//! started with the core, stopped on exit, plus the one command that takes
//! a token from the Settings screen straight to the OS keychain and the one
//! that shows a hint of it (never the token).
//!
//! Tokens never go through `dispatch`, never reach the core, and are never
//! logged, echoed or put in an event: `integrations_set_token` answers only
//! `Ok(())` or an error code.

use std::sync::{Arc, Mutex, PoisonError};

use halp_core::Core;
use halp_integrations::{KeychainSecrets, Opener, Runtime, RuntimeOptions};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use zeroize::Zeroizing;

/// The running runtime, if the core is open.
#[derive(Default)]
pub struct Slot(Mutex<Option<Runtime>>);

impl Slot {
    fn get(&self) -> Option<Runtime> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// Token commands answer the main window only (the capability grants them
/// to `main` alone; this is the second lock).
fn main_only(label: &str) -> Result<(), String> {
    if label == crate::MAIN {
        Ok(())
    } else {
        Err("forbidden".into())
    }
}

/// Opens an authorization page in the default browser, after checking it
/// is `https` on the host the OAuth provider was configured with.
fn opener() -> Opener {
    Arc::new(|url: &str, expected_host: &str| {
        let u = url::Url::parse(url).map_err(|_| "not a URL".to_string())?;
        if u.scheme() != "https" || u.host_str() != Some(expected_host) || expected_host.is_empty()
        {
            return Err("refusing to open a non-https or unexpected authorization URL".into());
        }
        if !u.username().is_empty() || u.password().is_some() {
            return Err("refusing to open a URL with credentials".into());
        }
        let s = u.as_str();
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("/usr/bin/open");
        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = std::process::Command::new("rundll32.exe");
            c.arg("url.dll,FileProtocolHandler");
            c
        };
        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        let mut cmd = std::process::Command::new("/usr/bin/xdg-open");
        cmd.arg(s)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|_| "could not open the browser".to_string())
    })
}

/// Starts the runtime for `core` (call once the core is open).
pub fn start(app: &AppHandle, core: &Arc<Core>, data_dir: &std::path::Path) {
    let mut opts = RuntimeOptions::new(
        data_dir.join("integrations"),
        Arc::new(KeychainSecrets::default()),
    );
    opts.opener = Some(opener());
    let handle = tauri::async_runtime::handle().inner().clone();
    match Runtime::start(core, opts, handle) {
        Ok(rt) => {
            *app.state::<Slot>()
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(rt)
        }
        Err(e) => eprintln!("halp: integrations unavailable: {e}"),
    }
}

/// Stops syncs and kills MCP server processes.
pub fn stop(app: &AppHandle) {
    if let Some(rt) = app
        .state::<Slot>()
        .0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take()
    {
        rt.stop();
    }
}

/// Stores a personal access token / API key for integration `id` in the OS
/// keychain. Error codes: `not_running`, `unknown_integration`, `empty`,
/// `too_long` (over 4 KiB), `not_printable_ascii`, `invalid_url` (a
/// calendar feed that is not an https URL), `keychain_error`, `forbidden`
/// (not the main window).
#[tauri::command]
pub async fn integrations_set_token(
    window: WebviewWindow,
    slot: State<'_, Slot>,
    id: String,
    token: String,
) -> Result<(), String> {
    let token = Zeroizing::new(token);
    main_only(window.label())?;
    let rt = slot.get().ok_or("not_running")?;
    if id.is_empty() || id.len() > 64 {
        return Err("unknown_integration".into());
    }
    tauri::async_runtime::spawn_blocking(move || rt.set_token(&id, &token))
        .await
        .map_err(|_| "internal".to_string())?
        .map_err(str::to_string)
}

/// What Settings may show of the stored secret for integration `id`: a
/// connector-made hint such as a calendar feed's host. Never the secret;
/// `None` when nothing is stored, the connector shows nothing, or the
/// runtime is not running. `forbidden` outside the main window.
#[tauri::command]
pub async fn integrations_secret_hint(
    window: WebviewWindow,
    slot: State<'_, Slot>,
    id: String,
) -> Result<Option<String>, String> {
    main_only(window.label())?;
    let Some(rt) = slot.get() else {
        return Ok(None);
    };
    if id.is_empty() || id.len() > 64 {
        return Ok(None);
    }
    tauri::async_runtime::spawn_blocking(move || rt.secret_hint(&id))
        .await
        .map_err(|_| "internal".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_opener_refuses_unexpected_urls() {
        let o = opener();
        assert!(o("http://github.com/login/oauth/authorize", "github.com").is_err());
        assert!(o("https://evil.example/authorize", "github.com").is_err());
        assert!(o("https://u:p@github.com/x", "github.com").is_err());
        assert!(o("file:///etc/passwd", "").is_err());
    }

    #[test]
    fn token_commands_are_for_the_main_window_only() {
        assert!(main_only(crate::MAIN).is_ok());
        assert_eq!(main_only(crate::CAPTURE), Err("forbidden".into()));
        assert!(main_only("").is_err());
    }
}
