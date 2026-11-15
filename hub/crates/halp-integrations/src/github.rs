//! GitHub over REST: the open pull requests that ask for the user's review
//! or that the user wrote, each as a `dev.pull_request` task, and the
//! latest workflow run per workflow on each PR's head as a `dev.ci_run`
//! bullet.
//!
//! Requests carry `If-None-Match`, so an unchanged resource answers 304
//! (which GitHub does not count against the rate limit). A 401 is an auth
//! error; a 403/429 with rate-limit headers is retried after the reset.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::Duration;

use regex::Regex;
use reqwest::Method;
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderName, HeaderValue, IF_NONE_MATCH};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::connector::{Batch, BoxFuture, Connector, SyncCx, SyncError, cap, https_url, str_at};
use crate::http::Http;
use crate::oauth::OAuthProvider;

pub const DEFAULT_API: &str = "https://api.github.com/";
/// At most this many PRs are followed per sync.
pub const DEFAULT_MAX_PRS: u32 = 30;
const MAX_CACHE: usize = 1000;
/// Bytes of response bodies the ETag cache holds in all; a body larger
/// than [`MAX_CACHED_BODY`] is not cached.
const MAX_CACHE_BYTES: usize = 32 * 1024 * 1024;
const MAX_CACHED_BODY: usize = 1024 * 1024;
/// Pages of reviews read per PR (100 each). GitHub lists reviews oldest
/// first, so past this the first page and the newest pages are read.
const MAX_REVIEW_PAGES: u32 = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitHubSettings {
    /// Only these `owner/name` repositories (default: all).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
    /// GitHub Enterprise: `https://host/api/v3/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<Url>,
    /// `token` (a personal access token, the default) or `oauth`.
    #[serde(default = "token_auth")]
    pub auth: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth: Option<OAuthProvider>,
    /// Also sync workflow runs (default true).
    #[serde(default = "yes")]
    pub ci: bool,
    #[serde(default = "max_prs")]
    pub max_prs: u32,
}

fn token_auth() -> String {
    "token".into()
}
fn yes() -> bool {
    true
}
fn max_prs() -> u32 {
    DEFAULT_MAX_PRS
}

fn repo_re() -> Regex {
    #[allow(clippy::unwrap_used)]
    Regex::new(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$").unwrap()
}

/// `owner/name` with no `.` or `..` segment (which would leave
/// `repos/` once joined to the API base).
fn repo_ok(re: &Regex, r: &str) -> bool {
    r.len() <= 200 && re.is_match(r) && r.split('/').all(|seg| seg != "." && seg != "..")
}

/// The `page` of the `rel="last"` link in a `Link` header.
fn last_page(link: &str) -> Option<u32> {
    link.split(',').find_map(|part| {
        let (url, params) = part.split_once(';')?;
        if !params.split(';').any(|p| p.trim() == "rel=\"last\"") {
            return None;
        }
        let url = Url::parse(url.trim().trim_start_matches('<').trim_end_matches('>')).ok()?;
        url.query_pairs()
            .find(|(k, _)| k == "page")
            .and_then(|(_, v)| v.parse().ok())
    })
}

impl GitHubSettings {
    pub fn parse(v: &Value) -> Result<GitHubSettings, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("github settings: {e}"))
    }

    fn base(&self) -> Result<Url, String> {
        let mut u = match &self.api_url {
            Some(u) => u.clone(),
            None => Url::parse(DEFAULT_API).map_err(|e| e.to_string())?,
        };
        if !u.path().ends_with('/') {
            let p = format!("{}/", u.path());
            u.set_path(&p);
        }
        Ok(u)
    }
}

#[derive(Clone)]
struct Cached {
    etag: String,
    body: Value,
    /// The `rel="last"` page of the answer's `Link` header.
    last: Option<u32>,
    bytes: usize,
}

#[derive(Default)]
pub struct GitHub {
    /// (integration, url) → cached answer.
    cache: Mutex<HashMap<(String, String), Cached>>,
}

impl GitHub {
    pub fn new() -> Self {
        Self::default()
    }

    async fn get(&self, cx: &SyncCx<'_>, url: &Url) -> Result<Value, SyncError> {
        Ok(self.get_paged(cx, url).await?.0)
    }

    /// Every review of a PR, oldest first: all pages up to
    /// [`MAX_REVIEW_PAGES`], else the first page and the newest ones, so
    /// a reviewer's latest decision always counts.
    async fn reviews(&self, cx: &SyncCx<'_>, url: &Url) -> Result<Value, SyncError> {
        let page = |p: u32| {
            let mut u = url.clone();
            u.query_pairs_mut()
                .append_pair("per_page", "100")
                .append_pair("page", &p.to_string());
            u
        };
        let (first, last) = self.get_paged(cx, &page(1)).await?;
        let mut all: Vec<Value> = first.as_array().cloned().unwrap_or_default();
        let last = last.unwrap_or(1).max(1);
        let from = if last <= MAX_REVIEW_PAGES {
            2
        } else {
            last - (MAX_REVIEW_PAGES - 2)
        };
        for p in from..=last {
            if cx.cancel.is_cancelled() {
                return Err(SyncError::Cancelled);
            }
            let (v, _) = self.get_paged(cx, &page(p)).await?;
            all.extend(v.as_array().cloned().unwrap_or_default());
        }
        Ok(Value::Array(all))
    }

    /// A GET with the ETag cache; the body and the `rel="last"` page.
    async fn get_paged(
        &self,
        cx: &SyncCx<'_>,
        url: &Url,
    ) -> Result<(Value, Option<u32>), SyncError> {
        let key = (cx.id.to_string(), url.to_string());
        let cached = self
            .cache
            .lock()
            .map_err(|_| SyncError::transient("cache poisoned"))?
            .get(&key)
            .cloned();
        let mut headers: Vec<(HeaderName, HeaderValue)> = vec![
            (
                ACCEPT,
                HeaderValue::from_static("application/vnd.github+json"),
            ),
            (
                HeaderName::from_static("x-github-api-version"),
                HeaderValue::from_static("2022-11-28"),
            ),
        ];
        if let Some(t) = cx.token {
            let mut v = HeaderValue::from_str(&format!("Bearer {t}"))
                .map_err(|_| SyncError::Auth("the stored token is not a valid header".into()))?;
            v.set_sensitive(true);
            headers.push((AUTHORIZATION, v));
        }
        if let Some(c) = &cached
            && let Ok(v) = HeaderValue::from_str(&c.etag)
        {
            headers.push((IF_NONE_MATCH, v));
        }
        let r = cx
            .http
            .send(Method::GET, url, &headers, None, cx.cancel)
            .await?;
        let status = r.status.as_u16();
        match status {
            304 => cached
                .map(|c| (c.body, c.last))
                .ok_or_else(|| SyncError::transient("GitHub answered 304 without a cached copy")),
            200..=299 => {
                let body = r.json().map_err(SyncError::from)?;
                let last = r.header("link").and_then(last_page);
                let bytes = r.body.len();
                if let Some(etag) = r.header("etag")
                    && bytes <= MAX_CACHED_BODY
                {
                    let mut c = self
                        .cache
                        .lock()
                        .map_err(|_| SyncError::transient("cache poisoned"))?;
                    c.remove(&key);
                    let held: usize = c.values().map(|e| e.bytes).sum();
                    if c.len() >= MAX_CACHE || held + bytes > MAX_CACHE_BYTES {
                        c.clear();
                    }
                    c.insert(
                        key,
                        Cached {
                            etag: etag.to_string(),
                            body: body.clone(),
                            last,
                            bytes,
                        },
                    );
                }
                Ok((body, last))
            }
            401 => Err(SyncError::Auth(
                "GitHub rejected the token (401); add a new one".into(),
            )),
            403 | 429 => {
                let limited = status == 429
                    || r.header("x-ratelimit-remaining") == Some("0")
                    || r.header("retry-after").is_some();
                if limited {
                    let after = r
                        .header("retry-after")
                        .and_then(|s| s.parse::<u64>().ok())
                        .or_else(|| {
                            r.header("x-ratelimit-reset")
                                .and_then(|s| s.parse::<i64>().ok())
                                .map(|reset| (reset - cx.now.timestamp()).clamp(1, 3600) as u64)
                        })
                        .unwrap_or(60);
                    Err(SyncError::Transient {
                        message: "GitHub rate limit reached".into(),
                        retry_after: Some(Duration::from_secs(after.min(3600))),
                    })
                } else {
                    Err(SyncError::Config(
                        "GitHub refused access (403); check the token's scopes".into(),
                    ))
                }
            }
            404 => Err(SyncError::Config(format!(
                "GitHub: {} not found (404)",
                url.path()
            ))),
            _ => Err(SyncError::transient(format!("GitHub answered {status}"))),
        }
    }

    async fn run(&self, cx: SyncCx<'_>) -> Result<Vec<Batch>, SyncError> {
        let s = GitHubSettings::parse(cx.settings).map_err(SyncError::Config)?;
        let base = s.base().map_err(SyncError::Config)?;
        let join = |p: &str| base.join(p).map_err(|e| SyncError::Config(e.to_string()));

        // Which PRs, and why.
        let mut found: BTreeMap<(String, u64), (Value, bool, bool)> = BTreeMap::new();
        let mut complete = true;
        for (qual, requested) in [("review-requested:@me", true), ("author:@me", false)] {
            let mut u = join("search/issues")?;
            u.query_pairs_mut()
                .append_pair("q", &format!("is:pr is:open archived:false {qual}"))
                .append_pair("per_page", "50")
                .append_pair("sort", "updated");
            let body = self.get(&cx, &u).await?;
            if body.get("incomplete_results").and_then(Value::as_bool) == Some(true) {
                complete = false;
            }
            let total = body.get("total_count").and_then(Value::as_u64).unwrap_or(0);
            let items = body
                .get("items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if total > items.len() as u64 {
                complete = false;
            }
            for it in items {
                let Some((repo, n)) = pr_ref(&it) else {
                    continue;
                };
                if !s.repos.is_empty() && !s.repos.iter().any(|r| r.eq_ignore_ascii_case(&repo)) {
                    continue;
                }
                let e = found.entry((repo, n)).or_insert((it, false, false));
                if requested {
                    e.1 = true;
                } else {
                    e.2 = true;
                }
            }
        }
        if found.len() > s.max_prs as usize {
            complete = false;
        }

        let mut prs = Vec::new();
        let mut runs = Vec::new();
        for ((repo, n), (search, requested, authored)) in found.into_iter().take(s.max_prs as usize)
        {
            if cx.cancel.is_cancelled() {
                return Err(SyncError::Cancelled);
            }
            let pr = self
                .get(&cx, &join(&format!("repos/{repo}/pulls/{n}"))?)
                .await?;
            let ru = join(&format!("repos/{repo}/pulls/{n}/reviews"))?;
            let reviews = self.reviews(&cx, &ru).await?;
            let wf = match (s.ci, str_at(&pr, "/head/sha")) {
                (true, Some(sha)) => {
                    let mut u = join(&format!("repos/{repo}/actions/runs"))?;
                    u.query_pairs_mut()
                        .append_pair("head_sha", sha)
                        .append_pair("per_page", "30");
                    self.get(&cx, &u).await?
                }
                _ => json!({ "workflow_runs": [] }),
            };
            let latest = latest_runs(&wf);
            prs.push(map_pr(
                &repo, n, &search, &pr, &reviews, &latest, requested, authored, s.ci,
            ));
            for r in &latest {
                if let Some(item) = map_run(&repo, Some(n), r) {
                    runs.push(item);
                }
            }
        }
        let mut out = vec![Batch {
            scope: "prs".into(),
            complete,
            items: prs,
            patches: Vec::new(),
        }];
        if s.ci {
            out.push(Batch {
                scope: "ci".into(),
                complete,
                items: runs,
                patches: Vec::new(),
            });
        }
        Ok(out)
    }
}

/// `(owner/name, number)` of a search hit that is a pull request.
fn pr_ref(it: &Value) -> Option<(String, u64)> {
    it.get("pull_request")?;
    let n = it.get("number")?.as_u64()?;
    let ru = str_at(it, "/repository_url")?;
    let mut seg = ru.trim_end_matches('/').rsplit('/');
    let name = seg.next()?;
    let owner = seg.next()?;
    let repo = format!("{owner}/{name}");
    (repo_ok(&repo_re(), &repo) && n >= 1).then_some((repo, n))
}

/// The latest run of each workflow.
fn latest_runs(v: &Value) -> Vec<Value> {
    let mut by: BTreeMap<String, Value> = BTreeMap::new();
    for r in v
        .get("workflow_runs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = r
            .get("workflow_id")
            .map(Value::to_string)
            .unwrap_or_else(|| str_at(r, "/name").unwrap_or("?").to_string());
        let newer = match by.get(&key) {
            None => true,
            Some(old) => {
                r.get("run_number").and_then(Value::as_u64)
                    > old.get("run_number").and_then(Value::as_u64)
            }
        };
        if newer {
            by.insert(key, r.clone());
        }
    }
    by.into_values().collect()
}

fn run_status(r: &Value) -> &'static str {
    match (str_at(r, "/status"), str_at(r, "/conclusion")) {
        (Some("completed"), Some("success" | "neutral" | "skipped")) => "passing",
        (Some("completed"), Some("cancelled")) => "cancelled",
        (Some("completed"), _) => "failing",
        (Some("in_progress"), _) => "running",
        _ => "queued",
    }
}

fn ci_summary(runs: &[Value]) -> &'static str {
    let st: Vec<&str> = runs.iter().map(run_status).collect();
    if st.is_empty() {
        "unknown"
    } else if st.contains(&"failing") {
        "failing"
    } else if st.iter().any(|s| *s == "queued" || *s == "running") {
        "pending"
    } else if st.iter().all(|s| *s == "passing" || *s == "cancelled") && st.contains(&"passing") {
        "passing"
    } else {
        "unknown"
    }
}

#[allow(clippy::too_many_arguments)]
fn map_pr(
    repo: &str,
    n: u64,
    search: &Value,
    pr: &Value,
    reviews: &Value,
    runs: &[Value],
    requested: bool,
    authored: bool,
    with_ci: bool,
) -> Value {
    // The latest decisive review per reviewer.
    let mut verdict: BTreeMap<String, &str> = BTreeMap::new();
    for r in reviews.as_array().into_iter().flatten() {
        let Some(who) = str_at(r, "/user/login") else {
            continue;
        };
        match str_at(r, "/state") {
            Some(s @ ("APPROVED" | "CHANGES_REQUESTED")) => {
                verdict.insert(who.to_string(), s);
            }
            Some("DISMISSED") => {
                verdict.remove(who);
            }
            _ => {}
        }
    }
    let requested_reviewers: Vec<String> = pr
        .get("requested_reviewers")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|u| str_at(u, "/login"))
        .take(30)
        .map(|s| cap(s, 100))
        .collect();
    let draft = pr.get("draft").and_then(Value::as_bool).unwrap_or(false);
    let state = if draft {
        "draft"
    } else if verdict.values().any(|v| *v == "CHANGES_REQUESTED") {
        "changes_requested"
    } else if verdict.values().any(|v| *v == "APPROVED") {
        "approved"
    } else if requested || !requested_reviewers.is_empty() {
        "review_requested"
    } else {
        "open"
    };
    let mut f = serde_json::Map::new();
    f.insert("repo".into(), json!(repo));
    f.insert("number".into(), json!(n));
    f.insert("state".into(), json!(state));
    f.insert("requested_from_me".into(), json!(requested));
    f.insert("authored_by_me".into(), json!(authored));
    if with_ci {
        f.insert("ci".into(), json!(ci_summary(runs)));
    }
    if let Some(u) = https_url(str_at(pr, "/html_url").or(str_at(search, "/html_url"))) {
        f.insert("url".into(), json!(u));
    }
    if let Some(a) = str_at(pr, "/user/login").or(str_at(search, "/user/login")) {
        f.insert("author".into(), json!(cap(a, 100)));
    }
    if !requested_reviewers.is_empty() {
        f.insert("reviewers".into(), json!(requested_reviewers));
    }
    if let Some(t) = crate::connector::timestamp(pr.get("updated_at").or(search.get("updated_at")))
    {
        f.insert("updated_at".into(), json!(t));
    }
    for k in ["additions", "deletions"] {
        if let Some(v) = pr.get(k).and_then(Value::as_u64) {
            f.insert(k.into(), json!(v.min(100_000_000)));
        }
    }
    if let Some(b) = str_at(pr, "/head/ref") {
        f.insert("branch".into(), json!(cap(b, 255)));
    }
    let mergeable = match str_at(pr, "/mergeable_state") {
        Some("clean") => "clean",
        Some("dirty") => "conflicting",
        Some("blocked") => "blocked",
        _ => "unknown",
    };
    f.insert("mergeable".into(), json!(mergeable));
    let title = str_at(pr, "/title")
        .or(str_at(search, "/title"))
        .unwrap_or("(untitled pull request)");
    json!({
        "external_id": format!("github:{repo}#{n}"),
        "title": cap(&format!("{title} ({repo}#{n})"), 300),
        "kind": "task",
        "type_id": "dev.pull_request",
        "tags": ["github"],
        "fields": f,
    })
}

fn map_run(repo: &str, pr: Option<u64>, r: &Value) -> Option<Value> {
    let branch = cap(str_at(r, "/head_branch")?, 255);
    let workflow = cap(str_at(r, "/name")?, 200);
    let wid = r
        .get("workflow_id")
        .and_then(Value::as_u64)
        .map(|w| w.to_string())
        .unwrap_or_else(|| workflow.clone());
    let status = run_status(r);
    let mut f = serde_json::Map::new();
    f.insert("repo".into(), json!(repo));
    f.insert("branch".into(), json!(branch));
    f.insert("workflow".into(), json!(workflow));
    f.insert("status".into(), json!(status));
    if let Some(u) = https_url(str_at(r, "/html_url")) {
        f.insert("url".into(), json!(u));
    }
    if let Some(t) = crate::connector::timestamp(r.get("run_started_at").or(r.get("created_at"))) {
        f.insert("started_at".into(), json!(t));
    }
    if status != "queued"
        && status != "running"
        && let Some(t) = crate::connector::timestamp(r.get("updated_at"))
    {
        f.insert("finished_at".into(), json!(t));
    }
    if let Some(n) = pr {
        f.insert("pr_number".into(), json!(n));
    }
    Some(json!({
        "external_id": cap(&format!("github:{repo}@{branch}:wf{wid}"), 300),
        "title": cap(&format!("{workflow} on {branch}"), 300),
        "kind": "bullet",
        "type_id": "dev.ci_run",
        "tags": ["github", "ci"],
        "fields": f,
    }))
}

impl Connector for GitHub {
    fn kind(&self) -> &'static str {
        "github"
    }

    fn validate(&self, settings: &Value, http: &Http) -> Result<Value, String> {
        let mut s = GitHubSettings::parse(settings)?;
        if s.repos.len() > 100 {
            return Err("github: at most 100 repos".into());
        }
        let re = repo_re();
        for r in &s.repos {
            if !repo_ok(&re, r) {
                return Err(format!("github: repo {r:?} must be owner/name"));
            }
        }
        if let Some(u) = &s.api_url {
            http.check(u).map_err(|e| e.message)?;
            if u.query().is_some() || u.fragment().is_some() {
                return Err("github: api_url takes no query".into());
            }
        }
        s.api_url = match &s.api_url {
            Some(_) => Some(s.base()?),
            None => None,
        };
        match s.auth.as_str() {
            "token" => {}
            "oauth" => s
                .oauth
                .as_ref()
                .ok_or("github: auth \"oauth\" needs an oauth provider")?
                .check(http)?,
            _ => return Err("github: auth is \"token\" or \"oauth\"".into()),
        }
        if !(1..=100).contains(&s.max_prs) {
            return Err("github: max_prs is 1..100".into());
        }
        serde_json::to_value(&s).map_err(|e| e.to_string())
    }

    fn needs_token(&self, _settings: &Value) -> bool {
        true
    }

    fn secret_audience(&self, settings: &Value) -> String {
        let Ok(s) = GitHubSettings::parse(settings) else {
            return "invalid".into();
        };
        let api = s
            .api_url
            .as_ref()
            .map(|u| u.origin().ascii_serialization())
            .unwrap_or_default();
        match (s.auth.as_str(), &s.oauth) {
            ("oauth", Some(p)) => format!(
                "oauth:{api}:{}:{}",
                p.token_url.origin().ascii_serialization(),
                p.client_id
            ),
            (a, _) => format!("{a}:{api}"),
        }
    }

    fn oauth(&self, settings: &Value) -> Option<OAuthProvider> {
        let s = GitHubSettings::parse(settings).ok()?;
        (s.auth == "oauth").then_some(s.oauth).flatten()
    }

    fn sync<'a>(&'a self, cx: SyncCx<'a>) -> BoxFuture<'a, Result<Vec<Batch>, SyncError>> {
        Box::pin(self.run(cx))
    }

    fn forget(&self, id: &str) {
        if let Ok(mut c) = self.cache.lock() {
            c.retain(|(i, _), _| i != id);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn ci_rollup() {
        let ok = json!({"status":"completed","conclusion":"success"});
        let bad = json!({"status":"completed","conclusion":"failure"});
        let run = json!({"status":"in_progress"});
        assert_eq!(ci_summary(&[]), "unknown");
        assert_eq!(ci_summary(std::slice::from_ref(&ok)), "passing");
        assert_eq!(ci_summary(&[ok.clone(), run]), "pending");
        assert_eq!(ci_summary(&[ok, bad]), "failing");
    }

    #[test]
    fn pr_refs_need_a_valid_repo() {
        let it = json!({"number": 3, "pull_request": {}, "repository_url": "https://api.github.com/repos/o/r"});
        assert_eq!(pr_ref(&it), Some(("o/r".into(), 3)));
        let issue = json!({"number": 3, "repository_url": "https://api.github.com/repos/o/r"});
        assert_eq!(pr_ref(&issue), None);
        let bad =
            json!({"number": 3, "pull_request": {}, "repository_url": "https://x/repos/o/r b"});
        assert_eq!(pr_ref(&bad), None);
        for dots in [
            "https://api.github.com/repos/../..",
            "https://api.github.com/repos/o/..",
            "https://api.github.com/repos/./r",
        ] {
            let it = json!({"number": 3, "pull_request": {}, "repository_url": dots});
            assert_eq!(pr_ref(&it), None, "{dots}");
        }
        assert!(repo_ok(&repo_re(), "o/r.js"));
        assert!(repo_ok(&repo_re(), "o/.github"));
        assert!(!repo_ok(&repo_re(), "../r"));
    }

    #[test]
    fn link_last_page() {
        let l = r#"<https://api.github.com/repositories/1/pulls/2/reviews?per_page=100&page=2>; rel="next", <https://api.github.com/repositories/1/pulls/2/reviews?per_page=100&page=14>; rel="last""#;
        assert_eq!(last_page(l), Some(14));
        assert_eq!(last_page(r#"<https://x/?page=1>; rel="prev""#), None);
        assert_eq!(last_page("garbage"), None);
    }
}
