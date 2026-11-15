# HalpTask Hub P5: the extension platform (design)

Status: design for implementers. Owner: lead. Date: 2026-11-11. Binding where it says "must". It changes CONTRACT.md as v1.10 (see §9).

## 0. Where we start (checked against the code)

- **Packs are compiled in.** `BUILTIN` uses `include_str!` for 3 packs (`hub/crates/halp-core/src/packs.rs:23-33`). `builtin_packs()` (`packs.rs:106`) is called once in `Core::open` (`halp-core/src/lib.rs:207`). `State` keeps `packs: Vec<Pack>` (`state.rs:176`), registers their types as `TaskTypeOrigin::Pack` in `reload` (`state.rs:366-382`), and seeds layouts in a non-undoable `system.seed` (`state.rs:546-580`). Packs are never stored (ARCHITECTURE.md:96).
- **Pack shape.** `Pack {id,title,accent,widgets,task_types,layout}` uses `deny_unknown_fields` (`packs.rs:8-21`). `Pack::validate` does the text checks, layout and kinds, type class (`state.rs:54`), immutable check (`immutable.rs:58`), and the reserved `base.` namespace (`packs.rs:48-93`). A role id is `^[a-z][a-z0-9-]{0,47}$` (`halp-domain/src/layout.rs:153`). Actor and kind ids use `simple_id`, `[A-Za-z0-9_.-]{1,64}` (`halp-domain/src/limits.rs:322`).
- **Trust gap (must fix in P5).** `TaskTypeOrigin` is `Builtin|Pack|User` (`halp-domain/src/task_type.rs:45-49`). The privileged schema annotations are refused only for `User`: `x-halp-immutable-when` at `immutable.rs:98`, `x-halp-ext-writable` at `commands/integrations.rs:1084`. A community pack loaded as `Pack` would gain both.
- **Uninstall gap (must fix).** `type_class` returns `general` for an unknown type id (`state.rs:429-436`), and `validate_fields` fails with `unknown task type` (`task_type.rs:294`). Removing a pack would make its private items visible to agents, end their immutability, and make them impossible to edit.
- **Extension slot.** `Core::set_extension` and `extension<T>()` (`lib.rs:223-236`, `state.rs:191`, `state.rs:350`) are how halp-integrations attaches with a `Weak<Core>` (ARCHITECTURE.md:40). Tier B uses the same pattern.
- **Policy.** `Actor::Module(String)` already exists and serializes as `module:<id>` (`halp-policy/src/actor.rs`). The engine allows a module T0 of class `general` and confirms everything else (`halp-policy/src/engine.rs:228-229`, `:439-445`). Other classes are denied for modules (`engine.rs` `allows_class`, the `_ =>` arm). **Decision: the "ext:<id>" actor of the brief is `module:<pack_id>`.** No new actor kind.
- **Non-undoable admin.** Policy-state events are not undoable, so an agent cannot `history.undo` a revoke (ARCHITECTURE.md:80). Pack state must follow this.
- **Prompts** are embedded at build time by `halp-mcp/build.rs` and served from `Arc<Vec<PackPrompt>>` (`halp-mcp/src/server.rs:64,77,572-601`).
- **Frontend.** Widget kinds are a compiled map (`hub/app/src/lib/widgets/registry.ts:76-110`). An unknown kind gets `FALLBACK` (`registry.ts:122-127`), and Grid renders via `resolveWidget` (`app/src/lib/layout/Grid.svelte:149`). Per-kind config schemas live only in TS (`widgets/dev/config.ts`), so the core does not validate widget config by kind.
- **Shell.** The CSP has `frame-src 'none'`, `script-src 'self'`, `connect-src ipc: http://ipc.localhost` (`app/src-tauri/tauri.conf.json:18-24`). The single capability `hub-window` allows `dispatch` (runs as user) and token commands (`capabilities/hub-window.json`). Navigation is limited to app origins (`src-tauri/src/nav.rs:7`, wired at `src-tauri/src/lib.rs:193`). The invoke handler list is at `src-tauri/src/lib.rs:392`. Tauri is 2.12.5 and wry 0.57.2 (`app/src-tauri/Cargo.lock`).
- **Dependencies in `hub/Cargo.lock`:** `ring 0.17.14` (transitive, has Ed25519), `sha2 0.11.1`, `semver 1.0.28`, `jsonschema 0.61.1`, `base64 0.23.1`, `hex 0.4.3`, `walkdir 2.5.0`. **Not present:** wasmtime, ed25519-dalek, minisign, zip, tar, notify.
- **Workspace lints:** `unsafe_code = "forbid"`, `unwrap_used = "deny"` (`hub/Cargo.toml`). Our code needs no unsafe; wasmtime's safe `Memory::data(_mut)` API suffices.

## 1. Principles

1. **Only the user installs, updates, enables, trusts keys, or turns on dev mode.** These commands are `user_only`, `mcp:false`, and recorded as **non-undoable** events.
2. **The database is the source of truth for installed packs.** Bytes are copied into SQLCipher at install, and the hash is re-checked on every load. Files on disk, other than the explicit dev folder, are only an import source. An agent with a shell (§8a, 2026-11-10) must not be able to change an installed pack.
3. **Trust controls capability, never the policy.** Every extension call goes through `Core::dispatch` as `module:<pack_id>`, under the same engine, class filtering, audit and taint rules as an agent.
4. **Tier A stays data.** Everything that runs code (B, C) needs a signature from a trusted key and declared capabilities shown at install.
5. **Fail closed.** A pack that fails verification or validation is not loaded. Its role disappears, its types become *retired* (§3.7), and nothing else breaks.

## 2. Trust levels

| Trust | Source | Tier A | Privileged annotations¹ | Tier B | Tier C | MCP prompts |
|---|---|---|---|---|---|---|
| `builtin` | compiled in (`packs.rs`) | yes | yes | n/a | n/a | yes |
| `first_party` | signed by a compiled-in HalpTask key | yes | yes | yes | yes | yes |
| `community` | signed by a key the user trusted (§4.3) | yes | yes, shown at install | yes | yes | yes, labelled |
| `unsigned` | no or unknown signature, user accepted | yes | **no** (load fails) | no | no | yes, labelled |
| `dev` | dev folder (§3.8) | yes | ignored, with a warning | yes, `general` reads only | yes, DEV frame | **no** |

¹ `x-halp-immutable-when`, `x-halp-ext-writable`. `x-halp-data-class` is allowed at every trust level because it only restricts. It must name a built-in class or a class in the pack's own namespace (§3.1).

Implementation: replace `TaskTypeOrigin::Pack` with `Pack { trust: PackTrust }` (serialized `origin: "pack"` plus `trust`, keeping CONTRACT v1.3 `types.list` compatible), and add `Retired`. Add `TaskTypeOrigin::may_annotate() -> bool` (true for `Builtin`, `Pack{builtin|first_party|community}` and `Retired`), and use it at `immutable.rs:98` and `integrations.rs:1084` instead of `!= User`.

## 3. Tier A: declarative packs

### 3.1 Identity and naming
- **Pack id:** `^[a-z][a-z0-9-]{0,31}(\.[a-z][a-z0-9-]{0,31}){1,3}$`, ≤ 64 characters (fits `simple_id` and so `module:<id>`). The first segment is the **namespace** (`acme`).
  - Namespaces reserved for first-party: `halp base dev em cso ext x`.
  - Built-ins get package ids `halp.developer`, `halp.eng-manager` and `halp.cso`; their role ids stay as they are.
- **Role:** the manifest has `role.id`, which must match the role regex, must not equal a built-in role id, and must be unique among installed packs (`conflict` otherwise).
- **Type ids:** must start with `<namespace>.`. Existing collision checks stay (`task_type.rs:224-238`).
- **Data classes:** a built-in class (`policy::is_data_class`) or `<namespace>.*`.
- **Tier C widget kinds:** `x.<pack_id>.<name>`, ≤ 64 characters. **Tier B transforms:** `<pack_id>/<name>`.
- **Version:** semver (`semver` crate, already in the lock). Each installed id has one active version, and the previous version is kept for rollback.

### 3.2 Package format
A pack is a directory or a single-file bundle `*.halppack`. The bundle is JSON with no archive dependency:
`{ "format": "halp-pack/1", "manifest": "<exact text of halp-pack.json>", "signature": {"alg":"ed25519","key_id":"<16 hex>","sig":"<base64>"}|null, "files": {"<path>": "<base64>"} }`

Manifest `halp-pack.json` (`deny_unknown_fields` everywhere):
```
{ "format": 1, "id", "version", "title", "description", "license", "homepage"?,
  "publisher": {"name"}, "min_hub": "0.x",
  "extends": {"id", "version": "<semver req>"}?,
  "role": {"id", "title", "accent"},
  "widgets": {"add": [kind], "remove": [kind]},          // offered kinds
  "layout": Layout? | "layout_patch": [op]?,            // exactly one, or neither when extending
  "task_types": [TaskType], "type_overrides": [override],
  "migrations": [migration],
  "prompts": {"add": ["prompts/x.md"], "remove": ["name"]},
  "modules": [module],        // Tier B, §5
  "ui_widgets": [ui_widget],  // Tier C, §6
  "files": {"<path>": {"sha256": "<hex>", "size": n}} }
```

Limits:
- Bundle ≤ 8 MiB, ≤ 256 files, each file ≤ 2 MiB, manifest ≤ 64 KiB.
- Paths are relative, `[a-z0-9_./-]`, no `..` or empty segments, depth ≤ 6.
- Every file in the package is listed exactly once, and nothing else is present.
- Manifest text goes through `limits::check_json_text` (CONTRACT v1.9 note 7).

### 3.3 Inheritance and merge rules (`extends`)
Single inheritance, chain depth ≤ 4, no cycles. The parent must be installed (it may be disabled as a role). A child's trust never raises its parent's, and a child is **not** blocked by a less trusted parent. An unsigned child of `halp.developer` is fine. The resolver (pure, in `halp-pack`) produces today's `Pack` struct plus prompts, so the rest of the core is unchanged.

| Part | Rule |
|---|---|
| title, accent | child replaces |
| offered widgets | `parent ∪ add − remove`; removing a kind the parent's layout uses is fine if the child also patches the layout |
| layout | `layout` replaces it entirely; otherwise `layout_patch` ops in order: `{"op":"set","id","patch":{x?,y?,w?,h?,title?,config?}}`, where `config` is an RFC 7386 merge patch onto the parent's config; `{"op":"remove","id"}`; `{"op":"add","widget"}`. The result must pass `Layout::validate` and the kind check (`packs.rs:70-77`) |
| task_types | the child adds types in its own namespace; parent types are inherited by reference, not copied |
| type_overrides | **additive only** on a parent type: `add_properties` (never added to `required`), `add_enum: {prop: [values]}`, `title`, `icon`, `display`, property `description`. No `x-halp-*` key may be added, removed or changed. Two enabled packs overriding the same type is a `conflict` on enable. The override applies globally because types are global. Existing items stay valid because every op only widens |
| prompts | by `name`: the child replaces, `remove` drops; prompts belong to the child's role |
| modules, ui_widgets | not inherited (code belongs to its signer) |

**Fork** = `packs.fork {parent_id, new_id}` writes a skeleton child (`extends` set, empty overrides) into the dev folder. Publishing it means signing it (§4) and installing it.

### 3.4 Install, update, remove (user only)
1. **Preview.** The UI reads a file with `<input type="file">` (no fs capability or dialog plugin is needed) and calls `packs.preview {bundle_base64}`. The core verifies the bundle (§4.2), resolves the chain, validates the effective pack, dry-runs migrations and diffs against the installed version. It returns a report and a `preview_id` bound to the bundle's SHA-256. The report contains the id, version, trust, key fingerprint, file list, chain, effective role, types, privileged annotations, data classes, modules and their inputs, ui_widgets and their calls, prompts, the **permission diff**, migration counts and warnings. Previews live in memory for 10 minutes.
2. **Install or update.** `packs.install {preview_id, trust_key?: bool}` is T3, so the UI confirms in place (CONTRACT v1.3 note 1). It installs exactly the previewed bytes. An update that adds any capability (a new module, a call, an annotation, a data class, a prompt) shows the diff and needs this consent again (PLAN §4). A downgrade is refused; `packs.rollback {id}` is the only way back.
3. **Commit.** One non-undoable event `packs.install` does the following: stores the manifest and blobs, seeds the new role's layout (same rule as `seed`, `state.rs:546-580`), applies migrations, and reloads. On update, the user's layout is **never** overwritten. The Packs screen offers "Reset layout to pack default" (`packs.reset_layout`, T1).
4. **Disable and enable.** `packs.disable` is T1, `packs.enable` is T3. While disabled, the role is hidden, types become retired, modules and widgets are off, and prompts are withdrawn.
5. **Uninstall.** `packs.uninstall {id}` is T3. Its types become retired, its role and layout are removed, and its blobs are deleted. Packs that extend it block the uninstall (`conflict`, with their ids listed).

### 3.5 Store (halp-store, next schema version, forward-only as at `store.rs:129`)
- `packs (id PK, data)`: `{id, version, trust, key_id|null, enabled, manifest_text, manifest_sha256, file_index{path:{sha256,size}}, installed_at, previous: {version, manifest_text, file_index, migration_event_id}|null}`.
- `pack_blobs (sha256 PK, bytes BLOB)`: content-addressed, reference-counted by the packs.
- `pack_keys (key_id PK, data)`: `{key_id, pubkey_b64, label, kind: "community", pinned_packs: [id], added_at, revoked_at|null}`.
- `retired_types (id PK, data)`: a `TaskType` snapshot, plus `{from_pack, retired_at}`.
- New `EntityKind`s: `Pack`, `PackKey`, `RetiredType` (`store.rs:26`). Blobs are kept outside the event payload; events record `{id, version, manifest_sha256}`.

### 3.6 Load path
`Core::open` loads built-in packs, then the enabled installed packs in dependency order. Each pack's blobs are re-hashed against `file_index` and the manifest against `manifest_sha256`; the chain is resolved and validated. Retired types fill in last, with origin `Retired`, but only for ids that no live pack defines.

`State` gains `pack_meta: BTreeMap<String, PackMeta>` (trust, enabled, chain) next to `packs`. A pack that fails to load is skipped with a warning (`State::warnings`) and a `packs.status` event. Its types become retired in memory, so class and immutability still hold.

### 3.7 Type changes and item migration
- **Retired types.** When a type leaves the registry (pack uninstalled, disabled, or the type dropped in an update), its last schema is written to `retired_types` in the same event. A retired type:
  - validates fields and keeps its `x-halp-data-class` and `x-halp-immutable-when` (this closes the uninstall gap);
  - refuses `tasks.create` with that `type_id` (`invalid_args: type … is retired`);
  - allows the user to edit and to set `type_id: null` (the existing clear). Clearing to null needs empty `fields`, so the UI offers "Convert to plain task (drops N fields)" as a T3 action.
- **Migrations** (pack's own types only). `{"type", "from": "<semver req of the old pack version>", "ops": [...]}` with ops:
  - `rename {from,to}`
  - `default {field,value}` (sets the field only when absent)
  - `drop {field}`
  - `map {field, values:{old:new}}`
  - `retype {to}` (to another type of the same pack)

  Ops are declarative and run in `halp-pack`; no code is involved. The preview applies them to every item of the type and validates against the new schema. **If any item would fail, the update is refused** and the preview lists the item ids. Items are never half-migrated.
- **Locked items** (`immutable::locked`) are never migrated. An update that would change one is refused.
- **Commit.** The migration is part of the non-undoable `packs.install` event, with inverses kept. `packs.rollback` reinstalls `previous` and applies that event's inverse in one event, refused with `conflict` if any migrated item changed since. Writer and taint bookkeeping: a migration writes as `module:<pack_id>`, so agents reading migrated items are tainted by it (ARCHITECTURE taint rules).

### 3.8 Dev mode
- `packs.dev_set {path|null}` is T3 and user only. The path is **not persisted**, so a restart turns dev mode off. When set, the shell polls the folder every second (mtime and size walk with `walkdir`, ≤ 512 files, ≤ 8 MiB) and calls `packs.dev_reload` as the user. No `notify` dependency is needed.
- Each subfolder with a `halp-pack.json` loads with trust `dev`, id `dev.<id>`, role id `dev-<role>`. Dev packs are never written to the store and are not signed.
- Dev packs follow the `dev` row of §2. Their module actor is `module:dev.<id>`. Their prompts are not served over MCP (an agent could have written them).
- While dev mode is on, the app chrome shows a fixed, non-dismissable "DEV MODE: <path>" strip outside any widget. Tier C frames from dev packs get a striped DEV border.
- Threat note: assume an agent with a shell can write the dev folder. Nothing in dev mode exceeds what that agent could already read over MCP with the `general` grant.

## 4. Signing and trust

### 4.1 Crypto
- Ed25519 via `ring` 0.17 (`ring::signature::ED25519`, `Ed25519KeyPair::generate_pkcs8`). It is already in `hub/Cargo.lock` at 0.17.14, so adding it as a direct dependency of `halp-pack` adds no new crate.
- **Deviation from PLAN §4** ("cosign or minisign"): minisign uses a BLAKE2b prehash and its own text format, and neither crate is in the lock. A minisign import can come later.
- Signed message: `b"halp-pack-sig/1\0" || manifest_bytes`, the exact bytes with no canonicalization. File integrity comes from the manifest's `files` SHA-256 entries (`sha2` 0.11, already a workspace dependency).
- `key_id` = the first 16 hex characters of the SHA-256 of the raw 32-byte public key. The UI shows the full fingerprint as 4 groups of 16 hex characters.

### 4.2 Verification order (`halp_pack::verify`)
1. Size, path and duplicate checks.
2. Every file hash matches its `files` entry, and the sets are equal.
3. Signature: if present, the key must be in the compiled-in first-party set, or in `pack_keys` and not revoked; otherwise the trust is `unsigned`. A bad signature is an error, never a downgrade to unsigned.
4. Revocation: the compiled-in list `REVOKED_KEYS` / `REVOKED_PACK_HASHES` (updated with app releases) refuses the bundle.
5. **Key pinning:** an update must be signed by the key pinned for that id at first install. A different key is `denied` ("signed by a different publisher"). Unsigned updates of a signed pack are refused.

### 4.3 Trust store
- **First-party keys:** `const FIRST_PARTY_KEYS: &[(&str, [u8;32])]` in `halp-pack`. They cannot be removed in the app.
- **Community keys:**
  - `packs.keys_add {pubkey_b64, label}` (T3).
  - `packs.keys_list` (T0, user only).
  - `packs.keys_revoke {key_id}` (T3): disables every pack signed with that key and retires their types.
  - Or trust on first use: `packs.install {preview_id, trust_key:true}` adds the key with a label taken from the manifest's publisher, after the user has seen the fingerprint.
- **Publisher tooling** (new `halpd pack` subcommand): `keygen --out key.pk8` (file mode 0600), `sign <dir> --key key.pk8 --out x.halppack`, `verify <bundle>`. The Hub never holds a signing key.

### 4.4 Agents
Agents can never install, update, enable, trust keys, fork or set dev mode: those commands are `user_only`, `mcp:false`, and their events are non-undoable. An agent's `history.undo` cannot reach them (the same rule as `clients.*`, ARCHITECTURE.md:80). The socket user channel does **not** accept any `packs.*` write (like `grants.set`, CONTRACT v1.6 note 6).

## 5. Tier B: sandboxed logic (wasmtime)

### 5.1 Prototype scope (P5): pure transforms only
`modules: [{ "name", "wasm": "wasm/x.wasm", "inputs": {"<key>": {"command": <T0 command>, "args": {...}}}, "output_schema": {...} }]`

- Allowed input commands: `tasks.list`, `tasks.get`, `types.list`, `types.get`, `day.state`, `focus.get`, `layout.get`.
- The host runs each input through `Core::dispatch` as `module:<pack_id>`, so the policy, class hiding and audit apply. Today's engine allows module T0 only for `general` (`engine.rs:439-445`), so a module never sees private classes.
- The module gets `{inputs, config, now, today}` and returns JSON. The output is checked against `output_schema`, ≤ 256 KiB, depth ≤ 32, and `check_json_text`.
- Output goes **only to the UI**, to a new first-party widget kind `computed` (`config: {transform: "<pack>/<name>", view: "metric"|"list"|"series"|"table"}`). It never goes to agents or into storage.

### 5.2 ABI (core wasm, no component model in P5)
- **Exports:** `memory`, `halp_alloc(len: i32) -> i32`, `halp_transform(ptr: i32, len: i32) -> i64` (`ptr << 32 | len`).
- **Imports:** only `halp.log(ptr, len)`, capped at 4 KiB per run and sent to the dev console only. Instantiation fails on any other import, so there is no WASI.
- No clock, randomness, filesystem or network import.

### 5.3 Limits and engine config
- Engine settings:
  - `Config::consume_fuel(true)`, 10M fuel per run;
  - `epoch_interruption(true)` with a 200 ms deadline from a ticker thread;
  - `StoreLimitsBuilder`: memory 16 MiB, 1 instance, 1 memory, 10k table elements;
  - `max_wasm_stack(512 KiB)`;
  - threads, multi-memory, memory64 and the component model all off.
- The module file is ≤ 2 MiB.
- Compiled modules are cached **in memory only**, keyed by blob SHA-256. The wasmtime on-disk `cache` feature stays off, because an agent with a shell could tamper with an on-disk cache.
- Runs execute on a 2-thread pool in `halp-ext`, **never under the core lock**: inputs are dispatched first, then the module runs.
- Debounce 2 s per widget, and re-run only on `halp://event` changes. This also keeps module T0 reads from flooding `audit_log`, which records non-user T0 reads (ARCHITECTURE "User and system T0 … not audited").

### 5.4 Crate, dependency and wiring
- New crate `hub/crates/halp-ext`. Its dependency is `wasmtime` (**new**, not in the lock). Pin it exactly (`=X.Y.Z`, like `rmcp`) at the current stable release, with `default-features = false, features = ["cranelift", "runtime", "std"]`, and add it to cargo-deny/audit.
- It attaches with `Core::set_extension(Arc<ExtHost>)` and holds a `Weak<Core>`, as halp-integrations does.
- The shell exposes `ext_run_transform {pack_id, name, widget_id}`. It runs the module as `module:<pack_id>`; the user only triggers the run.
- The policy change in `engine.rs:439-445` is for `Module`: non-T0 calls are **denied, not confirmed**, so a module can never raise consent prompts. Modules get T1 later, via declared capabilities.

### 5.5 Cut for the prototype (defer)
Host `call` imports, writes (T1), computed fields stored or evaluated on read, automations and timers, connectors with network egress (they need manifest-declared hosts, an HTTP host function and secrets, which is closer to Tier D), WIT/component model, and module grants for non-`general` classes.

## 6. Tier C: custom UI (sandboxed iframe)

### 6.1 Feasibility (Tauri 2.12)
- **Possible today:** a custom scheme via `Builder::register_asynchronous_uri_scheme_protocol("halpext", …)`, plus an iframe with `sandbox="allow-scripts"` (opaque origin). On macOS and Linux the URL is `halpext://<pack_id>/<path>`; on Windows it is `http://halpext.localhost/<pack_id>/<path>`.
- **Blocked by today's CSP:** `frame-src 'none'` (`tauri.conf.json:24`, `:36`).
- **Open questions, which make a spike gate (P5-S) mandatory before any Tier C code:**
  1. Whether `__TAURI_INTERNALS__` or init scripts are injected into subframes. WebView2 runs document-created scripts in all frames.
  2. Whether IPC from a `null`-origin frame is refused.
  3. Whether `on_navigation` (`lib.rs:193`) fires for subframes and would block the iframe or let it navigate.
- **Required spike result**, from a red-team widget on all three OSes: no `__TAURI_INTERNALS__`, `fetch('http://ipc.localhost/…')` and `ipc:` both fail, `parent.document` throws, `top.location=` and `window.open` fail, `<a target=_top>` fails, and form submission fails.
- **Fallback if the spike fails:** render Tier C in a separate child webview or window with **no capability** and the same scheme. That costs more and gives poorer layout, but it is isolated by Tauri's capability model.

### 6.4 P5-S spike result (2026-11-11)

**Verdict: the iframe is OK with conditions.** On macOS this is verified by running the spike, and on Linux it rests on reading the source. **Windows (WebView2) fails the spike's own "no `__TAURI_INTERNALS__`" criterion by source.** Plan to use a child webview with no capabilities there unless a WebView2 run of this spike shows otherwise. The conditions are listed below.

Spike path: `/private/tmp/claude-501/-Users-kenth-code-halptask/7ee8b799-03b7-45ec-a092-f4dbbd5947bd/scratchpad/p5-spike`. It is a standalone Tauri app pinned to tauri =2.12.5, tauri-build =2.7.4 and wry 0.57.2 (confirmed with `cargo tree`), using a copy of the Hub's Cargo.lock. It runs with plain `cargo build`; no tauri-cli was installed. Files:
- `src/main.rs`: the halpext handler, `report`, the invoke and navigation logging, and auto-quit.
- `dist/host.js`: the ExtFrame-style handshake over a MessageChannel.
- `packs/redteam/widget.js`: the attacks.
- `results/run*.json`: raw results. `python3 -I summarize.py <file>` prints them.

Env knobs: `SPIKE_CORP=0|1`, `SPIKE_NAV=hub`, `SPIKE_SECS`, `SPIKE_OUT`. Each run takes about 70 s and quits itself.

Three packs were served:
- `redteam`: the full §6.2 headers.
- `weak`: the same files with no CSP header (iframe sandbox attribute only).
- `otherpack`: a secret.json, a script and an image, as the cross-pack target.

The `redteam` probe also ran with no `sandbox` attribute, so only the CSP `sandbox` header applied.

#### macOS (WKWebView) per attempt, strict pack, `sandbox="allow-scripts"`
Source runs: run3 and run4 with CORP off. Every attempt passed.

1. **`__TAURI_INTERNALS__`, `__TAURI__`, `isTauri`, `window.ipc` absent: PASS.** All were false, and no global name matched tauri, ipc or invoke. Note: `window.webkit.messageHandlers.ipc` **is present** in the subframe.
2. **Invoke via internals: PASS**, since there are none. As an extra attempt, the widget sent `webkit.messageHandlers.ipc.postMessage` with a guessed key. It **reached Rust** and was rejected only by the invoke-key check. Stderr showed `__TAURI_INVOKE_KEY__ expected … but received bogus-key-from-frame`, once per probe frame (tauri `src/webview/mod.rs:2010-2023`).
3. **`fetch('ipc://localhost/report')`: PASS.** It failed with "Load failed" because CSP `connect-src 'none'` blocked it. Without the CSP header (`weak` pack) it reached Tauri and got `500 "Origin header is not a valid URL"`, because a sandboxed frame sends `Origin: null` (`src/ipc/protocol.rs:488-495`).
4. **`fetch('http://ipc.localhost/…')`: PASS.** "Load failed" in all variants.
5. **`parent.document`, `top.document`, `parent.location.href`, `parent.__TAURI_INTERNALS__`: PASS.** Each threw `SecurityError: Sandbox access violation`.
6. **`top.location = 'tauri://localhost/pwned.html'`: PASS.** It threw `SecurityError: The operation is insecure.` The host stayed alive and on_navigation was never called.
7. **`window.open`, with `_blank` and `_top`: PASS.** Both returned `null`, and `on_new_window` was never called.
8. **`<a target=_top>` click: PASS.** Blocked silently, with no navigation event and the host alive.
9. **Form submit to `_top`: PASS.** Blocked even in the `weak` pack, where only the sandbox applies.
10. **`parent.postMessage` spoofing `ui/initialize` and `tools/call`: delivered, as postMessage always is.** The host saw `origin "null"` and `source === iframe.contentWindow`. That is identical to a real handshake retry, so the host must never give window messages any authority. It cannot forge anything into the host's MessagePort.
11. **The frame navigating itself to `tauri://localhost/pwned.html`, `/index.html` or `http://127.0.0.1:9`: PASS.**
    - None of them reached on_navigation, and no pwned or host-in-frame report arrived.
    - `href` was unchanged 500 ms later.
    - A second `load` event fired, probably an error page.
    - Self-navigating to `halpext://otherpack/secret.json` **was allowed**, because on_navigation lets halpext through. The frame then showed another pack's document, but the attacker cannot read it.
12. **Reading other packs' files: PASS with the CSP header.** fetch, XHR, `<img>` and `<script>` to `halpext://otherpack/…` were all blocked. **It FAILS without the CSP header:** in the `weak` pack the image loaded and `evil.js` loaded and ran. fetch and XHR were blocked by CORS in both packs.
13. **Other checks:**
    - `self.origin` was `"null"`, `localStorage` threw `SecurityError` and `document.cookie` was `""`.
    - The frame with no sandbox attribute behaved the same, so WKWebView honours the `sandbox` directive in the CSP header of a custom-scheme response.

**Rust side:**
- IPC reports not from the host's main frame: 0 in every run. All accepted invokes (62 in run4) came from the host.
- **on_navigation fires for subframes.** Every iframe `src` was logged. With the Hub's exact `nav::is_app_url` policy, the iframe was denied and never loaded (run5, `SPIKE_NAV=hub`).

**Unexpected finding: `Cross-Origin-Resource-Policy: same-origin` breaks widgets.** With CORP on (runs 1, 2 and 6), the first mount worked. Every later mount loaded `index.html`, but `widget.js` came from WebKit's memory cache and was blocked, so the widget never ran: `widget.js` was requested once and nothing came back after that. A sandboxed requester has an opaque origin and never matches `same-origin`.

#### Source analysis for other platforms
**Why the invoke key is the last line of defence (all platforms):** `is_local_url` treats any registered custom scheme as **local** (`tauri-2.12.5/src/webview/mod.rs:1961-2002`; the macOS/Linux check is at `:1984`, the Windows `<name>.localhost` check at `:1987-1997`). So a halpext frame holding the invoke key gets the **main window's capability** (`Origin::Local`, `:2044-2058`). The key is one per app, 16 random bytes (`lib.rs:1281`, `manager/mod.rs:215`), and lives only inside the init script (`scripts/ipc-protocol.js:12`).

**Tauri's own init scripts are all main-frame-only:** `manager/webview.rs:161-220`, plugin `js_init_script` at `plugin.rs:81, 371`.

**WebKitGTK: likely the same as macOS. High confidence on injection, medium on navigation.**
- `UserContentInjectedFrames::TopFrame` applies when main-only is set (`wry-0.57.2/src/webkitgtk/mod.rs:826-840`), and wry's `window.ipc` shim is main-frame-only (`:352`). So no internals and no key in the iframe.
- The IPC URL is the top-level `webview.uri()` (`:743`). The message handler (`:738, :755`) is probably reachable from subframes, so as on macOS the key check is the barrier.
- `decide-policy` NavigationAction has no main-frame filter (`:563-589`), so on_navigation should fire for iframes, as on macOS.
- Custom-scheme response headers are applied (`web_context.rs:237`), so the CSP header should hold.

**WebView2: fails criterion 1. High confidence on injection, medium on the IPC path.**
- wry documents that "scripts are always added to subframes regardless of the `for_main_frame_only` option" (`lib.rs:1029`), and every init script uses `AddScriptToExecuteOnDocumentCreated` (`webview2/mod.rs:516-518, 1413-1420`). So **the iframe gets `__TAURI_INTERNALS__`, a working `invoke`, and wry's `window.ipc` shim** (`:965-969`).
- The key can be stolen: the widget stubs `window.chrome.webview.postMessage` and calls `invoke`. The fetch fails under CSP, so `ipc-protocol.js:60-82` falls back to `window.ipc.postMessage(data)`, and `data` contains `__TAURI_INVOKE_KEY__`.
- Delivery is still probably blocked:
  - The custom-protocol path is stopped by CSP `connect-src 'none'`, and if not, by `Origin: null` (`protocol.rs:488-495`).
  - For the postMessage path, wry only subscribes to `CoreWebView2.WebMessageReceived` (`:972`) and never to `ICoreWebView2Frame`. Per Microsoft's docs (from memory, not checked here), that event fires only for the top-level document. Medium confidence.
- Navigation: wry hooks only `NavigationStarting` (main frame, `:756`) and never `FrameNavigationStarting`. So on_navigation does **not** see iframe navigations, and a widget can navigate its own frame anywhere, which is an exfiltration path.

#### Conditions for "iframe OK"
These settings worked on macOS:
1. **App CSP:** as in the Hub, with `"frame-src": "halpext:"` (add `http://halpext.localhost` for Windows). The capability file is unchanged (main window only), and there is no `remote` capability for halpext.
2. **Handler response headers:**
   - `Content-Security-Policy: default-src 'none'; script-src halpext://<pack>; style-src halpext://<pack> 'unsafe-inline'; img-src halpext://<pack> data:; font-src halpext://<pack>; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; sandbox allow-scripts`
   - `X-Content-Type-Options: nosniff` and `Referrer-Policy: no-referrer`.
   - **Drop `Cross-Origin-Resource-Policy: same-origin` from §6.2.**
   - The CSP must be on every response, HTML first of all. The handler cannot tell which pack is asking: navigations and no-cors loads have no Origin or Referer, and CORS loads send `Origin: null`. So isolation between packs comes only from the exact-host CSP lists.
3. **iframe:** `sandbox="allow-scripts"`, which is not redundant with the header sandbox.
4. **on_navigation:** the Hub's `nav::is_app_url` must also allow `halpext://`, otherwise the iframe never loads. Allow halpext only, never other schemes.
5. **ExtFrame:** treat any `load` event after the first as a navigation and tear the frame down. The widget can load another pack's halpext page into its own slot.
6. **Window messages:** carry no authority, as §6.3 says; a spoofed message is indistinguishable from a handshake retry.
7. **Init scripts:** never add an all-frames script (`*_on_all_frames`), and check each new plugin for one. A key leak equals the main window's capability, because halpext counts as local.
8. **Windows:** do not ship the iframe until this spike passes on WebView2. By source it exposes `__TAURI_INTERNALS__` and does not police frame navigation, so fall back to a child webview or window with no capability there.

#### Caveats
- These were debug builds with devtools on; the ACL path is the same in release.
- WebView2 and WebKitGTK were not run. Their findings come only from reading the source.
- Background throttling was turned off in the spike so its timers would run.

**Lead decision (2026-11-11), based on this spike:**
- Tier C uses the sandboxed iframe on **macOS** (verified) and **Linux** (by source), with every condition below. P5-D implements all of them.
- **On Windows, Tier C is switched off in this prototype.** The pack installs, but its `x.*` widgets render a "not available on Windows yet" placeholder. Reason: WebView2 injects `__TAURI_INTERNALS__` and the invoke key into subframes and does not report frame navigations. A leaked key carries the main window's capability, because `halpext` counts as a local URL.
- A child webview with no capabilities is the follow-up for Windows, and must be verified on a WebView2 machine.
- `Cross-Origin-Resource-Policy` is dropped from §6.2: it breaks widgets on every mount after the first.

### 6.2 Serving
The scheme handler serves only files listed under a `ui_widgets[].root`, from verified blobs (hash re-checked).

- Content types come from an allow-list by extension: `html js mjs css png woff2 json`. Everything else gets 404 (SVG is left out because it can run script as a document; see PLAN §8a).
- Headers on every response:
  - `Content-Security-Policy: default-src 'none'; script-src <self-url>; style-src <self-url> 'unsafe-inline'; img-src <self-url> data:; font-src <self-url>; connect-src 'none'; frame-src 'none'; form-action 'none'; base-uri 'none'; sandbox allow-scripts`, where `<self-url>` is the exact `halpext://<pack_id>` or `http://halpext.localhost` URL. The opaque origin means `'self'` does not match.
  - `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Resource-Policy: same-origin`.
- **App CSP change:** `frame-src halpext: http://halpext.localhost`, in both `csp` and `devCsp`. Nothing else changes, and the capability file stays the same: the iframe gets no capability.

### 6.3 Host component and protocol
`ExtFrame.svelte` renders `<iframe sandbox="allow-scripts" allow="" referrerpolicy="no-referrer" src=…>` inside host chrome that shows the pack title, its trust badge (DEV stripe), and a "this widget is from <publisher>" tooltip.

**Handshake:**
1. On `load`, the host creates a `MessageChannel` and posts `{"jsonrpc":"2.0","method":"ui/initialize","params":{…}}` with `port2` to `iframe.contentWindow`, with target `'*'` (the origin is opaque). That message carries no data.
2. From then on only the port is used. Window `message` events are ignored, except handshake retries whose `event.source === iframe.contentWindow`.

**Messages** are JSON-RPC 2.0, shaped like MCP Apps where they overlap. Full MCP Apps compatibility, and running the same widget in Claude Desktop, is **not** a P5 goal; check method names against the MCP Apps spec when implementing.
- Host to frame:
  - `ui/initialize {theme tokens, ambient, privacy_blur, widget config}`
  - `ui/notifications/host-context-changed`
  - `ui/notifications/data-changed {entities}`, debounced and triggered by `halp://event`
- Frame to host:
  - `tools/call {name, arguments}`, with names in MCP tool form (`tasks_list`). The name must be in the widget's manifest `calls`, intersected with the host allow-list `{tasks_list, tasks_get, types_list, types_get, day_state, focus_get}` (all T0).
  - `ui/open-item {id}`: the host focuses the item in its own UI.
  - `ui/size-changed {h}`: clamped to the grid cell.
- Limits: each message ≤ 256 KiB, ≤ 20 calls/s per frame, and anything else gets JSON-RPC error `-32601`.
- **Relay:** the trusted Svelte host calls a **new** Tauri command `ext_call {pack_id, widget, name, arguments}`. Do not use `dispatch`, which runs as the user. In Rust, `ext_call` re-checks that the pack is enabled, its trust allows Tier C, and the call is in the allow-list from the installed manifest (not the request). It then dispatches as `module:<pack_id>`. Results are filtered the same way (`general` only).
- **Privacy blur:** when blur is on, the host sends `privacy_blur:true`, and the host also blurs the frame element itself, so it does not rely on the widget.
- **Residual risks:**
  - Exfiltration through DNS prefetch or CSS: the frame only ever holds `general` data an agent could read anyway.
  - In-frame phishing text: consent never renders in a frame, and the frame border names its publisher.

## 7. MCP surface

| What | Agents (MCP) | User only |
|---|---|---|
| `packs.list` T0 → `{packs:[{id, version, title, role_id, trust, enabled, extends, key_fp, capabilities:{modules, ui_widgets, prompts, annotations}, status}]}` | yes | |
| `packs.get {id}` T0 → manifest metadata + effective role + type ids (no file bytes, no dev path, no `previous`) | yes | |
| resource `halp://packs` (= `packs.list`), updated on `packs.*` events | yes | |
| prompts from enabled `builtin`/`first_party`/`community`/`unsigned` packs; descriptions of non-first-party prompts prefixed with `[community: <pack id>]` / `[unsigned: <pack id>]`; `notifications/prompts/list_changed` on change | yes | |
| `packs.preview`, `install`, `uninstall`, `enable`, `disable`, `rollback`, `reset_layout`, `fork`, `dev_set`, `dev_reload`, `keys_*`, `migrate_preview` | no (`denied`, `mcp:false`) | yes |
| `ext_call`, `ext_run_transform` (Tauri only) | no | n/a (run as `module:`) |

Agents may still add a `x.*` or `computed` widget to a role that offers it (`layout.add_widget`, T1), because the user already installed it. Built-in prompts stay compiled in (`halp-mcp/build.rs`). Pack prompts come from a new `Core::pack_prompts()` that halp-mcp merges at list time; the server's `Arc<Vec<PackPrompt>>` (`server.rs:64`) becomes a lookup that also asks the core.

## 8. Work breakdown

Each piece's file ownership is **exclusive**. Shared single lines are named; their owner adds them on request.

| Piece | Model | Owns (exclusive) | Depends on |
|---|---|---|---|
| **P5-S** Tier C isolation spike | opus | throwaway branch, report appended to this doc §6.1 | none (wave 0) |
| **P5-A** `halp-pack` crate | opus | `hub/crates/halp-pack/**` (new), `hub/Cargo.toml` (members, `ring = "0.17"`, `semver = "1"`, `wasmtime` entry for P5-C), `hub/crates/halpd/src/pack.rs` (new) + one dispatch line in `halpd/src/main.rs` | none (wave 1) |
| **P5-C1** `halp-ext` host | opus | `hub/crates/halp-ext/**` (new) | none (wave 1); trait `InputSource` stubbed |
| **P5-E1** Packs UI vs mock | sonnet | `app/src/lib/settings/PacksSection.svelte`, `InstallReview.svelte`, `KeysSection.svelte` (new), `app/src/lib/api/mock/packs.ts` (new), packs types appended to `api/types.ts` | §3/§4/§7 shapes (wave 1) |
| **P5-B** core + store wiring | opus | `halp-store/src/store.rs` (migration, EntityKinds), `halp-domain/src/task_type.rs` (origin/trust, `Retired`), `halp-core/src/packs.rs`, `state.rs`, `immutable.rs:98`, `commands/integrations.rs:1084`, `commands/packs.rs` (new), `commands/mod.rs`, `lib.rs` (open path, `pack_prompts`), `docs/fixtures/commands.json`, CONTRACT v1.10 | P5-A (wave 2) |
| **P5-D** Tier C | opus (Rust/CSP) + sonnet (Svelte) | `app/src-tauri/**` (all of it: scheme, `ext_call`, `ext_run_transform`, dev poller `packs_dev.rs`, CSP, `nav.rs`, `lib.rs` handler list, capabilities), `app/src/lib/widgets/ext/**` (new: `ExtFrame.svelte`, `bridge.ts`, `Computed.svelte`), `registry.ts` (resolve `x.*` and `computed`) | P5-S, P5-B (wave 2) |
| **P5-C2** Tier B wiring | sonnet | `halp-ext` (wiring to `Weak<Core>`), `halp-policy/src/engine.rs` (module non-T0 → deny) | P5-B, P5-C1 (wave 2) |
| **P5-E2** MCP + docs + example | sonnet | `halp-mcp/src/prompts.rs`, `server.rs` (runtime prompts, `halp://packs`), `docs/MCP.md`, `docs/ARCHITECTURE.md`, `hub/examples/packs/acme-sre/**` (signed with a test key in `hub/examples/keys/`, test-only) | P5-B, P5-D (wave 3) |

**Acceptance and tests**
- **P5-S:** a red-team widget whose escape attempts (§6.1 list) all fail on macOS, Windows and Linux. A written verdict: iframe or child webview.
- **P5-A:**
  - Round trip with `keygen`, `sign`, `verify`.
  - Tampering is rejected: a changed byte, an extra file, a missing file, a `../` path, an oversize file, a bad signature (never downgraded to unsigned), a revoked key, a pinned-key mismatch on update.
  - Merge tests per row of §3.3, including a type override that tries to touch `x-halp-*` or `required` (rejected), cycle or depth > 4 (rejected), and a layout patch producing overlap (rejected).
  - Migration ops are pure and tested, and an unknown op is rejected.
  - Property test: for any additive override, every item valid before is valid after.
- **P5-B:**
  - Install, update, rollback and uninstall integration tests against a temp store.
  - **Gap regressions:**
    - an unsigned pack with `x-halp-immutable-when` or `x-halp-ext-writable` fails to load;
    - after uninstalling a pack with a private class, an agent still gets `not_found` for its items;
    - after uninstall, a locked item stays locked;
    - an agent's `history.undo` after `packs.disable` does nothing to pack state;
    - an agent calling any `packs.*` write gets `denied`;
    - a tampered blob in the DB means the pack is not loaded and its types are retired.
  - Migration refused when any item would fail.
  - Fixture regenerated.
- **P5-C1:**
  - A WAT test corpus: an infinite loop is stopped by fuel; memory growth past 16 MiB fails; a WASI import or any unknown import fails at instantiation; oversize output and bidi characters in output are rejected; a transform round trip works.
  - No disk cache is written (assert the temp dir is empty).
- **P5-C2:** a module's input as `module:x` is audited; a non-`general` item never reaches the module; a module T1 attempt is `denied` and nothing is parked.
- **P5-D:**
  - Playwright or WebDriver run of the P5-S widget in the built app; `ext_call` outside the allow-list is `denied`; `ext_call` for a disabled pack is `denied`; the scheme returns 404 for unlisted paths and for a hash mismatch.
  - The main webview's CSP is unchanged except `frame-src`.
  - The DEV strip is visible while dev mode is on.
- **P5-E1/E2:**
  - The install review shows the fingerprint, trust and the permission diff; approve is disabled until the preview is current.
  - A community prompt appears with its label, and a dev prompt does not appear.
  - The example pack extends `halp.developer`, adds a `computed` burn widget (Tier B) and an `x.acme.platform-sre.oncall` widget (Tier C).
  - **This example meets the PLAN P5 done-when:** a third party forks a pack and ships a sandboxed widget.

Reviews: the mandatory adversarial and security reviews (opus) run on the final code of each wave. Wave 2 must pass both before wave 3 merges.

## 9. CONTRACT v1.10 (to add with P5-B)
1. The `packs.*` commands of §7, their shapes and tiers; the `PackView` fields from §7.
2. `TaskType` origin: `"builtin"|"pack"|"user"|"retired"`, plus `trust` on pack types.
3. A retired type refuses create. `x-halp-immutable-when` and `x-halp-ext-writable` count only when `origin.may_annotate()`.
4. Events `packs.install|update|disable|enable|uninstall|rollback|keys.*` are non-undoable, as actor `user`. Migrations write as `module:<pack_id>`. Actor `module:<pack_id>` is the identity for Tier B and C. Modules: T0 `general` allowed, everything else `denied`.
5. Widget kinds `computed` and `x.<pack_id>.<name>`.
6. Tauri commands `ext_call` and `ext_run_transform`. They run as `module:` and are never offered to frames directly.

## 10. Risks and deferrals

**Risks**
1. **Tier C isolation (high).** WebView2 frame injection and IPC origin checks are unverified; this is why P5-S is a gate and the child-webview fallback exists.
2. **The trust gap and the uninstall gap of §0.** If P5-B ships without the retired types and origin trust, community packs gain lock powers and uninstall leaks private items. The P5-B regression tests are mandatory.
3. **Same-user tampering.** Installed bytes live in SQLCipher and are re-hashed on load; the dev folder is untrusted by design; there is no on-disk wasm cache.
4. **wasmtime weight and CVEs.** It adds build time and binary size, and is a large dependency. Pin it exactly, run cargo-audit, and keep it in its own crate so `halpd` and the core tests do not link it.
5. **Prompt injection through community prompts.** Prompts are shown in the install review, labelled over MCP, and never come from dev packs. Prompts are user-invoked in MCP clients, but they remain a channel.
6. **Type overrides are global.** Two packs overriding one type conflict at enable; overrides are additive only.
7. **Trust on first use for keys.** Pinning protects updates but not first contact. There is no revocation feed until a registry exists.
8. **Audit volume from module reads.** Mitigated by the debounce; watch `audit_log` growth.
9. **Widget config is not validated in the core.** It stays checked only in TS (`widgets/*/config.ts`). A bad pack config reaches widgets, which re-check every value.

**Defer past P5:**
- registry, review and a revocation feed;
- key rotation (`next_key`) and minisign import;
- Tier B host calls, writes, network and connectors, computed fields, automations;
- WIT and the component model;
- Tier C write calls, `ui/open-link` and clipboard;
- serving Tier C in Claude Desktop (full MCP Apps);
- themes beyond accent;
- zip/tar packages;
- type overrides beyond additive, and keeping failing items on an old schema;
- generating a shared widget-config schema fixture so the core can validate pack widget config.

### Critical files for implementation
- /Users/kenth/code/halptask/hub/crates/halp-core/src/packs.rs
- /Users/kenth/code/halptask/hub/crates/halp-core/src/state.rs
- /Users/kenth/code/halptask/hub/crates/halp-domain/src/task_type.rs
- /Users/kenth/code/halptask/hub/crates/halp-policy/src/engine.rs
- /Users/kenth/code/halptask/hub/app/src-tauri/tauri.conf.json and /Users/kenth/code/halptask/hub/app/src-tauri/src/lib.rs
