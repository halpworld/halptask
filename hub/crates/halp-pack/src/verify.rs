//! Bundle verification in the order of §4.2: (1) sizes, paths,
//! duplicates; (2) file hashes against the manifest index; (3) the
//! signature and the trust it earns; (4) revocation; then the manifest is
//! validated for that trust. Key pinning (step 5)
//! needs the installed pack's pin, so it is [`check_pin`].

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};

use crate::bundle::{self, ParsedBundle};
use crate::crypto;
use crate::error::{PackError, Result};
use crate::ids::sha256_hex;
use crate::manifest::{FileIndex, Manifest, unique_map};
use crate::trust::PackTrust;

/// Compiled-in HalpTask signing keys, `(label, raw public key)`. They
/// cannot be removed in the app. Empty until HalpTask publishes a key;
/// tests use a test-only key.
#[cfg(not(test))]
pub const FIRST_PARTY_KEYS: &[(&str, [u8; 32])] = &[];
#[cfg(test)]
pub const FIRST_PARTY_KEYS: &[(&str, [u8; 32])] = &[("test first-party", tests::FIRST_PARTY_PUB)];

/// Keys refused outright, by full fingerprint (64 hex; updated with app
/// releases).
pub const REVOKED_KEYS: &[&str] = &[];
/// Manifest SHA-256 values refused outright (updated with app releases).
pub const REVOKED_PACK_HASHES: &[&str] = &[];

/// A key the user trusted (`pack_keys`, §3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommunityKey {
    pub key_id: String,
    pub pubkey: [u8; 32],
    /// `revoked_at` is set.
    pub revoked: bool,
}

/// Revocation lists. [`verify`] uses the compiled-in ones.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Revocations {
    pub keys: Vec<String>,
    pub pack_hashes: Vec<String>,
}

impl Revocations {
    pub fn compiled() -> Self {
        Self {
            keys: REVOKED_KEYS.iter().map(|s| s.to_string()).collect(),
            pack_hashes: REVOKED_PACK_HASHES.iter().map(|s| s.to_string()).collect(),
        }
    }
}

/// Who signed, when the signature checked out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerKind {
    FirstParty,
    Community,
    /// A valid signature by a key nobody trusted yet: the trust is
    /// `unsigned`, and the user may trust the key on first use.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signer {
    pub key_id: String,
    pub pubkey_b64: String,
    pub fingerprint: String,
    pub kind: SignerKind,
}

/// A bundle that passed verification.
#[derive(Debug, Clone, PartialEq)]
pub struct VerifiedBundle {
    pub manifest: Manifest,
    pub manifest_text: String,
    pub manifest_sha256: String,
    pub bundle_sha256: String,
    pub files: BTreeMap<String, Vec<u8>>,
    pub file_index: FileIndex,
    pub trust: PackTrust,
    /// Set whenever a signature is present (it is always valid here).
    pub signer: Option<Signer>,
    pub warnings: Vec<String>,
}

impl VerifiedBundle {
    /// The key id to pin at first install: the signer's, when it is
    /// trusted (first-party or community).
    pub fn pin_key_id(&self) -> Option<&str> {
        self.signer
            .as_ref()
            .filter(|s| s.kind != SignerKind::Unknown)
            .map(|s| s.key_id.as_str())
    }

    /// The full fingerprint (64 hex) to pin at first install, when the
    /// signer is trusted; [`check_pin`] compares it in full.
    pub fn pin_fingerprint(&self) -> Option<String> {
        self.signer
            .as_ref()
            .filter(|s| s.kind != SignerKind::Unknown)
            .map(|s| s.fingerprint.replace(' ', ""))
    }
}

/// Verifies a bundle against the compiled-in first-party keys and
/// revocation lists and the user's community keys.
pub fn verify(bytes: &[u8], community: &[CommunityKey]) -> Result<VerifiedBundle> {
    verify_with(bytes, community, &Revocations::compiled())
}

/// [`verify`] with extra revocations (the compiled-in ones always apply).
pub fn verify_with(
    bytes: &[u8],
    community: &[CommunityKey],
    extra: &Revocations,
) -> Result<VerifiedBundle> {
    // 1.
    let ParsedBundle {
        manifest_text,
        signature,
        files,
        sha256: bundle_sha256,
    } = bundle::parse_bundle(bytes)?;
    // 2. Only the index is read here; the manifest is validated below,
    // once the trust is known.
    #[derive(Deserialize)]
    struct OnlyFiles {
        #[serde(default, deserialize_with = "unique_map")]
        files: FileIndex,
    }
    let listed: OnlyFiles = serde_json::from_str(&manifest_text)
        .map_err(|e| PackError::invalid(format!("manifest is malformed: {e}")))?;
    bundle::check_files(&listed.files, &files)?;
    // 3.
    let signer = match &signature {
        None => None,
        Some(s) => Some(check_signature(s, manifest_text.as_bytes(), community)?),
    };
    let trust = match signer.as_ref().map(|s| s.kind) {
        Some(SignerKind::FirstParty) => PackTrust::FirstParty,
        Some(SignerKind::Community) => PackTrust::Community,
        Some(SignerKind::Unknown) | None => PackTrust::Unsigned,
    };
    // 4.
    let manifest_sha256 = sha256_hex(manifest_text.as_bytes());
    let compiled = Revocations::compiled();
    // Revocation lists name keys by full fingerprint (64 hex); a 16-hex
    // key id entry is still honoured.
    if let Some(s) = &signer
        && [&compiled, extra].iter().any(|r| {
            let fp = s.fingerprint.replace(' ', "");
            r.keys.iter().any(|k| *k == s.key_id || *k == fp)
        })
    {
        return Err(PackError::denied(format!(
            "the key {} that signed this pack has been revoked",
            s.key_id
        )));
    }
    if compiled.pack_hashes.contains(&manifest_sha256)
        || extra.pack_hashes.contains(&manifest_sha256)
    {
        return Err(PackError::denied("this pack has been revoked"));
    }
    let (manifest, warnings) = Manifest::parse(&manifest_text, trust)?;
    Ok(VerifiedBundle {
        manifest,
        manifest_text,
        manifest_sha256,
        bundle_sha256,
        files,
        file_index: listed.files,
        trust,
        signer,
        warnings,
    })
}

fn check_signature(
    s: &bundle::BundleSignature,
    manifest: &[u8],
    community: &[CommunityKey],
) -> Result<Signer> {
    if s.alg != crypto::ALG {
        return Err(PackError::invalid(format!(
            "signature algorithm {:?} is not supported",
            s.alg
        )));
    }
    let pubkey = crypto::decode_pubkey(&s.pubkey)?;
    let key_id = crypto::key_id(&pubkey);
    if s.key_id != key_id {
        return Err(PackError::invalid(
            "the signature's key_id does not match its public key",
        ));
    }
    let sig = STANDARD
        .decode(s.sig.as_bytes())
        .map_err(|_| PackError::invalid("the signature is not base64"))?;
    // A bad signature is an error, never a downgrade to unsigned.
    if !crypto::verify_manifest(&pubkey, manifest, &sig) {
        return Err(PackError::invalid(
            "the signature does not match the manifest (the pack was changed after signing)",
        ));
    }
    let kind = if FIRST_PARTY_KEYS.iter().any(|(_, k)| *k == pubkey) {
        SignerKind::FirstParty
    } else if let Some(c) = community.iter().find(|c| c.pubkey == pubkey) {
        if c.revoked {
            return Err(PackError::denied(format!(
                "the key {key_id} that signed this pack has been revoked"
            )));
        }
        SignerKind::Community
    } else {
        SignerKind::Unknown
    };
    Ok(Signer {
        key_id,
        pubkey_b64: crypto::encode_pubkey(&pubkey),
        fingerprint: crypto::fingerprint(&pubkey),
        kind,
    })
}

/// Whether `signer` is the pinned key. A pin is the key's full
/// fingerprint (64 hex, SHA-256 of the raw public key), compared in full;
/// a 16-hex pin is a legacy 64-bit key id (records written before v1.10),
/// still honoured for those rows.
fn pin_matches(pinned: &str, signer: &Signer) -> bool {
    if pinned.len() == 64 {
        signer.fingerprint.replace(' ', "") == pinned.to_ascii_lowercase()
    } else {
        signer.key_id == pinned
    }
}

/// Step 5, key pinning: an update of a pack whose first install pinned a
/// key (`pinned`: its 64-hex fingerprint, or a legacy key id) must be
/// signed by that key, and the key must still be trusted. A pack
/// installed unsigned (`None`) has no pin.
pub fn check_pin(pinned: Option<&str>, update: &VerifiedBundle) -> Result<()> {
    let Some(pinned) = pinned else {
        return Ok(());
    };
    match &update.signer {
        None => Err(PackError::denied(format!(
            "pack {} was installed signed; an unsigned update is refused",
            update.manifest.id
        ))),
        Some(s) if !pin_matches(pinned, s) => Err(PackError::denied(format!(
            "pack {} is signed by a different publisher (key {}, expected {pinned})",
            update.manifest.id, s.key_id
        ))),
        Some(s) if s.kind == SignerKind::Unknown => Err(PackError::denied(format!(
            "pack {}: its publisher's key {pinned} is no longer trusted",
            update.manifest.id
        ))),
        Some(_) => Ok(()),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    /// Test-only first-party key (PKCS#8 v2, hex). Never shipped: the
    /// constant set is empty outside `cfg(test)`.
    pub(crate) const FIRST_PARTY_PK8_HEX: &str = "3051020101300506032b657004220420836c86fae2241c22fb49d687fe01f2c9c7983f62959a8f369b2837abaf9dcf9d8121006a68638dab515f13c2fcb63e821c9d009799c52415703392b91d15f0807faa7e";
    pub(crate) const FIRST_PARTY_PUB: [u8; 32] = [
        106, 104, 99, 141, 171, 81, 95, 19, 194, 252, 182, 62, 130, 28, 157, 0, 151, 153, 197, 36,
        21, 112, 51, 146, 185, 29, 21, 240, 128, 127, 170, 126,
    ];
}
