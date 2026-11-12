#![allow(dead_code, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use halp_core::{Core, Ctx, Request, Response};
use halp_integrations::{HttpOptions, MemorySecrets, Runtime, RuntimeOptions};
use halp_store::{RawKeyProvider, Store};
use serde_json::{Value, json};
use tempfile::TempDir;

pub struct H {
    pub core: Arc<Core>,
    pub rt: Runtime,
    pub secrets: Arc<MemorySecrets>,
    pub dir: TempDir,
}

pub fn start() -> H {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("hub.db"), &RawKeyProvider::new([7; 32])).unwrap();
    let core = Arc::new(Core::with_store(store, Arc::new(halp_core::SystemClock)).unwrap());
    let secrets = Arc::new(MemorySecrets::default());
    let mut opts = RuntimeOptions::new(dir.path().join("integrations"), secrets.clone());
    opts.http = HttpOptions {
        allow_loopback_http: true,
        ..HttpOptions::default()
    };
    let rt = Runtime::start(&core, opts, tokio::runtime::Handle::current()).unwrap();
    H {
        core,
        rt,
        secrets,
        dir,
    }
}

impl H {
    pub fn call(&self, cmd: &str, args: Value) -> Response {
        self.core.dispatch(&Ctx::user(), Request::new(cmd, args))
    }

    pub fn ok(&self, cmd: &str, args: Value) -> Value {
        match self.call(cmd, args) {
            Response::Ok(v) => v,
            Response::Err(e) => panic!("{cmd} failed: {e:?}"),
        }
    }

    pub fn entry(&self, id: &str) -> Value {
        self.ok("integrations.list", json!({}))["integrations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["config"]["id"] == id)
            .cloned()
            .unwrap()
    }

    pub fn state(&self, id: &str) -> String {
        self.entry(id)["status"]["state"]
            .as_str()
            .unwrap()
            .to_string()
    }

    /// Waits until `f` holds (10 s at most).
    pub async fn until(&self, what: &str, f: impl Fn(&H) -> bool) {
        for _ in 0..200 {
            if f(self) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!(
            "timed out waiting for {what}; status: {}",
            self.ok("integrations.list", json!({}))
        );
    }

    /// Synced items of an integration.
    pub fn items(&self, id: &str) -> Vec<Value> {
        self.ok("tasks.list", json!({}))["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|i| i["origin"] == format!("integration:{id}"))
            .cloned()
            .collect()
    }
}
