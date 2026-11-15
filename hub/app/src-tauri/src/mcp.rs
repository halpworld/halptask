//! MCP host: lets agents reach the same in-process core over a local socket
//! (`<data_dir>/halp.sock`, 0600 in a 0700 directory; see `halp_mcp::host`).
//!
//! The host gets the same `Arc<Core>` the webview uses, so agent changes
//! reach the UI as `halp://event`s and agent T2/T3 calls show up in the
//! consent dialog via `consent.requested`. Agents authenticate with a token
//! from `halpd clients pair`; the host, never the client, sets their actor.

use std::path::Path;
use std::sync::Arc;

use halp_core::Core;
use halp_mcp::host::{HostHandle, HostOptions};

pub struct Host(HostHandle);

pub async fn start(core: Arc<Core>, data_dir: &Path) -> Result<Host, String> {
    halp_mcp::host::serve(core, HostOptions::new(data_dir))
        .await
        .map(Host)
        .map_err(|e| format!("MCP host could not start: {e}"))
}

impl Host {
    pub fn socket_path(&self) -> &Path {
        self.0.socket_path()
    }

    /// Stops accepting, closes connections, removes the socket and user token.
    pub async fn shutdown(self) {
        self.0.shutdown().await;
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use halp_core::SystemClock;
    use halp_store::{RawKeyProvider, Store};

    #[tokio::test(flavor = "multi_thread")]
    async fn host_listens_in_the_data_dir_and_cleans_up() {
        // Unix socket paths are short (~104 bytes on macOS); keep it in /tmp.
        let dir = tempfile::Builder::new()
            .prefix("halp")
            .tempdir_in("/tmp")
            .expect("tempdir");
        let store =
            Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([3; 32])).expect("store");
        let core = Arc::new(Core::with_store(store, Arc::new(SystemClock)).expect("core"));
        let host = start(core, dir.path()).await.expect("host");
        let sock = host.socket_path().to_path_buf();
        assert_eq!(sock, dir.path().join(halp_mcp::SOCKET_NAME));
        assert!(sock.exists());
        host.shutdown().await;
        assert!(!sock.exists());
    }
}
