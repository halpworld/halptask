//! OAuth 2.1 + PKCE against a mock provider: success, state mismatch,
//! timeout, refresh.
#![allow(clippy::unwrap_used)]

use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use halp_integrations::oauth::{self, AuthError, OAuthProvider, Opener};
use halp_integrations::{Http, HttpOptions};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use url::Url;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn http() -> Http {
    Http::new(HttpOptions {
        allow_loopback_http: true,
        ..HttpOptions::default()
    })
    .unwrap()
}

fn provider(s: &MockServer) -> OAuthProvider {
    OAuthProvider {
        authorize_url: Url::parse(&format!("{}/authorize", s.uri())).unwrap(),
        token_url: Url::parse(&format!("{}/token", s.uri())).unwrap(),
        client_id: "hub-test".into(),
        scopes: vec!["repo".into(), "read:org".into()],
    }
}

/// A "browser" that follows the redirect with `state` rewritten by `f`.
fn browser(seen: Arc<Mutex<Vec<String>>>, f: fn(&str) -> String) -> Opener {
    Arc::new(move |url: &str, host: &str| {
        seen.lock().unwrap().push(format!("{host} {url}"));
        let u = Url::parse(url).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        let redirect = Url::parse(&q["redirect_uri"]).unwrap();
        let state = f(&q["state"]);
        let target = format!("{}?code=the-code&state={state}", redirect.path());
        let addr = format!("127.0.0.1:{}", redirect.port().unwrap());
        std::thread::spawn(move || {
            let mut s = std::net::TcpStream::connect(addr).unwrap();
            write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
        });
        Ok(())
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn code_flow_with_pkce() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=authorization_code"))
        .and(body_string_contains("code=the-code"))
        .and(body_string_contains("code_verifier="))
        .and(body_string_contains("client_id=hub-test"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token": "at-1", "refresh_token": "rt-1", "expires_in": 3600, "token_type": "bearer"}),
        ))
        .expect(1)
        .mount(&s)
        .await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let t = oauth::authorize(
        &http(),
        &provider(&s),
        &browser(seen.clone(), str::to_string),
        Duration::from_secs(10),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(t.access_token, "at-1");
    assert_eq!(t.refresh_token.as_deref(), Some("rt-1"));
    assert!(t.expires_at.is_some());
    let opened = seen.lock().unwrap()[0].clone();
    assert!(opened.starts_with("127.0.0.1 "), "{opened}");
    assert!(opened.contains("code_challenge_method=S256"));
    assert!(opened.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A"));
    assert!(opened.contains("scope=repo+read%3Aorg"));
    assert!(!opened.contains("code_verifier"));

    // Refresh keeps the old refresh token when the provider omits it.
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("grant_type=refresh_token"))
        .and(body_string_contains("refresh_token=rt-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "at-2"})))
        .mount(&s)
        .await;
    let r = oauth::refresh(&http(), &provider(&s), "rt-1", &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(r.access_token, "at-2");
    assert_eq!(r.refresh_token.as_deref(), Some("rt-1"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_state_is_rejected_and_no_code_is_exchanged() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"access_token": "x"})))
        .expect(0)
        .mount(&s)
        .await;
    let r = oauth::authorize(
        &http(),
        &provider(&s),
        &browser(Arc::default(), |_| "forged".into()),
        Duration::from_secs(1),
        &CancellationToken::new(),
    )
    .await;
    // Ignored, so the flow runs to its timeout and says why.
    assert_eq!(r.unwrap_err(), AuthError::StateMismatch);
}

/// Sends `GET target` to the loopback port and reads the answer.
fn hit(port: u16, target: &str) {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(s, "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n").unwrap();
    let mut out = String::new();
    let _ = s.read_to_string(&mut out);
}

/// Another local process (or a web page) that finds the port can neither
/// end the flow with a forged callback nor stall it with a connection
/// that never sends anything.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forged_and_stalled_callbacks_do_not_end_the_flow() {
    let s = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/token"))
        .and(body_string_contains("code=the-code"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token": "at-1", "expires_in": i64::MAX})),
        )
        .expect(1)
        .mount(&s)
        .await;
    let open: Opener = Arc::new(|url: &str, _: &str| {
        let u = Url::parse(url).unwrap();
        let q: std::collections::HashMap<_, _> = u.query_pairs().into_owned().collect();
        let port = Url::parse(&q["redirect_uri"]).unwrap().port().unwrap();
        let state = q["state"].clone();
        std::thread::spawn(move || {
            // Stalled: connects and sends nothing, kept open throughout.
            let _idle = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
            hit(port, "/callback?code=evil&state=forged");
            hit(port, "/callback?error=access_denied&state=forged");
            hit(port, &format!("/callback?code=the-code&state={state}"));
            std::thread::sleep(Duration::from_secs(2));
        });
        Ok(())
    });
    let started = std::time::Instant::now();
    let t = oauth::authorize(
        &http(),
        &provider(&s),
        &open,
        Duration::from_secs(20),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(t.access_token, "at-1");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "the stalled connection held the flow up"
    );
    // A huge expires_in neither panics nor wraps into the past.
    let at = t.expires_at.unwrap();
    assert!(at > chrono::Utc::now().timestamp() + 3600, "{at}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_callback_times_out_and_cancel_stops() {
    let s = MockServer::start().await;
    let idle: Opener = Arc::new(|_: &str, _: &str| Ok(()));
    let r = oauth::authorize(
        &http(),
        &provider(&s),
        &idle,
        Duration::from_millis(200),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(r.unwrap_err(), AuthError::Timeout);

    let c = CancellationToken::new();
    c.cancel();
    let r = oauth::authorize(&http(), &provider(&s), &idle, Duration::from_secs(10), &c).await;
    assert_eq!(r.unwrap_err(), AuthError::Cancelled);

    // Non-https providers are refused before anything opens.
    let strict = Http::new(HttpOptions::default()).unwrap();
    let r = oauth::authorize(&strict, &provider(&s), &idle, Duration::from_secs(1), &c).await;
    assert!(matches!(r, Err(AuthError::Failed(_))));
}
