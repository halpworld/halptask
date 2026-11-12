//! The `halpext://` handler (§6.2 with the §6.4 changes).
//!
//! URL: `halpext://<pack_id>/<widget>/<path>`, where `<path>` is relative
//! to the widget's root. It serves only files the installed manifest lists
//! for that widget, from verified bytes (re-hashed here), with an allow-list
//! of types. Everything else is a 404 that names nothing.
//!
//! Every response, 404s included, carries the CSP, `nosniff` and
//! `no-referrer`. The handler cannot tell which frame asks (navigations and
//! no-cors loads have no Origin or Referer; CORS loads send `Origin: null`),
//! so isolation between packs rests on the exact-host lists in the CSP.
//! `Cross-Origin-Resource-Policy` is deliberately absent: it breaks every
//! mount after the first (§6.4).

use sha2::{Digest, Sha256};
use tauri::http::{Method, Response, StatusCode, header};

use super::tickets::{Tickets, is_ticket};
use super::{ExtSource, SCHEME, SUPPORTED, is_pack_id, is_widget_name};

/// Most bytes in a path below the widget (`halp-pack` `MAX_PATH_BYTES`).
const MAX_PATH_BYTES: usize = 255;
/// Most segments in a path below the widget.
const MAX_PATH_DEPTH: usize = 6;

/// The CSP for every response about `pack_id` (§6.4 condition 2). The
/// frame's origin is opaque, so `'self'` would match nothing: each list
/// names the pack's exact origin.
pub fn csp(pack_id: &str) -> String {
    let s = format!("{SCHEME}://{pack_id}");
    format!(
        "default-src 'none'; script-src {s}; style-src {s} 'unsafe-inline'; img-src {s} data:; \
         font-src {s}; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; \
         sandbox allow-scripts"
    )
}

/// The CSP for a 404 whose pack is not known: nothing at all.
pub const CSP_NONE: &str = "default-src 'none'; form-action 'none'; base-uri 'none'; sandbox";

/// The content type for a file name, from the allow-list `html js mjs css
/// png woff2 json`; `None` for anything else. SVG is left out: it can run
/// script as a document, and only `.html` documents are ticketed.
pub fn mime_for(path: &str) -> Option<&'static str> {
    let ext = path.rsplit_once('.')?.1;
    Some(match ext {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "png" => "image/png",
        "woff2" => "font/woff2",
        "json" => "application/json",
        _ => return None,
    })
}

/// A relative bundle path (§3.2): `[a-z0-9_./-]`, no empty, `.` or `..`
/// segments, at most [`MAX_PATH_DEPTH`] segments and [`MAX_PATH_BYTES`].
/// `%` is not allowed, so no encoded form of anything gets through.
pub fn is_rel_path(p: &str) -> bool {
    !p.is_empty()
        && p.len() <= MAX_PATH_BYTES
        && p.bytes().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'.' | b'/' | b'-')
        })
        && p.split('/').count() <= MAX_PATH_DEPTH
        && p.split('/')
            .all(|seg| !seg.is_empty() && seg != "." && seg != "..")
}

/// A parsed, checked request target.
#[derive(Debug, PartialEq, Eq)]
pub struct Target {
    pub pack_id: String,
    pub widget: String,
    pub path: String,
    pub ticket: Option<String>,
}

/// Parses `halpext://<pack_id>/<widget>/<path>[?ticket=<hex>]`. Anything
/// else (another scheme, user info, a port, a fragment, a bad id or path,
/// other query keys) is `None`.
pub fn parse_target(raw: &str) -> Option<Target> {
    // Reject encodings and odd characters before the URL parser can
    // normalise them away.
    if raw.len() > 1024
        || raw
            .bytes()
            .any(|c| c == b'%' || c == b'\\' || !c.is_ascii_graphic())
    {
        return None;
    }
    // The URL parser resolves `.` and `..` segments; refuse them first so
    // `<widget>/../<other>` can never reach another widget's files.
    let authority_and_path = raw.strip_prefix("halpext://")?.split(['?', '#']).next()?;
    if authority_and_path
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return None;
    }
    let u = url::Url::parse(raw).ok()?;
    if u.scheme() != SCHEME
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.fragment().is_some()
    {
        return None;
    }
    let pack_id = u.host_str()?;
    if !is_pack_id(pack_id) {
        return None;
    }
    let (widget, path) = u.path().strip_prefix('/')?.split_once('/')?;
    if !is_widget_name(widget) || !is_rel_path(path) {
        return None;
    }
    let ticket = match u.query() {
        None => None,
        Some(q) => {
            let t = q.strip_prefix("ticket=")?;
            if !is_ticket(t) {
                return None;
            }
            Some(t.to_string())
        }
    };
    Some(Target {
        pack_id: pack_id.to_string(),
        widget: widget.to_string(),
        path: path.to_string(),
        ticket,
    })
}

fn respond(
    status: StatusCode,
    csp: &str,
    content_type: Option<&str>,
    body: Vec<u8>,
) -> Response<Vec<u8>> {
    let mut b = Response::builder()
        .status(status)
        .header(header::CONTENT_SECURITY_POLICY, csp)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "no-referrer")
        // Disabled, uninstalled or updated packs must not live on in a cache.
        .header(header::CACHE_CONTROL, "no-store");
    if let Some(ct) = content_type {
        b = b.header(header::CONTENT_TYPE, ct);
    }
    b.body(body).unwrap_or_else(|_| Response::new(Vec::new()))
}

fn not_found(csp: &str) -> Response<Vec<u8>> {
    respond(StatusCode::NOT_FOUND, csp, None, Vec::new())
}

/// Serves one request. `source` is `None` while the Hub is locked.
/// `webview` is the label of the asking webview: only the main window
/// shows widgets.
pub fn serve(
    source: Option<&dyn ExtSource>,
    tickets: &Tickets,
    webview: &str,
    method: &Method,
    uri: &str,
) -> Response<Vec<u8>> {
    if !SUPPORTED || webview != crate::MAIN || method != Method::GET {
        return not_found(CSP_NONE);
    }
    let Some(t) = parse_target(uri) else {
        return not_found(CSP_NONE);
    };
    let csp = csp(&t.pack_id);
    let Some(mime) = mime_for(&t.path) else {
        return not_found(&csp);
    };
    let Some(source) = source else {
        return not_found(&csp);
    };
    // A document needs a fresh ticket from `ext_widget` for this widget.
    if t.path.ends_with(".html")
        && !t
            .ticket
            .as_deref()
            .is_some_and(|k| tickets.redeem(k, &t.pack_id, &t.widget))
    {
        return not_found(&csp);
    }
    let Some(info) = source.widget(&t.pack_id, &t.widget) else {
        return not_found(&csp);
    };
    if !info.may_run || info.kind != super::kind_of(&t.pack_id, &t.widget) {
        return not_found(&csp);
    }
    let Some(file) = source.file(&t.pack_id, &t.widget, &t.path) else {
        return not_found(&csp);
    };
    // Re-hash: serve only bytes that match the installed index.
    if !hex::encode(Sha256::digest(&file.bytes)).eq_ignore_ascii_case(&file.sha256) {
        eprintln!(
            "halp: {} widget {} file {} failed its hash check; not served",
            t.pack_id, t.widget, t.path
        );
        return not_found(&csp);
    }
    respond(StatusCode::OK, &csp, Some(mime), file.bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::super::{ServedFile, WidgetInfo};
    use super::*;
    use serde_json::Value;
    use std::collections::HashMap;

    pub struct Fake {
        pub widgets: HashMap<(String, String), WidgetInfo>,
        pub files: HashMap<(String, String, String), ServedFile>,
    }

    fn sha(b: &[u8]) -> String {
        hex::encode(Sha256::digest(b))
    }

    pub fn fake() -> Fake {
        let mut widgets = HashMap::new();
        let w = |pack: &str, name: &str, trust: &'static str, may_run: bool| WidgetInfo {
            kind: format!("x.{pack}.{name}"),
            calls: vec!["tasks_list".into(), "tasks_create".into()],
            entry: "index.html".into(),
            trust,
            may_run,
        };
        widgets.insert(
            ("acme.sre".into(), "oncall".into()),
            w("acme.sre", "oncall", "community", true),
        );
        widgets.insert(
            ("acme.off".into(), "oncall".into()),
            w("acme.off", "oncall", "unsigned", false),
        );
        let mut files = HashMap::new();
        let mut add = |name: &str, bytes: &[u8], good: bool| {
            let h = if good {
                sha(bytes)
            } else {
                sha(b"something else")
            };
            files.insert(
                ("acme.sre".into(), "oncall".into(), name.to_string()),
                ServedFile {
                    bytes: bytes.to_vec(),
                    sha256: h,
                },
            );
        };
        add(
            "index.html",
            b"<!doctype html><script src=w.js></script>",
            true,
        );
        add("w.js", b"console.log(1)", true);
        add("img/a.png", b"\x89PNG", true);
        add("tampered.js", b"evil()", false);
        add("notes.txt", b"hi", true);
        files.insert(
            ("acme.off".into(), "oncall".into(), "w.js".into()),
            ServedFile {
                bytes: b"x".to_vec(),
                sha256: sha(b"x"),
            },
        );
        Fake { widgets, files }
    }

    impl ExtSource for Fake {
        fn widget(&self, p: &str, w: &str) -> Option<WidgetInfo> {
            self.widgets.get(&(p.to_string(), w.to_string())).cloned()
        }
        fn file(&self, p: &str, w: &str, path: &str) -> Option<ServedFile> {
            self.files
                .get(&(p.to_string(), w.to_string(), path.to_string()))
                .cloned()
        }
        fn dispatch_as_module(&self, pack_id: &str, command: &str, args: Value) -> Value {
            serde_json::json!({"ok": true, "result": {"actor": format!("module:{pack_id}"), "command": command, "args": args}})
        }
    }

    fn get(src: &Fake, tickets: &Tickets, uri: &str) -> Response<Vec<u8>> {
        serve(Some(src), tickets, crate::MAIN, &Method::GET, uri)
    }

    fn h(r: &Response<Vec<u8>>, name: header::HeaderName) -> &str {
        r.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    }

    #[test]
    fn the_csp_is_exactly_the_spike_verified_one() {
        assert_eq!(
            csp("acme.sre"),
            "default-src 'none'; script-src halpext://acme.sre; style-src halpext://acme.sre 'unsafe-inline'; \
             img-src halpext://acme.sre data:; font-src halpext://acme.sre; connect-src 'none'; frame-src 'none'; \
             form-action 'none'; base-uri 'none'; sandbox allow-scripts"
        );
        assert!(!csp("a.b").contains("'self'"));
        assert!(!csp("a.b").contains("unsafe-eval"));
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn listed_files_are_served_with_the_headers() {
        let (src, t) = (fake(), Tickets::default());
        let r = get(&src, &t, "halpext://acme.sre/oncall/w.js");
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(r.body(), b"console.log(1)");
        assert_eq!(
            h(&r, header::CONTENT_TYPE),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(h(&r, header::CONTENT_SECURITY_POLICY), csp("acme.sre"));
        assert_eq!(h(&r, header::X_CONTENT_TYPE_OPTIONS), "nosniff");
        assert_eq!(h(&r, header::REFERRER_POLICY), "no-referrer");
        assert!(
            r.headers().get("cross-origin-resource-policy").is_none(),
            "CORP breaks remounts (§6.4)"
        );
        let r = get(&src, &t, "halpext://acme.sre/oncall/img/a.png");
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(h(&r, header::CONTENT_TYPE), "image/png");
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn the_entry_document_needs_a_fresh_ticket() {
        let (src, t) = (fake(), Tickets::default());
        assert_eq!(
            get(&src, &t, "halpext://acme.sre/oncall/index.html").status(),
            StatusCode::NOT_FOUND
        );
        let k = t.issue("acme.sre", "oncall");
        let url = format!("halpext://acme.sre/oncall/index.html?ticket={k}");
        let r = get(&src, &t, &url);
        assert_eq!(r.status(), StatusCode::OK);
        assert_eq!(h(&r, header::CONTENT_TYPE), "text/html; charset=utf-8");
        assert_eq!(h(&r, header::CONTENT_SECURITY_POLICY), csp("acme.sre"));
        assert_eq!(
            get(&src, &t, &url).status(),
            StatusCode::NOT_FOUND,
            "used up"
        );
        let other = t.issue("acme.off", "oncall");
        assert_eq!(
            get(
                &src,
                &t,
                &format!("halpext://acme.sre/oncall/index.html?ticket={other}")
            )
            .status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn everything_else_is_a_404_with_the_csp() {
        let (src, t) = (fake(), Tickets::default());
        for uri in [
            "halpext://acme.sre/oncall/missing.js",  // unlisted
            "halpext://acme.sre/oncall/tampered.js", // hash mismatch
            "halpext://acme.sre/oncall/notes.txt",   // type not allowed
            "halpext://acme.sre/other/w.js",         // unknown widget
            "halpext://nope.pack/oncall/w.js",       // unknown pack
            "halpext://acme.off/oncall/w.js",        // trust or state does not allow code
            "halpext://acme.sre/oncall/../oncall/w.js",
            "halpext://acme.sre/oncall/%2e%2e/w.js",
            "halpext://acme.sre/oncall/W.JS",
            "halpext://acme.sre/oncall/w.js#x",
            "halpext://acme.sre/oncall/w.js?x=1",
            "halpext://acme.sre:80/oncall/w.js",
            "halpext://u@acme.sre/oncall/w.js",
            "halpext://acme.sre/oncall/",
            "halpext://acme.sre/oncall",
            "halpext://acme.sre/",
            "halpext://ACME.sre/oncall/w.js",
            "halpext://acme/oncall/w.js",
            "tauri://localhost/index.html",
            "http://halpext.localhost/acme.sre/oncall/w.js",
            "halpext://acme.sre/oncall/a//w.js",
            "halpext://acme.sre/oncall/a b.js",
        ] {
            let r = get(&src, &t, uri);
            assert_eq!(r.status(), StatusCode::NOT_FOUND, "{uri}");
            assert!(r.body().is_empty(), "{uri}");
            assert!(
                h(&r, header::CONTENT_SECURITY_POLICY).starts_with("default-src 'none'"),
                "{uri}"
            );
            assert!(
                h(&r, header::CONTENT_SECURITY_POLICY).contains("sandbox"),
                "{uri}"
            );
            assert_eq!(h(&r, header::X_CONTENT_TYPE_OPTIONS), "nosniff", "{uri}");
        }
    }

    #[test]
    fn only_gets_from_the_main_window_while_unlocked() {
        let (src, t) = (fake(), Tickets::default());
        let u = "halpext://acme.sre/oncall/w.js";
        assert_eq!(
            serve(Some(&src), &t, crate::CAPTURE, &Method::GET, u).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            serve(Some(&src), &t, crate::MAIN, &Method::POST, u).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            serve(None, &t, crate::MAIN, &Method::GET, u).status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    #[cfg(target_os = "windows")]
    fn nothing_is_served_on_windows() {
        let (src, t) = (fake(), Tickets::default());
        assert_eq!(
            get(&src, &t, "halpext://acme.sre/oncall/w.js").status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn paths_and_types() {
        for ok in ["index.html", "a/b/c.js", "x_y-z.mjs", "a/b/c/d/e/f.css"] {
            assert!(is_rel_path(ok), "{ok}");
        }
        for bad in [
            "",
            "/a.js",
            "a/",
            "a//b",
            "./a",
            "a/../b",
            "..",
            "A.js",
            "a%2f.js",
            "a\\b",
            "a/b/c/d/e/f/g.js",
            &"a".repeat(256),
        ] {
            assert!(!is_rel_path(bad), "{bad}");
        }
        assert_eq!(mime_for("a.woff2"), Some("font/woff2"));
        assert_eq!(mime_for("a.json"), Some("application/json"));
        for bad in [
            "a.svg",
            "a.xhtml",
            "a.wasm",
            "a.htm",
            "a",
            "a.txt",
            "a.html.exe",
        ] {
            assert_eq!(mime_for(bad), None, "{bad}");
        }
    }
}
