use std::fmt;

/// Broad error classes. They map one-to-one onto the contract's error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    NotFound,
    Invalid,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainError {
    pub kind: ErrorKind,
    pub message: String,
}

impl DomainError {
    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::NotFound,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Invalid,
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Conflict,
            message: message.into(),
        }
    }
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DomainError {}

pub type Result<T> = std::result::Result<T, DomainError>;
