//! The host writes nothing to disk: no compiled-module cache, no temp
//! files. A child process runs transforms with TMPDIR, HOME and the XDG
//! directories pointed at an empty temp dir, which must stay empty.

mod common;

use common::*;
use halp_ext::ExtHost;
use serde_json::json;
use std::path::Path;
use std::sync::Arc;

const CHILD_ENV: &str = "HALP_EXT_NO_DISK_CHILD";

fn entries(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).expect("read_dir") {
            let e = e.expect("entry");
            out.push(e.path().display().to_string());
            if e.path().is_dir() {
                stack.push(e.path());
            }
        }
    }
    out
}

#[test]
fn runs_leave_temp_and_cache_dirs_empty() {
    let dir = tempfile::tempdir().expect("temp dir");
    let exe = std::env::current_exe().expect("test binary");
    let status = std::process::Command::new(exe)
        .args([
            "--ignored",
            "--exact",
            "child_runs_transforms",
            "--test-threads=1",
        ])
        .env(CHILD_ENV, "1")
        .env("TMPDIR", dir.path())
        .env("TMP", dir.path())
        .env("TEMP", dir.path())
        .env("HOME", dir.path())
        .env("XDG_CACHE_HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .status()
        .expect("child runs");
    assert!(status.success(), "child failed: {status}");
    let left = entries(dir.path());
    assert!(left.is_empty(), "files written: {left:?}");
}

#[test]
#[ignore = "run by runs_leave_temp_and_cache_dirs_empty in a child process"]
fn child_runs_transforms() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let host = ExtHost::new(Arc::new(FakeSource::default())).expect("host");
    for i in 0..3 {
        let out = host
            .run_transform_blocking(
                "acme.sre",
                &spec(any_schema()),
                &static_module(format!("{{\"i\":{i}}}").as_bytes()),
                ctx(),
            )
            .expect("run");
        assert_eq!(out.output, json!({"i": i}));
    }
    host.run_transform_blocking("acme.sre", &spec(any_schema()), &echo_module(), ctx())
        .expect("echo");
    assert_eq!(host.cached_module_count(), 4);
}
