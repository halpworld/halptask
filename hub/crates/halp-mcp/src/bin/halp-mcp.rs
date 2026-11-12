//! `halp-mcp`: the stdio shim MCP clients launch to reach the Hub.
//!
//! ```text
//! halp-mcp [--socket PATH]
//! ```
//!
//! The client token comes from `HALP_MCP_TOKEN` or from the file named by
//! `HALP_MCP_TOKEN_FILE`, never from arguments (they show in `ps`). The shim
//! connects to the Hub's socket (`HALP_SOCKET`, `--socket`, or
//! `<data dir>/halp.sock`), sends the token, and then only copies bytes
//! between stdio and the socket. The MCP server runs inside the Hub.

use std::path::PathBuf;
use std::process::ExitCode;

#[cfg(unix)]
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

const USAGE: &str = "usage: halp-mcp [--socket PATH]\n\
Set HALP_MCP_TOKEN (or HALP_MCP_TOKEN_FILE) to the token from `halpd clients pair`.\n\
Tokens are never accepted as arguments.";
const MAX_TOKEN_BYTES: u64 = 4096;
/// How long replies may still arrive after stdin closes.
#[cfg(unix)]
const EOF_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

fn socket_from_args() -> Result<Option<PathBuf>, String> {
    let mut args = std::env::args().skip(1);
    let mut socket = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--socket" => {
                socket = Some(PathBuf::from(args.next().ok_or("--socket needs a path")?));
            }
            "-h" | "--help" => return Err(USAGE.into()),
            _ => {
                // Never echo the argument: it may be a token pasted by mistake.
                return Err(format!(
                    "unexpected argument (tokens are never accepted as arguments)\n{USAGE}"
                ));
            }
        }
    }
    Ok(socket)
}

#[cfg(unix)]
fn default_socket() -> Result<PathBuf, String> {
    if let Some(p) = std::env::var_os("HALP_SOCKET") {
        return Ok(PathBuf::from(p));
    }
    let dirs = directories::ProjectDirs::from("", "", "halptask-hub")
        .ok_or("no home directory; set HALP_SOCKET")?;
    Ok(dirs.data_dir().join(halp_mcp::SOCKET_NAME))
}

fn token() -> Result<String, String> {
    if let Ok(t) = std::env::var("HALP_MCP_TOKEN") {
        let t = t.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let Some(path) = std::env::var_os("HALP_MCP_TOKEN_FILE").map(PathBuf::from) else {
        return Err(format!("no token\n{USAGE}"));
    };
    let meta =
        std::fs::metadata(&path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "halp-mcp: warning: {} is readable by other users; run chmod 600 on it",
                path.display()
            );
        }
    }
    if meta.len() > MAX_TOKEN_BYTES {
        return Err(format!("{} is too large to be a token", path.display()));
    }
    let raw = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let t = raw.trim().to_string();
    if t.is_empty() || t.contains(char::is_whitespace) {
        return Err(format!("{} does not hold a token", path.display()));
    }
    Ok(t)
}

#[cfg(unix)]
async fn run() -> Result<(), String> {
    let socket = match socket_from_args()? {
        Some(s) => s,
        None => default_socket()?,
    };
    let token = token()?;
    let stream = tokio::net::UnixStream::connect(&socket)
        .await
        .map_err(|e| {
            format!(
                "cannot reach the Hub at {}: {e} (is it running?)",
                socket.display()
            )
        })?;
    let (r, mut w) = stream.into_split();
    w.write_all(format!("{}{token}\n", halp_mcp::host::AUTH_PREFIX).as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    drop(token);
    let mut r = BufReader::new(r);
    let mut line = String::new();
    (&mut r)
        .take(256)
        .read_line(&mut line)
        .await
        .map_err(|e| e.to_string())?;
    if !line.starts_with("OK ") {
        return Err("the Hub refused the token (revoked or mistyped?)".into());
    }
    let up = async {
        let mut stdin = tokio::io::stdin();
        let r = tokio::io::copy(&mut stdin, &mut w).await;
        let _ = w.shutdown().await;
        r
    };
    let down = async {
        let mut stdout = tokio::io::stdout();
        let r = tokio::io::copy_buf(&mut r, &mut stdout).await;
        let _ = stdout.flush().await;
        r
    };
    // The session ends when the Hub closes the socket, or shortly after the
    // client closes stdin (a call still waiting for consent would otherwise
    // keep the shim alive until the request expires; the request itself
    // stays in the Hub until the user decides).
    tokio::pin!(down);
    tokio::select! {
        r = &mut down => return r.map(|_| ()).map_err(|e| e.to_string()),
        _ = up => {}
    }
    match tokio::time::timeout(EOF_GRACE, down).await {
        Ok(r) => r.map(|_| ()).map_err(|e| e.to_string()),
        Err(_) => Ok(()),
    }
}

#[cfg(not(unix))]
async fn run() -> Result<(), String> {
    let _ = socket_from_args()?;
    let _ = token()?;
    Err("the Hub's MCP socket is not available on this platform yet".into())
}

fn main() -> ExitCode {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("halp-mcp: {e}");
            return ExitCode::from(2);
        }
    };
    let result = rt.block_on(run());
    // A blocking stdin read may still be pending; do not wait for it.
    rt.shutdown_timeout(std::time::Duration::from_millis(100));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("halp-mcp: {e}");
            ExitCode::from(2)
        }
    }
}
