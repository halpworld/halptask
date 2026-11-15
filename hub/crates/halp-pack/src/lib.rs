//! HalpTask Hub extension packs (P5-A): pure functions, no store, no core.
//!
//! * [`bundle`]: the `*.halppack` format, its limits and the file index;
//! * [`verify`]: verification in the order of §4.2, the trust it earns,
//!   revocation and key pinning;
//! * [`crypto`]: Ed25519 signing via `ring`, key ids and fingerprints;
//! * [`manifest`]: `halp-pack.json`, strict (`deny_unknown_fields`);
//! * [`resolve`]: `extends` chains into an effective pack in the shape of
//!   halp-core's `Pack` ([`resolve::EffectivePack`]);
//! * [`overrides`]: additive type overrides;
//! * [`migrate`]: declarative item migrations;
//! * [`trust`]: trust levels and the privileged-annotation rule.
//!
//! The core wiring (store, load path, commands) is P5-B's and maps these
//! types onto its own.

pub mod bundle;
pub mod crypto;
pub mod error;
pub mod ids;
pub mod manifest;
pub mod migrate;
pub mod overrides;
pub mod resolve;
pub mod trust;
pub mod verify;

#[cfg(test)]
mod tests;

pub use bundle::{
    BUNDLE_FORMAT, BundleSignature, ParsedBundle, build_bundle, file_index, parse_bundle,
};
pub use crypto::{SigningKey, fingerprint, generate_pkcs8, key_id};
pub use error::{PackError, PackErrorKind, Result};
pub use manifest::{FileEntry, FileIndex, Manifest, TypeOverride};
pub use migrate::{Migrated, Migration, MigrationOp, apply_ops, migrate_item};
pub use overrides::apply_override;
pub use resolve::{
    BuiltinBase, Chain, ChainLink, EffectivePack, Parent, PromptRef, Resolved, collect_chain,
    resolve,
};
pub use trust::{PRIVILEGED_ANNOTATIONS, PackTrust, check_trust};
pub use verify::{
    CommunityKey, FIRST_PARTY_KEYS, Revocations, Signer, SignerKind, VerifiedBundle, check_pin,
    verify, verify_with,
};
