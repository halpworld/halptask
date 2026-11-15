//! Shared grid fixtures (`hub/docs/fixtures/grid-cases.json`): the Rust grid
//! is the authority and the TS port (`hub/app/src/lib/layout/grid.ts`) runs
//! the same file. `HALP_UPDATE_FIXTURES=1` rewrites the expectations from
//! this implementation.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;

use halp_domain::Widget;
use halp_domain::grid;
use serde_json::{Value, json};

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../docs/fixtures/grid-cases.json")
}

fn num(args: &Value, k: &str) -> Option<u32> {
    args.get(k)?.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn widget(v: &Value) -> Widget {
    Widget {
        id: v["id"].as_str().unwrap().into(),
        kind: "k".into(),
        title: String::new(),
        x: num(v, "x").unwrap(),
        y: num(v, "y").unwrap(),
        w: num(v, "w").unwrap(),
        h: num(v, "h").unwrap(),
        config: Default::default(),
    }
}

fn out(ws: &[Widget]) -> Value {
    Value::Array(
        ws.iter()
            .map(|w| json!({"id": w.id, "x": w.x, "y": w.y, "w": w.w, "h": w.h}))
            .collect(),
    )
}

const INVALID: &str = "invalid_args";

fn run(case: &Value) -> Value {
    let cols = num(case, "columns").unwrap();
    let ws: Vec<Widget> = case["widgets"]
        .as_array()
        .unwrap()
        .iter()
        .map(widget)
        .collect();
    let a = &case["args"];
    let err = || json!({"error": INVALID});
    match case["op"].as_str().unwrap() {
        "move" => {
            let (Some(x), Some(y)) = (num(a, "x"), num(a, "y")) else {
                return err();
            };
            if grid::check_position(x, y, cols).is_err() {
                return err();
            }
            out(&grid::move_widget(
                &ws,
                a["id"].as_str().unwrap(),
                x,
                y,
                cols,
            ))
        }
        "resize" => {
            let (Some(w), Some(h)) = (num(a, "w"), num(a, "h")) else {
                return err();
            };
            if grid::check_size(w, h, cols).is_err() {
                return err();
            }
            out(&grid::resize_widget(
                &ws,
                a["id"].as_str().unwrap(),
                w,
                h,
                cols,
            ))
        }
        "compact" => out(&grid::compact(&ws, None)),
        "find_space" => {
            let (Some(w), Some(h)) = (num(a, "w"), num(a, "h")) else {
                return err();
            };
            if grid::check_size(w, h, cols).is_err() {
                return err();
            }
            let (x, y) = grid::find_space(&ws, w, h, cols);
            json!({"x": x, "y": y})
        }
        other => panic!("unknown op {other}"),
    }
}

#[test]
fn grid_cases_match() {
    let raw = std::fs::read_to_string(path()).unwrap();
    let mut cases: Vec<Value> = serde_json::from_str(&raw).unwrap();
    assert!(cases.len() >= 10);
    let update = std::env::var("HALP_UPDATE_FIXTURES").is_ok_and(|v| v == "1");
    for c in &mut cases {
        let got = run(c);
        if update {
            c["expect"] = got;
        } else {
            assert_eq!(got, c["expect"], "case {}", c["name"]);
        }
    }
    if update {
        let text = serde_json::to_string_pretty(&cases).unwrap() + "\n";
        std::fs::write(path(), text).unwrap();
    }
}
