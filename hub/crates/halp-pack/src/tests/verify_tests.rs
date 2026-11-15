use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde_json::{Value, json};

use super::*;
use crate::error::PackErrorKind;
use crate::trust::PackTrust;
use crate::verify::{CommunityKey, Revocations, SignerKind, check_pin, verify, verify_with};

fn community(k: &SigningKey, revoked: bool) -> CommunityKey {
    CommunityKey {
        key_id: k.key_id(),
        pubkey: k.public_key(),
        revoked,
    }
}

fn sample_files() -> BTreeMap<String, Vec<u8>> {
    files(&[
        ("prompts/triage.md", b"# Triage\nSteps."),
        ("assets/logo.png", &[0u8, 1, 2, 3]),
    ])
}

fn sample_manifest() -> Value {
    let mut m = child_manifest();
    m["prompts"] = json!({"add": ["prompts/triage.md"]});
    m
}

/// Rewrites the bundle JSON with `f`.
fn edit(bundle: &[u8], f: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v: Value = serde_json::from_slice(bundle).unwrap();
    f(&mut v);
    serde_json::to_vec(&v).unwrap()
}

fn err_kind(r: crate::Result<crate::VerifiedBundle>) -> PackErrorKind {
    match r {
        Ok(v) => panic!("verified with trust {:?}", v.trust),
        Err(e) => e.kind,
    }
}

#[test]
fn round_trip_trust_levels() {
    let k = key();
    let b = bundle(sample_manifest(), &sample_files(), Some(&k));
    // Unknown key: valid signature, trust unsigned, signer shown for TOFU.
    let v = verify(&b, &[]).unwrap();
    assert_eq!(v.trust, PackTrust::Unsigned);
    let s = v.signer.clone().unwrap();
    assert_eq!(s.kind, SignerKind::Unknown);
    assert_eq!(s.key_id, k.key_id());
    assert_eq!(v.pin_key_id(), None);
    assert_eq!(v.files, sample_files());
    assert_eq!(v.manifest.id, "acme.sre");
    // Trusted community key.
    let v = verify(&b, &[community(&k, false)]).unwrap();
    assert_eq!(v.trust, PackTrust::Community);
    assert_eq!(v.pin_key_id(), Some(k.key_id().as_str()));
    // First-party (test-only compiled-in key).
    let fp = first_party_key();
    let v = verify(&bundle(sample_manifest(), &sample_files(), Some(&fp)), &[]).unwrap();
    assert_eq!(v.trust, PackTrust::FirstParty);
    // No signature.
    let v = verify(&bundle(sample_manifest(), &sample_files(), None), &[]).unwrap();
    assert_eq!(v.trust, PackTrust::Unsigned);
    assert!(v.signer.is_none());
}

#[test]
fn changed_file_byte_rejected() {
    let k = key();
    let b = bundle(sample_manifest(), &sample_files(), Some(&k));
    let t = edit(&b, |v| {
        v["files"]["assets/logo.png"] = json!(STANDARD.encode([0u8, 1, 2, 4]))
    });
    assert_eq!(
        err_kind(verify(&t, &[community(&k, false)])),
        PackErrorKind::Invalid
    );
}

#[test]
fn changed_manifest_byte_is_a_bad_signature_not_unsigned() {
    let k = key();
    let b = bundle(sample_manifest(), &sample_files(), Some(&k));
    let t = edit(&b, |v| {
        let m = v["manifest"]
            .as_str()
            .unwrap()
            .replace("On-call tooling", "On-call toolinG");
        v["manifest"] = json!(m);
    });
    // Even with the key unknown, a bad signature is an error.
    for keys in [vec![], vec![community(&k, false)]] {
        let e = verify(&t, &keys).unwrap_err();
        assert_eq!(e.kind, PackErrorKind::Invalid);
        assert!(e.message.contains("signature"), "{e}");
    }
}

#[test]
fn bad_signatures_rejected() {
    let k = key();
    let other = key();
    let b = bundle(sample_manifest(), &sample_files(), Some(&k));
    type Tamper<'a> = Box<dyn Fn(&mut Value) + 'a>;
    let cases: Vec<Tamper<'_>> = vec![
        // Garbage signature bytes.
        Box::new(|v| v["signature"]["sig"] = json!(STANDARD.encode([7u8; 64]))),
        Box::new(|v| v["signature"]["sig"] = json!("not base64!")),
        // Someone else's public key under the original key id.
        Box::new(|v| {
            v["signature"]["pubkey"] = json!(crate::crypto::encode_pubkey(&other.public_key()))
        }),
        // Someone else's key with a matching id: the signature fails.
        Box::new(|v| {
            v["signature"]["pubkey"] = json!(crate::crypto::encode_pubkey(&other.public_key()));
            v["signature"]["key_id"] = json!(other.key_id());
        }),
        Box::new(|v| v["signature"]["alg"] = json!("rsa")),
        Box::new(|v| v["signature"]["extra"] = json!(1)),
    ];
    for (i, c) in cases.iter().enumerate() {
        let t = edit(&b, c);
        assert_eq!(
            err_kind(verify(&t, &[community(&k, false)])),
            PackErrorKind::Invalid,
            "case {i}"
        );
        assert!(verify(&t, &[]).is_err(), "case {i} downgraded to unsigned");
    }
}

#[test]
fn extra_and_missing_files_rejected() {
    let b = bundle(sample_manifest(), &sample_files(), None);
    let extra = edit(&b, |v| {
        v["files"]["assets/extra.png"] = json!(STANDARD.encode(b"x"))
    });
    assert!(
        verify(&extra, &[])
            .unwrap_err()
            .message
            .contains("not listed")
    );
    let missing = edit(&b, |v| {
        v["files"]
            .as_object_mut()
            .unwrap()
            .remove("assets/logo.png");
    });
    assert!(
        verify(&missing, &[])
            .unwrap_err()
            .message
            .contains("missing")
    );
}

#[test]
fn bad_paths_rejected() {
    let b = bundle(sample_manifest(), &sample_files(), None);
    for p in [
        "../evil",
        "a/../../evil",
        "/etc/passwd",
        "Up.md",
        "a//b",
        "halp-pack.json",
    ] {
        let t = edit(&b, |v| v["files"][p] = json!(STANDARD.encode(b"x")));
        assert!(verify(&t, &[]).is_err(), "{p}");
    }
    // In the manifest's index too.
    let mut m = sample_manifest();
    m["files"] = json!({"../evil": {"sha256": "0".repeat(64), "size": 1}});
    let raw =
        json!({"format": "halp-pack/1", "manifest": m.to_string(), "signature": null, "files": {}});
    assert!(verify(raw.to_string().as_bytes(), &[]).is_err());
}

#[test]
fn duplicate_keys_rejected() {
    let b = bundle(sample_manifest(), &files(&[("a.txt", b"1")]), None);
    let s = String::from_utf8(b).unwrap();
    let enc = STANDARD.encode(b"1");
    let dup = s.replacen(
        &format!("\"a.txt\":\"{enc}\""),
        &format!("\"a.txt\":\"{enc}\",\"a.txt\":\"{enc}\""),
        1,
    );
    assert_ne!(dup, s);
    assert!(
        verify(dup.as_bytes(), &[])
            .unwrap_err()
            .message
            .contains("duplicate")
    );
}

#[test]
fn size_limits() {
    // A file over 2 MiB.
    let big = vec![b'a'; 2 * 1024 * 1024 + 1];
    let mut m = sample_manifest();
    let fs = files(&[("prompts/triage.md", b"x"), ("big.bin", &big)]);
    m["files"] = serde_json::to_value(file_index(&fs)).unwrap();
    let raw = json!({
        "format": "halp-pack/1",
        "manifest": m.to_string(),
        "signature": null,
        "files": fs.iter().map(|(p, d)| (p.clone(), json!(STANDARD.encode(d)))).collect::<serde_json::Map<_, _>>(),
    });
    assert!(
        verify(raw.to_string().as_bytes(), &[])
            .unwrap_err()
            .message
            .contains("exceeds")
    );
    // build_bundle refuses it too.
    assert!(crate::build_bundle(&m.to_string(), &fs, None).is_err());
    // Over 8 MiB in total.
    let huge = vec![b' '; 8 * 1024 * 1024 + 1];
    assert!(verify(&huge, &[]).unwrap_err().message.contains("exceeds"));
    // More than 256 files.
    let many: BTreeMap<String, Vec<u8>> = (0..257)
        .map(|i| (format!("f/{i}.txt"), vec![b'x']))
        .collect();
    let mut m = sample_manifest();
    m["prompts"] = json!({});
    m["files"] = serde_json::to_value(file_index(&many)).unwrap();
    assert!(crate::build_bundle(&m.to_string(), &many, None).is_err());
    // Manifest over 64 KiB.
    let mut m = sample_manifest();
    m["description"] = json!("x".repeat(70 * 1024));
    let raw =
        json!({"format": "halp-pack/1", "manifest": m.to_string(), "signature": null, "files": {}});
    assert!(verify(raw.to_string().as_bytes(), &[]).is_err());
}

#[test]
fn revoked_keys_and_packs_refused() {
    let k = key();
    let b = bundle(sample_manifest(), &sample_files(), Some(&k));
    // Revoked in pack_keys.
    assert_eq!(
        err_kind(verify(&b, &[community(&k, true)])),
        PackErrorKind::Denied
    );
    // In the revocation list (compiled-in in production; extra here).
    let rev = Revocations {
        keys: vec![k.key_id()],
        pack_hashes: vec![],
    };
    assert_eq!(
        err_kind(verify_with(&b, &[community(&k, false)], &rev)),
        PackErrorKind::Denied
    );
    assert_eq!(err_kind(verify_with(&b, &[], &rev)), PackErrorKind::Denied);
    let v = verify(&b, &[]).unwrap();
    let rev = Revocations {
        keys: vec![],
        pack_hashes: vec![v.manifest_sha256.clone()],
    };
    assert_eq!(err_kind(verify_with(&b, &[], &rev)), PackErrorKind::Denied);
    assert!(
        crate::verify::REVOKED_KEYS.is_empty() && crate::verify::REVOKED_PACK_HASHES.is_empty()
    );
}

#[test]
fn pinned_key_mismatch_on_update() {
    let k = key();
    let other = key();
    let keys = [community(&k, false), community(&other, false)];
    let mut m = sample_manifest();
    m["version"] = json!("1.1.0");
    let by_k = verify(&bundle(m.clone(), &sample_files(), Some(&k)), &keys).unwrap();
    let by_other = verify(&bundle(m.clone(), &sample_files(), Some(&other)), &keys).unwrap();
    let unsigned = verify(&bundle(m.clone(), &sample_files(), None), &keys).unwrap();
    let unknown = verify(&bundle(m, &sample_files(), Some(&k)), &[]).unwrap();
    let pin = k.key_id();
    check_pin(Some(&pin), &by_k).unwrap();
    let e = check_pin(Some(&pin), &by_other).unwrap_err();
    assert_eq!(e.kind, PackErrorKind::Denied);
    assert!(e.message.contains("different publisher"), "{e}");
    assert_eq!(
        check_pin(Some(&pin), &unsigned).unwrap_err().kind,
        PackErrorKind::Denied
    );
    assert_eq!(
        check_pin(Some(&pin), &unknown).unwrap_err().kind,
        PackErrorKind::Denied
    );
    // A pack first installed unsigned has no pin.
    check_pin(None, &by_other).unwrap();
    check_pin(None, &unsigned).unwrap();
}

/// A pin is the full 64-hex fingerprint, compared in full: a key whose
/// 64-bit key id collides with the pinned one (same first 16 hex) is not
/// the pinned key. Revocation lists match full fingerprints too.
#[test]
fn pins_and_revocations_use_the_full_fingerprint() {
    let k = key();
    let mut m = sample_manifest();
    m["version"] = json!("1.1.0");
    let b = bundle(m, &sample_files(), Some(&k));
    let by_k = verify(&b, &[community(&k, false)]).unwrap();
    let pin = by_k.pin_fingerprint().unwrap();
    assert_eq!(pin.len(), 64);
    assert!(pin.starts_with(&k.key_id()));
    check_pin(Some(&pin), &by_k).unwrap();
    // Same 64-bit key id prefix, different key.
    let mut forged = pin.clone();
    let last = if forged.ends_with('0') { "1" } else { "0" };
    forged.replace_range(63.., last);
    assert!(forged.starts_with(&k.key_id()));
    let e = check_pin(Some(&forged), &by_k).unwrap_err();
    assert_eq!(e.kind, PackErrorKind::Denied);
    // Revoked by fingerprint.
    let rev = Revocations {
        keys: vec![pin.clone()],
        pack_hashes: vec![],
    };
    assert_eq!(err_kind(verify_with(&b, &[], &rev)), PackErrorKind::Denied);
    // A fingerprint sharing only the key id prefix revokes nothing.
    let rev = Revocations {
        keys: vec![forged],
        pack_hashes: vec![],
    };
    verify_with(&b, &[], &rev).unwrap();
}

#[test]
fn unsigned_packs_may_not_carry_privileged_annotations_or_code() {
    for (key, schema_extra) in [
        ("x-halp-immutable-when", json!({"sev": ["1"]})),
        ("x-halp-ext-writable", json!(["service"])),
    ] {
        let mut m = sample_manifest();
        m["task_types"][0]["schema"][key] = schema_extra;
        let k = key_for_test();
        // Unsigned (no signature, or an untrusted key): denied.
        assert_eq!(
            err_kind(verify(&bundle(m.clone(), &sample_files(), None), &[])),
            PackErrorKind::Denied
        );
        assert_eq!(
            err_kind(verify(&bundle(m.clone(), &sample_files(), Some(&k)), &[])),
            PackErrorKind::Denied
        );
        // Community: allowed.
        let v = verify(
            &bundle(m.clone(), &sample_files(), Some(&k)),
            &[community(&k, false)],
        )
        .unwrap();
        assert_eq!(v.trust, PackTrust::Community);
        // Dev: loads, with a warning (the core ignores the annotation).
        let mut dm: crate::Manifest = serde_json::from_value(m.clone()).unwrap();
        dm.id = "dev.acme.sre".into();
        dm.files = file_index(&sample_files());
        let w = dm.validate(PackTrust::Dev).unwrap();
        assert!(w.iter().any(|w| w.contains(key)), "{w:?}");
    }
    // Tier B module in an unsigned pack.
    let mut m = sample_manifest();
    let fs = files(&[("prompts/triage.md", b"x"), ("wasm/burn.wasm", b"\0asm")]);
    m["modules"] = json!([{"name": "burn", "wasm": "wasm/burn.wasm", "inputs": {"tasks": {"command": "tasks.list"}}, "output_schema": {"type": "object"}}]);
    assert_eq!(
        err_kind(verify(&bundle(m.clone(), &fs, None), &[])),
        PackErrorKind::Denied
    );
    let k = key();
    verify(&bundle(m, &fs, Some(&k)), &[community(&k, false)]).unwrap();
}

fn key_for_test() -> SigningKey {
    key()
}

#[test]
fn reserved_namespace_needs_first_party() {
    let mut m = sample_manifest();
    m["id"] = json!("halp.sre");
    m["task_types"][0]["id"] = json!("halp.incident");
    let k = key();
    assert_eq!(
        err_kind(verify(
            &bundle(m.clone(), &sample_files(), Some(&k)),
            &[community(&k, false)]
        )),
        PackErrorKind::Denied
    );
    let fp = first_party_key();
    assert_eq!(
        verify(&bundle(m, &sample_files(), Some(&fp)), &[])
            .unwrap()
            .trust,
        PackTrust::FirstParty
    );
}

#[test]
fn manifest_is_strict() {
    let mut m = sample_manifest();
    m["surprise"] = json!(true);
    assert!(verify(&bundle(m, &sample_files(), None), &[]).is_err());
    let mut m = sample_manifest();
    m["role"]["extra"] = json!(1);
    assert!(verify(&bundle(m, &sample_files(), None), &[]).is_err());
    let mut m = sample_manifest();
    m["title"] = json!("a\u{202E}b");
    assert!(verify(&bundle(m, &sample_files(), None), &[]).is_err());
    let mut m = sample_manifest();
    m["role"]["id"] = json!("developer");
    assert_eq!(
        err_kind(verify(&bundle(m, &sample_files(), None), &[])),
        PackErrorKind::Conflict
    );
    let mut m = sample_manifest();
    m["task_types"][0]["id"] = json!("other.incident");
    assert!(verify(&bundle(m, &sample_files(), None), &[]).is_err());
    let mut m = sample_manifest();
    m["task_types"][0]["schema"]["x-halp-data-class"] = json!("other.secret");
    assert!(verify(&bundle(m.clone(), &sample_files(), None), &[]).is_err());
    m["task_types"][0]["schema"]["x-halp-data-class"] = json!("acme.secret");
    verify(&bundle(m.clone(), &sample_files(), None), &[]).unwrap();
    m["task_types"][0]["schema"]["x-halp-data-class"] = json!("notes.private");
    verify(&bundle(m, &sample_files(), None), &[]).unwrap();
    let mut m = sample_manifest();
    m["version"] = json!("one");
    assert!(verify(&bundle(m, &sample_files(), None), &[]).is_err());
    // A prompt not in files.
    let mut m = sample_manifest();
    m["prompts"] = json!({"add": ["prompts/missing.md"]});
    assert!(verify(&bundle(m, &sample_files(), None), &[]).is_err());
}
