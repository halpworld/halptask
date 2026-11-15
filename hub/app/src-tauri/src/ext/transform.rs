//! `ext_run_transform`: runs the Tier B transform behind a placed
//! `computed` widget (docs/hub/P5-EXTENSIONS.md §5.3, §5.4) and returns
//! its checked output for the UI.
//!
//! The request carries names only. `halp_ext::wiring::run_computed` takes
//! everything else from the core: the module and its wasm from the
//! installed manifest (enabled pack, trust that may run code, file hash
//! checked again), the config from the widget on the current layout, and
//! the inputs through `Core::dispatch` as `module:<pack_id>` (T0 `general`
//! reads only; every read audited). The run happens off the core lock in
//! `halp-ext`'s sandbox, with its fuel, memory, time and output limits.
//!
//! Tier B is not platform-limited: unlike Tier C it has no frame and no
//! web content, so it runs on Windows too.

use std::time::{Duration, Instant};

use halp_core::{Core, PackTrust};
use halp_ext::ExtError;
use halp_ext::wiring::{self, ComputedError, ComputedRun};
use serde_json::{Value, json};

use super::call::RateLimit;
use super::{is_pack_id, is_widget_name};

/// Most runs per pack in [`RATE_WINDOW`], over all its widgets. The UI
/// already debounces each widget to one run per 2 s (§5.3).
pub const MAX_RUNS_PER_WINDOW: usize = 20;
pub const RATE_WINDOW: Duration = Duration::from_secs(10);

/// The rate limit for transform runs.
pub fn rate_limit() -> RateLimit {
    RateLimit::new(MAX_RUNS_PER_WINDOW, RATE_WINDOW)
}

/// The shell's view of the core for Tier B. [`Core`] implements it; tests
/// use a fake.
pub trait TransformSource: Send + Sync {
    fn run_computed(
        &self,
        pack_id: &str,
        name: &str,
        widget_id: &str,
    ) -> Result<ComputedRun, ComputedError>;
}

impl TransformSource for Core {
    fn run_computed(
        &self,
        pack_id: &str,
        name: &str,
        widget_id: &str,
    ) -> Result<ComputedRun, ComputedError> {
        wiring::run_computed(self, pack_id, name, widget_id)
    }
}

/// A layout widget id: 1 to 64 ASCII letters, digits, `-` or `_`.
pub fn is_widget_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

fn error(code: &str, message: &str) -> Value {
    json!({"ok": false, "error": {"code": code, "message": message}})
}

/// Checks and runs one transform; returns the contract-style envelope.
/// `source` is `None` while the Hub is locked.
///
/// - `ok: true`: `result` is `{output, trust, title, publisher}`, plus
///   `log` (the module's `halp.log` text) for dev packs only.
/// - `denied`: bad names, or the pack is not installed, is disabled, may
///   not run code, or the widget is not a `computed` widget for this
///   transform.
/// - `busy`: rate-limited, or the run queue is full (the UI keeps the last
///   output and tries again later).
/// - `module_failed`: the module failed; `reason` is the `halp-ext` code
///   (`timeout`, `fuel_exhausted`, `schema_mismatch`, ...).
/// - `internal`: locked, or the extension host is not running.
pub fn run_transform(
    source: Option<&dyn TransformSource>,
    rate: &RateLimit,
    now: Instant,
    pack_id: &str,
    name: &str,
    widget_id: &str,
) -> Value {
    let Some(source) = source else {
        return error("internal", "the Hub is locked; unlock it first");
    };
    if !is_pack_id(pack_id) || !is_widget_name(name) || !is_widget_id(widget_id) {
        return error("denied", "unknown transform");
    }
    if !rate.allow(pack_id, now) {
        return error("busy", "too many transform runs; slow down");
    }
    match source.run_computed(pack_id, name, widget_id) {
        Ok(run) => {
            let mut result = json!({
                "output": run.output,
                "trust": run.trust.as_str(),
                "title": run.title,
                "publisher": run.publisher,
            });
            if run.trust == PackTrust::Dev {
                result["log"] = json!(run.log);
            }
            json!({"ok": true, "result": result})
        }
        Err(ComputedError::Unavailable(m)) => error("denied", &m),
        Err(ComputedError::Internal(m)) => error("internal", &m),
        Err(ComputedError::Module(ExtError::Busy)) => {
            error("busy", "the extension host is busy; try again shortly")
        }
        Err(ComputedError::Module(e)) => json!({
            "ok": false,
            "error": {"code": "module_failed", "reason": e.code(), "message": e.to_string()}
        }),
    }
}

/// Runs transform `name` of `pack_id` for the `computed` widget
/// `widget_id`.
#[tauri::command]
pub async fn ext_run_transform(
    window: tauri::WebviewWindow,
    app: tauri::State<'_, crate::AppState>,
    pack_id: String,
    name: String,
    widget_id: String,
) -> Result<Value, String> {
    super::main_only(window.label())?;
    let core = app.core();
    let handle = tauri::Manager::app_handle(&window).clone();
    tauri::async_runtime::spawn_blocking(move || {
        use tauri::Manager;
        let ext = handle.state::<super::ExtState>();
        run_transform(
            core.as_deref().map(|c| c as &dyn TransformSource),
            &ext.transform_rate,
            Instant::now(),
            &pack_id,
            &name,
            &widget_id,
        )
    })
    .await
    .map_err(|_| "internal".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Runs `acme.sre/burn` for widget `w-1` only; records every call.
    struct Fake {
        trust: PackTrust,
        fail: Option<ComputedError>,
        calls: Mutex<Vec<(String, String, String)>>,
    }

    fn fake(trust: PackTrust) -> Fake {
        Fake {
            trust,
            fail: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    impl TransformSource for Fake {
        fn run_computed(
            &self,
            pack_id: &str,
            name: &str,
            widget_id: &str,
        ) -> Result<ComputedRun, ComputedError> {
            self.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((pack_id.into(), name.into(), widget_id.into()));
            if let Some(e) = &self.fail {
                return Err(e.clone());
            }
            if (pack_id, name, widget_id) != ("acme.sre", "burn", "w-1") {
                return Err(ComputedError::Unavailable("not installed".into()));
            }
            Ok(ComputedRun {
                output: json!({"value": 3}),
                log: "hello".into(),
                trust: self.trust,
                title: "Acme SRE".into(),
                publisher: "Acme".into(),
            })
        }
    }

    fn run(src: &Fake, rate: &RateLimit, pack: &str, name: &str, w: &str) -> Value {
        run_transform(Some(src), rate, Instant::now(), pack, name, w)
    }

    #[test]
    fn runs_and_returns_the_checked_output() {
        let (src, rate) = (fake(PackTrust::Community), rate_limit());
        let v = run(&src, &rate, "acme.sre", "burn", "w-1");
        assert_eq!(v["ok"], true);
        assert_eq!(v["result"]["output"], json!({"value": 3}));
        assert_eq!(v["result"]["trust"], "community");
        assert_eq!(v["result"]["publisher"], "Acme");
        assert_eq!(v["result"]["title"], "Acme SRE");
        assert!(
            v["result"].get("log").is_none(),
            "log is for dev packs only"
        );
        let dev = fake(PackTrust::Dev);
        let v = run(&dev, &rate, "acme.sre", "burn", "w-1");
        assert_eq!(v["result"]["log"], "hello");
        assert_eq!(v["result"]["trust"], "dev");
    }

    #[test]
    fn bad_names_never_reach_the_core() {
        let (src, rate) = (fake(PackTrust::Community), rate_limit());
        for (p, n, w) in [
            ("../x", "burn", "w-1"),
            ("acme", "burn", "w-1"),
            ("acme.sre", "Burn", "w-1"),
            ("acme.sre", "a/b", "w-1"),
            ("acme.sre", "burn", ""),
            ("acme.sre", "burn", "w 1"),
            ("acme.sre", "burn", "w/1"),
            ("acme.sre", "burn", &"w".repeat(65)),
        ] {
            let v = run(&src, &rate, p, n, w);
            assert_eq!(v["error"]["code"], "denied", "{p} {n} {w}");
        }
        assert!(
            src.calls
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_empty()
        );
    }

    #[test]
    fn unavailable_transforms_are_denied() {
        let (src, rate) = (fake(PackTrust::Community), rate_limit());
        for (p, n, w) in [
            ("acme.off", "burn", "w-1"),
            ("acme.sre", "other", "w-1"),
            ("acme.sre", "burn", "w-2"),
        ] {
            assert_eq!(run(&src, &rate, p, n, w)["error"]["code"], "denied");
        }
    }

    #[test]
    fn locked_and_failures_map_to_envelopes() {
        let rate = rate_limit();
        let v = run_transform(None, &rate, Instant::now(), "acme.sre", "burn", "w-1");
        assert_eq!(v["error"]["code"], "internal");
        let mut src = fake(PackTrust::Community);
        src.fail = Some(ComputedError::Module(ExtError::Timeout));
        let v = run(&src, &rate, "acme.sre", "burn", "w-1");
        assert_eq!(v["error"]["code"], "module_failed");
        assert_eq!(v["error"]["reason"], "timeout");
        src.fail = Some(ComputedError::Module(ExtError::Busy));
        assert_eq!(
            run(&src, &rate, "acme.sre", "burn", "w-1")["error"]["code"],
            "busy"
        );
        src.fail = Some(ComputedError::Internal("host not running".into()));
        assert_eq!(
            run(&src, &rate, "acme.sre", "burn", "w-1")["error"]["code"],
            "internal"
        );
    }

    #[test]
    fn runs_are_rate_limited_per_pack() {
        let (src, rate, t0) = (fake(PackTrust::Community), rate_limit(), Instant::now());
        for _ in 0..MAX_RUNS_PER_WINDOW {
            let v = run_transform(Some(&src), &rate, t0, "acme.sre", "burn", "w-1");
            assert_eq!(v["ok"], true);
        }
        let v = run_transform(Some(&src), &rate, t0, "acme.sre", "burn", "w-1");
        assert_eq!(v["error"]["code"], "busy");
        // Another pack has its own budget.
        let v = run_transform(Some(&src), &rate, t0, "acme.ops", "burn", "w-1");
        assert_eq!(
            v["error"]["code"], "denied",
            "not rate-limited, just unknown"
        );
        // After the window, runs again.
        let later = t0 + RATE_WINDOW;
        let v = run_transform(Some(&src), &rate, later, "acme.sre", "burn", "w-1");
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn tier_b_is_not_platform_limited() {
        // Unlike `ext_call`, nothing here checks `SUPPORTED`: the same
        // answer on every platform.
        let (src, rate) = (fake(PackTrust::Community), rate_limit());
        assert_eq!(run(&src, &rate, "acme.sre", "burn", "w-1")["ok"], true);
    }
}
