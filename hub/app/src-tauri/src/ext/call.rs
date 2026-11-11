//! `ext_call`: a Tier C widget's `tools/call`, relayed by the trusted host
//! component (§6.3) and run as `module:<pack_id>` (§7, CONTRACT v1.10).
//!
//! It never trusts the request for anything but names: the allow-list is
//! the installed manifest's `calls` intersected with [`HOST_CALLS`] (all
//! T0 reads; Tier C write calls are deferred, §10), the pack must be
//! installed, enabled and trusted to run code, and the policy engine then
//! applies the module rules (T0 `general` only).

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{ExtSource, SUPPORTED, is_pack_id, is_widget_name, kind_of};

/// MCP tool names a widget may call, and the core command each one is.
/// Read-only in P5.
pub const HOST_CALLS: &[(&str, &str)] = &[
    ("tasks_list", "tasks.list"),
    ("tasks_get", "tasks.get"),
    ("types_list", "types.list"),
    ("types_get", "types.get"),
    ("day_state", "day.state"),
    ("focus_get", "focus.get"),
];

/// Most bytes in one call's arguments (§6.3: each message ≤ 256 KiB).
pub const MAX_ARGS_BYTES: usize = 256 * 1024;
/// Most calls per second for one pack, over all its frames. The host
/// component already holds each frame to 20 per second.
pub const MAX_CALLS_PER_SEC: usize = 40;

/// The manifest's calls that the host allows, in manifest order.
pub fn effective_calls(calls: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in calls {
        if HOST_CALLS.iter().any(|(n, _)| n == c) && !out.contains(c) {
            out.push(c.clone());
        }
    }
    out
}

fn command_for(name: &str) -> Option<&'static str> {
    HOST_CALLS.iter().find(|(n, _)| *n == name).map(|(_, c)| *c)
}

fn denied(msg: &str) -> Value {
    json!({"ok": false, "error": {"code": "denied", "message": msg}})
}

fn invalid(msg: &str) -> Value {
    json!({"ok": false, "error": {"code": "invalid_args", "message": msg}})
}

/// A sliding window of calls per pack: at most `max` in any `window`.
/// The default is `ext_call`'s ([`MAX_CALLS_PER_SEC`] per second).
pub struct RateLimit {
    max: usize,
    window: Duration,
    seen: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self::new(MAX_CALLS_PER_SEC, Duration::from_secs(1))
    }
}

impl RateLimit {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            seen: Mutex::new(HashMap::new()),
        }
    }

    pub fn allow(&self, pack_id: &str, now: Instant) -> bool {
        let window = self.window;
        let mut m = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        m.retain(|_, q| {
            q.back()
                .is_some_and(|t| now.saturating_duration_since(*t) < window)
        });
        let q = m.entry(pack_id.to_string()).or_default();
        while q
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= window)
        {
            q.pop_front();
        }
        if q.len() >= self.max {
            return false;
        }
        q.push_back(now);
        true
    }
}

/// Checks and runs one widget call; returns the contract envelope.
/// `source` is `None` while the Hub is locked.
pub fn ext_call(
    source: Option<&dyn ExtSource>,
    rate: &RateLimit,
    now: Instant,
    pack_id: &str,
    widget: &str,
    name: &str,
    arguments: Value,
) -> Value {
    if !SUPPORTED {
        return denied("custom widgets are not yet supported on Windows");
    }
    let Some(source) = source else {
        return json!({"ok": false, "error": {"code": "internal", "message": "the Hub is locked; unlock it first"}});
    };
    if !is_pack_id(pack_id) || !is_widget_name(widget) {
        return denied("unknown widget");
    }
    let Some(command) = command_for(name) else {
        return denied("this call is not available to widgets");
    };
    let args = match arguments {
        Value::Null => json!({}),
        Value::Object(o) => Value::Object(o),
        _ => return invalid("arguments must be an object"),
    };
    if serde_json::to_vec(&args).map_or(true, |b| b.len() > MAX_ARGS_BYTES) {
        return invalid("arguments are too large");
    }
    // The installed manifest decides, never the request.
    let Some(info) = source.widget(pack_id, widget) else {
        return denied("this widget's pack is not installed or is disabled");
    };
    if !info.may_run || info.kind != kind_of(pack_id, widget) {
        return denied("this widget's pack may not run code");
    }
    if !effective_calls(&info.calls).iter().any(|c| c == name) {
        return denied("this widget did not declare this call");
    }
    if !rate.allow(pack_id, now) {
        return denied("too many calls; slow down");
    }
    source.dispatch_as_module(pack_id, command, args)
}

#[cfg(test)]
mod tests {
    use super::super::scheme::tests::fake;
    use super::*;

    fn call(name: &str, args: Value) -> Value {
        ext_call(
            Some(&fake()),
            &RateLimit::default(),
            Instant::now(),
            "acme.sre",
            "oncall",
            name,
            args,
        )
    }

    fn code(v: &Value) -> &str {
        v["error"]["code"].as_str().unwrap_or("")
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn an_allowed_call_runs_as_the_module() {
        let r = call("tasks_list", json!({"limit": 5}));
        assert_eq!(r["ok"], json!(true), "{r}");
        assert_eq!(r["result"]["actor"], json!("module:acme.sre"));
        assert_eq!(r["result"]["command"], json!("tasks.list"));
        assert_eq!(r["result"]["args"], json!({"limit": 5}));
        let r = call("tasks_list", Value::Null);
        assert_eq!(r["result"]["args"], json!({}));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn calls_outside_the_allow_list_are_denied() {
        // Declared by the manifest but not a host call (a write): denied.
        assert_eq!(code(&call("tasks_create", json!({}))), "denied");
        // A host call the manifest did not declare: denied.
        assert_eq!(code(&call("types_list", json!({}))), "denied");
        // Core command spellings and junk are not tool names.
        for n in [
            "tasks.list",
            "dispatch",
            "",
            "tasks_list ",
            "packs_install",
            "consent_approve",
        ] {
            assert_eq!(code(&call(n, json!({}))), "denied", "{n}");
        }
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn disabled_unknown_or_untrusted_packs_are_denied() {
        let (src, rate, now) = (fake(), RateLimit::default(), Instant::now());
        for (p, w) in [
            ("acme.off", "oncall"),
            ("nope.pack", "oncall"),
            ("acme.sre", "other"),
            ("ACME", "oncall"),
            ("acme.sre", "../x"),
        ] {
            let r = ext_call(Some(&src), &rate, now, p, w, "tasks_list", json!({}));
            assert_eq!(code(&r), "denied", "{p}/{w}: {r}");
        }
        assert_eq!(
            code(&ext_call(
                None,
                &rate,
                now,
                "acme.sre",
                "oncall",
                "tasks_list",
                json!({})
            )),
            "internal"
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn arguments_are_bounded() {
        assert_eq!(code(&call("tasks_list", json!([1]))), "invalid_args");
        assert_eq!(code(&call("tasks_list", json!("x"))), "invalid_args");
        let big = "x".repeat(MAX_ARGS_BYTES);
        assert_eq!(
            code(&call("tasks_list", json!({ "q": big }))),
            "invalid_args"
        );
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn calls_are_rate_limited_per_pack() {
        let (src, rate, t0) = (fake(), RateLimit::default(), Instant::now());
        for i in 0..MAX_CALLS_PER_SEC {
            let r = ext_call(
                Some(&src),
                &rate,
                t0,
                "acme.sre",
                "oncall",
                "tasks_list",
                json!({}),
            );
            assert_eq!(r["ok"], json!(true), "call {i}");
        }
        let r = ext_call(
            Some(&src),
            &rate,
            t0,
            "acme.sre",
            "oncall",
            "tasks_list",
            json!({}),
        );
        assert_eq!(code(&r), "denied");
        let r = ext_call(
            Some(&src),
            &rate,
            t0 + Duration::from_secs(1),
            "acme.sre",
            "oncall",
            "tasks_list",
            json!({}),
        );
        assert_eq!(r["ok"], json!(true));
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn ext_call_refuses_on_windows() {
        assert_eq!(code(&call("tasks_list", json!({}))), "denied");
    }

    #[test]
    fn effective_calls_intersect_the_host_list() {
        let c = effective_calls(&[
            "tasks_create".into(),
            "tasks_list".into(),
            "tasks_list".into(),
            "focus_get".into(),
        ]);
        assert_eq!(c, vec!["tasks_list".to_string(), "focus_get".to_string()]);
        for (_, cmd) in HOST_CALLS {
            assert!(matches!(
                *cmd,
                "tasks.list" | "tasks.get" | "types.list" | "types.get" | "day.state" | "focus.get"
            ));
        }
    }
}
