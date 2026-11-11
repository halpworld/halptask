//! `context.capture` on real temporary repositories, including a symlink
//! that points out of the configured roots.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;
use std::process::Command;

use serde_json::json;

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new(halp_integrations::git::find_git().unwrap())
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["-c", "user.name=T", "-c", "user.email=t@example.com"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        ok.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn captures_a_work_tree_and_rejects_escapes() {
    if halp_integrations::git::find_git().is_none() {
        eprintln!("git not installed; skipping");
        return;
    }
    let h = common::start();
    let root = h.dir.path().join("code");
    let repo = root.join("api");
    let outside = h.dir.path().join("elsewhere");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    git(&repo, &["init", "-q", "-b", "feature/x"]);
    git(
        &repo,
        &["remote", "add", "origin", "git@github.com:acme/api.git"],
    );
    std::fs::write(repo.join("a.txt"), "1").unwrap();
    git(&repo, &["add", "a.txt"]);
    git(&repo, &["commit", "-q", "-m", "First \u{1b}[31mcommit"]);
    std::fs::write(repo.join("b.txt"), "2").unwrap();
    git(&outside, &["init", "-q"]);
    std::os::unix::fs::symlink(&outside, root.join("sneaky")).unwrap();

    h.ok(
        "integrations.connect",
        json!({"id": "git", "kind": "git", "title": "Repos", "settings": {"roots": [root.to_str().unwrap()]}}),
    );
    let t = h.ok(
        "context.capture",
        json!({"repo_path": repo.to_str().unwrap()}),
    );
    assert_eq!(t["queued"], true);
    h.until("a context item", |h| {
        !h.items("git").iter().all(|i| i["type_id"] != "dev.context")
    })
    .await;
    let items = h.items("git");
    let c = items
        .iter()
        .find(|i| i["type_id"] == "dev.context")
        .unwrap();
    assert_eq!(c["kind"], "bullet");
    assert_eq!(c["fields"]["repo"], "acme/api");
    assert_eq!(c["fields"]["branch"], "feature/x");
    assert_eq!(c["fields"]["files"], json!(["b.txt"]));
    assert_eq!(c["fields"]["note"], "First [31mcommit");
    assert_eq!(c["title"], "acme/api @ feature/x");

    let bad = |p: &str| match h.call("context.capture", json!({"repo_path": p})) {
        halp_core::Response::Err(e) => e.message,
        halp_core::Response::Ok(v) => panic!("captured {p}: {v}"),
    };
    assert!(bad(root.join("sneaky").to_str().unwrap()).contains("not under a configured root"));
    assert!(bad(outside.to_str().unwrap()).contains("not under a configured root"));
    assert!(bad(root.join("api/../../elsewhere").to_str().unwrap()).contains("not under"));
    assert!(bad("relative/path").contains("absolute"));
    // A subdirectory is not the top of the work tree: queued, then refused.
    std::fs::create_dir_all(repo.join("sub")).unwrap();
    h.ok(
        "context.capture",
        json!({"repo_path": repo.join("sub").to_str().unwrap()}),
    );
    h.until("an error", |h| h.state("git") == "error").await;
    assert!(
        h.entry("git")["status"]["error"]
            .as_str()
            .unwrap()
            .contains("top of a git work tree")
    );
}

/// Runs git on the fixture with no overrides at all (as a user would),
/// for the control half of [`a_hostile_repo_runs_nothing`].
fn plain_git(dir: &Path, args: &[&str]) {
    let _ = Command::new(halp_integrations::git::find_git().unwrap())
        .current_dir(dir)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .unwrap();
}

fn git_out(dir: &Path, args: &[&str]) -> String {
    let o = Command::new(halp_integrations::git::find_git().unwrap())
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(args)
        .output()
        .unwrap();
    assert!(o.status.success(), "{args:?}");
    String::from_utf8(o.stdout).unwrap().trim().to_string()
}

/// A repository whose own `.git/config` names a program for every route
/// git has to run one during status-like reads: clean, smudge and process
/// filters (with racily clean files, so a comparison has to re-hash
/// them), fsmonitor, a signed HEAD with `log.showSignature` and
/// `gpg.program`, `diff.external`, a textconv driver, hooks, ssh, a pager
/// and askpass. Each program appends to its own marker file. A capture
/// must leave no marker; the same repo under plain git must leave them
/// (so the fixture really is hostile).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_hostile_repo_runs_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let Some(bin) = halp_integrations::git::find_git() else {
        eprintln!("git not installed; skipping");
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let base = tmp.path().canonicalize().unwrap();
    let marks = base.join("marks");
    let progs = base.join("progs");
    let repo = base.join("evil");
    for d in [&marks, &progs, &repo, &progs.join("hooks")] {
        std::fs::create_dir_all(d).unwrap();
    }
    let routes = [
        "clean",
        "smudge",
        "process",
        "fsmonitor",
        "gpg",
        "extdiff",
        "textconv",
        "hook",
        "ssh",
        "pager",
        "askpass",
    ];
    for r in routes {
        let p = progs.join(r);
        let tail = if r == "process" { "exit 1" } else { "cat" };
        std::fs::write(
            &p,
            format!(
                "#!/bin/sh\necho hit >> '{}'\n{tail}\n",
                marks.join(r).display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for h in [
        "post-index-change",
        "reference-transaction",
        "post-checkout",
    ] {
        std::fs::copy(progs.join("hook"), progs.join("hooks").join(h)).unwrap();
    }

    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    std::fs::write(repo.join("b.txt"), "b\n").unwrap();
    std::fs::write(repo.join("p.txt"), "p\n").unwrap();
    git(&repo, &["add", "a.txt", "b.txt", "p.txt"]);
    git(&repo, &["commit", "-q", "-m", "init"]);
    // HEAD: a commit with a different tree and a (bogus) signature.
    std::fs::write(repo.join("b.txt"), "b2\n").unwrap();
    git(&repo, &["add", "b.txt"]);
    let tree = git_out(&repo, &["write-tree"]);
    let parent = git_out(&repo, &["rev-parse", "HEAD"]);
    let raw = format!(
        "tree {tree}\nparent {parent}\nauthor a <a@b> 1700000000 +0000\n\
         committer a <a@b> 1700000000 +0000\ngpgsig -----BEGIN PGP SIGNATURE-----\n \n \
         AAAA\n -----END PGP SIGNATURE-----\n\nSigned subject\nsecond line\n\nbody\n"
    );
    std::fs::write(base.join("commit.txt"), raw).unwrap();
    let c = git_out(
        &base,
        &[
            "--git-dir",
            repo.join(".git").to_str().unwrap(),
            "hash-object",
            "-t",
            "commit",
            "-w",
            base.join("commit.txt").to_str().unwrap(),
        ],
    );
    git(&repo, &["update-ref", "refs/heads/main", &c]);
    std::fs::write(
        repo.join(".gitattributes"),
        "* filter=x diff=y\np.txt filter=p diff=y\n",
    )
    .unwrap();
    let prog = |r: &str| progs.join(r).display().to_string();
    let config = format!(
        "[filter \"x\"]\n\tclean = {}\n\tsmudge = {}\n\trequired = true\n\
         [filter \"p\"]\n\tprocess = {}\n\
         [diff \"y\"]\n\ttextconv = {}\n\
         [diff]\n\texternal = {}\n\
         [log]\n\tshowSignature = true\n\
         [gpg]\n\tprogram = {}\n\
         [core]\n\tfsmonitor = {}\n\tsshCommand = {}\n\thooksPath = {}\n\tpager = {}\n\taskPass = {}\n",
        prog("clean"),
        prog("smudge"),
        prog("process"),
        prog("textconv"),
        prog("extdiff"),
        prog("gpg"),
        prog("fsmonitor"),
        prog("ssh"),
        progs.join("hooks").display(),
        prog("pager"),
        prog("askpass"),
    );
    let mut cfg = std::fs::read_to_string(repo.join(".git/config")).unwrap();
    cfg.push_str(&config);
    std::fs::write(repo.join(".git/config"), cfg).unwrap();
    // Racily clean a.txt and p.txt: their index stat matches the file and
    // is no older than the index itself, so a comparison must re-hash
    // them (through the filter) to tell whether they changed.
    let old = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    for f in ["a.txt", "p.txt"] {
        std::fs::File::options()
            .write(true)
            .open(repo.join(f))
            .unwrap()
            .set_modified(old)
            .unwrap();
    }
    git(
        &repo,
        &[
            "-c",
            "filter.x.clean=",
            "-c",
            "filter.x.required=false",
            "-c",
            "filter.p.process=",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=/dev/null",
            "update-index",
            "-q",
            "--refresh",
        ],
    );
    std::fs::File::options()
        .write(true)
        .open(repo.join(".git/index"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    // A dirty file and an untracked one.
    std::fs::write(repo.join("b.txt"), "b3 changed\n").unwrap();
    std::fs::write(repo.join("new.txt"), "n\n").unwrap();
    let hits = || -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(&marks)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        v.sort();
        v
    };

    let snap =
        halp_integrations::git::snapshot(&bin, &repo, &tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();
    assert_eq!(
        hits(),
        Vec::<String>::new(),
        "capture ran a configured program"
    );
    assert_eq!(snap.branch, "main");
    assert_eq!(snap.subject.as_deref(), Some("Signed subject second line"));
    assert!(
        snap.files.contains(&"b.txt".to_string()),
        "{:?}",
        snap.files
    );
    assert!(
        snap.files.contains(&"new.txt".to_string()),
        "{:?}",
        snap.files
    );
    assert!(snap.files.contains(&".gitattributes".to_string()));
    // The racily clean files are unchanged: hashed without the filter.
    assert!(
        !snap.files.contains(&"a.txt".to_string()),
        "{:?}",
        snap.files
    );

    // Control: plain git on the same repo does run them. The process
    // filter goes first: a status that gets through refreshes the index,
    // after which nothing is racy any more.
    plain_git(
        &repo,
        &[
            "-c",
            "filter.x.clean=",
            "-c",
            "filter.x.required=false",
            "-c",
            "core.fsmonitor=false",
            "status",
        ],
    );
    plain_git(
        &repo,
        &[
            "-c",
            "filter.p.process=",
            "-c",
            "core.fsmonitor=false",
            "status",
        ],
    );
    plain_git(
        &repo,
        &[
            "-c",
            "filter.x.clean=",
            "-c",
            "filter.x.required=false",
            "-c",
            "filter.p.process=",
            "status",
        ],
    );
    plain_git(&repo, &["log", "-1", "--format=%s"]);
    plain_git(&repo, &["diff", "HEAD~1", "HEAD"]);
    plain_git(&repo, &["diff", "--no-ext-diff", "HEAD~1", "HEAD"]);
    let seen = hits();
    for r in [
        "clean",
        "process",
        "fsmonitor",
        "gpg",
        "extdiff",
        "textconv",
    ] {
        assert!(
            seen.contains(&r.to_string()),
            "control did not run {r}: {seen:?}"
        );
    }
}
