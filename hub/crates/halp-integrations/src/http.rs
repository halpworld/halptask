//! The one HTTP client integrations use: rustls, https only (plain http
//! only to loopback, and only when the runtime allows it for tests),
//! redirects only to the same origin, a user agent, timeouts, a response
//! size cap, bounded concurrency and global and per-host rate limits.
//! Proxies come from the environment (`HTTPS_PROXY`, system settings).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use reqwest::{Method, StatusCode};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::redact::redact;

pub const USER_AGENT: &str = "HalpTaskHub/0.1";

#[derive(Debug, Clone)]
pub struct HttpOptions {
    /// Allow `http://127.0.0.1` / `localhost` (tests with mock servers).
    pub allow_loopback_http: bool,
    pub max_body_bytes: usize,
    pub timeout: Duration,
    pub connect_timeout: Duration,
    pub max_in_flight: usize,
    /// Global token bucket: burst, then per second.
    pub global_rate: (u32, f64),
    /// Per host.
    pub host_rate: (u32, f64),
}

impl Default for HttpOptions {
    fn default() -> Self {
        HttpOptions {
            allow_loopback_http: false,
            max_body_bytes: 4 * 1024 * 1024,
            timeout: Duration::from_secs(20),
            connect_timeout: Duration::from_secs(10),
            max_in_flight: 8,
            global_rate: (30, 10.0),
            host_rate: (15, 5.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpErrorKind {
    /// Not an allowed URL (scheme, host, credentials).
    Policy,
    /// Connection, TLS or protocol failure.
    Network,
    Timeout,
    TooLarge,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct HttpError {
    pub kind: HttpErrorKind,
    /// Redacted.
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

fn err(kind: HttpErrorKind, m: impl AsRef<str>) -> HttpError {
    HttpError {
        kind,
        message: redact(m.as_ref(), &[]),
    }
}

#[derive(Debug)]
pub struct HttpResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
    pub fn json(&self) -> Result<serde_json::Value, HttpError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| err(HttpErrorKind::Network, format!("invalid JSON: {e}")))
    }
}

/// A token bucket.
#[derive(Debug)]
struct Bucket {
    cap: f64,
    per_sec: f64,
    tokens: f64,
    at: Instant,
}

impl Bucket {
    fn new((cap, per_sec): (u32, f64)) -> Bucket {
        Bucket {
            cap: cap.max(1) as f64,
            per_sec: per_sec.max(0.01),
            tokens: cap.max(1) as f64,
            at: Instant::now(),
        }
    }
    /// Takes a token, or says how long until one is there.
    fn take(&mut self, now: Instant) -> Result<(), Duration> {
        let dt = now.saturating_duration_since(self.at).as_secs_f64();
        self.tokens = (self.tokens + dt * self.per_sec).min(self.cap);
        self.at = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(Duration::from_secs_f64((1.0 - self.tokens) / self.per_sec))
        }
    }
}

/// Hosts with their own bucket; past this, idle buckets are dropped.
const MAX_HOST_BUCKETS: usize = 256;

/// Makes room in the per-host buckets: drops every bucket that has refilled
/// (it is the same as a new one), else the least recently used one.
fn evict(hosts: &mut HashMap<String, Bucket>, now: Instant) {
    hosts.retain(|_, b| {
        let dt = now.saturating_duration_since(b.at).as_secs_f64();
        b.tokens + dt * b.per_sec < b.cap
    });
    if hosts.len() >= MAX_HOST_BUCKETS
        && let Some(oldest) = hosts
            .iter()
            .min_by_key(|(_, b)| b.at)
            .map(|(k, _)| k.clone())
    {
        hosts.remove(&oldest);
    }
}

pub struct Http {
    client: reqwest::Client,
    opts: HttpOptions,
    in_flight: Semaphore,
    global: Mutex<Bucket>,
    hosts: Mutex<HashMap<String, Bucket>>,
}

fn is_loopback(u: &Url) -> bool {
    match u.host() {
        Some(url::Host::Ipv4(a)) => a.is_loopback(),
        Some(url::Host::Ipv6(a)) => a.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    }
}

/// Whether `u` may be fetched: https (or loopback http when allowed), a
/// host, no credentials in the URL.
pub fn check_url(u: &Url, allow_loopback_http: bool) -> Result<(), HttpError> {
    if u.host_str().is_none_or(str::is_empty) {
        return Err(err(HttpErrorKind::Policy, "URL has no host"));
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err(err(
            HttpErrorKind::Policy,
            "URL must not contain credentials",
        ));
    }
    match u.scheme() {
        "https" => Ok(()),
        "http" if allow_loopback_http && is_loopback(u) => Ok(()),
        s => Err(err(
            HttpErrorKind::Policy,
            format!("{s} URLs are not allowed (https only)"),
        )),
    }
}

impl Http {
    pub fn new(opts: HttpOptions) -> Result<Http, String> {
        let client = Self::builder(&opts)
            .build()
            .map_err(|e| format!("could not build the HTTP client: {e}"))?;
        Ok(Http {
            client,
            in_flight: Semaphore::new(opts.max_in_flight.max(1)),
            global: Mutex::new(Bucket::new(opts.global_rate)),
            hosts: Mutex::new(HashMap::new()),
            opts,
        })
    }

    /// The reqwest builder with the redirect, TLS and timeout policy (also
    /// used for the MCP Streamable HTTP transport).
    pub fn builder(opts: &HttpOptions) -> reqwest::ClientBuilder {
        let allow_loopback = opts.allow_loopback_http;
        reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(opts.timeout)
            .connect_timeout(opts.connect_timeout)
            .redirect(reqwest::redirect::Policy::custom(move |a| {
                let Some(first) = a.previous().first() else {
                    return a.stop();
                };
                let same = a.url().scheme() == first.scheme()
                    && a.url().host_str() == first.host_str()
                    && a.url().port_or_known_default() == first.port_or_known_default();
                if a.previous().len() > 3 || !same || check_url(a.url(), allow_loopback).is_err() {
                    a.stop()
                } else {
                    a.follow()
                }
            }))
    }

    pub fn options(&self) -> &HttpOptions {
        &self.opts
    }

    pub fn check(&self, u: &Url) -> Result<(), HttpError> {
        check_url(u, self.opts.allow_loopback_http)
    }

    async fn pace(&self, host: &str, cancel: &CancellationToken) -> Result<(), HttpError> {
        loop {
            let wait = {
                let now = Instant::now();
                let mut g = self
                    .global
                    .lock()
                    .map_err(|_| err(HttpErrorKind::Network, "rate limiter poisoned"))?;
                let mut hosts = self
                    .hosts
                    .lock()
                    .map_err(|_| err(HttpErrorKind::Network, "rate limiter poisoned"))?;
                if !hosts.contains_key(host) && hosts.len() >= MAX_HOST_BUCKETS {
                    evict(&mut hosts, now);
                }
                let h = hosts
                    .entry(host.to_string())
                    .or_insert_with(|| Bucket::new(self.opts.host_rate));
                match (g.take(now), h.take(now)) {
                    (Ok(()), Ok(())) => None,
                    (Err(a), Ok(())) => {
                        h.tokens += 1.0;
                        Some(a)
                    }
                    (Ok(()), Err(b)) => {
                        g.tokens += 1.0;
                        Some(b)
                    }
                    (Err(a), Err(b)) => Some(a.max(b)),
                }
            };
            match wait {
                None => return Ok(()),
                Some(d) => tokio::select! {
                    _ = cancel.cancelled() => return Err(err(HttpErrorKind::Cancelled, "cancelled")),
                    _ = tokio::time::sleep(d) => {}
                },
            }
        }
    }

    /// Sends one request and reads at most `max_body_bytes` of the answer.
    pub async fn send(
        &self,
        method: Method,
        url: &Url,
        headers: &[(HeaderName, HeaderValue)],
        body: Option<Vec<u8>>,
        cancel: &CancellationToken,
    ) -> Result<HttpResponse, HttpError> {
        self.send_limited(method, url, headers, body, self.opts.max_body_bytes, cancel)
            .await
    }

    /// [`Http::send`] with its own response size cap (calendar feeds).
    pub async fn send_limited(
        &self,
        method: Method,
        url: &Url,
        headers: &[(HeaderName, HeaderValue)],
        body: Option<Vec<u8>>,
        max_body_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<HttpResponse, HttpError> {
        self.check(url)?;
        let host = url.host_str().unwrap_or_default().to_string();
        self.pace(&host, cancel).await?;
        let _permit = tokio::select! {
            _ = cancel.cancelled() => return Err(err(HttpErrorKind::Cancelled, "cancelled")),
            p = self.in_flight.acquire() => p.map_err(|_| err(HttpErrorKind::Cancelled, "closed"))?,
        };
        let mut rb = self.client.request(method, url.clone());
        for (k, v) in headers {
            rb = rb.header(k, v);
        }
        if let Some(b) = body {
            rb = rb.body(b);
        }
        let fut = async {
            let mut resp = rb.send().await.map_err(map_reqwest)?;
            if resp
                .content_length()
                .is_some_and(|n| n > max_body_bytes as u64)
            {
                return Err(err(HttpErrorKind::TooLarge, "response too large"));
            }
            let status = resp.status();
            let headers = resp.headers().clone();
            let mut body = Vec::new();
            while let Some(chunk) = resp.chunk().await.map_err(map_reqwest)? {
                if body.len() + chunk.len() > max_body_bytes {
                    return Err(err(HttpErrorKind::TooLarge, "response too large"));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(HttpResponse {
                status,
                headers,
                body,
            })
        };
        tokio::select! {
            _ = cancel.cancelled() => Err(err(HttpErrorKind::Cancelled, "cancelled")),
            r = fut => r,
        }
    }
}

fn map_reqwest(e: reqwest::Error) -> HttpError {
    // reqwest's text includes the URL; strip it (it may carry a query).
    let e = e.without_url();
    if e.is_timeout() {
        err(HttpErrorKind::Timeout, "request timed out")
    } else if e.is_redirect() {
        err(HttpErrorKind::Policy, "redirect refused")
    } else {
        err(HttpErrorKind::Network, format!("request failed: {e}"))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn only_https_and_allowed_loopback() {
        assert!(check_url(&u("https://api.github.com/x"), false).is_ok());
        assert!(check_url(&u("http://api.github.com/x"), false).is_err());
        assert!(check_url(&u("http://127.0.0.1:9/x"), false).is_err());
        assert!(check_url(&u("http://127.0.0.1:9/x"), true).is_ok());
        assert!(check_url(&u("http://localhost:9/x"), true).is_ok());
        assert!(check_url(&u("http://10.0.0.1/x"), true).is_err());
        assert!(check_url(&u("https://u:p@api.github.com/"), false).is_err());
        assert!(check_url(&u("file:///etc/passwd"), true).is_err());
    }

    #[test]
    fn host_buckets_stay_bounded() {
        let now = Instant::now();
        let mut hosts: HashMap<String, Bucket> = HashMap::new();
        for i in 0..MAX_HOST_BUCKETS {
            let mut b = Bucket::new((15, 5.0));
            b.take(now).unwrap();
            hosts.insert(format!("h{i}"), b);
        }
        // All busy: the least recently used goes.
        evict(&mut hosts, now);
        assert_eq!(hosts.len(), MAX_HOST_BUCKETS - 1);
        // Later, every bucket has refilled: all go.
        evict(&mut hosts, now + Duration::from_secs(10));
        assert!(hosts.is_empty());
    }

    #[test]
    fn buckets_limit_and_refill() {
        let mut b = Bucket::new((2, 1.0));
        let t = Instant::now();
        assert!(b.take(t).is_ok() && b.take(t).is_ok());
        assert!(b.take(t).is_err());
        assert!(b.take(t + Duration::from_secs(2)).is_ok());
    }
}
