//! Local git context snapshots (`context.capture`): the branch, changed
//! files and last commit subject of a work tree under one of the
//! configured roots, as a `dev.context` bullet.
//!
//! The path is canonicalised (so a symlink pointing out of the roots is
//! rejected) and must be the top of a work tree. `git` runs from an
//! absolute path with a cleared environment, no global or system config,
//! fixed arguments and a timeout. The work tree's own `.git/config` is
//! attacker-controlled (an archive, a fixture, an agent's files), so only
//! plumbing that cannot reach a configured program is used, and config
//! that names a program (filters, fsmonitor, hooks, gpg, external diff,
//! ssh) is overridden from the environment; see [`snapshot`].

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::connector::{Batch, BoxFuture, Connector, SyncCx, SyncError, cap};
use crate::http::Http;

pub const MAX_ROOTS: usize = 20;
const MAX_FILES: usize = 100;
const MAX_OUTPUT: u64 = 1024 * 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitSettings {
    /// Directories whose work trees may be captured.
    pub roots: Vec<String>,
}

impl GitSettings {
    pub fn parse(v: &Value) -> Result<GitSettings, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("git settings: {e}"))
    }
}

/// The `git` binary: the first of the usual absolute locations.
pub fn find_git() -> Option<PathBuf> {
    [
        "/usr/bin/git",
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
        "/bin/git",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| p.is_file())
}

/// The canonical work tree for `repo_path`, if it lies under a root.
pub fn check_path(s: &GitSettings, repo_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(repo_path);
    if !p.is_absolute() {
        return Err("repo_path must be absolute".into());
    }
    let canon = p
        .canonicalize()
        .map_err(|_| "repo_path does not exist".to_string())?;
    if !canon.is_dir() {
        return Err("repo_path is not a directory".into());
    }
    let inside = s.roots.iter().any(|r| {
        Path::new(r)
            .canonicalize()
            .is_ok_and(|root| canon.starts_with(&root))
    });
    if !inside {
        return Err("repo_path is not under a configured root".into());
    }
    Ok(canon)
}

/// Config forced on every run. It is passed through `GIT_CONFIG_*`, which
/// beats the repository's own `.git/config`: a work tree under a root may
/// come from an archive or an agent, so its config is attacker-chosen and
/// must not get to name a program git runs. None of the commands below
/// runs these anyway (no fetch, no diff text, no signature check, no
/// hooks); the overrides are a second line.
const FORCED: &[(&str, &str)] = &[
    ("core.fsmonitor", "false"),
    ("core.useBuiltinFSMonitor", "false"),
    ("core.hooksPath", "/dev/null"),
    ("core.attributesFile", "/dev/null"),
    ("core.quotePath", "false"),
    ("core.pager", "cat"),
    ("core.sshCommand", ""),
    ("core.askPass", ""),
    ("core.untrackedCache", "false"),
    ("credential.helper", ""),
    ("diff.external", ""),
    ("gpg.program", "false"),
    ("gpg.openpgp.program", "false"),
    ("gpg.ssh.program", "false"),
    ("gpg.x509.program", "false"),
    ("log.showSignature", "false"),
    ("protocol.allow", "never"),
];

/// At most this many filter drivers are neutralised; a repo with more is
/// refused rather than captured half-protected.
const MAX_FILTERS: usize = 32;

/// Runs one `git` command and returns its stdout (cut at
/// [`MAX_OUTPUT`], at the last separator). `ok` lists exit codes besides
/// 0 that count as success (`git config` exits 1 when nothing matches).
async fn git_run(
    bin: &Path,
    cwd: &Path,
    args: &[&str],
    extra: &[(String, String)],
    ok: &[i32],
    cancel: &CancellationToken,
) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.current_dir(cwd)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_PROTOCOL_FROM_USER", "0")
        .env("LC_ALL", "C");
    let forced = FORCED
        .iter()
        .map(|(k, v)| (*k, *v))
        .chain(extra.iter().map(|(k, v)| (k.as_str(), v.as_str())));
    let mut n = 0usize;
    for (k, v) in forced {
        cmd.env(format!("GIT_CONFIG_KEY_{n}"), k)
            .env(format!("GIT_CONFIG_VALUE_{n}"), v);
        n += 1;
    }
    cmd.env("GIT_CONFIG_COUNT", n.to_string())
        .args(["--no-pager", "--no-optional-locks"])
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|_| "could not run git".to_string())?;
    let mut out = Vec::new();
    let stdout = child.stdout.take().ok_or("git has no stdout")?;
    let run = async {
        stdout
            .take(MAX_OUTPUT)
            .read_to_end(&mut out)
            .await
            .map_err(|e| e.to_string())?;
        if out.len() as u64 >= MAX_OUTPUT {
            // Cut: stop git rather than wait for it to block on the pipe.
            let _ = child.start_kill();
            let _ = child.wait().await;
            let keep = out
                .iter()
                .rposition(|b| *b == 0 || *b == b'\n')
                .map_or(0, |i| i + 1);
            out.truncate(keep);
            return Ok(true);
        }
        let st = child.wait().await.map_err(|e| e.to_string())?;
        Ok::<bool, String>(st.success() || st.code().is_some_and(|c| ok.contains(&c)))
    };
    let fine = tokio::select! {
        _ = cancel.cancelled() => return Err("cancelled".into()),
        _ = tokio::time::sleep(GIT_TIMEOUT) => return Err("git timed out".into()),
        r = run => r?,
    };
    if !fine {
        return Err(format!("git {} failed", args.first().unwrap_or(&"")));
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

async fn git(
    bin: &Path,
    cwd: &Path,
    args: &[&str],
    extra: &[(String, String)],
    cancel: &CancellationToken,
) -> Result<String, String> {
    git_run(bin, cwd, args, extra, &[], cancel).await
}

/// Overrides that switch off every filter driver the repository's config
/// defines (`filter.<name>.clean|smudge|process`). Comparing the index
/// with the work tree re-hashes "racily clean" files, and hashing runs a
/// file's clean or process filter; an empty command means no filter.
/// The driver names are read from the same config sources the later
/// commands read, so every driver an attribute can name is covered.
async fn filter_overrides(
    bin: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<Vec<(String, String)>, String> {
    let keys = git_run(
        bin,
        path,
        &["config", "-z", "--name-only", "--get-regexp", r"^filter\."],
        &[],
        &[1],
        cancel,
    )
    .await?;
    let mut drivers: Vec<&str> = Vec::new();
    for k in keys.split(['\0', '\n']) {
        let Some(rest) = k.strip_prefix("filter.") else {
            continue;
        };
        let Some((name, _var)) = rest.rsplit_once('.') else {
            continue;
        };
        if !drivers.contains(&name) {
            drivers.push(name);
        }
    }
    if drivers.len() > MAX_FILTERS {
        return Err("the repository defines too many filter drivers".into());
    }
    let mut out = Vec::new();
    for d in drivers {
        for (var, v) in [
            ("clean", ""),
            ("smudge", ""),
            ("process", ""),
            ("required", "false"),
        ] {
            out.push((format!("filter.{d}.{var}"), v.to_string()));
        }
    }
    Ok(out)
}

/// NUL-separated names, cleaned and capped, into `files` (no duplicates).
fn add_names(files: &mut Vec<String>, out: &str) {
    for e in out.split('\0') {
        let name = halp_domain::integration::strip_controls(e);
        if name.is_empty() || files.len() >= MAX_FILES {
            continue;
        }
        let name = cap(&name, 500);
        if !files.contains(&name) {
            files.push(name);
        }
    }
}

/// The subject of a raw commit object (`git cat-file commit`): the first
/// paragraph of the message joined into one line, as `%s` gives it.
fn commit_subject(raw: &str) -> Option<String> {
    let (_, msg) = raw.split_once("\n\n")?;
    let subject = msg
        .lines()
        .map(str::trim)
        .skip_while(|l| l.is_empty())
        .take_while(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let subject = subject.trim().to_string();
    (!subject.is_empty()).then_some(subject)
}

/// What a capture saw.
#[derive(Debug, Clone, PartialEq)]
pub struct Snapshot {
    pub path: PathBuf,
    pub repo: String,
    pub branch: String,
    pub files: Vec<String>,
    pub subject: Option<String>,
}

/// Reads a work tree without running anything its config names. Only
/// plumbing that never reaches a configured program is used:
/// `rev-parse`, `symbolic-ref`, `config --get`, `cat-file commit` (the
/// subject, with no signature check), `diff-index --cached` (staged, index
/// against `HEAD`), `diff-files` (changed in the work tree; every filter
/// driver switched off, submodules ignored) and `ls-files --others`
/// (untracked). A file whose timestamps changed but whose content did not
/// may be listed as changed, since nothing refreshes the index.
///
/// Everything read here is the repository's own text: the runtime writes
/// it as the integration (`origin: integration:<id>`), so it is untrusted
/// content to agents that read it.
pub async fn snapshot(
    bin: &Path,
    path: &Path,
    cancel: &CancellationToken,
) -> Result<Snapshot, String> {
    let top = git(bin, path, &["rev-parse", "--show-toplevel"], &[], cancel).await?;
    let top = Path::new(top.trim())
        .canonicalize()
        .map_err(|_| "not a git work tree".to_string())?;
    if top != path {
        return Err("repo_path must be the top of a git work tree".into());
    }
    let filters = filter_overrides(bin, path, cancel).await?;
    let branch = match git(
        bin,
        path,
        &["symbolic-ref", "--quiet", "--short", "HEAD"],
        &[],
        cancel,
    )
    .await
    {
        Ok(b) if !b.trim().is_empty() => b.trim().to_string(),
        _ => match git(bin, path, &["rev-parse", "--short", "HEAD"], &[], cancel).await {
            Ok(h) => format!("detached@{}", h.trim()),
            Err(_) => "HEAD".into(),
        },
    };
    let mut files = Vec::new();
    let has_head = git(
        bin,
        path,
        &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"],
        &[],
        cancel,
    )
    .await
    .is_ok();
    let staged = if has_head {
        git(
            bin,
            path,
            &[
                "diff-index",
                "--cached",
                "--name-only",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=all",
                "HEAD",
                "--",
            ],
            &filters,
            cancel,
        )
        .await?
    } else {
        // No commit yet: everything in the index is staged.
        git(bin, path, &["ls-files", "-z"], &filters, cancel).await?
    };
    add_names(&mut files, &staged);
    let changed = git(
        bin,
        path,
        &[
            "diff-files",
            "--name-only",
            "-z",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=all",
            "--",
        ],
        &filters,
        cancel,
    )
    .await?;
    add_names(&mut files, &changed);
    let untracked = git(
        bin,
        path,
        &[
            "ls-files",
            "--others",
            "--exclude-standard",
            "--directory",
            "--no-empty-directory",
            "-z",
        ],
        &filters,
        cancel,
    )
    .await?;
    add_names(&mut files, &untracked);
    let subject = git(bin, path, &["cat-file", "commit", "HEAD"], &[], cancel)
        .await
        .ok()
        .as_deref()
        .and_then(commit_subject);
    let remote = git(
        bin,
        path,
        &["config", "--get", "remote.origin.url"],
        &[],
        cancel,
    )
    .await
    .ok();
    let dir = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(Snapshot {
        path: path.to_path_buf(),
        repo: repo_name(remote.as_deref(), &dir),
        branch: cap(&halp_domain::integration::strip_controls(&branch), 255),
        files,
        subject,
    })
}

/// `owner/name` from the origin URL, else `local/<dir>`.
pub fn repo_name(remote: Option<&str>, dir: &str) -> String {
    let clean = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
            .take(99)
            .collect()
    };
    if let Some(r) = remote {
        let r = r.trim().trim_end_matches('/').trim_end_matches(".git");
        let tail = r.rsplit([':', '/']).take(2).collect::<Vec<_>>();
        if let [name, owner] = tail[..] {
            let (o, n) = (clean(owner), clean(name));
            if !o.is_empty() && !n.is_empty() && o == owner && n == name {
                return format!("{o}/{n}");
            }
        }
    }
    let d = clean(dir);
    format!("local/{}", if d.is_empty() { "repo".into() } else { d })
}

/// The `dev.context` item for a snapshot.
pub fn context_item(s: &Snapshot, now: DateTime<Utc>) -> Value {
    let h = hex::encode(Sha256::digest(s.path.to_string_lossy().as_bytes()));
    let mut f = serde_json::Map::new();
    f.insert("repo".into(), json!(s.repo));
    f.insert("branch".into(), json!(s.branch));
    f.insert(
        "captured_at".into(),
        json!(now.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)),
    );
    if !s.files.is_empty() {
        f.insert("files".into(), json!(s.files));
    }
    if let Some(subj) = &s.subject {
        f.insert("note".into(), json!(cap(subj, 2000)));
    }
    json!({
        "external_id": cap(&format!("git:{}#{}", &h[..16], s.branch), 300),
        "title": cap(&format!("{} @ {}", s.repo, s.branch), 300),
        "kind": "bullet",
        "type_id": "dev.context",
        "tags": ["git"],
        "fields": f,
    })
}

/// The `git` kind: configuration only; it does not poll.
pub struct Git;

impl Connector for Git {
    fn kind(&self) -> &'static str {
        "git"
    }

    fn validate(&self, settings: &Value, _http: &Http) -> Result<Value, String> {
        let s = GitSettings::parse(settings)?;
        if s.roots.is_empty() || s.roots.len() > MAX_ROOTS {
            return Err(format!("git: 1..{MAX_ROOTS} roots"));
        }
        let mut roots = Vec::new();
        for r in &s.roots {
            let p = Path::new(r);
            if !p.is_absolute() {
                return Err(format!("git: root {r:?} must be absolute"));
            }
            let c = p
                .canonicalize()
                .map_err(|_| format!("git: root {r:?} does not exist"))?;
            if c.parent().is_none() {
                return Err("git: the filesystem root cannot be a root".into());
            }
            roots.push(c.to_string_lossy().into_owned());
        }
        serde_json::to_value(GitSettings { roots }).map_err(|e| e.to_string())
    }

    fn needs_token(&self, _settings: &Value) -> bool {
        false
    }

    fn polls(&self) -> bool {
        false
    }

    fn sync<'a>(&'a self, _cx: SyncCx<'a>) -> BoxFuture<'a, Result<Vec<Batch>, SyncError>> {
        Box::pin(async { Ok(Vec::new()) })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn repo_names() {
        assert_eq!(repo_name(Some("git@github.com:o/r.git"), "x"), "o/r");
        assert_eq!(repo_name(Some("https://github.com/o/r"), "x"), "o/r");
        assert_eq!(
            repo_name(Some("/srv/weird path/r b"), "my dir"),
            "local/mydir"
        );
        assert_eq!(repo_name(None, ""), "local/repo");
    }
}
