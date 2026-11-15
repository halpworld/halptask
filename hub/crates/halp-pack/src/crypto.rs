//! Ed25519 signing and verification via `ring` (§4.1).
//!
//! The signed message is `b"halp-pack-sig/1\0" || manifest_bytes`: the
//! manifest's exact bytes, no canonicalization. Files are covered through
//! the manifest's SHA-256 index.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use ring::rand::SystemRandom;
use ring::signature::{ED25519, Ed25519KeyPair, KeyPair, UnparsedPublicKey};
use sha2::{Digest, Sha256};

use crate::error::{PackError, Result};

/// The domain-separation prefix of the signed message.
pub const SIG_CONTEXT: &[u8] = b"halp-pack-sig/1\0";
/// The only signature algorithm.
pub const ALG: &str = "ed25519";

/// The message a signature covers.
pub fn signed_message(manifest: &[u8]) -> Vec<u8> {
    let mut m = Vec::with_capacity(SIG_CONTEXT.len() + manifest.len());
    m.extend_from_slice(SIG_CONTEXT);
    m.extend_from_slice(manifest);
    m
}

/// A key's id: the first 16 hex characters of SHA-256 of the raw 32-byte
/// public key.
pub fn key_id(pubkey: &[u8; 32]) -> String {
    let mut h = hex::encode(Sha256::digest(pubkey));
    h.truncate(16);
    h
}

/// The full fingerprint shown to the user: SHA-256 of the public key as 4
/// groups of 16 hex characters, separated by spaces.
pub fn fingerprint(pubkey: &[u8; 32]) -> String {
    let h = hex::encode(Sha256::digest(pubkey));
    h.as_bytes()
        .chunks(16)
        .map(|c| String::from_utf8_lossy(c).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Decodes a base64 public key; it must be exactly 32 bytes.
pub fn decode_pubkey(b64: &str) -> Result<[u8; 32]> {
    let raw = STANDARD
        .decode(b64.trim())
        .map_err(|_| PackError::invalid("public key is not base64"))?;
    raw.try_into()
        .map_err(|_| PackError::invalid("public key must be 32 bytes (Ed25519)"))
}

pub fn encode_pubkey(pubkey: &[u8; 32]) -> String {
    STANDARD.encode(pubkey)
}

/// A new Ed25519 key pair as PKCS#8 v2 DER bytes (it embeds the public key).
pub fn generate_pkcs8() -> Result<Vec<u8>> {
    Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map(|d| d.as_ref().to_vec())
        .map_err(|_| PackError::invalid("could not generate a key"))
}

/// A signing key loaded from PKCS#8 bytes.
pub struct SigningKey {
    pair: Ed25519KeyPair,
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey")
            .field("key_id", &self.key_id())
            .finish_non_exhaustive()
    }
}

impl SigningKey {
    pub fn from_pkcs8(der: &[u8]) -> Result<Self> {
        Ed25519KeyPair::from_pkcs8(der)
            .map(|pair| Self { pair })
            .map_err(|_| PackError::invalid("not an Ed25519 PKCS#8 key"))
    }

    pub fn public_key(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(self.pair.public_key().as_ref());
        out
    }

    pub fn key_id(&self) -> String {
        key_id(&self.public_key())
    }

    /// Signs manifest bytes (the context prefix is added here).
    pub fn sign_manifest(&self, manifest: &[u8]) -> Vec<u8> {
        self.pair.sign(&signed_message(manifest)).as_ref().to_vec()
    }
}

/// Checks a signature over manifest bytes.
pub fn verify_manifest(pubkey: &[u8; 32], manifest: &[u8], sig: &[u8]) -> bool {
    UnparsedPublicKey::new(&ED25519, pubkey)
        .verify(&signed_message(manifest), sig)
        .is_ok()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let k = SigningKey::from_pkcs8(&generate_pkcs8().unwrap()).unwrap();
        let pk = k.public_key();
        let sig = k.sign_manifest(b"{}");
        assert!(verify_manifest(&pk, b"{}", &sig));
        assert!(!verify_manifest(&pk, b"{ }", &sig));
        // The context prefix is part of the message.
        let raw = k.pair.sign(b"{}");
        assert!(!verify_manifest(&pk, b"{}", raw.as_ref()));
        let id = key_id(&pk);
        assert_eq!(id.len(), 16);
        let fp = fingerprint(&pk);
        assert_eq!(fp.len(), 64 + 3);
        assert!(fp.replace(' ', "").starts_with(&id));
        assert_eq!(decode_pubkey(&encode_pubkey(&pk)).unwrap(), pk);
        assert!(decode_pubkey("AAAA").is_err());
        assert!(SigningKey::from_pkcs8(b"nope").is_err());
    }
}
