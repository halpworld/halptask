//! Shared helpers for the WAT test corpus.
#![allow(dead_code)]

use chrono::{NaiveDate, TimeZone, Utc};
use halp_ext::{InputError, InputSource, ModuleSpec, TransformContext};
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::sync::Mutex;

/// Records every fetch and answers with a fixed document.
#[derive(Default)]
pub struct FakeSource {
    pub calls: Mutex<Vec<(String, String, Value)>>,
}

impl InputSource for FakeSource {
    fn fetch(&self, pack_id: &str, command: &str, args: &Value) -> Result<Value, InputError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push((pack_id.into(), command.into(), args.clone()));
        if command == "tasks.get" {
            return Err(InputError {
                code: "not_found".into(),
                message: "no such item".into(),
            });
        }
        Ok(json!({"command": command, "args": args, "items": [1, 2, 3]}))
    }
}

pub fn ctx() -> TransformContext {
    TransformContext {
        config: json!({"view": "metric"}),
        now: Utc
            .with_ymd_and_hms(2026, 11, 11, 9, 30, 0)
            .single()
            .expect("valid time"),
        today: NaiveDate::from_ymd_opt(2026, 11, 11).expect("valid date"),
    }
}

pub fn spec(output_schema: Value) -> ModuleSpec {
    serde_json::from_value(json!({
        "name": "t",
        "wasm": "wasm/t.wasm",
        "output_schema": output_schema,
    }))
    .expect("spec parses")
}

pub fn any_schema() -> Value {
    json!({})
}

pub fn wasm(wat: &str) -> Vec<u8> {
    wat::parse_str(wat).expect("WAT parses")
}

/// `halp_alloc`: a bump allocator from 1024 that grows memory as needed.
pub const BUMP_ALLOC: &str = r#"
  (global $next (mut i32) (i32.const 1024))
  (func (export "halp_alloc") (param $len i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $next))
    (if (i32.gt_u (i32.add (local.get $p) (local.get $len))
                  (i32.mul (memory.size) (i32.const 65536)))
      (then (drop (memory.grow
        (i32.add (i32.div_u (local.get $len) (i32.const 65536)) (i32.const 1))))))
    (global.set $next (i32.add (local.get $p) (local.get $len)))
    (local.get $p))
"#;

/// Echoes its input back as output.
pub fn echo_module() -> Vec<u8> {
    wasm(&format!(
        r#"(module
  (memory (export "memory") 1)
  {BUMP_ALLOC}
  (func (export "halp_transform") (param $ptr i32) (param $len i32) (result i64)
    (i64.or (i64.shl (i64.extend_i32_u (local.get $ptr)) (i64.const 32))
            (i64.extend_i32_u (local.get $len)))))"#
    ))
}

fn wat_bytes(data: &[u8]) -> String {
    let mut s = String::with_capacity(data.len() * 3);
    for b in data {
        let _ = write!(s, "\\{b:02x}");
    }
    s
}

/// Returns `output` (stored in a data segment at 0) whatever the input.
pub fn static_module(output: &[u8]) -> Vec<u8> {
    let data_end = output.len().div_ceil(16) * 16 + 16;
    let pages = (data_end + 65536).div_ceil(65536) + 1;
    wasm(&format!(
        r#"(module
  (memory (export "memory") {pages})
  (data (i32.const 0) "{data}")
  (func (export "halp_alloc") (param i32) (result i32) (i32.const {data_end}))
  (func (export "halp_transform") (param i32 i32) (result i64) (i64.const {len})))"#,
        data = wat_bytes(output),
        len = output.len(),
    ))
}

/// A module whose `halp_transform` body is `body` (result i64), with a
/// one-page memory and a fixed allocator.
pub fn transform_module(extra: &str, body: &str) -> Vec<u8> {
    wasm(&format!(
        r#"(module
  {extra}
  (memory (export "memory") 1)
  (func (export "halp_alloc") (param i32) (result i32) (i32.const 1024))
  (func (export "halp_transform") (param $ptr i32) (param $len i32) (result i64)
    {body}))"#
    ))
}
