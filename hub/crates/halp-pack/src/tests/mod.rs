//! Acceptance tests for P5-A (§8): verification and tampering, the merge
//! table, chains, and trust rules. Unit tests (not `tests/`) so the
//! test-only first-party key is compiled in.

#![allow(clippy::unwrap_used)]

mod resolve_tests;
mod verify_tests;

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::bundle::{build_bundle, file_index};
use crate::crypto::{SigningKey, generate_pkcs8};
use crate::resolve::{BuiltinBase, EffectivePack};

pub(crate) fn developer_base() -> BuiltinBase {
    let pack: EffectivePack =
        serde_json::from_str(include_str!("../../../../packs/developer/pack.json")).unwrap();
    BuiltinBase {
        package_id: "halp.developer".into(),
        version: semver::Version::new(0, 1, 0),
        pack,
    }
}

/// A minimal child of `halp.developer`.
pub(crate) fn child_manifest() -> Value {
    json!({
        "format": 1,
        "id": "acme.sre",
        "version": "1.0.0",
        "title": "Acme SRE",
        "description": "On-call tooling",
        "license": "MIT",
        "publisher": {"name": "Acme"},
        "min_hub": "0.x",
        "extends": {"id": "halp.developer", "version": "^0.1"},
        "role": {"id": "acme-sre", "title": "SRE", "accent": "#112233"},
        "task_types": [{
            "id": "acme.incident",
            "title": "Incident",
            "schema": {
                "type": "object",
                "properties": {"sev": {"enum": ["1", "2"]}, "service": {"type": "string"}},
                "additionalProperties": false
            }
        }]
    })
}

pub(crate) fn key() -> SigningKey {
    SigningKey::from_pkcs8(&generate_pkcs8().unwrap()).unwrap()
}

pub(crate) fn first_party_key() -> SigningKey {
    SigningKey::from_pkcs8(&hex::decode(crate::verify::tests::FIRST_PARTY_PK8_HEX).unwrap())
        .unwrap()
}

/// Fills `files` in the manifest from `files` and builds the bundle.
pub(crate) fn bundle(
    mut manifest: Value,
    files: &BTreeMap<String, Vec<u8>>,
    key: Option<&SigningKey>,
) -> Vec<u8> {
    manifest["files"] = serde_json::to_value(file_index(files)).unwrap();
    build_bundle(
        &serde_json::to_string_pretty(&manifest).unwrap(),
        files,
        key,
    )
    .unwrap()
}

pub(crate) fn files(list: &[(&str, &[u8])]) -> BTreeMap<String, Vec<u8>> {
    list.iter()
        .map(|(p, d)| (p.to_string(), d.to_vec()))
        .collect()
}

#[test]
#[ignore]
fn print_key() {
    let k = generate_pkcs8().unwrap();
    let s = SigningKey::from_pkcs8(&k).unwrap();
    println!("PK8 {}", hex::encode(&k));
    println!("PUB {:?}", s.public_key());
}
