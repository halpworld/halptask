//! Random ids, token hashing and canonical argument hashes.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Prefix of client tokens (makes them recognisable to secret scanners).
pub const TOKEN_PREFIX: &str = "halp_mcp_";

/// 256 random bits as 64 lowercase hex characters.
pub fn random_hex() -> Result<String, String> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| format!("no system randomness: {e}"))?;
    Ok(hex::encode(b))
}

/// A new client token: `halp_mcp_` + 256 random bits in hex.
pub fn new_token() -> Result<String, String> {
    Ok(format!("{TOKEN_PREFIX}{}", random_hex()?))
}

/// SHA-256 of the token, hex. Only this is stored.
pub fn token_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Compares two byte strings in time independent of where they differ.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut d = 0u8;
    for (x, y) in a.iter().zip(b) {
        d |= x ^ y;
    }
    d == 0
}

/// JSON with object keys sorted at every level and no whitespace.
pub fn canonical_json(v: &Value) -> String {
    let mut out = String::new();
    write_canonical(v, &mut out);
    out
}

fn write_canonical(v: &Value, out: &mut String) {
    match v {
        Value::Object(m) => {
            let mut keys: Vec<&String> = m.keys().collect();
            keys.sort();
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&Value::String((*k).clone()).to_string());
                out.push(':');
                if let Some(x) = m.get(*k) {
                    write_canonical(x, out);
                }
            }
            out.push('}');
        }
        Value::Array(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(x, out);
            }
            out.push(']');
        }
        other => out.push_str(&other.to_string()),
    }
}

/// SHA-256 (hex) of the canonical JSON of `[command, args]`. A `null` args
/// value hashes like `{}`, as the dispatcher treats them the same.
pub fn args_hash(command: &str, args: &Value) -> String {
    let args = if args.is_null() {
        Value::Object(Default::default())
    } else {
        args.clone()
    };
    let doc = Value::Array(vec![Value::String(command.to_string()), args]);
    hex::encode(Sha256::digest(canonical_json(&doc).as_bytes()))
}

/// Largest argument document kept verbatim in an audit record or consent
/// payload; larger ones are replaced by their size and hash.
pub const MAX_AUDIT_ARGS_BYTES: usize = 2048;

/// `args` for logs: kept when small, else `{"truncated": true, "bytes": n,
/// "sha256": ...}`. Long strings inside are cut to 200 characters.
pub fn audit_args(args: &Value) -> Value {
    let text = canonical_json(args);
    if text.len() > MAX_AUDIT_ARGS_BYTES {
        return serde_json::json!({
            "truncated": true,
            "bytes": text.len(),
            "sha256": hex::encode(Sha256::digest(text.as_bytes())),
        });
    }
    cut_strings(args)
}

fn cut_strings(v: &Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > 200 => {
            let mut t: String = s.chars().take(200).collect();
            t.push('…');
            Value::String(t)
        }
        Value::Array(a) => Value::Array(a.iter().map(cut_strings).collect()),
        Value::Object(m) => {
            Value::Object(m.iter().map(|(k, x)| (k.clone(), cut_strings(x))).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_json_sorts_keys_recursively() {
        let a = json!({"b": 1, "a": {"y": [1, {"d": 2, "c": 3}], "x": "s"}});
        assert_eq!(
            canonical_json(&a),
            r#"{"a":{"x":"s","y":[1,{"c":3,"d":2}]},"b":1}"#
        );
    }

    #[test]
    fn args_hash_ignores_key_order_and_binds_the_command() {
        let h1 = args_hash("tasks.delete", &json!({"id": "x", "a": 1}));
        let h2 = args_hash("tasks.delete", &json!({"a": 1, "id": "x"}));
        assert_eq!(h1, h2);
        assert_ne!(h1, args_hash("tasks.get", &json!({"id": "x", "a": 1})));
        assert_ne!(h1, args_hash("tasks.delete", &json!({"id": "y", "a": 1})));
        assert_eq!(args_hash("c", &Value::Null), args_hash("c", &json!({})));
        assert_eq!(h1.len(), 64);
    }

    #[test]
    fn tokens_are_256_bit_and_hashes_compare_constant_time() {
        let t = new_token().expect("rng");
        assert!(t.starts_with(TOKEN_PREFIX));
        assert_eq!(t.len(), TOKEN_PREFIX.len() + 64);
        assert_ne!(t, new_token().expect("rng"));
        let h = token_hash(&t);
        assert!(ct_eq(h.as_bytes(), token_hash(&t).as_bytes()));
        assert!(!ct_eq(h.as_bytes(), token_hash("other").as_bytes()));
        assert!(!ct_eq(b"ab", b"abc"));
        assert_eq!(random_hex().expect("rng").len(), 64);
    }

    #[test]
    fn audit_args_truncates_large_documents() {
        let small = json!({"id": "x"});
        assert_eq!(audit_args(&small), small);
        let big = json!({"notes": "n".repeat(5000)});
        let a = audit_args(&big);
        assert_eq!(a["truncated"], true);
        assert!(a["sha256"].as_str().is_some_and(|s| s.len() == 64));
        let mid = json!({"t": "t".repeat(500)});
        let a = audit_args(&mid);
        assert_eq!(a["t"].as_str().map(|s| s.chars().count()), Some(201));
    }
}
