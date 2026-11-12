//! HalpTask Hub integrations: the runtime that syncs external systems
//! (GitHub, MCP servers, local git, calendars) into the core as typed items.
//!
//! See `docs/INTEGRATIONS.md`. In short: configuration lives in the core
//! (user-only `integrations.connect`), tokens live in the OS keychain and
//! reach this crate only from the shell ([`Runtime::set_token`]) or an
//! OAuth flow, and every write goes through `Core::dispatch` as the
//! integration's own actor (`integration:<id>`).

pub mod calendar;
pub mod connector;
pub mod energy;
pub mod git;
pub mod github;
pub mod http;
pub mod mcp;
pub mod oauth;
pub mod redact;
pub mod runtime;
pub mod secrets;

pub use connector::{Batch, Connector, FieldPatch, ItemSource, NoItems, SyncCx, SyncError};
pub use http::{Http, HttpOptions, USER_AGENT};
pub use oauth::{OAuthProvider, Opener};
pub use runtime::{Runtime, RuntimeOptions};
pub use secrets::{KEYCHAIN_SERVICE, KeychainSecrets, MemorySecrets, SecretStore, Token};
