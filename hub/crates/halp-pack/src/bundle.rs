//! The single-file bundle `*.halppack` (§3.2): JSON, no archive format.
//!
//! ```json
//! { "format": "halp-pack/1", "manifest": "<exact text of halp-pack.json>",
//!   "signature": {"alg":"ed25519","key_id":"<16 hex>","pubkey":"<base64>","sig":"<base64>"} | null,
//!   "files": {"<path>": "<base64>"} }
//! ```

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::crypto::{self, SigningKey};
use crate::error::{PackError, Result};
use crate::ids::{check_path, sha256_hex};
use crate::manifest::{
    FileEntry, FileIndex, MAX_FILE_BYTES, MAX_FILES, MAX_MANIFEST_BYTES, unique_map,
};

pub const BUNDLE_FORMAT: &str = "halp-pack/1";
/// Most bytes in a bundle.
pub const MAX_BUNDLE_BYTES: usize = 8 * 1024 * 1024;

/// A bundle's signature. `pubkey` (base64 of the raw 32-byte key) is an
/// addition to §3.2: it lets the Hub check a signature by a key it does
/// not know yet, so a bad signature is never mistaken for "unsigned" and
/// trust on first use (§4.3) can add the key. `key_id` must match it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundleSignature {
    pub alg: String,
    pub key_id: String,
    pub pubkey: String,
    pub sig: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBundle {
    format: String,
    manifest: String,
    signature: Option<BundleSignature>,
    #[serde(deserialize_with = "unique_map")]
    files: BTreeMap<String, String>,
}

/// A bundle after step 1 of verification: sizes, paths and duplicates
/// checked, files decoded. Nothing is trusted yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedBundle {
    pub manifest_text: String,
    pub signature: Option<BundleSignature>,
    pub files: BTreeMap<String, Vec<u8>>,
    /// SHA-256 of the whole bundle (what a preview is bound to).
    pub sha256: String,
}

/// Step 1 (§4.2): size, path and duplicate checks, and base64 decoding.
pub fn parse_bundle(bytes: &[u8]) -> Result<ParsedBundle> {
    if bytes.len() > MAX_BUNDLE_BYTES {
        return Err(PackError::invalid(format!(
            "bundle exceeds {MAX_BUNDLE_BYTES} bytes"
        )));
    }
    let raw: RawBundle = serde_json::from_slice(bytes)
        .map_err(|e| PackError::invalid(format!("bundle is malformed: {e}")))?;
    if raw.format != BUNDLE_FORMAT {
        return Err(PackError::invalid(format!(
            "bundle format {:?} is not supported (expected {BUNDLE_FORMAT})",
            raw.format
        )));
    }
    if raw.manifest.len() > MAX_MANIFEST_BYTES {
        return Err(PackError::invalid(format!(
            "manifest exceeds {MAX_MANIFEST_BYTES} bytes"
        )));
    }
    if raw.files.len() > MAX_FILES {
        return Err(PackError::invalid(format!(
            "bundle has more than {MAX_FILES} files"
        )));
    }
    let mut files = BTreeMap::new();
    for (path, b64) in raw.files {
        check_path(&path)?;
        // Cheap bound before decoding: base64 is 4 chars per 3 bytes.
        if b64.len() as u64 > MAX_FILE_BYTES.div_ceil(3) * 4 + 4 {
            return Err(PackError::invalid(format!(
                "file {path} exceeds {MAX_FILE_BYTES} bytes"
            )));
        }
        let data = STANDARD
            .decode(b64.as_bytes())
            .map_err(|_| PackError::invalid(format!("file {path} is not valid base64")))?;
        if data.len() as u64 > MAX_FILE_BYTES {
            return Err(PackError::invalid(format!(
                "file {path} exceeds {MAX_FILE_BYTES} bytes"
            )));
        }
        files.insert(path, data);
    }
    Ok(ParsedBundle {
        manifest_text: raw.manifest,
        signature: raw.signature,
        files,
        sha256: sha256_hex(bytes),
    })
}

/// The index of a set of files.
pub fn file_index(files: &BTreeMap<String, Vec<u8>>) -> FileIndex {
    files
        .iter()
        .map(|(p, d)| {
            (
                p.clone(),
                FileEntry {
                    sha256: sha256_hex(d),
                    size: d.len() as u64,
                },
            )
        })
        .collect()
}

/// Step 2 (§4.2): every file matches its index entry and the two sets
/// are equal.
pub fn check_files(index: &FileIndex, files: &BTreeMap<String, Vec<u8>>) -> Result<()> {
    if let Some(p) = files.keys().find(|p| !index.contains_key(*p)) {
        return Err(PackError::invalid(format!(
            "file {p} is in the bundle but not listed in the manifest"
        )));
    }
    for (p, entry) in index {
        let data = files.get(p).ok_or_else(|| {
            PackError::invalid(format!("file {p} is listed in the manifest but missing"))
        })?;
        if data.len() as u64 != entry.size || sha256_hex(data) != entry.sha256 {
            return Err(PackError::invalid(format!(
                "file {p} does not match its SHA-256 in the manifest"
            )));
        }
    }
    Ok(())
}

/// Builds a bundle from manifest text and files, signed with `key` when
/// given. The manifest's `files` index must already match the files
/// exactly (it is signed as written; nothing is rewritten).
pub fn build_bundle(
    manifest_text: &str,
    files: &BTreeMap<String, Vec<u8>>,
    key: Option<&SigningKey>,
) -> Result<Vec<u8>> {
    #[derive(Deserialize)]
    struct OnlyFiles {
        #[serde(default, deserialize_with = "unique_map")]
        files: FileIndex,
    }
    let listed: OnlyFiles = serde_json::from_str(manifest_text)
        .map_err(|e| PackError::invalid(format!("manifest is malformed: {e}")))?;
    let actual = file_index(files);
    if listed.files != actual {
        let want = serde_json::to_string_pretty(&actual).unwrap_or_default();
        return Err(PackError::invalid(format!(
            "the manifest's `files` does not match the pack's files; it should be:\n\"files\": {want}"
        )));
    }
    let signature = key.map(|k| {
        let pk = k.public_key();
        BundleSignature {
            alg: crypto::ALG.into(),
            key_id: crypto::key_id(&pk),
            pubkey: crypto::encode_pubkey(&pk),
            sig: STANDARD.encode(k.sign_manifest(manifest_text.as_bytes())),
        }
    });
    let raw = RawBundle {
        format: BUNDLE_FORMAT.into(),
        manifest: manifest_text.to_string(),
        signature,
        files: files
            .iter()
            .map(|(p, d)| (p.clone(), STANDARD.encode(d)))
            .collect(),
    };
    let out = serde_json::to_vec(&raw).map_err(|e| PackError::invalid(e.to_string()))?;
    // Same limits as on the way in.
    parse_bundle(&out)?;
    Ok(out)
}
