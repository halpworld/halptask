//! The wire envelope shared by the UI, MCP and tests (see `docs/CONTRACT.md`).

use halp_domain::{DomainError, ErrorKind};
use halp_store::StoreError;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `{ "command": "tasks.create", "args": { ... }, "request_id": "<ulid>" }`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub command: String,
    #[serde(default)]
    pub args: Value,
    #[serde(default)]
    pub request_id: Option<String>,
}

impl Request {
    pub fn new(command: &str, args: Value) -> Self {
        Request {
            command: command.to_string(),
            args,
            request_id: Some(halp_domain::new_id()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    NotFound,
    InvalidArgs,
    Denied,
    ConfirmationRequired,
    /// The target changed since the caller read it (stale `if_version`,
    /// or an undo/redo whose entities were edited in between).
    Conflict,
    Internal,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ErrorBody {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirmation_id: Option<String>,
}

/// `{ "ok": true, "result": ... }` or `{ "ok": false, "error": ... }`.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    Ok(Value),
    Err(ErrorBody),
}

impl Response {
    pub fn is_ok(&self) -> bool {
        matches!(self, Response::Ok(_))
    }

    pub fn result(&self) -> Option<&Value> {
        match self {
            Response::Ok(v) => Some(v),
            Response::Err(_) => None,
        }
    }

    pub fn error(&self) -> Option<&ErrorBody> {
        match self {
            Response::Ok(_) => None,
            Response::Err(e) => Some(e),
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Response::Ok(v) => serde_json::json!({"ok": true, "result": v}),
            Response::Err(e) => serde_json::json!({"ok": false, "error": e}),
        }
    }
}

impl Serialize for Response {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(s)
    }
}

impl<'de> Deserialize<'de> for Response {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Raw {
            ok: bool,
            #[serde(default)]
            result: Value,
            #[serde(default)]
            error: Option<ErrorBody>,
        }
        let r = Raw::deserialize(d)?;
        if r.ok {
            Ok(Response::Ok(r.result))
        } else {
            r.error
                .map(Response::Err)
                .ok_or_else(|| serde::de::Error::missing_field("error"))
        }
    }
}

/// A command failure, mapped onto the contract's error codes.
#[derive(Debug, Clone, PartialEq)]
pub struct CmdError {
    pub code: ErrorCode,
    pub message: String,
    pub confirmation_id: Option<String>,
}

impl CmdError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        CmdError {
            code,
            message: message.into(),
            confirmation_id: None,
        }
    }
    pub fn not_found(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, m)
    }
    pub fn invalid(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidArgs, m)
    }
    pub fn conflict(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, m)
    }
    pub fn internal(m: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, m)
    }
}

impl std::fmt::Display for CmdError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for CmdError {}

impl From<CmdError> for ErrorBody {
    fn from(e: CmdError) -> Self {
        ErrorBody {
            code: e.code,
            message: e.message,
            confirmation_id: e.confirmation_id,
        }
    }
}

impl From<DomainError> for CmdError {
    fn from(e: DomainError) -> Self {
        let code = match e.kind {
            ErrorKind::NotFound => ErrorCode::NotFound,
            ErrorKind::Invalid => ErrorCode::InvalidArgs,
            ErrorKind::Conflict => ErrorCode::Conflict,
        };
        CmdError::new(code, e.message)
    }
}

impl From<halp_pack::PackError> for CmdError {
    fn from(e: halp_pack::PackError) -> Self {
        use halp_pack::PackErrorKind as K;
        let code = match e.kind {
            K::Invalid => ErrorCode::InvalidArgs,
            K::Conflict => ErrorCode::Conflict,
            K::Denied => ErrorCode::Denied,
            K::NotFound => ErrorCode::NotFound,
        };
        CmdError::new(code, e.message)
    }
}

impl From<StoreError> for CmdError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::NotFound(m) => CmdError::not_found(m),
            StoreError::Conflict(m) => CmdError::conflict(m),
            StoreError::Denied(m) => CmdError::new(ErrorCode::Denied, m),
            StoreError::Invalid(m) | StoreError::Import(m) => CmdError::invalid(m),
            other => CmdError::internal(other.to_string()),
        }
    }
}

impl From<serde_json::Error> for CmdError {
    fn from(e: serde_json::Error) -> Self {
        CmdError::internal(format!("serialization failed: {e}"))
    }
}
