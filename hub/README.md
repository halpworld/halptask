# HalpTask Hub

An all-Rust (Tauri v2) desktop hub for tasks, role dashboards and agents. The
Go TUI at the repository root is the predecessor; the Hub can import its files.

| Path | What |
| --- | --- |
| `crates/halp-domain` | Pure data model: items, outline, filters, task types, layouts, grid rules, capture parsing, dates. |
| `crates/halp-store` | SQLCipher database, append-only event log, undo/redo, keys, TUI importer. |
| `crates/halp-core` | Command registry, dispatch, policy hook, consent, change events, role packs, `halpd` CLI. |
| `packs/<role>/pack.json` | Built-in role packs (developer, eng-manager, cso). |
| `app/` | Svelte frontend. |
| `docs/CONTRACT.md` | The command contract shared by UI, MCP and tests. |
| `docs/ARCHITECTURE.md` | Crate map, data model, event log, encryption and key handling. |

## Build and test

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Rust 1.99, edition 2024. The first build compiles SQLCipher and OpenSSL from source, which needs a C compiler and Perl.

## halpd

```sh
# Uses the OS keychain for the database key; the database lives in the
# platform data dir (…/halptask-hub/hub.db) unless --db is given.
halpd dispatch '{"command":"capture.quick","args":{"text":"Fix login #bug due:friday"}}'
halpd commands

# Passphrase-protected database: the passphrase comes from an env var or stdin, never argv.
HUB_PASS=… halpd --db ./hub.db --db-passphrase-env HUB_PASS dispatch '{"command":"day.state"}'

# Import a TUI file (protobuf or Markdown, plain or encrypted).
TUI_PASS=… halpd import path/to/tui-file.pb --passphrase-env TUI_PASS
```

`--actor agent:<id>` runs a request as another actor. The exit code is 0 on success, 1 on an error response and 2 when halpd could not start.
