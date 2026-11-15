//! The P5-C1 WAT corpus: limits, imports, output checks and round trips.

mod common;

use common::*;
use halp_ext::{ExtError, ExtHost, ExtLimits, InputError, InputSource, ModuleSpec};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

fn host() -> (ExtHost, Arc<FakeSource>) {
    let source = Arc::new(FakeSource::default());
    let host = ExtHost::new(source.clone()).expect("host starts");
    (host, source)
}

fn run(host: &ExtHost, wasm: &[u8], schema: Value) -> Result<halp_ext::RunOutput, ExtError> {
    host.run_transform_blocking("acme.sre", &spec(schema), wasm, ctx())
}

fn infinite_loop() -> Vec<u8> {
    transform_module("", "(loop $l (br $l)) (unreachable)")
}

// ---- round trips ----------------------------------------------------------

#[test]
fn round_trip_passes_inputs_config_and_clock() {
    let (host, source) = host();
    let spec: ModuleSpec = serde_json::from_value(json!({
        "name": "burn",
        "inputs": {"open": {"command": "tasks.list", "args": {"status": "open"}}},
        "output_schema": {
            "type": "object",
            "required": ["inputs", "config", "now", "today"]
        }
    }))
    .expect("spec");
    let out = host
        .run_transform_blocking("acme.sre", &spec, &echo_module(), ctx())
        .expect("run succeeds");
    assert_eq!(
        out.output,
        json!({
            "inputs": {"open": {"command": "tasks.list", "args": {"status": "open"}, "items": [1, 2, 3]}},
            "config": {"view": "metric"},
            "now": "2026-11-11T09:30:00+00:00",
            "today": "2026-11-11"
        })
    );
    let calls = source.calls.lock().expect("calls").clone();
    assert_eq!(
        calls,
        vec![(
            "acme.sre".to_string(),
            "tasks.list".to_string(),
            json!({"status": "open"})
        )]
    );
    assert!(out.log.is_empty());
}

#[tokio::test]
async fn async_round_trip() {
    let (host, _) = host();
    let out = host
        .run_transform(
            "acme.sre",
            &spec(json!({"type": "object"})),
            &static_module(br#"{"value": 42}"#),
            ctx(),
        )
        .await
        .expect("run succeeds");
    assert_eq!(out.output, json!({"value": 42}));
}

#[test]
fn input_failure_stops_the_run() {
    let (host, _) = host();
    let spec: ModuleSpec = serde_json::from_value(json!({
        "name": "x",
        "inputs": {"one": {"command": "tasks.get", "args": {"id": "nope"}}},
        "output_schema": {}
    }))
    .expect("spec");
    let err = host
        .run_transform_blocking("acme.sre", &spec, &echo_module(), ctx())
        .expect_err("input fails");
    assert!(
        matches!(&err, ExtError::Input(m) if m.contains("not_found")),
        "{err:?}"
    );
}

#[test]
fn disallowed_input_command_never_reaches_the_source() {
    let (host, source) = host();
    let spec: ModuleSpec = serde_json::from_value(json!({
        "name": "x",
        "inputs": {"w": {"command": "tasks.update", "args": {}}},
        "output_schema": {}
    }))
    .expect("spec");
    let err = host
        .run_transform_blocking("acme.sre", &spec, &echo_module(), ctx())
        .expect_err("rejected");
    assert!(matches!(err, ExtError::InvalidSpec(_)), "{err:?}");
    assert!(source.calls.lock().expect("calls").is_empty());
}

// ---- fuel, epoch, memory --------------------------------------------------

#[test]
fn infinite_loop_is_stopped_by_fuel() {
    let (host, _) = host();
    let err = run(&host, &infinite_loop(), any_schema()).expect_err("stopped");
    assert_eq!(err, ExtError::FuelExhausted);
    assert_eq!(err.code(), "fuel_exhausted");
}

#[test]
fn infinite_loop_is_stopped_by_the_epoch_deadline() {
    let limits = ExtLimits {
        fuel: u64::MAX,
        deadline: Duration::from_millis(100),
        ..ExtLimits::default()
    };
    let host = ExtHost::with_limits(Arc::new(FakeSource::default()), limits).expect("host");
    let started = Instant::now();
    let err = run(&host, &infinite_loop(), any_schema()).expect_err("stopped");
    assert_eq!(err, ExtError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn default_deadline_stops_a_fuel_rich_loop_within_bounds() {
    // With the default 200 ms deadline, a loop that would outlast its fuel
    // budget by far still ends quickly.
    let limits = ExtLimits {
        fuel: u64::MAX,
        ..ExtLimits::default()
    };
    let host = ExtHost::with_limits(Arc::new(FakeSource::default()), limits).expect("host");
    let started = Instant::now();
    assert_eq!(
        run(&host, &infinite_loop(), any_schema()).expect_err("stopped"),
        ExtError::Timeout
    );
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(150), "took {took:?}");
    assert!(took < Duration::from_secs(5), "took {took:?}");
}

#[test]
fn memory_growth_past_16_mib_fails() {
    let (host, _) = host();
    // 1 page + 256 pages = 16 MiB + 64 KiB.
    let grow = transform_module("", "(drop (memory.grow (i32.const 256))) (i64.const 0)");
    assert_eq!(
        run(&host, &grow, any_schema()).expect_err("grow refused"),
        ExtError::MemoryLimit
    );
    // Growing up to exactly 16 MiB is fine.
    let ok = transform_module(
        "(data (i32.const 0) \"1\")",
        "(drop (memory.grow (i32.const 255))) (i64.const 1)",
    );
    assert_eq!(
        run(&host, &ok, any_schema()).expect("fits").output,
        json!(1)
    );
}

#[test]
fn initial_memory_past_16_mib_fails_at_instantiation() {
    let (host, _) = host();
    let big = wasm(
        r#"(module
  (memory (export "memory") 257)
  (func (export "halp_alloc") (param i32) (result i32) (i32.const 0))
  (func (export "halp_transform") (param i32 i32) (result i64) (i64.const 0)))"#,
    );
    assert_eq!(
        run(&host, &big, any_schema()).expect_err("refused"),
        ExtError::MemoryLimit
    );
}

#[test]
fn table_growth_past_10k_elements_fails() {
    let (host, _) = host();
    let t = transform_module(
        "(table 1 funcref)",
        "(drop (table.grow (ref.null func) (i32.const 10000))) (i64.const 0)",
    );
    assert_eq!(
        run(&host, &t, any_schema()).expect_err("refused"),
        ExtError::MemoryLimit
    );
}

#[test]
fn deep_recursion_hits_the_stack_limit() {
    let (host, _) = host();
    let r = wasm(
        r#"(module
  (memory (export "memory") 1)
  (func $f (param i64) (result i64) (call $f (i64.add (local.get 0) (i64.const 1))))
  (func (export "halp_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "halp_transform") (param i32 i32) (result i64) (call $f (i64.const 0))))"#,
    );
    let err = run(&host, &r, any_schema()).expect_err("overflows");
    assert!(
        matches!(&err, ExtError::Trap(m) if m.contains("stack")),
        "{err:?}"
    );
}

// ---- imports and exports --------------------------------------------------

#[test]
fn wasi_import_fails_at_instantiation() {
    let (host, _) = host();
    let m = transform_module(
        r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
        "(i64.const 0)",
    );
    let err = run(&host, &m, any_schema()).expect_err("refused");
    assert!(
        matches!(&err, ExtError::Instantiate(msg) if msg.contains("wasi_snapshot_preview1.fd_write")),
        "{err:?}"
    );
}

#[test]
fn unknown_imports_fail_at_instantiation() {
    let (host, _) = host();
    for import in [
        r#"(import "env" "now" (func (result i64)))"#,
        r#"(import "halp" "call" (func (param i32 i32)))"#,
        // The right name with the wrong signature.
        r#"(import "halp" "log" (func (param i32)))"#,
        r#"(import "halp" "log" (func (param i32 i32) (result i32)))"#,
        // Not a function at all.
        r#"(import "halp" "log" (global i32))"#,
        r#"(import "halp" "log" (table 1 funcref))"#,
    ] {
        let m = transform_module(import, "(i64.const 0)");
        let err = run(&host, &m, any_schema()).expect_err("refused");
        assert!(matches!(err, ExtError::Instantiate(_)), "{import}: {err:?}");
    }
    let imported_memory = wasm(
        r#"(module
  (import "halp" "log" (memory 1))
  (export "memory" (memory 0))
  (func (export "halp_alloc") (param i32) (result i32) (i32.const 0))
  (func (export "halp_transform") (param i32 i32) (result i64) (i64.const 0)))"#,
    );
    assert!(matches!(
        run(&host, &imported_memory, any_schema()),
        Err(ExtError::Instantiate(_))
    ));
}

#[test]
fn missing_abi_exports_fail() {
    let (host, _) = host();
    let no_alloc = wasm(
        r#"(module (memory (export "memory") 1)
  (func (export "halp_transform") (param i32 i32) (result i64) (i64.const 0)))"#,
    );
    let wrong_sig = wasm(
        r#"(module (memory (export "memory") 1)
  (func (export "halp_alloc") (param i32) (result i32) (i32.const 0))
  (func (export "halp_transform") (param i32 i32) (result i32) (i32.const 0)))"#,
    );
    let no_memory = wasm(
        r#"(module (memory 1)
  (func (export "halp_alloc") (param i32) (result i32) (i32.const 0))
  (func (export "halp_transform") (param i32 i32) (result i64) (i64.const 0)))"#,
    );
    for m in [no_alloc, wrong_sig, no_memory] {
        let err = run(&host, &m, any_schema()).expect_err("refused");
        assert!(matches!(err, ExtError::Instantiate(_)), "{err:?}");
    }
}

#[test]
fn halp_log_is_captured_and_capped_at_4_kib() {
    let (host, _) = host();
    let m = transform_module(
        r#"(import "halp" "log" (func $log (param i32 i32)))
  (data (i32.const 0) "1hello\1b[31m\e2\80\ae")"#,
        r#"(call $log (i32.const 1) (i32.const 13))
    (call $log (i32.const 0) (i32.const 8000))
    (call $log (i32.const 0) (i32.const 10))
    (i64.const 1)"#,
    );
    let out = run(&host, &m, any_schema()).expect("runs");
    assert_eq!(out.output, json!(1));
    assert!(out.log_truncated);
    // The first line has its escape and bidi characters replaced.
    let first = out.log.lines().next().expect("first line");
    assert_eq!(first, "hello\u{FFFD}[31m\u{FFFD}");
    assert!(out.log.len() <= 4096, "kept {} bytes", out.log.len());
    assert!(out.log.len() > 4000, "kept {} bytes", out.log.len());
    assert_eq!(out.log.lines().count(), 2, "third call is dropped");
}

#[test]
fn halp_log_text_within_the_cap_is_kept_whole() {
    let (host, _) = host();
    let m = transform_module(
        r#"(import "halp" "log" (func $log (param i32 i32)))
  (data (i32.const 0) "1two words")"#,
        r#"(call $log (i32.const 1) (i32.const 9))
    (call $log (i32.const 1) (i32.const 3))
    (i64.const 1)"#,
    );
    let out = run(&host, &m, any_schema()).expect("runs");
    assert_eq!(out.log, "two words\ntwo");
    assert!(!out.log_truncated);
}

#[test]
fn halp_log_out_of_bounds_traps() {
    let (host, _) = host();
    let m = transform_module(
        r#"(import "halp" "log" (func $log (param i32 i32)))"#,
        "(call $log (i32.const 65530) (i32.const 100)) (i64.const 0)",
    );
    assert!(matches!(
        run(&host, &m, any_schema()),
        Err(ExtError::Trap(_))
    ));
}

// ---- module checks ------------------------------------------------------

#[test]
fn oversize_module_is_rejected() {
    let (host, _) = host();
    let mut big = echo_module();
    big.resize(2 * 1024 * 1024 + 1, 0);
    assert!(matches!(
        run(&host, &big, any_schema()),
        Err(ExtError::InvalidModule(m)) if m.contains("bytes")
    ));
}

#[test]
fn text_and_garbage_modules_are_rejected() {
    let (host, _) = host();
    let text = br#"(module (memory (export "memory") 1))"#;
    assert!(matches!(
        run(&host, text, any_schema()),
        Err(ExtError::InvalidModule(_))
    ));
    assert!(matches!(
        run(&host, b"\0asm\x01\0\0\0\xff", any_schema()),
        Err(ExtError::InvalidModule(_))
    ));
}

#[test]
fn shared_memory_and_memory64_are_refused() {
    let (host, _) = host();
    for wat in [
        r#"(module (memory (export "memory") 1 1 shared))"#,
        r#"(module (memory (export "memory") i64 1))"#,
        r#"(module (memory 1) (memory 1))"#,
    ] {
        let err = run(&host, &wasm(wat), any_schema()).expect_err("refused");
        assert!(matches!(err, ExtError::InvalidModule(_)), "{wat}: {err:?}");
    }
}

#[test]
fn compiled_modules_are_cached_by_hash() {
    let (host, _) = host();
    let a = static_module(b"1");
    let b = static_module(b"2");
    assert_eq!(host.cached_module_count(), 0);
    run(&host, &a, any_schema()).expect("a");
    run(&host, &a, any_schema()).expect("a again");
    assert_eq!(host.cached_module_count(), 1);
    run(&host, &b, any_schema()).expect("b");
    assert_eq!(host.cached_module_count(), 2);
}

// ---- output checks ------------------------------------------------------

#[test]
fn oversize_output_is_rejected() {
    let (host, _) = host();
    let mut big = b"\"".to_vec();
    big.extend(std::iter::repeat_n(b'a', 256 * 1024));
    big.push(b'"');
    let err = run(&host, &static_module(&big), any_schema()).expect_err("too big");
    assert!(
        matches!(&err, ExtError::InvalidOutput(m) if m.contains("bytes")),
        "{err:?}"
    );
    // A claimed length far past memory is refused before reading.
    let lying = transform_module("", "(i64.const 0x7fffffff)");
    assert!(matches!(
        run(&host, &lying, any_schema()),
        Err(ExtError::InvalidOutput(_))
    ));
}

#[test]
fn out_of_bounds_output_is_rejected() {
    let (host, _) = host();
    // ptr = 65000, len = 1000: crosses the end of the one-page memory.
    let m = transform_module("", "(i64.const 0x0000FDE8000003E8)");
    assert!(matches!(
        run(&host, &m, any_schema()),
        Err(ExtError::InvalidOutput(msg)) if msg.contains("out of bounds")
    ));
}

#[test]
fn bidi_output_is_rejected() {
    let (host, _) = host();
    for out in [
        "{\"v\":\"a\u{202E}b\"}".as_bytes(),
        "{\"v\":\"a\u{200F}b\"}".as_bytes(),
        "{\"a\u{2066}\":1}".as_bytes(),
        br#"["\u0007"]"#,
    ] {
        let err = run(&host, &static_module(out), any_schema()).expect_err("rejected");
        assert!(matches!(err, ExtError::InvalidOutput(_)), "{err:?}");
    }
}

#[test]
fn deep_output_is_rejected() {
    let (host, _) = host();
    // halp-domain json_depth counts container edges, so `[0]` is depth 1.
    let nest = |n: usize| format!("{}0{}", "[".repeat(n), "]".repeat(n));
    assert!(run(&host, &static_module(nest(32).as_bytes()), any_schema()).is_ok());
    assert!(matches!(
        run(&host, &static_module(nest(33).as_bytes()), any_schema()),
        Err(ExtError::InvalidOutput(m)) if m.contains("deeper")
    ));
}

#[test]
fn non_json_output_is_rejected() {
    let (host, _) = host();
    for out in [&b"not json"[..], b"{\"a\":1} trailing", b"\xff\xfe", b""] {
        assert!(matches!(
            run(&host, &static_module(out), any_schema()),
            Err(ExtError::InvalidOutput(_))
        ));
    }
}

#[test]
fn output_failing_the_schema_is_rejected() {
    let (host, _) = host();
    let schema = json!({
        "type": "object",
        "properties": {"v": {"type": "number"}},
        "required": ["v"]
    });
    let err = run(&host, &static_module(br#"{"v":"text"}"#), schema.clone()).expect_err("rejected");
    assert!(matches!(err, ExtError::SchemaMismatch(_)), "{err:?}");
    assert_eq!(err.code(), "schema_mismatch");
    let ok = run(&host, &static_module(br#"{"v":3}"#), schema).expect("valid");
    assert_eq!(ok.output, json!({"v": 3}));
}

// ---- pool -----------------------------------------------------------------

#[test]
fn concurrent_runs_all_complete() {
    let (host, _) = host();
    let host = Arc::new(host);
    let wasm = Arc::new(static_module(br#"{"ok":true}"#));
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let host = host.clone();
            let wasm = wasm.clone();
            std::thread::spawn(move || run(&host, &wasm, any_schema()))
        })
        .collect();
    for h in handles {
        let out = h.join().expect("thread").expect("run");
        assert_eq!(out.output, json!({"ok": true}));
    }
}

/// Blocks in `fetch` until released, to hold a run thread.
struct GateSource {
    entered: Mutex<Option<mpsc::Sender<()>>>,
    release: Mutex<mpsc::Receiver<()>>,
}

impl InputSource for GateSource {
    fn fetch(&self, _: &str, _: &str, _: &Value) -> Result<Value, InputError> {
        if let Some(tx) = self.entered.lock().expect("lock").take() {
            let _ = tx.send(());
        }
        let _ = self.release.lock().expect("lock").recv();
        Ok(json!([]))
    }
}

#[test]
fn full_queue_reports_busy() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let source = Arc::new(GateSource {
        entered: Mutex::new(Some(entered_tx)),
        release: Mutex::new(release_rx),
    });
    let limits = ExtLimits {
        threads: 1,
        max_queued: 0,
        ..ExtLimits::default()
    };
    let host = Arc::new(ExtHost::with_limits(source, limits).expect("host"));
    let spec: ModuleSpec = serde_json::from_value(json!({
        "name": "x",
        "inputs": {"a": {"command": "day.state"}},
        "output_schema": {}
    }))
    .expect("spec");
    let wasm = static_module(b"1");
    let first = {
        let host = host.clone();
        let spec = spec.clone();
        let wasm = wasm.clone();
        std::thread::spawn(move || host.run_transform_blocking("acme.sre", &spec, &wasm, ctx()))
    };
    entered_rx.recv().expect("first run entered fetch");
    let err = host
        .run_transform_blocking("acme.sre", &spec, &wasm, ctx())
        .expect_err("busy");
    assert_eq!(err, ExtError::Busy);
    release_tx.send(()).expect("release");
    assert_eq!(first.join().expect("thread").expect("run").output, json!(1));
}

#[test]
fn bad_pack_id_is_rejected() {
    let (host, _) = host();
    let err = host
        .run_transform_blocking("../evil", &spec(any_schema()), &echo_module(), ctx())
        .expect_err("rejected");
    assert!(matches!(err, ExtError::InvalidSpec(_)));
}
