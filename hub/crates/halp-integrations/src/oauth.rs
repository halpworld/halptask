//! OAuth 2.1 authorization code flow with PKCE (S256) for a native app
//! (RFC 8252): the redirect goes to a one-shot listener on
//! `127.0.0.1:<random port>`, the `state` is checked (callbacks with a
//! wrong one are ignored), the listener closes after the first callback
//! with the right `state` or a timeout. Opening the browser is a callback
//! the shell provides; it is only ever handed the authorization URL built
//! here, together with the host it must have.

use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use chrono::Utc;
use reqwest::Method;
use reqwest::header::{ACCEPT, CONTENT_TYPE, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use url::Url;
use zeroize::Zeroizing;

use crate::http::Http;
use crate::redact::redact;
use crate::secrets::Token;

/// The longest `expires_in` taken at face value (ten years).
const MAX_EXPIRES_IN: i64 = 10 * 365 * 24 * 3600;
/// Callback connections handled at once; more wait in the backlog.
const MAX_CALLBACK_CONNS: usize = 16;
/// How long one connection may take to send its request head.
const CALLBACK_READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Opens a URL in the user's browser. Gets the URL and the host it must
/// have; implementations re-check before opening.
pub type Opener = std::sync::Arc<dyn Fn(&str, &str) -> Result<(), String> + Send + Sync>;

/// An OAuth provider, from an integration's `settings.oauth`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OAuthProvider {
    pub authorize_url: Url,
    pub token_url: Url,
    pub client_id: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

impl OAuthProvider {
    pub fn check(&self, http: &Http) -> Result<(), String> {
        http.check(&self.authorize_url).map_err(|e| e.message)?;
        http.check(&self.token_url).map_err(|e| e.message)?;
        if self.client_id.is_empty()
            || self.client_id.len() > 200
            || !self.client_id.bytes().all(|b| (0x21..=0x7e).contains(&b))
        {
            return Err("oauth.client_id must be 1..200 printable characters".into());
        }
        if self.scopes.len() > 20 || self.scopes.iter().any(|s| s.len() > 100 || s.contains(' ')) {
            return Err("oauth.scopes: at most 20 scopes without spaces".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthError {
    /// No callback with the right `state` came in time, but one with a
    /// wrong `state` did (possible CSRF; it was ignored).
    StateMismatch,
    /// The user (or provider) refused.
    Denied(String),
    /// No callback in time.
    Timeout,
    Cancelled,
    /// Anything else (redacted).
    Failed(String),
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AuthError::StateMismatch => {
                f.write_str("authorization timed out after a callback with the wrong state")
            }
            AuthError::Denied(m) => write!(f, "authorization denied: {m}"),
            AuthError::Timeout => f.write_str("authorization timed out"),
            AuthError::Cancelled => f.write_str("authorization cancelled"),
            AuthError::Failed(m) => write!(f, "authorization failed: {m}"),
        }
    }
}

/// A PKCE verifier (43 characters from 32 random bytes) and its S256
/// challenge.
pub struct Pkce {
    pub verifier: Zeroizing<String>,
    pub challenge: String,
}

pub fn pkce() -> Result<Pkce, String> {
    let mut b = Zeroizing::new([0u8; 32]);
    getrandom::fill(b.as_mut()).map_err(|e| format!("no system randomness: {e}"))?;
    let verifier = Zeroizing::new(URL_SAFE_NO_PAD.encode(b.as_ref()));
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    Ok(Pkce {
        verifier,
        challenge,
    })
}

fn random_state() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("no system randomness: {e}"))?;
    Ok(hex::encode(b))
}

/// Runs the whole flow: listener, browser, callback, code exchange.
pub async fn authorize(
    http: &Http,
    p: &OAuthProvider,
    open: &Opener,
    timeout: Duration,
    cancel: &CancellationToken,
) -> Result<Token, AuthError> {
    p.check(http).map_err(AuthError::Failed)?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| AuthError::Failed(format!("could not listen on loopback: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| AuthError::Failed(e.to_string()))?
        .port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");
    let pk = pkce().map_err(AuthError::Failed)?;
    let state = random_state().map_err(AuthError::Failed)?;
    let mut url = p.authorize_url.clone();
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("response_type", "code")
            .append_pair("client_id", &p.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("code_challenge", &pk.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        if !p.scopes.is_empty() {
            q.append_pair("scope", &p.scopes.join(" "));
        }
    }
    let host = p.authorize_url.host_str().unwrap_or_default();
    if url.host_str() != Some(host) {
        return Err(AuthError::Failed("authorization URL host changed".into()));
    }
    open(url.as_str(), host).map_err(|e| AuthError::Failed(redact(&e, &[])))?;

    let code = tokio::select! {
        _ = cancel.cancelled() => return Err(AuthError::Cancelled),
        r = wait_callback(&listener, &state, timeout) => r?,
    };
    drop(listener);
    let code = Zeroizing::new(code);
    let body = form(&[
        ("grant_type", "authorization_code"),
        ("code", &code),
        ("redirect_uri", &redirect_uri),
        ("client_id", &p.client_id),
        ("code_verifier", &pk.verifier),
    ]);
    token_request(http, p, body, None, cancel).await
}

/// Renews an access token.
pub async fn refresh(
    http: &Http,
    p: &OAuthProvider,
    refresh_token: &str,
    cancel: &CancellationToken,
) -> Result<Token, AuthError> {
    p.check(http).map_err(AuthError::Failed)?;
    let body = form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", &p.client_id),
    ]);
    token_request(http, p, body, Some(refresh_token), cancel).await
}

fn form(pairs: &[(&str, &str)]) -> Zeroizing<Vec<u8>> {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    Zeroizing::new(s.finish().into_bytes())
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    token_type: Option<String>,
    error: Option<String>,
}

async fn token_request(
    http: &Http,
    p: &OAuthProvider,
    body: Zeroizing<Vec<u8>>,
    old_refresh: Option<&str>,
    cancel: &CancellationToken,
) -> Result<Token, AuthError> {
    let headers = [
        (ACCEPT, HeaderValue::from_static("application/json")),
        (
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        ),
    ];
    let r = http
        .send(
            Method::POST,
            &p.token_url,
            &headers,
            Some(body.to_vec()),
            cancel,
        )
        .await
        .map_err(|e| AuthError::Failed(e.message))?;
    let parsed: Option<TokenResponse> = serde_json::from_slice(&r.body).ok();
    let mut t = parsed.ok_or_else(|| {
        AuthError::Failed(format!(
            "token endpoint answered {} without JSON",
            r.status.as_u16()
        ))
    })?;
    if let Some(e) = &t.error {
        return Err(AuthError::Denied(redact(e, &[]).chars().take(80).collect()));
    }
    if !r.status.is_success() {
        return Err(AuthError::Failed(format!(
            "token endpoint answered {}",
            r.status.as_u16()
        )));
    }
    let Some(access) = t.access_token.take().filter(|a| !a.is_empty()) else {
        return Err(AuthError::Failed("no access_token in the answer".into()));
    };
    Ok(Token {
        access_token: access,
        // Providers that do not rotate refresh tokens omit it on refresh.
        refresh_token: t
            .refresh_token
            .take()
            .or_else(|| old_refresh.map(str::to_string)),
        // At most ten years out, so a huge `expires_in` cannot overflow.
        expires_at: t
            .expires_in
            .filter(|s| *s > 0)
            .map(|s| s.min(MAX_EXPIRES_IN))
            .and_then(|s| Utc::now().timestamp().checked_add(s)),
        token_type: t.token_type.take().unwrap_or_else(|| "bearer".into()),
    })
}

/// What one callback connection said.
enum Callback {
    /// Not a callback for this flow (another path, a bad request, a wrong
    /// `state`, a connection that sent nothing in time): ignored.
    Ignored { wrong_state: bool },
    /// The provider's answer: the code, or why there is none.
    Done(Result<String, AuthError>),
}

/// Accepts connections until `timeout`, handling them concurrently (each
/// with its own read deadline, so a stalled connection blocks nothing),
/// and returns the code of the first `/callback` whose `state` matches.
/// Callbacks with a wrong `state` are answered and ignored, so a local
/// process or web page that guesses the port cannot end the flow.
async fn wait_callback(
    listener: &TcpListener,
    state: &str,
    timeout: Duration,
) -> Result<String, AuthError> {
    let deadline = tokio::time::Instant::now() + timeout;
    let state: std::sync::Arc<str> = state.into();
    let mut conns = tokio::task::JoinSet::new();
    let mut wrong_state = false;
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                return Err(if wrong_state { AuthError::StateMismatch } else { AuthError::Timeout });
            }
            a = listener.accept(), if conns.len() < MAX_CALLBACK_CONNS => {
                let Ok((sock, _)) = a else { continue };
                let state = state.clone();
                conns.spawn(async move { one_callback(sock, &state).await });
            }
            Some(done) = conns.join_next(), if !conns.is_empty() => match done {
                Ok(Callback::Done(r)) => return r,
                Ok(Callback::Ignored { wrong_state: w }) => wrong_state |= w,
                Err(_) => {}
            },
        }
    }
}

async fn one_callback(mut sock: tokio::net::TcpStream, state: &str) -> Callback {
    let ignored = Callback::Ignored { wrong_state: false };
    let mut buf = vec![0u8; 8192];
    let mut n = 0;
    let read = tokio::time::timeout(CALLBACK_READ_TIMEOUT, async {
        while n < buf.len() {
            let k = sock.read(&mut buf[n..]).await?;
            if k == 0 {
                break;
            }
            n += k;
            if buf[..n].windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        Ok::<_, std::io::Error>(())
    })
    .await;
    if !matches!(read, Ok(Ok(()))) {
        return ignored;
    }
    let head = String::from_utf8_lossy(&buf[..n]);
    let target = head
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("GET "))
        .and_then(|l| l.split(' ').next())
        .unwrap_or("");
    let Ok(u) = Url::parse(&format!("http://127.0.0.1{target}")) else {
        respond(&mut sock, 400, "Bad request").await;
        return ignored;
    };
    if u.path() != "/callback" {
        respond(&mut sock, 404, "Not found").await;
        return ignored;
    }
    let q: std::collections::HashMap<String, String> = u.query_pairs().into_owned().collect();
    let got = q.get("state").map(String::as_str).unwrap_or("");
    if !halp_core::halp_policy::secret::ct_eq(got.as_bytes(), state.as_bytes()) {
        respond(
            &mut sock,
            400,
            "This is not the authorization HalpTask Hub is waiting for. You can close this window.",
        )
        .await;
        return Callback::Ignored { wrong_state: true };
    }
    // The right state: single use, whatever this callback says ends the flow.
    if let Some(e) = q.get("error") {
        respond(
            &mut sock,
            200,
            "Authorization was not granted. You can close this window.",
        )
        .await;
        return Callback::Done(Err(AuthError::Denied(
            redact(e, &[]).chars().take(80).collect(),
        )));
    }
    let Some(code) = q.get("code").filter(|c| !c.is_empty() && c.len() <= 2048) else {
        respond(
            &mut sock,
            400,
            "No authorization code. You can close this window.",
        )
        .await;
        return Callback::Done(Err(AuthError::Failed("callback without a code".into())));
    };
    respond(
        &mut sock,
        200,
        "HalpTask Hub is connected. You can close this window.",
    )
    .await;
    Callback::Done(Ok(code.clone()))
}

async fn respond(sock: &mut tokio::net::TcpStream, status: u16, text: &str) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Bad Request",
    };
    let body =
        format!("<!doctype html><meta charset=utf-8><title>HalpTask Hub</title><p>{text}</p>");
    let msg = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\nContent-Security-Policy: default-src 'none'\r\n\r\n{body}",
        body.len()
    );
    let _ = sock.write_all(msg.as_bytes()).await;
    let _ = sock.shutdown().await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pkce_is_s256_of_a_43_char_verifier() {
        let p = pkce().unwrap();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(
            p.challenge,
            URL_SAFE_NO_PAD.encode(Sha256::digest(p.verifier.as_bytes()))
        );
        assert_ne!(pkce().unwrap().challenge, p.challenge);
    }

    #[test]
    fn rfc7636_example() {
        let v = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            URL_SAFE_NO_PAD.encode(Sha256::digest(v.as_bytes())),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
}
