//! The crate's error: a kind that maps onto the core's error codes, and a
//! message for the user.

use halp_domain::{DomainError, ErrorKind};

/// Broad error classes. P5-B maps them onto `ErrorCode`: `Invalid` to
/// `invalid_args`, `Conflict` to `conflict`, `Denied` to `denied`,
/// `NotFound` to `not_found`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackErrorKind {
    /// Malformed, tampered with, over a limit, or a bad signature.
    Invalid,
    /// Clashes with something else (a role id, two overrides of one type).
    Conflict,
    /// Refused by trust: a revoked key or bundle, a different publisher
    /// on update, a capability the trust level does not allow.
    Denied,
    NotFound,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PackError {
    pub kind: PackErrorKind,
    pub message: String,
}

impl PackError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: PackErrorKind::Invalid,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: PackErrorKind::Conflict,
            message: message.into(),
        }
    }

    pub fn denied(message: impl Into<String>) -> Self {
        Self {
            kind: PackErrorKind::Denied,
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: PackErrorKind::NotFound,
            message: message.into(),
        }
    }
}

impl From<DomainError> for PackError {
    fn from(e: DomainError) -> Self {
        let kind = match e.kind {
            ErrorKind::Invalid => PackErrorKind::Invalid,
            ErrorKind::Conflict => PackErrorKind::Conflict,
            ErrorKind::NotFound => PackErrorKind::NotFound,
        };
        Self {
            kind,
            message: e.message,
        }
    }
}

pub type Result<T> = std::result::Result<T, PackError>;
