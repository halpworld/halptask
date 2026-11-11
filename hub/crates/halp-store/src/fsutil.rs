//! Owner-only file and directory helpers. On unix, data directories are
//! created 0700 and files 0600; elsewhere the platform defaults apply (on
//! Windows the per-user profile directory ACLs).

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use crate::error::Result;

/// Creates `dir` (and parents) if missing; a directory we create is 0700.
pub fn ensure_private_dir(dir: &Path) -> Result<()> {
    if dir.as_os_str().is_empty() || dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn private_options() -> OpenOptions {
    let mut o = OpenOptions::new();
    o.write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o
}

/// Restricts an existing file to the owner.
pub fn restrict_file(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Makes sure `path` exists as an owner-only file. Returns true when the
/// file is new (missing or empty), i.e. a database still to be created.
pub fn ensure_private_file(path: &Path) -> Result<bool> {
    match private_options().create_new(true).open(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            restrict_file(path)?;
            Ok(fs::metadata(path)?.len() == 0)
        }
        Err(e) => Err(e.into()),
    }
}

/// Writes `bytes` to `path` (replacing it) as an owner-only file.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = private_options().create(true).truncate(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    restrict_file(path)
}
