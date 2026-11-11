//! Identity and naming rules (§3.1) and bundle path rules (§3.2).

use std::sync::LazyLock;

use regex::Regex;
use sha2::{Digest, Sha256};

use crate::error::{PackError, Result};
use crate::trust::PackTrust;

/// Most characters in a pack id (so `module:<id>` fits `simple_id`).
pub const MAX_PACK_ID: usize = 64;
/// Namespaces only first-party (and built-in) packs may use.
pub const RESERVED_NAMESPACES: &[&str] = &["halp", "base", "dev", "em", "cso", "ext", "x"];
/// Role ids of the built-in packs (`hub/packs/*`). A pack must not take
/// one. Keep in sync with halp-core's `BUILTIN`; P5-B also checks role ids
/// against every loaded pack.
pub const BUILTIN_ROLE_IDS: &[&str] = &["developer", "eng-manager", "cso"];
/// Package ids of the built-in packs, in the order of [`BUILTIN_ROLE_IDS`].
pub const BUILTIN_PACKAGE_IDS: &[&str] = &["halp.developer", "halp.eng-manager", "halp.cso"];
/// The prefix a dev-folder pack's id gets (§3.8).
pub const DEV_PREFIX: &str = "dev.";

/// Most path segments in a bundle path.
pub const MAX_PATH_DEPTH: usize = 6;
/// Most bytes in a bundle path.
pub const MAX_PATH_BYTES: usize = 255;
/// The manifest's file name in a pack directory.
pub const MANIFEST_FILE: &str = "halp-pack.json";

static PACK_ID: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^[a-z][a-z0-9-]{0,31}(\.[a-z][a-z0-9-]{0,31}){1,3}$").ok());

/// Checks the shape of a pack id (not the namespace rules).
pub fn check_pack_id(id: &str) -> Result<()> {
    if id.len() <= MAX_PACK_ID && PACK_ID.as_ref().is_some_and(|r| r.is_match(id)) {
        Ok(())
    } else {
        Err(PackError::invalid(format!(
            "pack id {id:?} must look like `namespace.name` (2 to 4 lowercase segments of \
             letters, digits or '-', at most 64 characters)"
        )))
    }
}

/// The namespace of a pack: its id's first segment. For a dev pack
/// (`dev.<id>`) it is the first segment after `dev.`, the namespace the
/// pack's author chose; its types and data classes live there.
pub fn namespace(id: &str, trust: PackTrust) -> &str {
    let id = match trust {
        PackTrust::Dev => id.strip_prefix(DEV_PREFIX).unwrap_or(id),
        _ => id,
    };
    id.split('.').next().unwrap_or(id)
}

/// Checks a pack id against its trust: the shape, the reserved
/// namespaces (first-party and built-in only) and, for a dev pack, the
/// `dev.` prefix.
pub fn check_pack_identity(id: &str, trust: PackTrust) -> Result<()> {
    if trust == PackTrust::Dev {
        let Some(inner) = id.strip_prefix(DEV_PREFIX) else {
            return Err(PackError::invalid(format!(
                "dev pack id {id:?} must start with {DEV_PREFIX}"
            )));
        };
        check_pack_id(inner)?;
        if id.len() > MAX_PACK_ID {
            return Err(PackError::invalid(format!(
                "dev pack id {id:?} exceeds {MAX_PACK_ID} characters"
            )));
        }
    } else {
        check_pack_id(id)?;
    }
    let ns = namespace(id, trust);
    if RESERVED_NAMESPACES.contains(&ns) && !trust.may_use_reserved_namespace() {
        return Err(PackError::denied(format!(
            "pack {id}: the namespace {ns} is reserved for HalpTask's own packs"
        )));
    }
    Ok(())
}

/// Checks a path inside a bundle: relative, `[a-z0-9_./-]`, no empty,
/// `.` or `..` segments, at most 6 segments and 255 bytes.
pub fn check_path(path: &str) -> Result<()> {
    let bad = |why: &str| PackError::invalid(format!("bundle path {path:?} {why}"));
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return Err(bad("must be 1 to 255 bytes"));
    }
    if !path.bytes().all(|b| {
        b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'.' | b'/' | b'-')
    }) {
        return Err(bad("may only use a-z, 0-9, _, ., / and -"));
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments
        .iter()
        .any(|s| s.is_empty() || *s == "." || *s == "..")
    {
        return Err(bad("must be relative, with no empty, . or .. segments"));
    }
    if segments.len() > MAX_PATH_DEPTH {
        return Err(bad("nests deeper than 6 levels"));
    }
    if path == MANIFEST_FILE {
        return Err(bad("is the manifest, which travels outside `files`"));
    }
    Ok(())
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Whether `s` is 64 lowercase hex characters.
pub fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn pack_ids() {
        for ok in ["acme.sre", "acme.platform-sre", "a.b.c.d", "halp.developer"] {
            check_pack_id(ok).unwrap();
        }
        for bad in [
            "acme",
            "Acme.sre",
            "acme..sre",
            "acme.sre.",
            "1acme.sre",
            "a.b.c.d.e",
            "acme_x.sre",
            &format!("a{}.b", "x".repeat(40)),
        ] {
            assert!(check_pack_id(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn reserved_namespaces_need_first_party() {
        for ns in RESERVED_NAMESPACES {
            let id = format!("{ns}.thing");
            assert!(
                check_pack_identity(&id, PackTrust::Community).is_err(),
                "{id}"
            );
            assert!(
                check_pack_identity(&id, PackTrust::Unsigned).is_err(),
                "{id}"
            );
            check_pack_identity(&id, PackTrust::FirstParty).unwrap();
        }
        check_pack_identity("acme.sre", PackTrust::Unsigned).unwrap();
        // Dev packs carry `dev.` and are judged by the namespace after it.
        check_pack_identity("dev.acme.sre", PackTrust::Dev).unwrap();
        assert!(check_pack_identity("acme.sre", PackTrust::Dev).is_err());
        assert!(check_pack_identity("dev.halp.sre", PackTrust::Dev).is_err());
        assert_eq!(namespace("dev.acme.sre", PackTrust::Dev), "acme");
        assert_eq!(namespace("acme.sre", PackTrust::Community), "acme");
    }

    #[test]
    fn paths() {
        for ok in [
            "prompts/x.md",
            "wasm/burn.wasm",
            "ui/oncall/index.html",
            "a/b/c/d/e/f",
        ] {
            check_path(ok).unwrap();
        }
        for bad in [
            "",
            "/abs",
            "../x",
            "a/../x",
            "a//b",
            "./a",
            "a/./b",
            "A.md",
            "a b",
            "a\\b",
            "a/b/c/d/e/f/g",
            "halp-pack.json",
            &"a".repeat(256),
        ] {
            assert!(check_path(bad).is_err(), "{bad:?}");
        }
    }
}
