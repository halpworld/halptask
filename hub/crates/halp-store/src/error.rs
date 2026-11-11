use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    /// The database key (keychain key or passphrase) does not open this database.
    #[error("wrong key: the database could not be decrypted")]
    WrongKey,
    /// The key could not be obtained (keychain unavailable, missing sidecar, ...).
    #[error("key error: {0}")]
    Key(String),
    #[error("not found: {0}")]
    NotFound(String),
    /// The stored state changed since the operation was recorded.
    #[error("conflict: {0}")]
    Conflict(String),
    /// The caller may not do this (for example undo another actor's change).
    #[error("denied: {0}")]
    Denied(String),
    #[error("invalid: {0}")]
    Invalid(String),
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew { found: i64, supported: i64 },
    /// The imported file is not readable (bad format, wrong passphrase, limits).
    #[error("import failed: {0}")]
    Import(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("data error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;
