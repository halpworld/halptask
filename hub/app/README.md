# HalpTask Hub app

The Hub's desktop app: a Svelte 5 frontend (`src/`) in a Tauri v2 shell
(`src-tauri/`). The shell runs the core (`hub/crates/halp-core`) in-process
and hosts the MCP socket agents connect to.

## Develop

Needs Node 24, pnpm 10 and Rust 1.99 (`export PATH="/opt/homebrew/opt/rustup/bin:$PATH"` on macOS with Homebrew rustup).

```sh
pnpm install
pnpm dev            # browser only, on the in-memory mock core (seed data)
pnpm tauri dev      # the desktop app on the real core, with Vite hot reload
```

`pnpm dev` needs no Rust. The mock (`src/lib/api/mock/mock.ts`) follows the
core's contract: same commands, schemas, tiers and consent flow. Add
`#capture` to the URL to see the quick-capture window.

Checks (CI runs all of them):

```sh
pnpm check && pnpm test && pnpm build
cd src-tauri && cargo clippy --all-targets -- -D warnings && cargo test
```

`tests/contract.test.ts` checks every `dispatch` call in the UI against the
core's `commands.list` (`hub/docs/fixtures/commands.json`, or
`HALP_COMMANDS_JSON=/path`). It skips when there is no fixture. To make one
from the real core:

```sh
cd src-tauri && HALP_COMMANDS_OUT=/tmp/commands.json cargo test -- --ignored dump_commands
cd .. && HALP_COMMANDS_JSON=/tmp/commands.json pnpm test
```

## Build

```sh
pnpm tauri build --debug   # src-tauri/target/debug/bundle/macos/HalpTask Hub.app
pnpm tauri build           # release
```

The shell has its own Cargo workspace (`src-tauri/Cargo.toml`) and links the
core crates through path dependencies, so `cargo test` in `hub/` does not
build Tauri.

Environment variables (for testing):

| Variable | Effect |
| --- | --- |
| `HALP_HUB_DATA_DIR` | Data directory instead of the OS one (`~/Library/Application Support/halptask-hub` on macOS, shared with `halpd`). |
| `HALP_HUB_NO_KEYCHAIN=1` | For a new database, skip the keychain and go straight to the passphrase screen. |

## How it fits together

- **One door.** The webview calls a single command, `dispatch(request)`,
  which runs a core command and returns `{ok, result}` or `{ok:false, error}`.
  Change events come back as `halp://event`.
- **Windows.** `main` (1440x900, min 960x600, size and position remembered)
  and `capture` (a small window on top that hides when it loses focus).
  Closing `main` hides it; the tray (Quick capture, Show hub, Quit) and the MCP
  host keep running. **Cmd/Ctrl+Shift+Space** opens quick capture. If another
  app already owns that shortcut, the tray item still works.
- **Keys.** At startup the shell opens `hub.db` with the OS keychain. If the
  keychain cannot be used and there is no database yet, the app shows a
  first-run passphrase screen. A passphrase database (one with a `hub.db.kdf`
  sidecar) always opens on an unlock screen.
- **MCP.** Once the core is open, `halp_mcp::host::serve` listens on
  `<data_dir>/halp.sock` and gets the same `Arc<Core>`. Agent changes show up
  live, and agent T2/T3 calls open the consent dialog.

## Security model

The webview is treated as untrusted. It holds user data and shows text that
agents wrote. Everything below is there so that a script running in it can do
no more than the user can do by clicking.

**The webview is always the user.** `dispatch` runs every request with
`Ctx::user()`. The request type (`bridge::WebRequest`) has no actor or
session field, and any extra field such as `actor` is dropped before the core
sees the request (`bridge.rs` tests this). Agents are only reachable through
the MCP socket, where the host checks their token and sets their actor itself.
The shell never calls `set_policy`.

**IPC surface.** `capabilities/hub-window.json` grants the `main` and
`capture` windows exactly:

- `dispatch`, `key_status`, `unlock` and `hide_capture` (the app's own
  commands, declared in `build.rs`);
- `core:event:allow-listen` and `core:event:allow-unlisten`.

No other permission is granted, so the webview cannot reach the shell, fs,
http, dialog, clipboard, window or webview APIs. No plugin exposes commands to
the webview: window-state, single-instance and global-shortcut are used from
Rust only.

**Content Security Policy** (release):

```
default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:;
font-src 'self'; connect-src ipc: http://ipc.localhost; object-src 'none';
frame-src 'none'; base-uri 'none'; form-action 'none'
```

The policy holds without relaxation:

- Fonts are bundled, and Vite inlines nothing (`assetsInlineLimit: 0`).
- Svelte sets dynamic styles through the CSSOM (`style.cssText`, `setProperty`), which `style-src` does not block.
- The source has no inline `<script>` or `style="..."` attributes.
- `img-src data:` comes from the brief, for small inline images. The app uses none today, and an image cannot run script.
- Tauri adds hashes and nonces for its own bootstrap script.

`devCsp` applies only under `pnpm tauri dev`. It adds two things for Vite's
dev server, and the release build never uses it:

- `'unsafe-inline'` in `style-src`, because Vite injects `<style>` tags for hot reload.
- `ws://localhost:1420` in `connect-src`, for the HMR socket.

**Hardening.**

- `freezePrototype: true` and `withGlobalTauri: false`.
- Devtools are compiled in only for debug builds (`cfg!(debug_assertions)`).
- Navigation is limited to the bundled app (`nav.rs`):
  - `tauri://localhost` and `http(s)://tauri.localhost`, plus `http://localhost:1420` in `tauri dev` only;
  - everything else is refused;
  - new windows (`window.open`, `target=_blank`) are denied.
- No remote URL is ever loaded.

**Consent.** When the core answers `confirmation_required`, the consent dialog
shows the full request as plain text, never as HTML and never shortened:

- who is asking (name and raw actor id) and the tier;
- the command and its description;
- the summary, targets and effects;
- the complete arguments;
- the time left before it expires and the argument hash;
- a warning when the agent read external content first.

Approve or deny goes back through `dispatch` as the user.

**Passphrase.** The passphrase is typed in the webview (`KeyGate.svelte`) and
handled like this:

1. The form clears its fields before the call returns.
2. It is passed to Rust once, through `unlock`.
3. Rust moves it into a `Zeroizing<String>` straight away.
4. It is wiped when the key provider is dropped, right after the key is derived.

It is never logged, stored or sent back. Copies the JS engine and the IPC
layer make while the call is in flight are outside our control, and the
garbage collector frees them. That is the accepted limit of entering secrets
in a webview.

**Data on disk.**

- The data directory is created 0700.
- The database is encrypted (SQLCipher) with a key from the keychain or derived from the passphrase.
- The MCP socket is 0600 and `user.token` is rewritten on every start (see `halp_mcp::host`).

## Layout

```
src/                      Svelte app (shell/, widgets/, layout/, stores/, api/)
src/lib/api/tauri.ts      Tauri transport: invoke('dispatch'), listen('halp://event')
src/lib/api/shell.ts      key_status / unlock / hide_capture wrappers
src/lib/shell/KeyGate.svelte        keychain wait / passphrase screens
src/lib/shell/CaptureWindow.svelte  the quick-capture window
src-tauri/src/lib.rs      state, commands, windows, tray, shortcut, startup
src-tauri/src/bridge.rs   webview request -> core, always as the user
src-tauri/src/keys.rs     keychain / passphrase opening
src-tauri/src/nav.rs      navigation allow-list
src-tauri/src/mcp.rs      MCP host start/stop
src-tauri/capabilities/   the one capability file
```
