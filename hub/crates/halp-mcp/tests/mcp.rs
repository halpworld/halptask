//! MCP integration tests: an rmcp client talks to [`HalpServer`] over an
//! in-process duplex pipe, and to the real Unix socket host.
#![allow(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use chrono::{NaiveDate, TimeZone, Utc};
use halp_core::{Core, Ctx, FixedClock, Request, Response};
use halp_mcp::HalpServer;
use halp_store::{RawKeyProvider, Store};
use rmcp::model::{
    CallToolRequestParams, ClientCapabilities, ClientConfig, GetPromptRequestParams,
    Implementation, ProgressNotificationParam, ProtocolVersion, ReadResourceRequestParams,
    ResourceUpdatedNotificationParam, ServerNotification, SubscribeRequestParams,
    SubscriptionFilter,
};
use rmcp::service::{
    ClientLifecycleMode, NotificationContext, RoleClient, RunningService,
    serve_client_with_lifecycle,
};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::{Value, json};
use tempfile::TempDir;
use tokio::sync::mpsc;

fn clock() -> Arc<FixedClock> {
    Arc::new(FixedClock {
        now: Utc.with_ymd_and_hms(2026, 11, 10, 9, 0, 0).unwrap(),
        today: NaiveDate::from_ymd_opt(2026, 11, 10).unwrap(),
    })
}

struct H {
    core: Arc<Core>,
    // Keeps the database's temp dir alive; only the Unix socket tests read it.
    #[cfg_attr(not(unix), allow(dead_code))]
    dir: TempDir,
}

fn h() -> H {
    h_with(|_| {})
}

fn h_with(f: impl FnOnce(&mut Core)) -> H {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap();
    let mut core = Core::with_store(store, clock()).unwrap();
    f(&mut core);
    H {
        core: Arc::new(core),
        dir,
    }
}

impl H {
    fn user(&self, cmd: &str, args: Value) -> Value {
        match self.core.dispatch(&Ctx::user(), Request::new(cmd, args)) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd}: {e:?}"),
        }
    }
    fn pair(&self, id: &str) -> String {
        self.user("clients.pair", json!({"name": id, "id": id}))["token"]
            .as_str()
            .unwrap()
            .to_string()
    }
    fn pending(&self) -> Vec<Value> {
        self.user("consent.list", json!({}))["pending"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    async fn wait_pending(&self) -> Value {
        for _ in 0..500 {
            if let Some(p) = self.pending().into_iter().next() {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("nothing was parked");
    }
}

/// Records the notifications the server sends outside subscriptions.
#[derive(Clone)]
struct Client {
    info: ClientConfig,
    progress: mpsc::UnboundedSender<ProgressNotificationParam>,
    updated: mpsc::UnboundedSender<String>,
}

impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        self.info.clone()
    }
    async fn on_progress(
        &self,
        p: ProgressNotificationParam,
        _cx: NotificationContext<RoleClient>,
    ) {
        let _ = self.progress.send(p);
    }
    async fn on_resource_updated(
        &self,
        p: ResourceUpdatedNotificationParam,
        _cx: NotificationContext<RoleClient>,
    ) {
        let _ = self.updated.send(p.uri);
    }
}

struct Conn {
    client: RunningService<RoleClient, Client>,
    progress: mpsc::UnboundedReceiver<ProgressNotificationParam>,
    updated: mpsc::UnboundedReceiver<String>,
}

fn client(version: Option<ProtocolVersion>) -> (Client, Conn0) {
    let mut info = ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("halp-test", "0.0.0"),
    );
    if let Some(v) = version {
        info = info.with_protocol_version(v);
    }
    let (ptx, prx) = mpsc::unbounded_channel();
    let (utx, urx) = mpsc::unbounded_channel();
    (
        Client {
            info,
            progress: ptx,
            updated: utx,
        },
        Conn0 {
            progress: prx,
            updated: urx,
        },
    )
}

struct Conn0 {
    progress: mpsc::UnboundedReceiver<ProgressNotificationParam>,
    updated: mpsc::UnboundedReceiver<String>,
}

/// Starts a client: `server/discover` at the latest revision, or the
/// `initialize` handshake at the version the handler asks for.
async fn start<T, E, A>(handler: Client, t: T, discover: bool) -> RunningService<RoleClient, Client>
where
    T: rmcp::transport::IntoTransport<RoleClient, E, A>,
    E: std::error::Error + Send + Sync + 'static,
{
    let mode = if discover {
        ClientLifecycleMode::Discover {
            preferred_versions: vec![ProtocolVersion::LATEST],
        }
    } else {
        ClientLifecycleMode::Initialize
    };
    serve_client_with_lifecycle(handler, t, mode).await.unwrap()
}

/// Connects a client to a server acting as `agent:<id>` over a duplex pipe.
async fn connect(h: &H, id: &str, version: Option<ProtocolVersion>) -> Conn {
    let server = HalpServer::new(h.core.clone(), Ctx::agent(id).with_session("s-test"))
        .with_progress_interval(Duration::from_millis(20));
    let (a, b) = tokio::io::duplex(1 << 20);
    let (sr, sw) = tokio::io::split(a);
    tokio::spawn(async move {
        if let Ok(s) = server.serve((sr, sw)).await {
            let _ = s.waiting().await;
        }
    });
    let discover = version.is_none();
    let (handler, c0) = client(version);
    let (cr, cw) = tokio::io::split(b);
    let client = start(handler, (cr, cw), discover).await;
    Conn {
        client,
        progress: c0.progress,
        updated: c0.updated,
    }
}

fn args(v: Value) -> serde_json::Map<String, Value> {
    v.as_object().cloned().unwrap()
}

async fn call(c: &Conn, tool: &str, a: Value) -> rmcp::model::CallToolResult {
    c.client
        .call_tool(CallToolRequestParams::new(tool.to_string()).with_arguments(args(a)))
        .await
        .unwrap()
}

fn structured(r: &rmcp::model::CallToolResult) -> Value {
    assert_ne!(r.is_error, Some(true), "{r:?}");
    r.structured_content.clone().unwrap()
}

fn error_text(r: &rmcp::model::CallToolResult) -> String {
    assert_eq!(r.is_error, Some(true), "{r:?}");
    serde_json::to_string(&r.content).unwrap()
}

#[tokio::test]
async fn negotiates_the_latest_protocol() {
    let h = h();
    h.pair("bot");
    let c = connect(&h, "bot", None).await;
    let info = c.client.peer_info().unwrap();
    assert_eq!(info.protocol_version, ProtocolVersion::LATEST);
    assert_eq!(
        info.protocol_version.to_string(),
        halp_mcp::PROTOCOL_VERSION
    );

    let old = connect(&h, "bot", Some(ProtocolVersion::LATEST_WITH_INITIALIZE)).await;
    assert_eq!(
        old.client.peer_info().unwrap().protocol_version.to_string(),
        halp_mcp::PROTOCOL_VERSION_WITH_INITIALIZE
    );
}

#[tokio::test]
async fn tools_cover_the_agent_commands_only() {
    let h = h();
    h.pair("bot");
    let c = connect(&h, "bot", None).await;
    let tools = c.client.list_all_tools().await.unwrap();
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    for want in [
        "tasks_create",
        "tasks_delete",
        "day_state",
        "layout_get",
        "history_undo",
    ] {
        assert!(names.iter().any(|n| n == want), "missing {want}: {names:?}");
    }
    for never in [
        "consent_approve",
        "consent_deny",
        "clients_pair",
        "clients_revoke",
        "clients_list",
        "grants_set",
        "rules_create",
        "rules_delete",
        "history_purge",
        "import_tui",
    ] {
        assert!(!names.iter().any(|n| n == never), "{never} is exposed");
    }
    let get = |n: &str| tools.iter().find(|t| t.name == n).unwrap();
    let list = get("tasks_list").annotations.clone().unwrap();
    assert_eq!(list.read_only_hint, Some(true));
    assert_eq!(list.open_world_hint, Some(false));
    let del = get("tasks_delete").annotations.clone().unwrap();
    assert_eq!(del.destructive_hint, Some(true));
    assert_eq!(del.read_only_hint, Some(false));
    assert!(
        get("tasks_delete")
            .description
            .as_deref()
            .unwrap()
            .contains("always waits for the user's confirmation")
    );
}

#[tokio::test]
async fn calling_a_tool_runs_the_command_as_the_agent() {
    let h = h();
    h.pair("bot");
    let c = connect(&h, "bot", None).await;
    let r = call(&c, "tasks_create", json!({"title": "From MCP"})).await;
    let id = structured(&r)["id"].as_str().unwrap().to_string();
    let ev = h.user("history.list", json!({"limit": 5}));
    let first = &ev["events"][0];
    assert_eq!(first["actor"], "agent:bot");
    assert_eq!(first["entity"]["id"], id);

    // Bad arguments are a tool error, not a protocol error.
    let r = call(&c, "tasks_create", json!({"nope": 1})).await;
    assert!(error_text(&r).contains("invalid_args"));
    // Unknown tools (and hidden ones) are protocol errors.
    let e = c
        .client
        .call_tool(CallToolRequestParams::new("consent_approve"))
        .await;
    assert!(e.is_err());
}

#[tokio::test]
async fn resources_read_the_state() {
    let h = h();
    h.pair("bot");
    let id = h.user("tasks.create", json!({"title": "Read me"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let c = connect(&h, "bot", None).await;
    let res = c.client.list_all_resources().await.unwrap();
    let uris: Vec<&str> = res.iter().map(|r| r.uri.as_str()).collect();
    assert!(uris.contains(&"halp://day/state"));
    assert!(uris.contains(&"halp://layout/current"));
    let templates = c.client.list_all_resource_templates().await.unwrap();
    assert!(
        templates
            .iter()
            .any(|t| t.uri_template == "halp://task/{id}")
    );

    let read = |uri: String| {
        let c = &c;
        async move {
            let r = c
                .client
                .read_resource(ReadResourceRequestParams::new(uri))
                .await?;
            let v = serde_json::to_value(&r.contents[0]).unwrap();
            Ok::<Value, rmcp::ServiceError>(
                serde_json::from_str(v["text"].as_str().unwrap()).unwrap(),
            )
        }
    };
    let task = read(format!("halp://task/{id}")).await.unwrap();
    assert_eq!(task["title"], "Read me");
    assert!(read("halp://day/state".into()).await.is_ok());
    assert!(read("halp://layout/current".into()).await.is_ok());
    let ev = read("halp://events".into()).await.unwrap();
    let first = ev["events"][0]["id"].as_str().unwrap().to_string();
    assert!(read(format!("halp://events?since={first}")).await.is_ok());
    assert!(read("halp://task/01NOPE".into()).await.is_err());
    assert!(read("halp://task/../../etc".into()).await.is_err());
    assert!(read("file:///etc/passwd".into()).await.is_err());
}

#[tokio::test]
async fn prompts_come_from_the_packs() {
    let h = h();
    h.pair("bot");
    let c = connect(&h, "bot", None).await;
    let prompts = c.client.list_all_prompts().await.unwrap();
    let names: Vec<&str> = prompts.iter().map(|p| p.name.as_str()).collect();
    for want in [
        "morning-triage",
        "end-of-day",
        "prep-one-on-one",
        "weekly-update",
        "weekly-esg-digest",
        "data-chase-round",
        "prepare-board-brief",
        "evidence-gap-check",
    ] {
        assert!(names.contains(&want), "{want}: {names:?}");
    }
    let p = c
        .client
        .get_prompt(
            GetPromptRequestParams::new("prep-one-on-one")
                .with_arguments(args(json!({"person": "Ana"}))),
        )
        .await
        .unwrap();
    let text = serde_json::to_string(&p.messages).unwrap();
    assert!(text.contains("Ana"));
    assert!(text.to_lowercase().contains("untrusted"));
    assert!(text.contains("Draft, do not send"));
    // A required argument is required.
    assert!(
        c.client
            .get_prompt(GetPromptRequestParams::new("prep-one-on-one"))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn a_t3_call_waits_with_progress_and_returns_the_approved_result() {
    let h = h();
    h.pair("bot");
    let id = h.user("tasks.create", json!({"title": "Doomed"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut c = connect(&h, "bot", None).await;
    let peer = c.client.peer().clone();
    let tid = id.clone();
    let task = tokio::spawn(async move {
        // rmcp's client attaches a progress token to every call.
        peer.call_tool(
            CallToolRequestParams::new("tasks_delete").with_arguments(args(json!({"id": tid}))),
        )
        .await
    });
    let pending = h.wait_pending().await;
    assert_eq!(pending["actor"], "agent:bot");
    assert_eq!(pending["command"], "tasks.delete");
    let prog = tokio::time::timeout(Duration::from_secs(5), c.progress.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(prog.message.unwrap().contains("Waiting for the user"));
    h.user("consent.approve", json!({"id": pending["id"]}));
    let r = task.await.unwrap().unwrap();
    assert_ne!(r.is_error, Some(true), "{r:?}");
    let gone = h
        .core
        .dispatch(&Ctx::user(), Request::new("tasks.get", json!({"id": id})));
    assert!(!gone.is_ok());
}

#[tokio::test]
async fn a_denied_or_expired_call_is_a_tool_error() {
    let h = h_with(|c| c.set_consent_timeout(Duration::from_millis(150)));
    h.pair("bot");
    let a = h.user("tasks.create", json!({"title": "A"}))["id"].clone();
    let b = h.user("tasks.create", json!({"title": "B"}))["id"].clone();
    let c = connect(&h, "bot", None).await;

    let peer = c.client.peer().clone();
    let task = tokio::spawn(async move {
        peer.call_tool(
            CallToolRequestParams::new("tasks_delete").with_arguments(args(json!({"id": a}))),
        )
        .await
    });
    let pending = h.wait_pending().await;
    h.user("consent.deny", json!({"id": pending["id"]}));
    let r = task.await.unwrap().unwrap();
    assert!(error_text(&r).contains("denied"));

    let r = call(&c, "tasks_delete", json!({"id": b})).await;
    assert!(error_text(&r).contains("in time"));
}

#[tokio::test]
async fn an_agent_cannot_approve_its_own_request_through_any_tool() {
    let h = h();
    h.pair("bot");
    let id = h.user("tasks.create", json!({"title": "Keep"}))["id"].clone();
    let c = connect(&h, "bot", None).await;
    let peer = c.client.peer().clone();
    let task = tokio::spawn(async move {
        peer.call_tool(
            CallToolRequestParams::new("tasks_delete").with_arguments(args(json!({"id": id}))),
        )
        .await
    });
    let pending = h.wait_pending().await;
    // A second connection for the same client tries every way in.
    let c2 = connect(&h, "bot", None).await;
    for tool in ["consent_approve", "consent_deny"] {
        assert!(
            c2.client
                .call_tool(
                    CallToolRequestParams::new(tool)
                        .with_arguments(args(json!({"id": pending["id"]})))
                )
                .await
                .is_err()
        );
    }
    assert_eq!(h.pending().len(), 1);
    task.abort();
}

#[tokio::test]
async fn subscriptions_notify_on_change() {
    let h = h();
    h.pair("bot");
    let id = h.user("tasks.create", json!({"title": "Watch"}))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("halp://task/{id}");

    // 2026-07-28: subscriptions/listen.
    let c = connect(&h, "bot", None).await;
    let filter = SubscriptionFilter::builder()
        .resource_subscriptions([uri.clone(), "halp://day/state".to_string()])
        .build();
    let mut sub = c.client.listen(filter).await.unwrap();
    assert!(
        sub.acknowledged()
            .resource_subscriptions
            .as_ref()
            .unwrap()
            .contains(&uri)
    );
    h.user(
        "tasks.update",
        json!({"id": id, "patch": {"title": "Watched"}}),
    );
    let mut seen = Vec::new();
    while seen.len() < 2 {
        let n = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let ServerNotification::ResourceUpdatedNotification(n) = n {
            seen.push(n.params.uri);
        }
    }
    assert!(seen.contains(&uri), "{seen:?}");

    // 2025-11-25: resources/subscribe.
    let mut old = connect(&h, "bot", Some(ProtocolVersion::LATEST_WITH_INITIALIZE)).await;
    #[allow(deprecated)]
    old.client
        .subscribe(SubscribeRequestParams::new(uri.clone()))
        .await
        .unwrap();
    h.user(
        "tasks.update",
        json!({"id": id, "patch": {"title": "Again"}}),
    );
    let got = tokio::time::timeout(Duration::from_secs(5), old.updated.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, uri);
}

/// Review S5: an agent without the grant subscribed to a private item and
/// learned each time it changed.
#[tokio::test]
async fn agents_cannot_subscribe_to_items_they_cannot_read() {
    let h = h();
    h.pair("bot");
    let note = h.user(
        "tasks.create",
        json!({"title": "Private", "type_id": "em.private_note",
               "fields": {"person_id": "ana", "body": "b"}}),
    )["id"]
        .as_str()
        .unwrap()
        .to_string();
    let uri = format!("halp://task/{note}");
    let missing = "halp://task/no-such-item".to_string();

    let c = connect(&h, "bot", None).await;
    let filter = SubscriptionFilter::builder()
        .resource_subscriptions([uri.clone(), missing.clone(), "halp://day/state".to_string()])
        .build();
    let mut sub = c.client.listen(filter).await.unwrap();
    let acked = sub
        .acknowledged()
        .resource_subscriptions
        .clone()
        .unwrap_or_default();
    assert!(!acked.contains(&uri), "{acked:?}");
    assert!(!acked.contains(&missing), "{acked:?}");
    h.user(
        "tasks.update",
        json!({"id": note, "patch": {"title": "Changed"}}),
    );
    // Only the day state may arrive, never the private item.
    while let Ok(Ok(Some(n))) = tokio::time::timeout(Duration::from_millis(500), sub.next()).await {
        if let ServerNotification::ResourceUpdatedNotification(n) = n {
            assert_ne!(n.params.uri, uri);
        }
    }

    let mut old = connect(&h, "bot", Some(ProtocolVersion::LATEST_WITH_INITIALIZE)).await;
    #[allow(deprecated)]
    let r = old
        .client
        .subscribe(SubscribeRequestParams::new(uri.clone()))
        .await;
    assert!(r.is_err(), "{r:?}");
    h.user(
        "tasks.update",
        json!({"id": note, "patch": {"title": "Again"}}),
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(300), old.updated.recv())
            .await
            .is_err()
    );
}

#[cfg(unix)]
mod socket {
    use std::os::unix::fs::PermissionsExt;

    use halp_mcp::host::{AUTH_PREFIX, USER_AUTH_PREFIX, USER_TOKEN_FILE};
    use halp_mcp::{HostOptions, SOCKET_NAME};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    use super::*;

    async fn hello(path: &std::path::Path, line: &str) -> (String, UnixStream) {
        let mut s = UnixStream::connect(path).await.unwrap();
        s.write_all(line.as_bytes()).await.unwrap();
        let mut r = BufReader::new(&mut s);
        let mut reply = String::new();
        r.read_line(&mut reply).await.unwrap();
        (reply, s)
    }

    #[tokio::test]
    async fn the_host_authenticates_clients_and_keeps_its_files_private() {
        let h = h();
        let token = h.pair("bot");
        let data = h.dir.path().join("data");
        let host = halp_mcp::serve(h.core.clone(), HostOptions::new(&data))
            .await
            .unwrap();
        let sock = host.socket_path().to_path_buf();
        assert_eq!(sock, data.join(SOCKET_NAME));
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&data), 0o700);
        assert_eq!(mode(&sock), 0o600);
        assert_eq!(mode(&data.join(USER_TOKEN_FILE)), 0o600);

        // A second host on the same directory is refused.
        assert!(
            halp_mcp::serve(h.core.clone(), HostOptions::new(&data))
                .await
                .is_err()
        );

        let (r, _) = hello(&sock, &format!("{AUTH_PREFIX}not-a-token\n")).await;
        assert_eq!(r, "ERR unauthorized\n");
        let (r, _) = hello(&sock, "GET / HTTP/1.1\n").await;
        assert_eq!(r, "ERR unauthorized\n");
        // The user token is not a client token, and a client token is not
        // the user token.
        let user_token = std::fs::read_to_string(data.join(USER_TOKEN_FILE)).unwrap();
        let (r, _) = hello(&sock, &format!("{AUTH_PREFIX}{}", user_token)).await;
        assert_eq!(r, "ERR unauthorized\n");
        let (r, _) = hello(&sock, &format!("{USER_AUTH_PREFIX}{token}\n")).await;
        assert_eq!(r, "ERR unauthorized\n");

        // A good token gets MCP as agent:bot.
        let (r, s) = hello(&sock, &format!("{AUTH_PREFIX}{token}\n")).await;
        assert_eq!(r, "OK agent:bot\n");
        let (handler, _c0) = client(None);
        let (sr, sw) = s.into_split();
        let mcp = start(handler, (sr, sw), true).await;
        let out = mcp
            .call_tool(
                CallToolRequestParams::new("tasks_create")
                    .with_arguments(args(json!({"title": "Over the socket"}))),
            )
            .await
            .unwrap();
        assert_ne!(out.is_error, Some(true));
        let ev = h.user("history.list", json!({"limit": 1}));
        assert_eq!(ev["events"][0]["actor"], "agent:bot");

        // Revoking stops calls on the open connection and new connections.
        h.user("clients.revoke", json!({"id": "bot"}));
        let out = mcp
            .call_tool(
                CallToolRequestParams::new("tasks_create")
                    .with_arguments(args(json!({"title": "After revoke"}))),
            )
            .await
            .unwrap();
        assert!(error_text(&out).contains("denied"));
        let (r, _) = hello(&sock, &format!("{AUTH_PREFIX}{token}\n")).await;
        assert_eq!(r, "ERR unauthorized\n");

        host.shutdown().await;
        assert!(!sock.exists());
        assert!(!data.join(USER_TOKEN_FILE).exists());
    }

    #[tokio::test]
    async fn the_user_channel_only_decides_consent() {
        let h = h();
        h.pair("bot");
        let data = h.dir.path().join("data");
        let host = halp_mcp::serve(h.core.clone(), HostOptions::new(&data))
            .await
            .unwrap();
        let sock = host.socket_path().to_path_buf();
        let user_token = std::fs::read_to_string(data.join(USER_TOKEN_FILE)).unwrap();
        let (r, mut s) = hello(&sock, &format!("{USER_AUTH_PREFIX}{user_token}")).await;
        assert_eq!(r, "OK user\n");
        let (rd, mut wr) = s.split();
        let mut rd = BufReader::new(rd);
        let mut ask = async |req: Value| {
            wr.write_all(format!("{req}\n").as_bytes()).await.unwrap();
            let mut line = String::new();
            rd.read_line(&mut line).await.unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        };
        let r = ask(json!({"command": "consent.list", "args": {}})).await;
        assert_eq!(r["ok"], true, "{r}");
        for cmd in [
            "tasks.create",
            "clients.pair",
            "history.purge",
            "grants.set",
            "rules.create",
            "consent.approve",
        ] {
            let r = ask(json!({"command": cmd, "args": {"title": "x", "name": "x"}})).await;
            assert_eq!(r["error"]["code"], "denied", "{cmd}: {r}");
        }
        // Finding: nothing could cut an agent off while the Hub ran. The
        // channel lists and revokes clients (it only takes power away).
        let r = ask(json!({"command": "clients.list", "args": {}})).await;
        assert_eq!(r["ok"], true, "{r}");
        assert!(r["result"].to_string().contains("bot"), "{r}");
        let r = ask(json!({"command": "clients.revoke", "args": {"id": "bot"}})).await;
        assert_eq!(r["ok"], true, "{r}");
        let listed = h.user("clients.list", json!({}));
        let bot = listed["clients"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == "bot")
            .cloned()
            .unwrap();
        assert!(!bot["revoked_at"].is_null(), "{listed}");
        host.shutdown().await;
    }

    async fn user_line(s: &mut UnixStream, req: Value) -> Value {
        let (rd, mut wr) = s.split();
        wr.write_all(format!("{req}\n").as_bytes()).await.unwrap();
        let mut line = String::new();
        BufReader::new(rd).read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }

    /// Review S1: anything that can read `user.token` could read private
    /// items through the arguments of parked requests. The user channel
    /// now shows only that a request is about a private item.
    #[tokio::test]
    async fn the_user_channel_redacts_requests_about_private_items() {
        let h = h();
        h.pair("bot");
        h.user(
            "grants.set",
            json!({"client_id": "bot", "data_classes": ["general", "notes.private"]}),
        );
        let note = h.user(
            "tasks.create",
            json!({"title": "Ana is job hunting", "type_id": "em.private_note",
                   "fields": {"person_id": "ana", "body": "Ana is job hunting"}}),
        )["id"]
            .clone();
        let first = h.core.dispatch(
            &Ctx::agent("bot").with_session("s"),
            Request::new("tasks.delete", json!({ "id": note })),
        );
        let cid = first.error().unwrap().confirmation_id.clone().unwrap();
        // The app (in-process user) still sees everything.
        let full = h.user("consent.get", json!({ "id": cid }));
        assert!(full.to_string().contains(note.as_str().unwrap()), "{full}");

        let data = h.dir.path().join("data");
        let host = halp_mcp::serve(h.core.clone(), HostOptions::new(&data))
            .await
            .unwrap();
        let sock = host.socket_path().to_path_buf();
        let user_token = std::fs::read_to_string(data.join(USER_TOKEN_FILE)).unwrap();
        let (r, mut s) = hello(&sock, &format!("{USER_AUTH_PREFIX}{user_token}")).await;
        assert_eq!(r, "OK user\n");
        for req in [
            json!({"command": "consent.list", "args": {}}),
            json!({"command": "consent.get", "args": {"id": cid}}),
        ] {
            let r = user_line(&mut s, req).await;
            assert_eq!(r["ok"], true, "{r}");
            let text = r["result"].to_string();
            assert!(!text.contains("job hunting"), "{text}");
            assert!(!text.contains(note.as_str().unwrap()), "{text}");
            assert!(text.contains("private item"), "{text}");
        }
        // It can still be denied there.
        let r = user_line(
            &mut s,
            json!({"command": "consent.deny", "args": {"id": cid}}),
        )
        .await;
        assert_eq!(r["ok"], true, "{r}");
        host.shutdown().await;
    }

    /// Review S3: one pool for every connection let silent sockets or one
    /// busy agent lock out the user channel.
    #[tokio::test]
    async fn busy_agents_and_silent_sockets_do_not_lock_out_the_user() {
        let h = h();
        let token = h.pair("bot");
        let other = h.pair("other");
        let data = h.dir.path().join("data");
        let mut opts = HostOptions::new(&data);
        opts.max_connections = 3;
        opts.max_per_client = 2;
        opts.max_unauthenticated = 2;
        opts.auth_timeout = Duration::from_millis(300);
        let host = halp_mcp::serve(h.core.clone(), opts).await.unwrap();
        let sock = host.socket_path().to_path_buf();
        let user_token = std::fs::read_to_string(data.join(USER_TOKEN_FILE)).unwrap();

        // One client holds at most `max_per_client` connections.
        let mut held = Vec::new();
        for _ in 0..2 {
            let (r, s) = hello(&sock, &format!("{AUTH_PREFIX}{token}\n")).await;
            assert_eq!(r, "OK agent:bot\n");
            held.push(s);
        }
        let (r, _) = hello(&sock, &format!("{AUTH_PREFIX}{token}\n")).await;
        assert_eq!(r, "ERR busy\n");
        // Another client gets the last agent slot; then the pool is full.
        let (r, s) = hello(&sock, &format!("{AUTH_PREFIX}{other}\n")).await;
        assert_eq!(r, "OK agent:other\n");
        held.push(s);
        let (r, _) = hello(&sock, &format!("{AUTH_PREFIX}{other}\n")).await;
        assert_eq!(r, "ERR busy\n");
        // The user channel has its own slots.
        let (r, _u) = hello(&sock, &format!("{USER_AUTH_PREFIX}{user_token}")).await;
        assert_eq!(r, "OK user\n");

        // Sockets that never authenticate are closed after the auth
        // timeout, and then the user gets in again.
        let silent: Vec<_> = futures_silent(&sock, 4).await;
        tokio::time::sleep(Duration::from_millis(700)).await;
        let (r, _u2) = hello(&sock, &format!("{USER_AUTH_PREFIX}{user_token}")).await;
        assert_eq!(r, "OK user\n");
        drop(silent);

        // Closing a connection frees its slot.
        drop(held.remove(0));
        let mut freed = false;
        for _ in 0..100 {
            let (r, s) = hello(&sock, &format!("{AUTH_PREFIX}{token}\n")).await;
            if r == "OK agent:bot\n" {
                held.push(s);
                freed = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(freed);
        host.shutdown().await;
    }

    async fn futures_silent(path: &std::path::Path, n: usize) -> Vec<UnixStream> {
        let mut v = Vec::new();
        for _ in 0..n {
            v.push(UnixStream::connect(path).await.unwrap());
        }
        v
    }

    /// Finding: an agent with a shell read `user.token` and approved its own
    /// parked delete over `HALP-USER`. The channel is now deny-only.
    #[tokio::test]
    async fn an_agent_cannot_approve_its_own_request_with_the_user_token() {
        let h = h();
        h.pair("bot");
        let id = h.user("tasks.create", json!({"title": "Precious"}))["id"].clone();
        let data = h.dir.path().join("data");
        let host = halp_mcp::serve(h.core.clone(), HostOptions::new(&data))
            .await
            .unwrap();
        let sock = host.socket_path().to_path_buf();
        // The bot asks for the delete; it is parked.
        let first = h.core.dispatch(
            &Ctx::agent("bot").with_session("s"),
            Request::new("tasks.delete", json!({ "id": id })),
        );
        let cid = first.error().unwrap().confirmation_id.clone().unwrap();
        // ... then reads user.token and tries to approve it itself.
        let user_token = std::fs::read_to_string(data.join(USER_TOKEN_FILE)).unwrap();
        let (r, mut s) = hello(&sock, &format!("{USER_AUTH_PREFIX}{user_token}")).await;
        assert_eq!(r, "OK user\n");
        let (rd, mut wr) = s.split();
        let mut rd = BufReader::new(rd);
        let mut ask = async |req: Value| {
            wr.write_all(format!("{req}\n").as_bytes()).await.unwrap();
            let mut line = String::new();
            rd.read_line(&mut line).await.unwrap();
            serde_json::from_str::<Value>(&line).unwrap()
        };
        let r = ask(json!({"command": "consent.approve", "args": {"id": cid}})).await;
        assert_eq!(r["error"]["code"], "denied", "{r}");
        assert!(
            r["error"]["message"]
                .as_str()
                .unwrap()
                .contains("HalpTask Hub window"),
            "{r}"
        );
        // Nothing ran; the request still waits for the app.
        assert!(
            h.core
                .dispatch(&Ctx::user(), Request::new("tasks.get", json!({ "id": id })))
                .is_ok()
        );
        let r = ask(json!({"command": "consent.get", "args": {"id": cid}})).await;
        assert_eq!(r["ok"], true, "{r}");
        // Denying over the socket still works.
        let r = ask(json!({"command": "consent.deny", "args": {"id": cid}})).await;
        assert_eq!(r["ok"], true, "{r}");
        assert!(h.pending().is_empty());
        host.shutdown().await;
    }
}

#[cfg(unix)]
#[tokio::test]
async fn a_socket_path_that_is_too_long_is_a_clear_error() {
    let h = h();
    let deep = h.dir.path().join("x".repeat(120));
    let e = halp_mcp::serve(h.core.clone(), halp_mcp::HostOptions::new(&deep))
        .await
        .err()
        .unwrap();
    assert!(e.to_string().contains("too long"), "{e}");
}

// ---- P5-E2: pack prompts, halp://packs, no pack writes ----------------------

mod packs {
    use super::*;
    use base64::Engine as _;
    use halp_core::halp_pack::{self as hp, SigningKey};
    use std::collections::BTreeMap;

    fn signer() -> SigningKey {
        SigningKey::from_pkcs8(&hp::generate_pkcs8().unwrap()).unwrap()
    }

    fn bundle(id: &str, prompt: &str, key: Option<&SigningKey>) -> Vec<u8> {
        let files: BTreeMap<String, Vec<u8>> =
            BTreeMap::from([("prompts/triage.md".to_string(), prompt.as_bytes().to_vec())]);
        let m = json!({
            "format": 1, "id": id, "version": "0.1.0", "title": "Acme SRE",
            "description": "private text", "license": "MIT",
            "publisher": {"name": "Acme"}, "min_hub": "0.x",
            "extends": {"id": "halp.developer", "version": "^0.1"},
            "role": {"id": id.replace('.', "-"), "title": "SRE", "accent": "#112233"},
            "prompts": {"add": ["prompts/triage.md"]},
            "files": hp::file_index(&files),
        });
        hp::build_bundle(&serde_json::to_string(&m).unwrap(), &files, key).unwrap()
    }

    /// Installs as the user, approving the T3 confirmation in place.
    fn install(h: &H, bytes: &[u8], trust_key: bool) {
        let p = h.user(
            "packs.preview",
            json!({"bundle_base64": base64::engine::general_purpose::STANDARD.encode(bytes)}),
        );
        let args = json!({"preview_id": p["preview_id"], "trust_key": trust_key});
        match h
            .core
            .dispatch(&Ctx::user(), Request::new("packs.install", args))
        {
            Response::Err(e) if e.confirmation_id.is_some() => {
                h.user("consent.approve", json!({"id": e.confirmation_id.unwrap()}));
            }
            Response::Ok(_) => {}
            Response::Err(e) => panic!("install: {e:?}"),
        }
    }

    fn names(ps: &[rmcp::model::Prompt]) -> Vec<String> {
        ps.iter().map(|p| p.name.clone()).collect()
    }

    #[tokio::test]
    async fn community_prompts_are_labelled_and_withdrawn_with_the_pack() {
        let h = h();
        h.pair("bot");
        let k = signer();
        install(
            &h,
            &bundle("acme.sre", "# Triage\nCheck the pager.", Some(&k)),
            true,
        );
        let c = connect(&h, "bot", None).await;
        let ps = c.client.list_all_prompts().await.unwrap();
        let p = ps.iter().find(|p| p.name == "acme.sre.triage").unwrap();
        let d = p.description.clone().unwrap();
        assert!(d.starts_with("[community: acme.sre]"), "{d}");
        // Built-in prompts are still there.
        assert!(names(&ps).contains(&"morning-triage".to_string()));
        let got = c
            .client
            .get_prompt(GetPromptRequestParams::new("acme.sre.triage"))
            .await
            .unwrap();
        let text = serde_json::to_string(&got.messages).unwrap();
        assert!(text.contains("[community: acme.sre]"));
        assert!(text.contains("untrusted"));
        assert!(text.contains("Check the pager."));
        // Disabled: gone, and a new fetch fails.
        h.user("packs.disable", json!({"id": "acme.sre"}));
        let ps = c.client.list_all_prompts().await.unwrap();
        assert!(!names(&ps).contains(&"acme.sre.triage".to_string()));
        assert!(
            c.client
                .get_prompt(GetPromptRequestParams::new("acme.sre.triage"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn unsigned_pack_prompts_are_not_served() {
        let h = h();
        h.pair("bot");
        install(&h, &bundle("acme.sre", "Unsigned text.", None), false);
        let c = connect(&h, "bot", None).await;
        let ps = c.client.list_all_prompts().await.unwrap();
        assert!(
            !names(&ps).iter().any(|n| n.starts_with("acme.sre")),
            "{ps:?}"
        );
    }

    #[tokio::test]
    async fn pack_changes_send_prompts_list_changed() {
        let h = h();
        h.pair("bot");
        let c = connect(&h, "bot", None).await;
        let filter = SubscriptionFilter::builder().prompts_list_changed().build();
        let mut sub = c.client.listen(filter).await.unwrap();
        let k = signer();
        install(&h, &bundle("acme.sre", "Triage.", Some(&k)), true);
        let mut got = false;
        for _ in 0..10 {
            let n = tokio::time::timeout(Duration::from_secs(5), sub.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if matches!(n, ServerNotification::PromptListChangedNotification(_)) {
                got = true;
                break;
            }
        }
        assert!(got, "no prompts/list_changed after install");
    }

    #[tokio::test]
    async fn halp_packs_lists_packs_without_user_only_fields() {
        let h = h();
        h.pair("bot");
        let k = signer();
        install(&h, &bundle("acme.sre", "Triage.", Some(&k)), true);
        let c = connect(&h, "bot", None).await;
        let r = c
            .client
            .read_resource(ReadResourceRequestParams::new("halp://packs"))
            .await
            .unwrap();
        let text = serde_json::to_string(&r.contents).unwrap();
        assert!(text.contains("acme.sre"));
        let rows: Value = match &r.contents[0] {
            rmcp::model::ResourceContents::TextResourceContents { text, .. } => {
                serde_json::from_str(text).unwrap()
            }
            other => panic!("{other:?}"),
        };
        let pack = rows["packs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == "acme.sre")
            .unwrap();
        assert_eq!(pack["trust"], "community");
        for hidden in ["description", "publisher", "previous_version", "error"] {
            assert!(pack.get(hidden).is_none(), "{hidden} leaked to the agent");
        }
        assert!(!text.contains("private text"));
        let res = c.client.list_all_resources().await.unwrap();
        assert!(res.iter().any(|r| r.uri == "halp://packs"));
    }

    #[tokio::test]
    async fn agents_cannot_change_packs_over_mcp() {
        let h = h();
        h.pair("bot");
        let c = connect(&h, "bot", None).await;
        let tools = c.client.list_all_tools().await.unwrap();
        let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
        for w in [
            "packs_install",
            "packs_preview",
            "packs_enable",
            "packs_disable",
            "packs_uninstall",
            "packs_rollback",
            "packs_reset_layout",
            "packs_keys_add",
            "packs_keys_revoke",
            "packs_keys_list",
            "packs_dev_set",
            "packs_dev_reload",
            "packs_dev_get",
        ] {
            assert!(!names.iter().any(|n| n == w), "{w} is exposed");
        }
        // Reads are fine.
        assert!(names.iter().any(|n| n == "packs_list"));
        // Calling a write by name is refused as an unknown tool.
        let r = c
            .client
            .call_tool(
                CallToolRequestParams::new("packs_enable")
                    .with_arguments(args(json!({"id": "halp.developer"}))),
            )
            .await;
        assert!(r.is_err());
        // And the core denies the agent directly, before any confirmation.
        let resp = h.core.dispatch(
            &Ctx::agent("bot").with_session("s"),
            Request::new("packs.install", json!({"preview_id": "pv_x"})),
        );
        assert_eq!(resp.error().unwrap().code, halp_core::ErrorCode::Denied);
    }
}
