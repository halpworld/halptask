//! `halpd pack`: make a signing key, sign a pack directory into a
//! `*.halppack` bundle, and verify a bundle. Pure file work through
//! `halp-pack`: no database, no Core.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use halp_pack::crypto::encode_pubkey;
use halp_pack::ids::MANIFEST_FILE;
use halp_pack::{
    CommunityKey, SigningKey, build_bundle, crypto, fingerprint, generate_pkcs8, key_id, verify,
};

pub const USAGE: &str = "  pack keygen --out KEYFILE
  pack sign DIR --key KEYFILE --out BUNDLE.halppack
  pack verify BUNDLE.halppack [--trust PUBKEYFILE]";

/// A parsed `halpd pack ...` command.
#[derive(Debug, PartialEq, Eq)]
pub enum PackCmd {
    Keygen {
        out: PathBuf,
    },
    Sign {
        dir: PathBuf,
        key: PathBuf,
        out: PathBuf,
    },
    Verify {
        bundle: PathBuf,
        /// A file holding a base64 public key to treat as a trusted
        /// community key for this check (as a user would in Settings).
        trust: Option<PathBuf>,
    },
}

fn usage_err(msg: &str) -> String {
    format!("{msg}\nusage:\n{USAGE}")
}

/// Parses the arguments after `pack`.
pub fn parse(args: &[String]) -> Result<PackCmd, String> {
    let Some((sub, rest)) = args.split_first() else {
        return Err(usage_err("pack needs a command"));
    };
    let mut positional = Vec::new();
    let mut key = None;
    let mut out = None;
    let mut trust = None;
    let mut it = rest.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--key" => key = Some(PathBuf::from(it.next().ok_or("--key needs a value")?)),
            "--out" => out = Some(PathBuf::from(it.next().ok_or("--out needs a value")?)),
            "--trust" => {
                trust = Some(PathBuf::from(it.next().ok_or("--trust needs a value")?));
            }
            s if s.starts_with('-') => return Err(usage_err(&format!("unknown flag {s}"))),
            s => positional.push(PathBuf::from(s)),
        }
    }
    let cmd = match sub.as_str() {
        "keygen" | "sign" if trust.is_some() => {
            return Err(usage_err(&format!("pack {sub} does not take --trust")));
        }
        "keygen" if positional.is_empty() && key.is_none() => PackCmd::Keygen {
            out: out.ok_or_else(|| usage_err("pack keygen needs --out"))?,
        },
        "sign" if positional.len() == 1 => PackCmd::Sign {
            dir: positional.remove(0),
            key: key.ok_or_else(|| usage_err("pack sign needs --key"))?,
            out: out.ok_or_else(|| usage_err("pack sign needs --out"))?,
        },
        "verify" if positional.len() == 1 && key.is_none() && out.is_none() => PackCmd::Verify {
            bundle: positional.remove(0),
            trust,
        },
        "keygen" | "sign" | "verify" => {
            return Err(usage_err(&format!("bad pack {sub} arguments")));
        }
        other => return Err(usage_err(&format!("unknown pack command {other}"))),
    };
    Ok(cmd)
}

/// Writes a new private key (base64 PKCS#8) to `out`, which must not
/// exist; on unix it is created with mode 0600.
fn keygen(out: &Path) -> Result<String, String> {
    let der = generate_pkcs8().map_err(|e| e.to_string())?;
    let key = SigningKey::from_pkcs8(&der).map_err(|e| e.to_string())?;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(out)
        .map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    f.write_all(format!("{}\n", STANDARD.encode(&der)).as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let pk = key.public_key();
    Ok(format!(
        "wrote private key {}\nkey id:      {}\nfingerprint: {}\npublic key:  {}",
        out.display(),
        key.key_id(),
        fingerprint(&pk),
        encode_pubkey(&pk)
    ))
}

fn read_key(path: &Path) -> Result<SigningKey, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let der = STANDARD
        .decode(text.trim())
        .map_err(|_| format!("{} is not a halpd pack key", path.display()))?;
    SigningKey::from_pkcs8(&der).map_err(|e| format!("{}: {e}", path.display()))
}

/// Collects the pack's files: everything under `dir` except dotfiles,
/// dot-directories and the root manifest. Symlinks are refused so the
/// bundle holds exactly what is in the directory.
fn collect(dir: &Path, rel: &str, out: &mut BTreeMap<String, Vec<u8>>) -> Result<(), String> {
    let mut entries: Vec<_> = fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .collect::<Result<_, _>>()
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| format!("{} has a name that is not UTF-8", e.path().display()))?;
        if name.starts_with('.') {
            continue;
        }
        let path = if rel.is_empty() {
            name.to_string()
        } else {
            format!("{rel}/{name}")
        };
        if path == MANIFEST_FILE {
            continue;
        }
        let ft = e.file_type().map_err(|e| e.to_string())?;
        if ft.is_symlink() {
            return Err(format!(
                "{} is a symlink; packs may not contain links",
                e.path().display()
            ));
        } else if ft.is_dir() {
            collect(&e.path(), &path, out)?;
        } else if ft.is_file() {
            let data = fs::read(e.path()).map_err(|err| format!("cannot read {path}: {err}"))?;
            out.insert(path, data);
        } else {
            return Err(format!("{} is not a regular file", e.path().display()));
        }
    }
    Ok(())
}

fn sign(dir: &Path, key: &Path, out: &Path) -> Result<String, String> {
    let key = read_key(key)?;
    let manifest_path = dir.join(MANIFEST_FILE);
    let manifest = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read {}: {e}", manifest_path.display()))?;
    let mut files = BTreeMap::new();
    collect(dir, "", &mut files)?;
    let bytes = build_bundle(&manifest, &files, Some(&key)).map_err(|e| e.to_string())?;
    // Check it the way an installer will once the user trusts this key
    // (packs with modules or UI widgets need a trusted signer).
    let own = CommunityKey {
        key_id: key.key_id(),
        pubkey: key.public_key(),
        revoked: false,
    };
    let v = verify(&bytes, &[own]).map_err(|e| e.to_string())?;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    f.write_all(&bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    Ok(format!(
        "wrote {} ({} {}, {} files, {} bytes)\nsigned by key {}\nsha256 {}",
        out.display(),
        v.manifest.id,
        v.manifest.version,
        v.files.len(),
        bytes.len(),
        key.key_id(),
        v.bundle_sha256
    ))
}

fn read_pubkey(path: &Path) -> Result<CommunityKey, String> {
    let text =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let pubkey = crypto::decode_pubkey(text.trim()).map_err(|e| e.to_string())?;
    Ok(CommunityKey {
        key_id: key_id(&pubkey),
        pubkey,
        revoked: false,
    })
}

fn verify_file(path: &Path, trust: Option<&Path>) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let community = match trust {
        Some(p) => vec![read_pubkey(p)?],
        None => Vec::new(),
    };
    let v = verify(&bytes, &community).map_err(|e| e.to_string())?;
    let mut s = format!(
        "ok: {} {} ({})\ntrust: {}\n",
        v.manifest.id,
        v.manifest.version,
        v.manifest.title,
        v.trust.as_str()
    );
    match &v.signer {
        Some(sig) => s.push_str(&format!(
            "signer: {:?} key {}\nfingerprint: {}\n",
            sig.kind, sig.key_id, sig.fingerprint
        )),
        None => s.push_str("signer: none (unsigned)\n"),
    }
    s.push_str(&format!("sha256 {}\nfiles:\n", v.bundle_sha256));
    for (p, e) in &v.file_index {
        s.push_str(&format!("  {p} {} bytes {}\n", e.size, e.sha256));
    }
    for w in &v.warnings {
        s.push_str(&format!("warning: {w}\n"));
    }
    Ok(s.trim_end().to_string())
}

/// Runs a pack command, returning what to print.
pub fn exec(cmd: &PackCmd) -> Result<String, String> {
    match cmd {
        PackCmd::Keygen { out } => keygen(out),
        PackCmd::Sign { dir, key, out } => sign(dir, key, out),
        PackCmd::Verify { bundle, trust } => verify_file(bundle, trust.as_deref()),
    }
}

pub fn run(cmd: &PackCmd) -> Result<ExitCode, String> {
    println!("{}", exec(cmd)?);
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn parses() {
        assert_eq!(
            parse(&args("keygen --out k")).unwrap(),
            PackCmd::Keygen { out: "k".into() }
        );
        assert_eq!(
            parse(&args("sign d --key k --out b")).unwrap(),
            PackCmd::Sign {
                dir: "d".into(),
                key: "k".into(),
                out: "b".into()
            }
        );
        assert_eq!(
            parse(&args("verify b")).unwrap(),
            PackCmd::Verify {
                bundle: "b".into(),
                trust: None
            }
        );
        for bad in [
            "",
            "keygen",
            "sign d --out b",
            "sign --key k --out b",
            "verify",
            "verify a b",
            "frob x",
            "verify b --force",
        ] {
            assert!(parse(&args(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn keygen_sign_verify_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let key = tmp.path().join("pack.key");
        let out = exec(&PackCmd::Keygen { out: key.clone() }).unwrap();
        assert!(out.contains("key id:"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&key).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Never overwrites a key.
        assert!(exec(&PackCmd::Keygen { out: key.clone() }).is_err());

        let dir = tmp.path().join("pack");
        fs::create_dir_all(dir.join("prompts")).unwrap();
        fs::write(dir.join("prompts/triage.md"), "Triage the incident.").unwrap();
        fs::write(dir.join(".DS_Store"), "junk").unwrap();
        let sha = halp_pack::ids::sha256_hex(b"Triage the incident.");
        let manifest = serde_json::json!({
            "format": 1, "id": "acme.sre", "version": "1.0.0", "title": "Acme SRE",
            "description": "On-call", "license": "MIT", "publisher": {"name": "Acme"},
            "min_hub": "0.x", "extends": {"id": "halp.developer", "version": "^0.1"},
            "role": {"id": "acme-sre", "title": "SRE", "accent": "#112233"},
            "prompts": {"add": ["prompts/triage.md"]},
            "files": {"prompts/triage.md": {"sha256": sha, "size": 20}}
        });
        fs::write(dir.join(MANIFEST_FILE), manifest.to_string()).unwrap();

        let bundle = tmp.path().join("acme.halppack");
        let out = exec(&PackCmd::Sign {
            dir: dir.clone(),
            key: key.clone(),
            out: bundle.clone(),
        })
        .unwrap();
        assert!(out.contains("1 files"), "{out}");
        let out = exec(&PackCmd::Verify {
            bundle: bundle.clone(),
            trust: None,
        })
        .unwrap();
        // A fresh key is nobody's yet: a valid signature, unsigned trust.
        assert!(out.contains("trust: unsigned"), "{out}");
        assert!(out.contains("signer: Unknown"), "{out}");
        assert!(out.contains("prompts/triage.md"), "{out}");

        // Tampering with the bundle is caught.
        let text = fs::read_to_string(&bundle).unwrap();
        let tampered = tmp.path().join("bad.halppack");
        fs::write(&tampered, text.replace("Acme SRE", "Acme SRF")).unwrap();
        assert!(
            exec(&PackCmd::Verify {
                bundle: tampered,
                trust: None
            })
            .is_err()
        );

        // A file not in the manifest index is refused at signing.
        fs::write(dir.join("extra.txt"), "x").unwrap();
        let err = exec(&PackCmd::Sign {
            dir: dir.clone(),
            key: key.clone(),
            out: tmp.path().join("x.halppack"),
        })
        .unwrap_err();
        assert!(err.contains("does not match"), "{err}");
        fs::remove_file(dir.join("extra.txt")).unwrap();

        // Links are refused.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/hosts", dir.join("link.md")).unwrap();
            let err = exec(&PackCmd::Sign {
                dir,
                key,
                out: tmp.path().join("y.halppack"),
            })
            .unwrap_err();
            assert!(err.contains("symlink"), "{err}");
        }
    }

    /// A fork of the example pack (Tier B module + Tier C widget) can be
    /// signed and verified with halpd alone, trusting the signer's key.
    #[test]
    fn signs_and_verifies_a_pack_with_code() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let tmp = tempfile::tempdir().unwrap();
        let bundle = tmp.path().join("fork.halppack");
        exec(&PackCmd::Sign {
            dir: root.join("packs/acme-sre"),
            key: root.join("keys/NEVER-TRUST-test-only.key"),
            out: bundle.clone(),
        })
        .unwrap();
        let err = exec(&PackCmd::Verify {
            bundle: bundle.clone(),
            trust: None,
        })
        .unwrap_err();
        assert!(err.contains("trusted"), "{err}");
        let out = exec(&PackCmd::Verify {
            bundle,
            trust: Some(root.join("keys/NEVER-TRUST-test-only.pub")),
        })
        .unwrap();
        assert!(out.contains("trust: community"), "{out}");
        assert!(parse(&args("sign d --key k --out b --trust p")).is_err());
        assert_eq!(
            parse(&args("verify b --trust p")).unwrap(),
            PackCmd::Verify {
                bundle: "b".into(),
                trust: Some("p".into())
            }
        );
    }
}
