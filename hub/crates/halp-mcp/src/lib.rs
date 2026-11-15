//! HalpTask Hub MCP: the core's commands, state and role prompts for MCP
//! clients (Claude Code, Claude Desktop and other agents).
//!
//! * [`server::HalpServer`]: the MCP server for one connection, built on
//!   rmcp. Protocol: whatever the client negotiates up to
//!   [`PROTOCOL_VERSION`] (also 2025-11-25 and older with `initialize`).
//! * [`host::serve`]: the local socket the Hub listens on; clients prove
//!   who they are with a token from `halpd clients pair`.
//! * The `halp-mcp` binary: a stdio shim that MCP clients launch; it reads
//!   its token from the environment, sends it, then pipes bytes.

pub mod host;
pub mod prompts;
pub mod server;

pub use host::{HostHandle, HostOptions, SOCKET_NAME, serve};
pub use server::{HalpServer, tool_name};

/// The newest MCP revision this server speaks.
pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// The newest revision that still starts with `initialize`.
pub const PROTOCOL_VERSION_WITH_INITIALIZE: &str = "2025-11-25";
