//! The local MCP host: a Unix domain socket at `<data_dir>/halp.sock`
//! (directory 0700, socket 0600, peers must run as the directory's owner).
//!
//! A connection starts with one auth line, `HALP-AUTH <token>\n`. The host
//! answers `OK agent:<client id>\n` and then speaks MCP (newline-delimited
//! JSON-RPC) on the same stream, or `ERR unauthorized\n` and closes. The
//! actor of every call on the connection is `agent:<client id>` with a
//! fresh session id; the client cannot choose it. Revoking the client stops
//! its calls at once (every dispatch checks the client is still active).
//!
//! The host also writes a fresh user token to `<data_dir>/user.token`
//! (0600) on every start. A connection that opens with `HALP-USER <token>`
//! gets the user channel: JSON request lines as `user`, limited to listing,
//! reading and **denying** consent requests and listing and **revoking**
//! MCP clients, for `halpd consent` and `halpd clients`. Every one of them
//! only takes power away. Anything
//! running as the same OS user can read that file, which is why the
//! channel cannot approve: approval happens only in-process, in the Hub
//! window or in `halpd serve --interactive` (see `docs/MCP.md`).
//!
//! Connections come from separate pools (review S3): a few for sockets
//! still sending their auth line (closed after `auth_timeout`), agent slots
//! with a per-client cap, and slots kept for the user channel. A full pool
//! answers `ERR busy` and closes. The user channel shows requests about
//! private items without their arguments or summary (review S1), and the
//! host expires unanswered consent requests every
//! [`CONSENT_SWEEP_INTERVAL`] (review A5).
//!
//! There is no TCP listener. On Windows the host is not available yet (see
//! `docs/MCP.md`, residual risk).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use halp_core::Core;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::server::DEFAULT_PROGRESS_INTERVAL;

/// The socket's file name inside the data directory.
pub const SOCKET_NAME: &str = "halp.sock";
/// The first line a client sends: this prefix and its token.
pub const AUTH_PREFIX: &str = "HALP-AUTH ";
/// Longest auth line accepted.
pub const MAX_AUTH_LINE: usize = 512;
/// Longest MCP message accepted (the core refuses requests over 1 MiB).
pub const MAX_MESSAGE_BYTES: usize = 2 * 1024 * 1024;
/// Longest socket path accepted (the portable `sun_path` limit).
pub const MAX_SOCKET_PATH: usize = 103;
/// The first line `halpd consent` sends: this prefix and the user token.
pub const USER_AUTH_PREFIX: &str = "HALP-USER ";
/// The user token's file name inside the data directory (0600, rewritten
/// on every start).
pub const USER_TOKEN_FILE: &str = "user.token";
/// What the user channel may run: look at and deny consent requests, and
/// list and revoke clients. It only takes power away on purpose: an agent
/// with a shell can read `user.token`, and a channel that could approve,
/// pair, grant or add rules would let it raise its own power. (Such an
/// agent could revoke other clients or deny requests; that is the
/// accepted cost of being able to cut an agent off while the Hub runs.)
pub const USER_CHANNEL_COMMANDS: &[&str] = &[
    "consent.list",
    "consent.get",
    "consent.deny",
    "clients.list",
    "clients.revoke",
];
/// The refusal for `consent.approve` on the user channel.
pub const APPROVE_IN_APP: &str = "consent.approve is not available on the socket: approve in the HalpTask Hub window (or the terminal running `halpd serve --interactive`); `halpd consent` can only list and deny";

/// How often the host expires consent requests nobody answered.
pub const CONSENT_SWEEP_INTERVAL: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct HostOptions {
    pub data_dir: PathBuf,
    /// Authenticated agent connections served at once; more get
    /// `ERR busy` and are closed.
    pub max_connections: usize,
    /// Connections still sending their auth line, at once; more are
    /// closed on accept. A separate pool, so clients that never send an
    /// auth line cannot use up the slots of real ones.
    pub max_unauthenticated: usize,
    /// Connections one agent client may hold at once.
    pub max_per_client: usize,
    /// Slots kept for the user channel (`halpd consent|clients`), so the
    /// user can still deny requests and revoke clients while agents use
    /// every agent slot.
    pub user_slots: usize,
    /// Time a client has to send its auth line.
    pub auth_timeout: Duration,
    /// Progress notification interval while a call waits for consent.
    pub progress_interval: Duration,
    /// Write `user.token` and accept `HALP-USER` connections (for
    /// `halpd consent`).
    pub user_channel: bool,
}

impl HostOptions {
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        HostOptions {
            data_dir: data_dir.into(),
            max_connections: 16,
            max_unauthenticated: 8,
            max_per_client: 4,
            user_slots: 2,
            auth_timeout: Duration::from_secs(2),
            progress_interval: DEFAULT_PROGRESS_INTERVAL,
            user_channel: true,
        }
    }
}

/// A running host. Dropping it leaves the listener running until the
/// runtime stops; call [`HostHandle::shutdown`] to stop and remove the
/// socket.
pub struct HostHandle {
    socket_path: PathBuf,
    cancel: CancellationToken,
    task: JoinHandle<()>,
}

impl HostHandle {
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Stops accepting, closes every connection and removes the socket.
    pub async fn shutdown(self) {
        self.cancel.cancel();
        let _ = self.task.await;
        let _ = std::fs::remove_file(&self.socket_path);
        if let Some(dir) = self.socket_path.parent() {
            let _ = std::fs::remove_file(dir.join(USER_TOKEN_FILE));
        }
    }
}

#[cfg(unix)]
pub use unix::serve;

#[cfg(not(unix))]
/// Not available on this platform yet; see `docs/MCP.md`.
pub async fn serve(_core: Arc<Core>, _opts: HostOptions) -> io::Result<HostHandle> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "the MCP host needs Unix domain sockets; the Windows named pipe host is not built yet",
    ))
}

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    use halp_core::halp_policy::secret;
    use halp_core::{CmdError, Ctx, ErrorCode, Request, Response};
    use rmcp::ServiceExt;
    use rmcp::service::RoleServer;
    use rmcp::transport::async_rw::AsyncRwTransport;
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::Semaphore;

    use super::*;
    use crate::server::HalpServer;

    /// Starts the host. Fails when another process already serves the
    /// socket, when the data directory cannot be made private, or when
    /// something other than a socket sits at the socket path.
    pub async fn serve(core: Arc<Core>, opts: HostOptions) -> io::Result<HostHandle> {
        let dir = &opts.data_dir;
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
        let owner = fs::metadata(dir)?.uid();
        let path = dir.join(SOCKET_NAME);
        // sun_path is 104 bytes on macOS and the BSDs, 108 on Linux.
        if path.as_os_str().len() > MAX_SOCKET_PATH {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} is longer than {MAX_SOCKET_PATH} bytes, too long for a Unix socket; use a shorter data directory",
                    path.display()
                ),
            ));
        }
        match fs::symlink_metadata(&path) {
            Ok(m) if m.file_type().is_socket() => {
                if UnixStream::connect(&path).await.is_ok() {
                    return Err(io::Error::new(
                        io::ErrorKind::AddrInUse,
                        format!("{} is served by another process", path.display()),
                    ));
                }
                fs::remove_file(&path)?;
            }
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists and is not a socket", path.display()),
                ));
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(&path)?;
        // The directory is already 0700, so there is no window in which
        // another user could connect before this.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;

        let user_token: Option<Arc<str>> = if opts.user_channel {
            Some(write_user_token(dir)?.into())
        } else {
            None
        };

        let cancel = CancellationToken::new();
        let pools = Arc::new(Pools::new(&opts));
        // Requests nobody answers expire on time even when no call comes in.
        {
            let (core, cancel) = (core.clone(), cancel.clone());
            tokio::spawn(async move {
                let mut tick = tokio::time::interval(CONSENT_SWEEP_INTERVAL);
                tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    tokio::select! {
                        () = cancel.cancelled() => return,
                        _ = tick.tick() => {}
                    }
                    let core = core.clone();
                    let _ = tokio::task::spawn_blocking(move || core.sweep_expired()).await;
                }
            });
        }
        let task = {
            let cancel = cancel.clone();
            tokio::spawn(async move {
                loop {
                    let stream = tokio::select! {
                        () = cancel.cancelled() => return,
                        r = listener.accept() => match r {
                            Ok((s, _)) => s,
                            Err(e) => {
                                eprintln!("halp-mcp host: accept failed: {e}");
                                tokio::time::sleep(Duration::from_millis(100)).await;
                                continue;
                            }
                        },
                    };
                    // Only the auth line is read under this permit.
                    let Ok(pending) = pools.unauthenticated.clone().try_acquire_owned() else {
                        continue;
                    };
                    match stream.peer_cred() {
                        Ok(c) if c.uid() == owner => {}
                        _ => continue,
                    }
                    let core = core.clone();
                    let cancel = cancel.child_token();
                    let opts = opts.clone();
                    let user_token = user_token.clone();
                    let pools = pools.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            connection(stream, core, cancel, &opts, user_token, pools, pending)
                                .await
                        {
                            eprintln!("halp-mcp host: connection ended: {e}");
                        }
                    });
                }
            })
        };
        Ok(HostHandle {
            socket_path: path,
            cancel,
            task,
        })
    }

    /// The host's connection slots (review S3): unauthenticated connections,
    /// agents (and per agent client), and the user channel each have their
    /// own, so neither silent sockets nor one busy agent can lock the user
    /// out of denying and revoking.
    pub(super) struct Pools {
        unauthenticated: Arc<Semaphore>,
        agents: Arc<Semaphore>,
        user: Arc<Semaphore>,
        per_client: std::sync::Mutex<std::collections::HashMap<String, usize>>,
        max_per_client: usize,
    }

    /// One agent connection's hold on its client's count.
    struct ClientSlot {
        pools: Arc<Pools>,
        client: String,
    }

    impl Drop for ClientSlot {
        fn drop(&mut self) {
            let mut m = self
                .pools
                .per_client
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some(n) = m.get_mut(&self.client) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    m.remove(&self.client);
                }
            }
        }
    }

    impl Pools {
        fn new(o: &HostOptions) -> Self {
            Pools {
                unauthenticated: Arc::new(Semaphore::new(o.max_unauthenticated.max(1))),
                agents: Arc::new(Semaphore::new(o.max_connections.max(1))),
                user: Arc::new(Semaphore::new(o.user_slots.max(1))),
                per_client: std::sync::Mutex::new(std::collections::HashMap::new()),
                max_per_client: o.max_per_client.max(1),
            }
        }

        fn client(self: &Arc<Self>, id: &str) -> Option<ClientSlot> {
            let mut m = self
                .per_client
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let n = m.entry(id.to_string()).or_insert(0);
            if *n >= self.max_per_client {
                return None;
            }
            *n += 1;
            Some(ClientSlot {
                pools: self.clone(),
                client: id.to_string(),
            })
        }
    }

    async fn connection(
        stream: UnixStream,
        core: Arc<Core>,
        cancel: CancellationToken,
        opts: &HostOptions,
        user_token: Option<Arc<str>>,
        pools: Arc<Pools>,
        pending: tokio::sync::OwnedSemaphorePermit,
    ) -> io::Result<()> {
        let (r, mut w) = stream.into_split();
        let mut r = BufReader::new(r);
        let mut line = String::new();
        let ok = {
            let mut limited = (&mut r).take(MAX_AUTH_LINE as u64);
            let read = limited.read_line(&mut line);
            matches!(tokio::time::timeout(opts.auth_timeout, read).await, Ok(Ok(n)) if n > 0)
        };
        let preamble = line
            .strip_suffix('\n')
            .map(|l| l.trim_end_matches('\r'))
            .filter(|_| ok);
        let user = preamble
            .and_then(|l| l.strip_prefix(USER_AUTH_PREFIX))
            .zip(user_token.as_deref())
            .is_some_and(|(t, want)| secret::ct_eq(t.as_bytes(), want.as_bytes()));
        let client = match preamble.and_then(|l| l.strip_prefix(AUTH_PREFIX)) {
            Some(t) => {
                // Authenticating reloads clients from the database.
                let (core, t) = (core.clone(), t.to_string());
                tokio::task::spawn_blocking(move || core.authenticate(&t))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        line.clear();
        if user {
            drop(pending);
            let Ok(_slot) = pools.user.clone().try_acquire_owned() else {
                let _ = w.write_all(b"ERR busy\n").await;
                return Ok(());
            };
            w.write_all(b"OK user\n").await?;
            return user_channel(r, w, core, cancel).await;
        }
        let Some(client_id) = client else {
            // Slow down guessing (still holding the unauthenticated slot).
            tokio::time::sleep(Duration::from_millis(250)).await;
            let _ = w.write_all(b"ERR unauthorized\n").await;
            return Ok(());
        };
        drop(pending);
        let (Ok(_slot), Some(_mine)) = (
            pools.agents.clone().try_acquire_owned(),
            pools.client(&client_id),
        ) else {
            let _ = w.write_all(b"ERR busy\n").await;
            return Ok(());
        };
        let session = secret::random_hex()
            .map_err(io::Error::other)?
            .chars()
            .take(16)
            .collect::<String>();
        let ctx = Ctx::agent(&client_id).with_session(session);
        w.write_all(format!("OK agent:{client_id}\n").as_bytes())
            .await?;
        let server = HalpServer::new(core, ctx).with_progress_interval(opts.progress_interval);
        let transport =
            AsyncRwTransport::<RoleServer, _, _>::new(r, w).with_max_line_length(MAX_MESSAGE_BYTES);
        let running = server
            .serve_with_ct(transport, cancel)
            .await
            .map_err(io::Error::other)?;
        running.waiting().await.map_err(io::Error::other)?;
        Ok(())
    }

    /// Writes a fresh user token to `<dir>/user.token` (0600, replacing any
    /// old one) and returns it.
    fn write_user_token(dir: &Path) -> io::Result<String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = dir.join(USER_TOKEN_FILE);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let token = secret::new_token().map_err(io::Error::other)?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(token.as_bytes())?;
        f.write_all(b"\n")?;
        Ok(token)
    }

    /// The user channel: one JSON request per line, one JSON response per
    /// line, as `user`, limited to [`USER_CHANNEL_COMMANDS`] (never
    /// `consent.approve`).
    async fn user_channel(
        mut r: BufReader<tokio::net::unix::OwnedReadHalf>,
        mut w: tokio::net::unix::OwnedWriteHalf,
        core: Arc<Core>,
        cancel: CancellationToken,
    ) -> io::Result<()> {
        // Anything running as the user can reach this channel: it is not
        // shown what private items hold (review S1).
        let ctx = Ctx::user_cli();
        loop {
            let mut line = String::new();
            let n = {
                let mut limited = (&mut r).take(MAX_MESSAGE_BYTES as u64);
                tokio::select! {
                    () = cancel.cancelled() => return Ok(()),
                    n = limited.read_line(&mut line) => n?,
                }
            };
            if n == 0 {
                return Ok(());
            }
            let resp = match serde_json::from_str::<Request>(line.trim()) {
                Ok(req) if USER_CHANNEL_COMMANDS.contains(&req.command.as_str()) => {
                    let (core, ctx) = (core.clone(), ctx.clone());
                    match tokio::task::spawn_blocking(move || core.dispatch(&ctx, req)).await {
                        Ok(r) => r,
                        Err(e) => Response::Err(
                            CmdError::internal(format!("the request did not finish: {e}")).into(),
                        ),
                    }
                }
                Ok(req) if req.command == "consent.approve" => {
                    Response::Err(CmdError::new(ErrorCode::Denied, APPROVE_IN_APP).into())
                }
                Ok(req) => Response::Err(
                    CmdError::new(
                        ErrorCode::Denied,
                        format!(
                            "{} is not available on the user channel (only consent.list, consent.get, consent.deny, clients.list and clients.revoke)",
                            req.command
                        ),
                    )
                    .into(),
                ),
                Err(e) => Response::Err(CmdError::invalid(format!("bad request: {e}")).into()),
            };
            let mut out = resp.to_value().to_string();
            out.push('\n');
            w.write_all(out.as_bytes()).await?;
        }
    }
}
