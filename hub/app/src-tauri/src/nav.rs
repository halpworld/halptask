//! Navigation policy: the webview only ever shows the bundled app.

use tauri::Url;

/// The bundled app's origins (macOS/Linux `tauri://localhost`, Windows
/// `http(s)://tauri.localhost`) and, in `tauri dev` only, the Vite server.
pub fn is_app_url(url: &Url, dev: bool) -> bool {
    match (url.scheme(), url.host_str()) {
        ("tauri", Some("localhost")) => true,
        ("http" | "https", Some("tauri.localhost")) => true,
        ("http", Some("localhost")) if dev => url.port() == Some(1420),
        _ => false,
    }
}

/// A Tier C widget frame (`halpext://<pack_id>/...`, §6.4 condition 4).
/// `on_navigation` fires for subframes too, so without this the iframe never
/// loads. Only the `halpext` scheme with a well-formed pack id as host, no
/// port and no user info; never on Windows, where Tier C is off. The host
/// component tears a frame down on any load after its first, and the scheme
/// serves documents only with a one-time ticket.
pub fn is_ext_frame_url(url: &Url) -> bool {
    crate::ext::SUPPORTED
        && url.scheme() == crate::ext::SCHEME
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(crate::ext::is_pack_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).expect("url")
    }

    #[test]
    fn only_the_app_is_allowed() {
        assert!(is_app_url(&u("tauri://localhost/index.html"), false));
        assert!(is_app_url(&u("http://tauri.localhost/"), false));
        assert!(!is_app_url(&u("https://example.com/"), false));
        assert!(!is_app_url(&u("http://localhost:1420/"), false));
        assert!(is_app_url(&u("http://localhost:1420/"), true));
        assert!(!is_app_url(&u("http://localhost:8080/"), true));
        assert!(!is_app_url(&u("file:///etc/passwd"), true));
        assert!(!is_app_url(&u("tauri://evil.com/"), false));
        // The app policy itself never admits widget frames.
        assert!(!is_app_url(
            &u("halpext://acme.sre/oncall/index.html"),
            true
        ));
    }

    #[test]
    fn widget_frames_are_halpext_only() {
        let ok = is_ext_frame_url(&u("halpext://acme.sre/oncall/index.html?ticket=00"));
        assert_eq!(ok, cfg!(not(target_os = "windows")));
        for bad in [
            "halpext://acme/oncall/index.html",
            "halpext://acme.sre:9/oncall/index.html",
            "halpext://u@acme.sre/oncall/index.html",
            "halpext://ACME.SRE/x",
            "http://halpext.localhost/acme.sre/oncall/index.html",
            "https://acme.sre/",
            "tauri://localhost/index.html",
            "file:///etc/passwd",
            "data:text/html,hi",
            "javascript:alert(1)",
            "blob:halpext://acme.sre/x",
        ] {
            assert!(!is_ext_frame_url(&u(bad)), "{bad}");
        }
    }
}
