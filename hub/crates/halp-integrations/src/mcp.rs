//! MCP servers as sources: the runtime is an MCP *client* of a server the
//! user configured (a local command over stdio, or a remote server over
//! streamable HTTP), calls one fixed tool with fixed arguments, and maps
//! the answer through an adapter: `slack` → `dev.ask` tasks, `linear` and
//! `jira` → `ext.issue` tasks.
//!
//! Everything the server returns is untrusted: it is only ever read as
//! data, capped, stripped of controls, and validated by the core. The
//! client advertises no capabilities (no sampling, roots or elicitation),
//! so the server cannot ask anything of the hub; a tool call that wants
//! more input fails.
//!
//! stdio servers start from an absolute command with the environment
//! cleared (only a fixed `PATH`, a private `HOME`, locale and the
//! configured variables), in the integration's private directory, and are
//! killed when the sync ends.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use regex::Regex;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, ClientCapabilities, ClientConfig, Implementation,
};
use rmcp::service::{ClientLifecycleMode, serve_client_with_lifecycle};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::io::AsyncReadExt;
use url::Url;

use crate::connector::{
    Batch, BoxFuture, Connector, SyncCx, SyncError, cap, https_url, str_at, timestamp,
};
use crate::http::Http;

pub const MAX_ITEMS: usize = 200;
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Bytes a stdio server may write in one session.
pub const MAX_STDIO_BYTES: u64 = 8 * 1024 * 1024;
/// Bytes one SSE event from an HTTP server may carry (as `send_limited`).
pub const MAX_HTTP_EVENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_ARGS: usize = 32;
const MAX_ENV: usize = 32;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpSettings {
    /// `stdio` or `http`.
    pub transport: String,
    /// stdio: the server's absolute path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// stdio: extra environment (never secrets; see `token_env`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// stdio: the variable that receives the keychain token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    /// http: the server's URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<Url>,
    /// http: send the keychain token as a bearer token (default true).
    #[serde(default = "yes")]
    pub auth: bool,
    /// `slack`, `linear` or `jira` (`calendar` inside calendar settings).
    #[serde(default)]
    pub adapter: String,
    /// The tool to call.
    pub tool: String,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub tool_args: Map<String, Value>,
}

fn yes() -> bool {
    true
}

/// Variables a configured `env` (or `token_env`) may not set: they change
/// how the server's runtime, loader, shell, TLS or proxy behaves rather
/// than configuring the server. Names are also limited to
/// `^[A-Z][A-Z0-9_]{0,63}$`. Only the user can set `env`
/// (`integrations.connect` is user-only), so this is a guard against
/// mistakes and pasted configs, not against agents.
const RESERVED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "SHELL",
    "IFS",
    "ENV",
    "BASH_ENV",
    "LANG",
    "TMPDIR",
    "NODE_OPTIONS",
    "PYTHONSTARTUP",
    "PYTHONPATH",
    "PERL5OPT",
    "RUBYOPT",
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "JDK_JAVA_OPTIONS",
    "CLASSPATH",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
    "NODE_EXTRA_CA_CERTS",
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
];

/// Prefixes of loader, interpreter, tool and locale variable families.
const RESERVED_ENV_PREFIXES: &[&str] = &[
    "LD_", "DYLD_", "NODE_", "PYTHON", "PERL", "RUBY", "JAVA_", "GIT_", "SSH_", "NPM_", "LC_",
    "XDG_", "DOTNET_", "COR", "GCONV",
];

fn env_name_ok(k: &str) -> bool {
    #[allow(clippy::unwrap_used)]
    let re = Regex::new(r"^[A-Z][A-Z0-9_]{0,63}$").unwrap();
    let up = k.to_ascii_uppercase();
    re.is_match(k)
        && !RESERVED_ENV.contains(&up.as_str())
        && !RESERVED_ENV_PREFIXES.iter().any(|p| up.starts_with(p))
}

impl McpSettings {
    pub fn parse(v: &Value) -> Result<McpSettings, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("mcp settings: {e}"))
    }

    pub fn check(&self, http: &Http) -> Result<(), String> {
        match self.adapter.as_str() {
            "slack" | "linear" | "jira" => {}
            _ => return Err("mcp: adapter is slack, linear or jira".into()),
        }
        self.check_transport(http)
    }

    /// Everything but the adapter: the tool, its arguments and the
    /// transport (shared with the `calendar` kind's `mcp` source).
    pub fn check_transport(&self, http: &Http) -> Result<(), String> {
        if self.tool.is_empty() || self.tool.len() > 128 || self.tool.chars().any(char::is_control)
        {
            return Err("mcp: tool must be a name of 1..128 characters".into());
        }
        if serde_json::to_vec(&self.tool_args).map_or(true, |b| b.len() > 8192) {
            return Err("mcp: tool_args are at most 8 KiB".into());
        }
        match self.transport.as_str() {
            "stdio" => {
                let c = self
                    .command
                    .as_deref()
                    .ok_or("mcp: stdio needs a command")?;
                let p = Path::new(c);
                if !p.is_absolute() || !p.is_file() {
                    return Err("mcp: command must be an absolute path to a file".into());
                }
                if self.url.is_some() {
                    return Err("mcp: url is for the http transport".into());
                }
                if self.args.len() > MAX_ARGS
                    || self.args.iter().any(|a| a.len() > 1024 || a.contains('\0'))
                {
                    return Err(format!("mcp: at most {MAX_ARGS} args of 1 KiB"));
                }
                if self.env.len() > MAX_ENV {
                    return Err(format!("mcp: at most {MAX_ENV} env variables"));
                }
                for (k, v) in &self.env {
                    if !env_name_ok(k) || v.len() > 4096 || v.contains('\0') {
                        return Err(format!("mcp: env variable {k:?} is not allowed"));
                    }
                }
                if let Some(t) = &self.token_env
                    && (!env_name_ok(t) || self.env.contains_key(t))
                {
                    return Err(format!("mcp: token_env {t:?} is not allowed"));
                }
            }
            "http" => {
                let u = self.url.as_ref().ok_or("mcp: http needs a url")?;
                http.check(u).map_err(|e| e.message)?;
                if self.command.is_some() || !self.args.is_empty() || !self.env.is_empty() {
                    return Err("mcp: command, args and env are for stdio".into());
                }
                if self.token_env.is_some() {
                    return Err("mcp: token_env is for stdio".into());
                }
            }
            _ => return Err("mcp: transport is stdio or http".into()),
        }
        Ok(())
    }

    /// Who receives the token: the http server's origin, or the stdio
    /// program and the variable it is put in.
    pub(crate) fn audience(&self) -> String {
        match self.transport.as_str() {
            // The program, its arguments and its environment all decide
            // where the token ends up, so a change to any of them is a new
            // audience and drops the stored token. JSON keeps the parts
            // unambiguous (no separator tricks).
            "stdio" => format!(
                "stdio:{}",
                serde_json::json!([
                    self.command.as_deref().unwrap_or(""),
                    self.args,
                    self.env,
                    self.token_env.as_deref().unwrap_or("")
                ])
            ),
            t => format!(
                "{t}:{}",
                self.url
                    .as_ref()
                    .map(|u| u.origin().ascii_serialization())
                    .unwrap_or_default()
            ),
        }
    }

    pub(crate) fn needs_token(&self) -> bool {
        match self.transport.as_str() {
            "stdio" => self.token_env.is_some(),
            _ => self.auth,
        }
    }
}

fn client_info() -> ClientConfig {
    ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("HalpTaskHub", env!("CARGO_PKG_VERSION")),
    )
}

/// Calls the tool over an already connected transport.
pub async fn call_over<T, E, A>(
    transport: T,
    lifecycle: ClientLifecycleMode,
    tool: &str,
    args: &Map<String, Value>,
) -> Result<Value, SyncError>
where
    T: rmcp::transport::IntoTransport<rmcp::RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let client = serve_client_with_lifecycle(client_info(), transport, lifecycle)
        .await
        .map_err(|e| SyncError::transient(format!("MCP server did not start: {e}")))?;
    let params = CallToolRequestParams::new(tool.to_string()).with_arguments(args.clone());
    let r = client.call_tool_once(params).await;
    let _ = client.cancel().await;
    let r = match r.map_err(|e| SyncError::transient(format!("MCP tool call failed: {e}")))? {
        CallToolResponse::Complete(r) => r,
        _ => {
            return Err(SyncError::Config(
                "the MCP tool asked for input or deferred its answer; not supported".into(),
            ));
        }
    };
    if r.is_error == Some(true) {
        let msg = r
            .content
            .iter()
            .find_map(|c| c.as_text().map(|t| t.text.clone()))
            .unwrap_or_default();
        return Err(SyncError::transient(format!(
            "the MCP tool reported an error: {}",
            cap(&msg, 200)
        )));
    }
    if let Some(v) = r.structured_content {
        return Ok(v);
    }
    let text: String = r
        .content
        .iter()
        .filter_map(|c| c.as_text().map(|t| t.text.as_str()))
        .collect::<Vec<_>>()
        .join("");
    serde_json::from_str(&text)
        .map_err(|_| SyncError::Config("the MCP tool did not answer JSON".into()))
}

/// The records in a tool answer: the answer itself if it is an array, or
/// the first array under a familiar key.
fn records(v: &Value) -> (Vec<Value>, bool) {
    records_limited(v, MAX_ITEMS)
}

/// [`records`] with its own cap.
pub(crate) fn records_limited(v: &Value, limit: usize) -> (Vec<Value>, bool) {
    let arr = match v {
        Value::Array(a) => Some(a),
        Value::Object(m) => [
            "messages", "issues", "items", "results", "nodes", "data", "matches", "events", "value",
        ]
        .iter()
        .find_map(|k| match m.get(*k) {
            Some(Value::Array(a)) => Some(a),
            Some(Value::Object(inner)) => {
                ["nodes", "issues", "messages", "matches", "items", "events"]
                    .iter()
                    .find_map(|k| inner.get(*k).and_then(Value::as_array))
            }
            _ => None,
        }),
        _ => None,
    };
    match arr {
        None => (Vec::new(), false),
        Some(a) => (
            a.iter()
                .filter(|x| x.is_object())
                .take(limit)
                .cloned()
                .collect(),
            a.len() <= limit,
        ),
    }
}

fn first_str<'a>(v: &'a Value, paths: &[&str]) -> Option<&'a str> {
    paths.iter().find_map(|p| str_at(v, p))
}

fn first_line(s: &str) -> String {
    let l = s
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    cap(l, 200)
}

/// Slack message → `dev.ask`.
pub fn map_slack(r: &Value) -> Option<Value> {
    let ts = r.get("ts").or(r.get("timestamp")).or(r.get("date"))?;
    let received = timestamp(Some(ts))?;
    let from = first_str(
        r,
        &["/user_name", "/username", "/user/name", "/from", "/user"],
    )?;
    let channel = first_str(r, &["/channel_name", "/channel/name", "/channel"]);
    let text = first_str(r, &["/text", "/message"]).unwrap_or("(no text)");
    let ts_key = match ts {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut f = Map::new();
    f.insert("source".into(), json!("slack"));
    f.insert("from".into(), json!(cap(from, 200)));
    f.insert("received_at".into(), json!(received));
    if let Some(c) = channel {
        f.insert("channel".into(), json!(cap(c, 200)));
    }
    if let Some(u) = https_url(first_str(r, &["/permalink", "/url"])) {
        f.insert("permalink".into(), json!(u));
    }
    Some(json!({
        "external_id": cap(&format!("slack:{}:{}", channel.unwrap_or("-"), ts_key), 300),
        "title": first_line(text),
        "kind": "task",
        "type_id": "dev.ask",
        "tags": ["slack"],
        "notes": cap(text, 4000),
        "fields": f,
    }))
}

/// Linear or Jira issue → `ext.issue`.
pub fn map_issue(source: &str, r: &Value) -> Option<Value> {
    let (key, title, state, priority, project, assignee, updated, url) = if source == "jira" {
        (
            first_str(r, &["/key"])?,
            first_str(r, &["/fields/summary", "/summary", "/title"])?,
            first_str(r, &["/fields/status/name", "/status/name", "/status"]),
            first_str(r, &["/fields/priority/name", "/priority/name", "/priority"]),
            first_str(r, &["/fields/project/key", "/project/key", "/project"]),
            first_str(
                r,
                &[
                    "/fields/assignee/displayName",
                    "/assignee/displayName",
                    "/assignee",
                ],
            ),
            r.pointer("/fields/updated").or(r.get("updated")),
            first_str(r, &["/url", "/browse_url", "/webUrl"]),
        )
    } else {
        (
            first_str(r, &["/identifier", "/key", "/id"])?,
            first_str(r, &["/title"])?,
            first_str(r, &["/state/name", "/status", "/state"]),
            first_str(r, &["/priorityLabel", "/priority/name", "/priority"]),
            first_str(r, &["/project/name", "/team/key", "/project"]),
            first_str(r, &["/assignee/name", "/assignee/displayName", "/assignee"]),
            r.get("updatedAt").or(r.get("updated_at")),
            first_str(r, &["/url"]),
        )
    };
    let priority = priority.map(str::to_string).or_else(|| {
        // Linear sends a number.
        r.get("priority")
            .and_then(Value::as_i64)
            .map(|p| p.to_string())
    });
    let mut f = Map::new();
    f.insert("source".into(), json!(source));
    f.insert("key".into(), json!(cap(key, 100)));
    if let Some(u) = https_url(url) {
        f.insert("url".into(), json!(u));
    }
    for (k, v, n) in [
        ("state", state.map(str::to_string), 100),
        ("priority", priority, 50),
        ("project", project.map(str::to_string), 200),
        ("assignee", assignee.map(str::to_string), 200),
    ] {
        if let Some(v) = v {
            f.insert(k.into(), json!(cap(&v, n)));
        }
    }
    if let Some(t) = timestamp(updated) {
        f.insert("updated_at".into(), json!(t));
    }
    let desc = first_str(r, &["/description", "/fields/description"]);
    let mut item = json!({
        "external_id": cap(&format!("{source}:{key}"), 300),
        "title": cap(&format!("{key}: {}", first_line(title)), 300),
        "kind": "task",
        "type_id": "ext.issue",
        "tags": [source],
        "fields": f,
    });
    if let Some(d) = desc {
        item["notes"] = json!(cap(d, 4000));
    }
    Some(item)
}

/// Maps a tool answer through an adapter.
pub fn adapt(adapter: &str, answer: &Value) -> Batch {
    let (recs, all) = records(answer);
    let items: Vec<Value> = recs
        .iter()
        .filter_map(|r| match adapter {
            "slack" => map_slack(r),
            other => map_issue(other, r),
        })
        .collect();
    Batch {
        scope: if adapter == "slack" { "asks" } else { "issues" }.into(),
        complete: all && !recs.is_empty(),
        items,
        patches: Vec::new(),
    }
}

#[derive(Default)]
pub struct Mcp;

/// Calls the configured tool (stdio or http) within [`CALL_TIMEOUT`].
pub(crate) async fn call(s: &McpSettings, cx: &SyncCx<'_>) -> Result<Value, SyncError> {
    let call = async {
        match s.transport.as_str() {
            "stdio" => stdio(s, cx).await,
            _ => http(s, cx).await,
        }
    };
    tokio::select! {
        _ = cx.cancel.cancelled() => Err(SyncError::Cancelled),
        _ = tokio::time::sleep(CALL_TIMEOUT) => Err(SyncError::transient("the MCP server did not answer in time")),
        r = call => r,
    }
}

impl Mcp {
    async fn run(&self, cx: SyncCx<'_>) -> Result<Vec<Batch>, SyncError> {
        let s = McpSettings::parse(cx.settings).map_err(SyncError::Config)?;
        s.check(cx.http).map_err(SyncError::Config)?;
        let answer = call(&s, &cx).await?;
        Ok(vec![adapt(&s.adapter, &answer)])
    }
}

async fn stdio(s: &McpSettings, cx: &SyncCx<'_>) -> Result<Value, SyncError> {
    let dir: PathBuf = cx.data_dir.to_path_buf();
    let mut cmd = tokio::process::Command::new(s.command.as_deref().unwrap_or_default());
    cmd.args(&s.args)
        .current_dir(&dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HOME", &dir)
        .env("TMPDIR", &dir)
        .env("LANG", "C.UTF-8")
        .envs(&s.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let (Some(name), Some(t)) = (&s.token_env, cx.token) {
        cmd.env(name, t);
    }
    let mut child = cmd
        .spawn()
        .map_err(|_| SyncError::Config("could not start the MCP server command".into()))?;
    let out = child
        .stdout
        .take()
        .ok_or_else(|| SyncError::transient("no stdout"))?;
    let inp = child
        .stdin
        .take()
        .ok_or_else(|| SyncError::transient("no stdin"))?;
    let r = call_over(
        (out.take(MAX_STDIO_BYTES), inp),
        ClientLifecycleMode::Initialize,
        &s.tool,
        &s.tool_args,
    )
    .await;
    let _ = child.start_kill();
    let _ = child.wait().await;
    r
}

async fn http(s: &McpSettings, cx: &SyncCx<'_>) -> Result<Value, SyncError> {
    use rmcp::transport::StreamableHttpClientTransport;
    use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
    let url = s
        .url
        .as_ref()
        .ok_or_else(|| SyncError::Config("no url".into()))?;
    let client = Http::builder(cx.http.options())
        .build()
        .map_err(|e| SyncError::Config(format!("HTTP client: {e}")))?;
    // rmcp caps a JSON answer at 16 MiB; an SSE answer is only capped per
    // event, so keep each event to what any other response may be.
    let mut cfg = StreamableHttpClientTransportConfig::with_uri(url.as_str())
        .max_sse_event_size(MAX_HTTP_EVENT_BYTES);
    if s.auth {
        let t = cx
            .token
            .ok_or_else(|| SyncError::Auth("no token stored".into()))?;
        cfg = cfg.auth_header(t.to_string());
    }
    let transport = StreamableHttpClientTransport::with_client(client, cfg);
    call_over(
        transport,
        ClientLifecycleMode::Initialize,
        &s.tool,
        &s.tool_args,
    )
    .await
}

impl Connector for Mcp {
    fn kind(&self) -> &'static str {
        "mcp"
    }

    fn validate(&self, settings: &Value, http: &Http) -> Result<Value, String> {
        let s = McpSettings::parse(settings)?;
        s.check(http)?;
        serde_json::to_value(&s).map_err(|e| e.to_string())
    }

    fn needs_token(&self, settings: &Value) -> bool {
        McpSettings::parse(settings).is_ok_and(|s| s.needs_token())
    }

    fn secret_audience(&self, settings: &Value) -> String {
        McpSettings::parse(settings).map_or_else(|_| "invalid".into(), |s| s.audience())
    }

    fn private_dir(&self, _settings: &Value) -> bool {
        true
    }

    fn sync<'a>(&'a self, cx: SyncCx<'a>) -> BoxFuture<'a, Result<Vec<Batch>, SyncError>> {
        Box::pin(self.run(cx))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn stdio_audience_covers_args_and_env() {
        let base = McpSettings::parse(&serde_json::json!({
            "transport": "stdio", "command": "/usr/bin/npx",
            "args": ["@vendor/mcp"], "token_env": "SLACK_TOKEN", "adapter": "slack", "tool": "search"
        }))
        .unwrap();
        let mut args = base.clone();
        args.args = vec!["other-pkg".into()];
        let mut env = base.clone();
        env.env
            .insert("SLACK_API_HOST".into(), "evil.example".into());
        assert_ne!(base.audience(), args.audience());
        assert_ne!(base.audience(), env.audience());
        assert_eq!(base.audience(), base.clone().audience());
    }

    #[test]
    fn env_names() {
        assert!(env_name_ok("SLACK_TEAM"));
        assert!(!env_name_ok("LD_PRELOAD"));
        assert!(!env_name_ok("DYLD_INSERT_LIBRARIES"));
        assert!(!env_name_ok("NODE_OPTIONS"));
        assert!(!env_name_ok("path"));
        assert!(env_name_ok("SENTRY_AUTH_TOKEN"));
        assert!(env_name_ok("OPENAI_API_KEY"));
        for bad in [
            "JAVA_TOOL_OPTIONS",
            "_JAVA_OPTIONS",
            "JDK_JAVA_OPTIONS",
            "PERL5LIB",
            "NODE_PATH",
            "RUBYLIB",
            "PYTHONHOME",
            "GIT_SSH_COMMAND",
            "SSH_AUTH_SOCK",
            "NPM_CONFIG_PREFIX",
            "LC_ALL",
            "XDG_CONFIG_HOME",
            "DOTNET_STARTUP_HOOKS",
            "CORECLR_PROFILER",
            "COR_ENABLE_PROFILING",
            "GCONV_PATH",
            "CLASSPATH",
            "SSL_CERT_FILE",
            "NODE_EXTRA_CA_CERTS",
            "HTTPS_PROXY",
            "Https_Proxy",
            "https_proxy",
            "IFS",
            "SHELL",
            "TMPDIR",
            "_LEADING",
        ] {
            assert!(!env_name_ok(bad), "{bad} accepted");
        }
    }

    #[test]
    fn adapters_pick_records() {
        let a = json!({"issues": {"nodes": [
            {"identifier": "ENG-1", "title": "Fix\nit", "state": {"name": "Todo"}, "priority": 2,
             "url": "https://linear.app/x/issue/ENG-1", "updatedAt": "2026-11-01T10:00:00.000Z"},
            {"title": "no key"}
        ]}});
        let b = adapt("linear", &a);
        assert_eq!(b.scope, "issues");
        assert!(b.complete);
        assert_eq!(b.items.len(), 1);
        assert_eq!(b.items[0]["external_id"], "linear:ENG-1");
        assert_eq!(b.items[0]["title"], "ENG-1: Fix");
        assert_eq!(b.items[0]["fields"]["priority"], "2");
        assert_eq!(b.items[0]["fields"]["updated_at"], "2026-11-01T10:00:00Z");

        let s = adapt(
            "slack",
            &json!({"messages": [
                {"ts": "1730000000.0001", "user_name": "ana", "channel": "C1", "text": "can you look?",
                 "permalink": "javascript:alert(1)"}
            ]}),
        );
        assert_eq!(s.items[0]["type_id"], "dev.ask");
        assert!(s.items[0]["fields"].get("permalink").is_none());

        let j = adapt(
            "jira",
            &json!([{"key": "OPS-2", "fields": {"summary": "Disk", "status": {"name": "Open"}, "updated": "2026-11-01T10:00:00.000+0000"}}]),
        );
        assert_eq!(j.items[0]["fields"]["state"], "Open");
        assert!(!adapt("jira", &json!("nope")).complete);
    }
}
