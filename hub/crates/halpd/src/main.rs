//! `halpd`: the Hub without its window. Runs commands against the database,
//! hosts the MCP socket, pairs MCP clients and answers consent requests.
//!
//! ```text
//! halpd [--data-dir DIR] [--db PATH] [--db-passphrase-env VAR | --db-passphrase-stdin] <command>
//!
//! commands:
//!   dispatch '<json request>' | -     run one request as the user (`-` reads stdin)
//!   import <path> [--passphrase-env VAR | --passphrase-stdin] [--utc-offset +HH:MM]
//!   commands                          list commands with their schemas (this database)
//!   schema                            print every command descriptor as JSON (no database)
//!   serve [--interactive]             host MCP on <data dir>/halp.sock until Ctrl-C;
//!                                     --interactive asks on this terminal for each
//!                                     consent request
//!   clients pair --name NAME [--id ID]   pair an MCP client; prints its token once
//!                                     (needs the Hub stopped; or pair in the app)
//!   clients list | clients revoke ID  over a running Hub's socket when one runs
//!   consent list | consent deny ID    look at or deny requests waiting in a
//!                                     running host (over its socket)
//!   consent approve ID                explains where to approve: the socket
//!                                     cannot approve (see docs/MCP.md)
//! ```
//!
//! `halpd` acts as the user: anyone who can run it with access to the
//! database key has the user's power. Destructive (T3) requests still ask:
//! `dispatch` and `import` need an interactive terminal and the request's
//! approval code typed (shown in the prompt), and are refused (the request
//! denied) without one.
//!
//! Only one process opens the database at a time (`<data dir>/hub.lock`).
//! While the app or `halpd serve` runs, the commands that open it fail
//! with "HalpTask Hub is already running"; `consent list|deny` go through
//! the running host's socket instead, and so do `clients list|revoke` when a
//! Hub is running (so a misbehaving agent can be cut off at once).
//!
//! Passphrases are never taken as arguments (they would show in `ps`):
//! name an environment variable or pipe them on stdin (one line each; the
//! database passphrase first).

use std::collections::BTreeSet;
#[cfg(unix)]
use std::io::BufReader;
use std::io::{BufRead, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use base64::Engine as _;
use chrono::{FixedOffset, Local, Offset};
use directories::ProjectDirs;
use halp_core::{Core, Ctx, ErrorBody, ErrorCode, Request, Response};
use halp_store::keys::{passphrase_from_env, passphrase_from_reader};
use halp_store::{KeyProvider, KeychainKeyProvider, PassphraseKeyProvider, RawKeyProvider, Store};
use serde_json::{Value, json};
use zeroize::Zeroizing;

mod pack;

const USAGE: &str = "usage: halpd [--data-dir DIR] [--db PATH] [--db-passphrase-env VAR | --db-passphrase-stdin] <command>
commands:
  dispatch '<json>' | -
  import <path> [--passphrase-env VAR | --passphrase-stdin] [--utc-offset +HH:MM]
  commands | schema
  serve [--interactive]
  clients pair --name NAME [--id ID] | clients list | clients revoke ID
  consent list | consent deny ID   (approve in the Hub window or `serve --interactive`)
  pack keygen --out KEYFILE | pack sign DIR --key KEYFILE --out BUNDLE | pack verify BUNDLE";

enum Secret {
    None,
    Env(String),
    Stdin,
}

enum Cmd {
    Dispatch(String),
    Import {
        path: PathBuf,
        secret: Secret,
        utc_offset: Option<FixedOffset>,
    },
    Commands,
    Schema,
    Serve {
        interactive: bool,
    },
    ClientsPair {
        name: String,
        id: Option<String>,
    },
    ClientsList,
    ClientsRevoke(String),
    ConsentList,
    ConsentApprove(String),
    // The id is only read by the Unix user channel.
    #[cfg_attr(not(unix), allow(dead_code))]
    ConsentDeny(String),
    Pack(pack::PackCmd),
}

struct Opts {
    data_dir: Option<PathBuf>,
    db: Option<PathBuf>,
    db_secret: Secret,
    cmd: Cmd,
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_args(raw: Vec<String>) -> Result<Opts, String> {
    let mut args = raw.into_iter();
    let mut data_dir = None;
    let mut db = None;
    let mut db_secret = Secret::None;
    let mut cmd = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(value(&mut args, &a)?)),
            "--db" => db = Some(PathBuf::from(value(&mut args, "--db")?)),
            "--db-passphrase-env" => db_secret = Secret::Env(value(&mut args, &a)?),
            "--db-passphrase-stdin" => db_secret = Secret::Stdin,
            "--actor" => {
                return Err(
                    "--actor was removed: halpd always acts as the user; agents connect through halp-mcp"
                        .into(),
                );
            }
            "-h" | "--help" => return Err(USAGE.into()),
            "dispatch" => {
                cmd = Some(Cmd::Dispatch(value(&mut args, "dispatch")?));
                break;
            }
            "commands" => {
                cmd = Some(Cmd::Commands);
                break;
            }
            "schema" => {
                cmd = Some(Cmd::Schema);
                break;
            }
            "pack" => {
                let rest: Vec<String> = args.by_ref().collect();
                cmd = Some(Cmd::Pack(pack::parse(&rest)?));
                break;
            }
            "serve" => {
                let mut interactive = false;
                for f in args.by_ref() {
                    match f.as_str() {
                        "--interactive" => interactive = true,
                        other => return Err(format!("unknown serve option {other}")),
                    }
                }
                cmd = Some(Cmd::Serve { interactive });
                break;
            }
            "clients" => {
                let sub = value(&mut args, "clients")?;
                cmd = Some(match sub.as_str() {
                    "pair" => {
                        let mut name = None;
                        let mut id = None;
                        while let Some(f) = args.next() {
                            match f.as_str() {
                                "--name" => name = Some(value(&mut args, &f)?),
                                "--id" => id = Some(value(&mut args, &f)?),
                                other => return Err(format!("unknown pair option {other}")),
                            }
                        }
                        Cmd::ClientsPair {
                            name: name.ok_or("clients pair needs --name")?,
                            id,
                        }
                    }
                    "list" => Cmd::ClientsList,
                    "revoke" => Cmd::ClientsRevoke(value(&mut args, "clients revoke")?),
                    other => return Err(format!("unknown clients command {other}\n{USAGE}")),
                });
                break;
            }
            "consent" => {
                let sub = value(&mut args, "consent")?;
                cmd = Some(match sub.as_str() {
                    "list" => Cmd::ConsentList,
                    "approve" => Cmd::ConsentApprove(value(&mut args, "consent approve")?),
                    "deny" => Cmd::ConsentDeny(value(&mut args, "consent deny")?),
                    other => return Err(format!("unknown consent command {other}\n{USAGE}")),
                });
                break;
            }
            "import" => {
                let path = PathBuf::from(value(&mut args, "import")?);
                let mut secret = Secret::None;
                let mut utc_offset = None;
                while let Some(f) = args.next() {
                    match f.as_str() {
                        "--passphrase-env" => secret = Secret::Env(value(&mut args, &f)?),
                        "--passphrase-stdin" => secret = Secret::Stdin,
                        "--utc-offset" => {
                            let v = value(&mut args, &f)?;
                            utc_offset = Some(
                                v.parse::<FixedOffset>()
                                    .map_err(|_| format!("bad UTC offset {v}"))?,
                            );
                        }
                        other => return Err(format!("unknown import option {other}")),
                    }
                }
                cmd = Some(Cmd::Import {
                    path,
                    secret,
                    utc_offset,
                });
                break;
            }
            other => return Err(format!("unknown argument {other}\n{USAGE}")),
        }
    }
    if let Some(extra) = args.next() {
        return Err(format!("unexpected argument {extra}"));
    }
    let cmd = cmd.ok_or_else(|| USAGE.to_string())?;
    Ok(Opts {
        data_dir,
        db,
        db_secret,
        cmd,
    })
}

/// The Hub's data directory: the same one the desktop app uses.
fn default_data_dir() -> Result<PathBuf, String> {
    let dirs = ProjectDirs::from("", "", "halptask-hub")
        .ok_or("no home directory to keep the Hub's data in; pass --data-dir")?;
    Ok(dirs.data_dir().to_path_buf())
}

fn read_secret(s: &Secret, stdin: &mut impl BufRead) -> Result<Option<Zeroizing<String>>, String> {
    match s {
        Secret::None => Ok(None),
        Secret::Env(var) => passphrase_from_env(var)
            .map(Some)
            .map_err(|e| e.to_string()),
        Secret::Stdin => passphrase_from_reader(stdin)
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}

fn print_value(v: &Value) {
    match serde_json::to_string_pretty(v) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("halpd: {e}"),
    }
}

fn print(resp: &Response) -> ExitCode {
    print_value(&resp.to_value());
    if resp.is_ok() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn err_response(code: ErrorCode, msg: impl Into<String>) -> Response {
    Response::Err(ErrorBody {
        code,
        message: msg.into(),
        confirmation_id: None,
    })
}

fn interactive_terminal() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// The short code the user types to approve a request: the last four
/// characters of its (random) confirmation id. Input typed before the
/// prompt cannot know it, so a stray or early `y` never approves.
fn approval_code(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    chars[chars.len().saturating_sub(4)..]
        .iter()
        .collect::<String>()
        .to_lowercase()
}

/// Whether `answer` approves `pending` (the full `consent.get` view).
/// Only the exact code approves, and never when the request could not be
/// shown in full.
fn approves(pending: &Value, answer: &str) -> bool {
    let Some(id) = pending["id"].as_str() else {
        return false;
    };
    if pending["display_truncated"].as_bool() != Some(false) {
        return false;
    }
    let code = approval_code(id);
    !code.is_empty() && answer.trim().to_lowercase() == code
}

/// The question under a shown request.
fn prompt_text(pending: &Value) -> String {
    let id = pending["id"].as_str().unwrap_or("");
    if pending["display_truncated"].as_bool() != Some(false) {
        "This request is too large to show in full here, so it cannot be approved from this \
         terminal (approve it in the Hub window). Press Enter to deny it: "
            .to_string()
    } else {
        format!(
            "Type {} to approve, anything else denies: ",
            approval_code(id)
        )
    }
}

/// Shows a parked request (its full `consent.get` view) and asks for its
/// approval code on the terminal. Anything else, no terminal, or a request
/// too large to show in full is a no.
fn ask_approval(pending: &Value) -> bool {
    show_pending(pending);
    if !interactive_terminal() {
        eprintln!("halpd: approving needs the approval code typed on an interactive terminal");
        return false;
    }
    eprint!("{}", prompt_text(pending));
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    if std::io::stdin().lock().read_line(&mut line).is_err() {
        return false;
    }
    approves(pending, &line)
}

fn show_pending(p: &Value) {
    let s = |k: &str| p[k].as_str().unwrap_or("?").to_string();
    eprintln!();
    eprintln!("Confirmation needed: {}", s("summary"));
    eprintln!("  id:       {}", s("id"));
    eprintln!("  from:     {}", s("actor"));
    eprintln!("  command:  {} ({})", s("command"), s("tier"));
    eprintln!("  args:     {}", p["args"]);
    if p["display_truncated"].as_bool() != Some(false) {
        eprintln!("            (shortened: too large to show in full)");
    }
    if !p["details"].is_null() {
        eprintln!("  effect:   {}", p["details"]);
    }
    if p["tainted"].as_bool() == Some(true) {
        eprintln!(
            "  WARNING:  the requester read content from {} before asking; it may be acting on instructions planted there",
            p["taint_source"].as_str().unwrap_or("someone else")
        );
    }
    eprintln!("  expires:  {}", s("expires_at"));
}

/// Runs a request as the user; a parked (T3) request is confirmed on the
/// terminal or denied.
fn dispatch_confirmed(core: &Core, req: Request) -> Response {
    let user = Ctx::user();
    let first = core.dispatch(&user, req);
    let Some(id) = first
        .error()
        .filter(|e| e.code == ErrorCode::ConfirmationRequired)
        .and_then(|e| e.confirmation_id.clone())
    else {
        return first;
    };
    let pending = core
        .dispatch(&user, Request::new("consent.get", json!({ "id": id })))
        .result()
        .cloned()
        .unwrap_or(Value::Null);
    if ask_approval(&pending) {
        core.dispatch(&user, Request::new("consent.approve", json!({ "id": id })))
    } else {
        let _ = core.dispatch(&user, Request::new("consent.deny", json!({ "id": id })));
        err_response(ErrorCode::Denied, "not confirmed; the request was denied")
    }
}

/// Every command descriptor, from a throwaway database.
fn schema() -> Result<Value, String> {
    let tag = halp_core::halp_policy::secret::random_hex()?;
    let dir = std::env::temp_dir().join(format!("halpd-schema-{}", &tag[..16]));
    let result = (|| {
        let store = Store::open(&dir.join("hub.db"), &RawKeyProvider::new([0; 32]))
            .map_err(|e| e.to_string())?;
        let core =
            Core::with_store(store, Arc::new(halp_core::SystemClock)).map_err(|e| e.message)?;
        let mut cmds = core.descriptors();
        cmds.sort_by(|a, b| a.name.cmp(&b.name));
        serde_json::to_value(json!({ "commands": cmds })).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn pair_instructions(data_dir: &Path, default_dir: bool, v: &Value) {
    let id = v["client"]["id"].as_str().unwrap_or("client");
    let name = v["client"]["name"].as_str().unwrap_or(id);
    let token = v["token"].as_str().unwrap_or("");
    let file = format!("$HOME/.halptask/{id}.token");
    let socket_env = if default_dir {
        String::new()
    } else {
        format!(
            " --env HALP_SOCKET={}",
            data_dir.join(halp_mcp::SOCKET_NAME).display()
        )
    };
    println!("Paired {name} as agent:{id}.");
    println!();
    println!("Token (shown once; only a hash is kept):");
    println!("  {token}");
    println!();
    println!("Keep it in a file only you can read:");
    println!("  mkdir -p -m 700 $HOME/.halptask && (umask 077; cat > {file})");
    println!("  (paste the token, press Enter, then Ctrl-D)");
    println!();
    println!("Claude Code:");
    println!("  claude mcp add halptask --env HALP_MCP_TOKEN_FILE={file}{socket_env} -- halp-mcp");
    println!();
    println!("Claude Desktop and other clients (JSON config; use absolute paths):");
    let mut env =
        json!({ "HALP_MCP_TOKEN_FILE": "/Users/<you>/.halptask/".to_string() + id + ".token" });
    if !default_dir {
        env["HALP_SOCKET"] = json!(data_dir.join(halp_mcp::SOCKET_NAME));
    }
    print_value(&json!({ "mcpServers": { "halptask": { "command": "halp-mcp", "env": env } } }));
    println!();
    println!("The Hub (app or `halpd serve`) must be running for the client to connect.");
    println!("Revoke with: halpd clients revoke {id}");
}

/// The user channel of a running host (`halpd consent`, `halpd clients list|revoke`).
#[cfg(unix)]
struct UserChannel {
    r: BufReader<std::os::unix::net::UnixStream>,
    w: std::os::unix::net::UnixStream,
}

#[cfg(unix)]
impl UserChannel {
    fn open(data_dir: &Path) -> Result<UserChannel, String> {
        use halp_mcp::host::{USER_AUTH_PREFIX, USER_TOKEN_FILE};
        let sock = data_dir.join(halp_mcp::SOCKET_NAME);
        let token = std::fs::read_to_string(data_dir.join(USER_TOKEN_FILE))
            .map_err(|e| format!("no running Hub in {} ({e})", data_dir.display()))?;
        let mut w = std::os::unix::net::UnixStream::connect(&sock)
            .map_err(|e| format!("cannot reach the Hub at {}: {e}", sock.display()))?;
        w.write_all(format!("{USER_AUTH_PREFIX}{}\n", token.trim()).as_bytes())
            .map_err(|e| e.to_string())?;
        let mut r = BufReader::new(w.try_clone().map_err(|e| e.to_string())?);
        let mut line = String::new();
        (&mut r)
            .take(256)
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if line.trim() != "OK user" {
            return Err("the Hub refused the user token (was it restarted?)".into());
        }
        Ok(UserChannel { r, w })
    }

    fn call(&mut self, command: &str, args: Value) -> Result<Value, String> {
        let req = serde_json::to_string(&Request::new(command, args)).map_err(|e| e.to_string())?;
        self.w
            .write_all(format!("{req}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let mut line = String::new();
        self.r.read_line(&mut line).map_err(|e| e.to_string())?;
        serde_json::from_str(&line).map_err(|e| format!("bad reply from the Hub: {e}"))
    }
}

#[cfg(unix)]
fn reply_code(v: &Value) -> ExitCode {
    print_value(v);
    if v["ok"].as_bool() == Some(true) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

#[cfg(unix)]
fn consent(data_dir: &Path, cmd: Cmd) -> Result<ExitCode, String> {
    let mut ch = UserChannel::open(data_dir)?;
    match cmd {
        Cmd::ConsentList => Ok(reply_code(&ch.call("consent.list", json!({}))?)),
        Cmd::ConsentDeny(id) => Ok(reply_code(&ch.call("consent.deny", json!({ "id": id }))?)),
        Cmd::ClientsList => Ok(reply_code(&ch.call("clients.list", json!({}))?)),
        Cmd::ClientsRevoke(id) => Ok(reply_code(&ch.call("clients.revoke", json!({ "id": id }))?)),
        _ => Err("not a user-channel command".into()),
    }
}

/// Why `halpd consent approve` does not approve.
fn approve_elsewhere(id: &str) -> String {
    format!(
        "consent approve is not available from a shell: approve {id} in the HalpTask Hub window \
         (or in the terminal running `halpd serve --interactive`). Anything running as your user \
         can use the Hub's socket, including an agent with a shell, so the socket can only list \
         and deny (`halpd consent deny {id}`), and list or revoke clients."
    )
}

/// Whether a Hub is serving this data directory (its user channel answers).
#[cfg(unix)]
fn hub_running(data_dir: &Path) -> bool {
    data_dir.join(halp_mcp::host::USER_TOKEN_FILE).exists() && UserChannel::open(data_dir).is_ok()
}

#[cfg(not(unix))]
fn hub_running(_data_dir: &Path) -> bool {
    false
}

#[cfg(not(unix))]
fn consent(_data_dir: &Path, _cmd: Cmd) -> Result<ExitCode, String> {
    Err("halpd consent needs the MCP host, which is not available on this platform yet".into())
}

/// Whether a line read at `at` answers a prompt printed at `asked_at`.
fn answered_after(at: Instant, asked_at: Instant) -> bool {
    at >= asked_at
}

async fn serve(core: Arc<Core>, data_dir: PathBuf, interactive: bool) -> Result<ExitCode, String> {
    if interactive && !interactive_terminal() {
        return Err("serve --interactive needs an interactive terminal".into());
    }
    let host = halp_mcp::serve(core.clone(), halp_mcp::HostOptions::new(&data_dir))
        .await
        .map_err(|e| format!("cannot start the MCP host: {e}"))?;
    eprintln!(
        "halpd: MCP host on {} (MCP {}); Ctrl-C stops it",
        host.socket_path().display(),
        halp_mcp::PROTOCOL_VERSION
    );
    if !interactive {
        eprintln!(
            "halpd: no one can approve requests in this mode; use --interactive to approve here (`halpd consent list|deny` works over the socket)"
        );
    }

    // Terminal lines, read on a plain thread (stdin is blocking), each
    // stamped with when it arrived: an answer only counts if it came after
    // its prompt was printed, so a line typed early never answers a later
    // request.
    let (tx, mut lines) = tokio::sync::mpsc::unbounded_channel::<(Instant, String)>();
    if interactive {
        std::thread::spawn(move || {
            for l in std::io::stdin().lock().lines() {
                let Ok(l) = l else { break };
                if tx.send((Instant::now(), l)).is_err() {
                    break;
                }
            }
        });
    }
    let mut events = core.subscribe();
    let mut asked = BTreeSet::new();
    let user = Ctx::user();
    'outer: loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            ev = events.recv(), if interactive => {
                use tokio::sync::broadcast::error::RecvError;
                match ev {
                    Ok(e) if e.command != "consent.requested" => continue,
                    Ok(_) | Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
                let pending = core
                    .dispatch(&user, Request::new("consent.list", json!({})))
                    .result()
                    .and_then(|v| v["pending"].as_array().cloned())
                    .unwrap_or_default();
                for p in pending {
                    let Some(id) = p["id"].as_str().map(str::to_string) else { continue };
                    if !asked.insert(id.clone()) {
                        continue;
                    }
                    // The list entry may be shortened: show the full view.
                    let Some(full) = core
                        .dispatch(&user, Request::new("consent.get", json!({ "id": id })))
                        .result()
                        .cloned()
                    else {
                        continue; // decided or expired meanwhile
                    };
                    show_pending(&full);
                    // Drop anything typed before this prompt.
                    while lines.try_recv().is_ok() {}
                    eprint!("{}", prompt_text(&full));
                    let _ = std::io::stderr().flush();
                    let asked_at = Instant::now();
                    let answer = loop {
                        let l = tokio::select! {
                            _ = tokio::signal::ctrl_c() => break 'outer,
                            l = lines.recv() => l,
                        };
                        match l {
                            Some((at, _)) if !answered_after(at, asked_at) => continue,
                            Some((_, l)) => break l,
                            None => break String::new(),
                        }
                    };
                    let cmd = if approves(&full, &answer) { "consent.approve" } else { "consent.deny" };
                    let r = core.dispatch(&user, Request::new(cmd, json!({ "id": id })));
                    match r.error() {
                        None => eprintln!("halpd: {}", if cmd == "consent.approve" { "approved" } else { "denied" }),
                        Some(e) => eprintln!("halpd: {cmd} failed: {}", e.message),
                    }
                }
            }
        }
    }
    host.shutdown().await;
    eprintln!("halpd: stopped");
    Ok(ExitCode::SUCCESS)
}

fn run() -> Result<ExitCode, String> {
    let opts = parse_args(std::env::args().skip(1).collect())?;
    // Pack tooling touches only the files it is given.
    if let Cmd::Pack(ref p) = opts.cmd {
        return pack::run(p);
    }
    let default_dir = opts.data_dir.is_none();
    let data_dir = match &opts.data_dir {
        Some(d) => std::path::absolute(d).map_err(|e| format!("bad --data-dir: {e}"))?,
        None => default_data_dir()?,
    };

    // Commands that do not open the database.
    match opts.cmd {
        Cmd::Schema => {
            print_value(&schema()?);
            return Ok(ExitCode::SUCCESS);
        }
        Cmd::ConsentApprove(ref id) => return Err(approve_elsewhere(id)),
        Cmd::ConsentList | Cmd::ConsentDeny(_) => {
            return consent(&data_dir, opts.cmd);
        }
        // With a Hub running (it holds the database), list and revoke
        // through its socket so an agent can be cut off at once.
        Cmd::ClientsList | Cmd::ClientsRevoke(_) if hub_running(&data_dir) => {
            return consent(&data_dir, opts.cmd);
        }
        _ => {}
    }

    let uses_stdin = matches!(opts.db_secret, Secret::Stdin)
        || matches!(
            &opts.cmd,
            Cmd::Import {
                secret: Secret::Stdin,
                ..
            }
        );
    if uses_stdin && matches!(&opts.cmd, Cmd::Dispatch(r) if r == "-") {
        return Err("stdin cannot carry both a passphrase and the request".into());
    }
    if uses_stdin && matches!(&opts.cmd, Cmd::Serve { interactive: true }) {
        return Err("stdin cannot carry both a passphrase and consent answers".into());
    }
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();

    let db = opts.db.clone().unwrap_or_else(|| data_dir.join("hub.db"));
    let keys: Box<dyn KeyProvider> = match read_secret(&opts.db_secret, &mut stdin)? {
        Some(pass) => Box::new(PassphraseKeyProvider::new(pass).map_err(|e| e.to_string())?),
        None => Box::new(KeychainKeyProvider::default()),
    };
    let core = Core::open(&db, keys.as_ref())
        .map_err(|e| format!("cannot open {}: {}", db.display(), e.message))?;
    for w in core.startup_warnings() {
        eprintln!("halpd: warning: {w}");
    }
    let user = Ctx::user();

    match opts.cmd {
        Cmd::Dispatch(raw) => {
            let raw = if raw == "-" {
                let mut s = String::new();
                (&mut stdin)
                    .take(halp_core::MAX_REQUEST_BYTES as u64 + 1)
                    .read_to_string(&mut s)
                    .map_err(|e| format!("cannot read stdin: {e}"))?;
                s
            } else {
                raw
            };
            drop(stdin);
            let req = match serde_json::from_str::<Request>(&raw) {
                Ok(r) => r,
                // Let the core word the error the same way as every caller.
                Err(_) => return Ok(print(&core.dispatch_json(&user, &raw))),
            };
            Ok(print(&dispatch_confirmed(&core, req)))
        }
        Cmd::Commands => Ok(print(
            &core.dispatch(&user, Request::new("commands.list", json!({}))),
        )),
        Cmd::Import {
            path,
            secret,
            utc_offset,
        } => {
            let passphrase = read_secret(&secret, &mut stdin)?;
            drop(stdin);
            let bytes =
                std::fs::read(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            let offset = utc_offset.unwrap_or_else(|| Local::now().offset().fix());
            let mut args = json!({
                "content_base64": base64::engine::general_purpose::STANDARD.encode(&bytes),
                "file_name": path.file_name().map(|n| n.to_string_lossy().into_owned()),
                "utc_offset_minutes": offset.local_minus_utc() / 60,
            });
            if let Some(p) = &passphrase {
                args["passphrase"] = json!(p.as_str());
            }
            Ok(print(&dispatch_confirmed(
                &core,
                Request::new("import.tui", args),
            )))
        }
        Cmd::ClientsPair { name, id } => {
            drop(stdin);
            let mut args = json!({ "name": name });
            if let Some(id) = id {
                args["id"] = json!(id);
            }
            match core.dispatch(&user, Request::new("clients.pair", args)) {
                Response::Ok(v) => {
                    pair_instructions(&data_dir, default_dir, &v);
                    Ok(ExitCode::SUCCESS)
                }
                r => Ok(print(&r)),
            }
        }
        Cmd::ClientsList => Ok(print(
            &core.dispatch(&user, Request::new("clients.list", json!({}))),
        )),
        Cmd::ClientsRevoke(id) => Ok(print(
            &core.dispatch(&user, Request::new("clients.revoke", json!({ "id": id }))),
        )),
        Cmd::Serve { interactive } => {
            drop(stdin);
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .map_err(|e| e.to_string())?;
            rt.block_on(serve(Arc::new(core), data_dir, interactive))
        }
        Cmd::Schema
        | Cmd::ConsentList
        | Cmd::ConsentApprove(_)
        | Cmd::ConsentDeny(_)
        | Cmd::Pack(_) => Err("unreachable".into()),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("halpd: {e}");
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The socket is deny-only, so `consent approve` must not pretend to
    /// approve: it parses, and points at the Hub window.
    #[test]
    fn consent_approve_points_at_the_hub_window() {
        let o = parse_args(vec!["consent".into(), "approve".into(), "abc".into()])
            .unwrap_or_else(|e| panic!("{e}"));
        let Cmd::ConsentApprove(id) = o.cmd else {
            panic!("not consent approve")
        };
        let msg = approve_elsewhere(&id);
        assert!(msg.contains("HalpTask Hub window"), "{msg}");
        assert!(msg.contains("consent deny abc"), "{msg}");
    }

    /// Finding: the prompt took `yes`, so an early line typed (or piped)
    /// before a request arrived could approve it. Only the code shown in
    /// the prompt approves.
    #[test]
    fn only_the_shown_code_approves() {
        let p = json!({"id": "00ff00ffAB12", "display_truncated": false});
        assert_eq!(approval_code("00ff00ffAB12"), "ab12");
        assert!(approves(&p, "ab12\n"));
        assert!(approves(&p, " AB12 "));
        for a in ["yes", "y", "", "ab1", "00ff00ffab12"] {
            assert!(!approves(&p, a), "{a:?} approved");
        }
        assert!(prompt_text(&p).contains("Type ab12 to approve"));
    }

    /// Finding: a shortened request could be approved on the terminal.
    #[test]
    fn a_truncated_request_can_only_be_denied() {
        for p in [
            json!({"id": "00ff00ffab12", "display_truncated": true}),
            json!({"id": "00ff00ffab12"}),
        ] {
            assert!(!approves(&p, "ab12"));
            assert!(prompt_text(&p).contains("cannot be approved"));
        }
    }

    /// Finding: lines buffered before the prompt answered it.
    #[test]
    fn lines_typed_before_the_prompt_do_not_answer_it() {
        let early = Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let asked = Instant::now();
        assert!(!answered_after(early, asked));
        assert!(answered_after(Instant::now(), asked));
    }

    /// Finding: `halpd clients list|revoke` failed while the Hub ran.
    #[cfg(unix)]
    #[test]
    fn clients_commands_use_the_user_channel_when_no_hub_is_answering() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("{e}"));
        assert!(!hub_running(dir.path()));
        // A stale token without a socket is not a running Hub.
        std::fs::write(dir.path().join(halp_mcp::host::USER_TOKEN_FILE), "x")
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!hub_running(dir.path()));
    }
}
