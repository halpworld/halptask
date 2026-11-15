//! Integration tokens. Stored only in the OS keychain (service
//! `halptask-hub`, account `integration:<id>`), never in the database,
//! events, the audit log or logs. `Debug` never prints them.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// The keychain service, shared with the database key.
pub const KEYCHAIN_SERVICE: &str = "halptask-hub";
/// Longest token the app accepts from the user.
pub const MAX_TOKEN_BYTES: usize = 4096;

/// An access token and what is needed to renew it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Zeroize, ZeroizeOnDrop)]
pub struct Token {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default = "bearer")]
    pub token_type: String,
}

fn bearer() -> String {
    "bearer".into()
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Token")
            .field("access_token", &"[redacted]")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "[redacted]"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl Token {
    /// A personal access token (no expiry, no refresh).
    pub fn personal(access_token: String) -> Token {
        Token {
            access_token,
            refresh_token: None,
            expires_at: None,
            token_type: bearer(),
        }
    }

    /// Expired or expiring within `margin_secs`.
    pub fn expiring(&self, now: DateTime<Utc>, margin_secs: i64) -> bool {
        self.expires_at
            .is_some_and(|t| t <= now.timestamp() + margin_secs)
    }

    /// The secret strings, for redaction.
    pub fn secrets(&self) -> Vec<&str> {
        let mut v = vec![self.access_token.as_str()];
        if let Some(r) = &self.refresh_token {
            v.push(r);
        }
        v
    }
}

/// Validates a token typed by the user: 1..=4096 bytes of printable ASCII.
pub fn check_user_token(t: &str) -> Result<(), &'static str> {
    if t.is_empty() {
        return Err("empty");
    }
    if t.len() > MAX_TOKEN_BYTES {
        return Err("too_long");
    }
    if !t.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err("not_printable_ascii");
    }
    Ok(())
}

/// Where tokens live.
pub trait SecretStore: Send + Sync {
    fn get(&self, id: &str) -> Result<Option<Token>, String>;
    fn set(&self, id: &str, token: &Token) -> Result<(), String>;
    fn delete(&self, id: &str) -> Result<(), String>;
}

/// The OS keychain (macOS Keychain, Windows Credential Manager, Secret
/// Service on Linux).
pub struct KeychainSecrets {
    service: String,
}

impl Default for KeychainSecrets {
    fn default() -> Self {
        KeychainSecrets {
            service: KEYCHAIN_SERVICE.into(),
        }
    }
}

impl KeychainSecrets {
    pub fn with_service(service: &str) -> Self {
        KeychainSecrets {
            service: service.into(),
        }
    }

    fn entry(&self, id: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(&self.service, &format!("integration:{id}"))
            .map_err(|e| format!("OS keychain unavailable: {e}"))
    }
}

impl SecretStore for KeychainSecrets {
    fn get(&self, id: &str) -> Result<Option<Token>, String> {
        match self.entry(id)?.get_secret() {
            Ok(mut bytes) => {
                let t = serde_json::from_slice::<Token>(&bytes)
                    .map_err(|_| "keychain entry is not a token".to_string());
                bytes.zeroize();
                t.map(Some)
            }
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("could not read the OS keychain: {e}")),
        }
    }

    fn set(&self, id: &str, token: &Token) -> Result<(), String> {
        let mut bytes = serde_json::to_vec(token).map_err(|e| e.to_string())?;
        let r = self
            .entry(id)?
            .set_secret(&bytes)
            .map_err(|e| format!("could not write the OS keychain: {e}"));
        bytes.zeroize();
        r
    }

    fn delete(&self, id: &str) -> Result<(), String> {
        match self.entry(id)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("could not delete from the OS keychain: {e}")),
        }
    }
}

/// In memory (tests, or a platform without a keychain).
#[derive(Default)]
pub struct MemorySecrets {
    map: Mutex<HashMap<String, Token>>,
}

impl SecretStore for MemorySecrets {
    fn get(&self, id: &str) -> Result<Option<Token>, String> {
        Ok(self.map.lock().map_err(|_| "poisoned")?.get(id).cloned())
    }
    fn set(&self, id: &str, token: &Token) -> Result<(), String> {
        self.map
            .lock()
            .map_err(|_| "poisoned")?
            .insert(id.to_string(), token.clone());
        Ok(())
    }
    fn delete(&self, id: &str) -> Result<(), String> {
        self.map.lock().map_err(|_| "poisoned")?.remove(id);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_token() {
        let mut t = Token::personal("ghp_supersecret".into());
        t.refresh_token = Some("rt-secret".into());
        let d = format!("{t:?}");
        assert!(
            !d.contains("supersecret") && !d.contains("rt-secret"),
            "{d}"
        );
    }

    #[test]
    fn user_tokens_are_checked() {
        assert!(check_user_token("ghp_abc123").is_ok());
        assert_eq!(check_user_token(""), Err("empty"));
        assert_eq!(check_user_token("a b"), Err("not_printable_ascii"));
        assert_eq!(check_user_token("tok\n"), Err("not_printable_ascii"));
        assert_eq!(check_user_token("é"), Err("not_printable_ascii"));
        assert_eq!(check_user_token(&"a".repeat(4097)), Err("too_long"));
        assert!(check_user_token(&"a".repeat(4096)).is_ok());
    }
}
