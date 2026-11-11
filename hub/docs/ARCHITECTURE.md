# HalpTask Hub architecture (core, policy and MCP)

## Crate map

```
halp-domain   pure types and rules, no I/O
   ^
halp-policy   actors, tiers, policy engine (grants, rules, taint, rate limits), secrets
   ^
halp-store    SQLCipher database, event log, audit log, undo/redo, keys, TUI importer
   ^
halp-core     registry + dispatch + policy enforcement + consent + events + packs
   ^
halp-mcp      MCP server (rmcp), Unix socket host, `halp-mcp` stdio shim
   ^
halp-integrations  connector runtime (GitHub, MCP client, local git, OAuth, keychain); a HostHandle extension
halpd         CLI: dispatch, import, schema, serve, clients, consent
app (Tauri)   invoke("dispatch") -> Core::dispatch; events -> "halp://event"; runs the MCP host and the integration runtime
```

`halpd` is its own crate, not `halp-core/src/bin`, because it needs `halp-mcp`, which depends on `halp-core`. That differs from PLAN.md, which puts halpd in halp-core.

- **halp-domain**: the data model and its rules.
  - `Item` and the `Outline` tree: ordering, moves, derived parent status, inherited tags, role membership.
  - `TaskFilter`; the `TaskType` registry with JSON Schema validation of `fields`.
  - `Layout`, `Widget` and `grid` (a port of `app/src/lib/layout/grid.ts`).
  - Capture-text parsing; natural-language due dates.
  - Input limits: title/notes/tag lengths and id formats.
- **halp-store**: persistence.
  - A `Store` is one SQLCipher database. Opening it takes an exclusive advisory lock on `hub.lock` next to the database (`std::fs::File::try_lock`), held until the store is dropped. A second opener gets `conflict: HalpTask Hub is already running; use the agent socket (halp-mcp) instead`, so the app and halpd never run two Cores on one database. Every `commit` also compares each changed row with the `before` it was computed from (compare-and-swap) and refuses with `conflict` on a mismatch.
  - `commit(NewEvent)` writes entity changes and the event in one transaction.
  - `undo`, `redo`, `undo_event` and `redo_event` apply stored inverses.
  - `KeyProvider`s supply the database key.
  - `import` reads the TUI's files.
- **halp-policy**: the policy engine and its model.
  - `Actor`, `Ctx` (with an optional session id) and `Tier`.
  - `Engine` implements `Policy`. It is loaded with the clients and rules from the store, and keeps taint, rate buckets and the circuit breaker in memory.
  - `secret`: random tokens and ids, SHA-256 token hashes, argument hashes, and constant-time comparison.
- **halp-mcp**: the MCP surface (see MCP.md).
- **halp-integrations**: the integration runtime (see INTEGRATIONS.md). It depends on halp-core and attaches itself with `Core::set_extension(HostHandle)`. Host callbacks run under the core lock, so they only spawn work. Syncs dispatch `ext.upsert` and `ext.status` from `spawn_blocking` as `integration:<id>`. The runtime holds a `Weak<Core>`.
- **halp-core**: the only entry point.
  - `Core::dispatch(ctx, request) -> response` takes the contract envelope.
  - Every command is a registry entry: name, description, tier, input and output JSON Schemas (schemars), and a typed handler.
  - Arguments are deserialized strictly, so unknown fields are `invalid_args`.

## Request flow

1. `Core::dispatch` looks up the command; an unknown name is `not_found`.
2. `consent.approve` and `consent.deny` are handled by the dispatcher itself (see Consent below).
3. `PolicyHook::check(ctx, descriptor, args)` returns one of:
   - `Allow`
   - `Deny(msg)`, which becomes the `denied` error
   - `RequireConfirmation`, which parks the request (see Consent)

   `consent.list` and `consent.get` skip the policy.
4. The handler runs under the core mutex against `State`, which holds the `Store` plus caches: the outline, the type registry, layouts, roles, the current role, per-role focus and pending consents.
5. A write handler goes through `State::write`, which works in these steps:
   1. It opens an outline journal.
   2. It lets the handler mutate the outline and record other entity changes (layouts, settings, types, legacy rows).
   3. It drops changes whose before equals their after.
   4. It commits the remaining changes with **one event**, then broadcasts the event on a `tokio::sync::broadcast` channel (`Core::subscribe`).
   5. If anything fails, the journal is rolled back and every cache is reloaded from the store, so memory never drifts from disk.

The policy is always the `halp_policy::Engine`. `AllowAll` exists only behind a test feature and is never installed by default. In order, the dispatcher:

1. syncs the engine from the store (for agents, and in `authenticate`), so a pairing or revoke written by another process counts on the next call;
2. decodes and validates the arguments;
3. computes the **effective tier** for these arguments (see MCP.md, "Policy");
4. computes the request's **data class** (`request_class` in `halp-core/src/lib.rs`): the class of every item the request names or would create, change, move under or delete, and of every type it defines. An agent without a grant for that class gets `not_found`, as if the item did not exist;
5. asks the policy;
6. writes an `audit_log` row for the decision.

After an allowed or approved handler runs, two filters apply to the result before an agent sees it:

- **Hiding.** Item views of a class the agent has no grant for are dropped from lists, nulled where a single item is embedded (`day.state`'s focus) and turn a single-item answer into `not_found`. The user and system are never filtered.
- **Taint.** The agent is tainted by every actor it read content from: an item's `origin`, a task type's `origin` when that is an actor (built-in and pack types are not), and every non-user **writer** of an item, a widget's title or config, or an agent-defined type. Writers are stored per entity as settings `writers:item:<id>`, `writers:widget:<id>` and `writers:type:<id>` (at most 8 actors each), written in the same event as the change, so undo and redo carry them with the content. This covers results of approved requests as well (`decide`).

User and system T0 calls that are allowed are not audited: they are the UI's own reads and would flood the log. Every agent call and every write is audited. `audit_log` is append-only (triggers) and stores an argument hash, never the arguments themselves.

The policy state lives in the `clients` (token SHA-256, grants, revoked) and `policy_rules` tables. It is written only by user-only commands (`clients.*`, `grants.set`, `rules.*`). Those events are not undoable, so an agent cannot `history.undo` a revoke.

## Data model

Entities are stored as JSON documents in per-kind tables. `items` also keeps `parent_id` and `ord` as columns, for the index.

| Table | Contents |
| --- | --- |
| `items` | `Item` (see CONTRACT.md); `roles: []` means every role |
| `task_types` | user-defined `TaskType`s; built-in and pack types are code/pack data and never stored |
| `layouts` | one `Layout` per role, seeded from the packs |
| `settings` | `current_role`, `focus:<role id>` |
| `legacy_import` | one `LegacyRecord` per imported item: TUI id, node id, fold/focus state, raw due text |
| `events` | the event log (below) |
| `history_stack` | undo and redo stacks of event ids |

Roles come from the embedded packs (`packs/<role>/pack.json`, via `include_str!`). They are validated at startup and are not stored. First start seeds their layouts and the default role in a `system.seed` event, which is not undoable.

## Event log

- `events` is append-only: triggers abort any UPDATE or DELETE.
- Each row holds:
  - the contract `Event` fields (`id` ULID, `ts`, `actor`, `command`, `entity`, `summary`)
  - `seq`
  - `payload`: the args and the forward changes
  - `inverse`: the before images
  - `causation_id`: for undo/redo, the event it reverses
  - `undoable`
- **Undo** pops the newest undoable event and writes its inverse as a new `history.undo` event (causation = original). It then pushes the original onto the redo stack. **Redo** is symmetric. A new undoable write clears the redo stack. The undo stack keeps at most 500 entries.
- **Selective undo/redo** (`event_id`) applies only when every entity the event touched still has the state that event left. Otherwise it fails with `conflict`, and so does a plain undo/redo that hits an entity edited since.
- **Not undoable, but logged:**
  - `role.switch`, `focus.*` (view state, not content)
  - `system.seed`
  - consent audit events (`consent.requested`, `consent.approve`, `consent.deny`)
  - undo and redo themselves
- `history.list` reads the log newest first. It takes `limit` (max 500), `before` (paging) and `actor_prefix` (for example `agent:`).

## Consent

When the policy returns `RequireConfirmation`:

1. The core stores a `PendingConsent` in memory: `{id, command, tier, actor, args, display_truncated, args_hash, summary, details, tainted, taint_source, created_at, expires_at}`. `args` is the whole canonical request, with only secrets and imported file content replaced; past 64 KiB it is the short audit form and `display_truncated` is `true`. Privately the entry also keeps the **scope digest**: what the request would touch, resolved against the state at park time (`scope_digest` in `halp-core/src/lib.rs`).
2. It records a `consent.requested` audit event (actor = the requester, entity `confirmation/<id>`). The event payload carries the short args and `display_truncated`.
3. It returns `confirmation_required` with `confirmation_id`.

Arguments are validated before a request is parked, and a request about an item that does not exist is answered with `not_found` at once. Each request expires after 5 minutes, which counts as a denial. At most 16 requests may wait per actor and 64 in total; the user's and the system's own requests are not counted against the total, so agents cannot crowd them out. At park time the request is pinned: `capture.quick` without a `parent_id` gets the Inbox's id (when one exists), and natural-language due dates (`tomorrow`, `friday`, `+3d`, also `due:` words in capture text) become `YYYY-MM-DD`, so approving later runs what was shown. A due that does not parse stays as typed and fails at approval as it would have at once.

Deciding:

- Only `Actor::User` may call `consent.approve` or `consent.deny`; anyone else gets `denied`.
- **Approve** first recomputes the scope digest under the same lock. If it changed, nothing runs: the entry is recorded as `superseded`, re-parked under a new id (fresh summary, same args, tier, taint and deadline, new `consent.requested` event), the old id resolves to `ConsentOutcome::Superseded(new_id)`, and the approver gets `conflict` with the new `confirmation_id`. Re-parking does not count against the agent's park rate. If the digest cannot be computed (the step can no longer run), the request is settled as `failed`. No T1 write lock covers pending scopes: computing every write's footprint up front would be fragile and would block the user's own edits, while the approval-time check already refuses any approval of something the user did not see.
- Otherwise **approve** runs the parked command **as the original actor**, without asking the policy again. The `consent.approve` audit event is recorded together with the outcome (success or error), and the pending entry is dropped only once the result is known. If the original client has been revoked since it asked, the approval is refused and recorded as a denial.
- **Deny** records `consent.deny` and returns `{denied: id}`.
- A user's own T3 calls (for example `tasks.delete` from the UI) are parked as well. The UI confirms them on the spot.

Callers that wait on a parked request (MCP) read the decision with `Core::take_consent_outcome(id)`, which returns `Approved(response)`, `Denied`, `Expired` or `Superseded(new_id)`. Denied and expired map to a `denied` error for the original request, and superseded to `confirmation_required` with the new id. `Core::wait_for` follows a superseded request to its new id by itself.

The socket's user channel (`halpd consent`) can list and deny but not approve. Approval is only possible in-process: the webview's `dispatch` and `halpd serve --interactive` (see MCP.md, "User channel").

Pending consents do not survive a restart, which amounts to a denial.

When a `consent.requested` event arrives, the desktop shell shows and focuses the main window (even from the tray) and requests the user's attention. The tray tooltip (and on macOS the tray title) shows the number of pending requests, refreshed on every `consent.*` event.

## Encryption

- The database is SQLCipher 4, from `rusqlite`'s `bundled-sqlcipher-vendored-openssl` feature, so no system library is needed.
- The 256-bit key is given to SQLCipher as a raw key (`PRAGMA key = "x'…'"`), so SQLCipher runs no KDF of its own.
- Opening checks `cipher_version` (refusing a non-SQLCipher build) and reads the schema to verify the key; a wrong key is reported as a key error.
- `secure_delete` is on and the journal is WAL.
- Migrations are forward-only, tracked in `user_version`; a database newer than the binary is refused.

## Key handling

| Provider | Where the key comes from |
| --- | --- |
| `KeychainKeyProvider` (default) | A random key from the OS CSPRNG, kept in the OS credential store (macOS Keychain, Windows Credential Manager, Secret Service via pure-Rust zbus). Service `halptask-hub`, account `db-<sha256(path)[..8] hex>`. |
| `PassphraseKeyProvider` | Argon2id (default m=64 MiB, t=3, p=1) over the passphrase. Salt and parameters sit in `<db>.kdf` (mode 0600 on Unix); the passphrase is never stored. Parameters in the sidecar are bounds-checked before use. |
| `RawKeyProvider` | A fixed key, for tests and tooling. |

- Key material (`DbKey`, passphrases) lives in `Zeroizing` buffers and is wiped on drop.
- `halpd` never accepts a passphrase on the command line, because it would show in `ps`. It reads passphrases from a named environment variable or one line of stdin.

## TUI import

`halp_store::import::parse_tui` reads the TUI's storage file:

- protobuf, or legacy Markdown
- either one plain, or encrypted with base64(salt‖nonce‖AES-256-GCM) and PBKDF2-HMAC-SHA256 at 100k iterations

Hardening:

- size cap of 64 MiB
- depth and item-count caps
- no panics on malformed input (fuzzed with truncations in tests)

- Format detection: the file extension decides when it is known. Without one, protobuf is tried first and Markdown is the fallback, as the Go TUI does it.
- Markdown nested deeper than 64 levels is not an error: the overflow is attached to the deepest allowed ancestor, and the import reports a warning. Warnings are capped at 100, plus a line saying "and N more".

The import is the `import.tui` command (T3, user only; `halpd import` sends it). It inserts the tree in pre-order after the existing roots. It records a `LegacyRecord` per item so nothing is lost, all as **one undoable `import.tui` event**.

**Deleted TUI items are kept as tombstones.** The Go TUI keeps items marked deleted in its file (`ItemFromProto`). The Hub stores each one, with its subtree, as a hidden `legacy` entity (`tomb:<ulid>`) outside the outline. It keeps the legacy id, the parent link, the text, the note, the status, the tags and the timestamps. Tombstones were chosen over importing such items as archived: they never show up in lists or counts, and nothing in the source file is lost.

## Date and capture parsing: deliberate differences from Go

`capture.quick`, `capture.parse` (a T0 preview that runs the same parser without writing) and `due` share one parser in `halp-domain`. It follows the Go TUI except in four places, all on purpose:

| Input | Go | Hub |
| --- | --- | --- |
| `+1m` on Jan 31 | overflows into March | clamps to the end of the month (Feb 28/29) |
| `02-29` when the year is not a leap year | rolls over to Mar 1 | is an error, so the token stays in the title and `due` is `null` |
| `#bug,` | the tag is `bug,` | trailing punctuation is trimmed, so the tag is `bug` |
| `@due()` (empty) | removed | stays in the title, with no due date |

## Event retention

The event log keeps every change, with before and after state, until it is purged. `history.purge` (T3, user only) takes either or both of these options:

- `older_than_days`: redact events older than that many days;
- `deleted_items`: redact every event about an item that no longer exists, including the whole consent trail (request and decision) that mentions such items.

Redaction replaces the payload, the inverse and the summary, and takes the event off the undo and redo stacks. The row itself stays, so the sequence, actor, command and timestamps can still be audited; a trigger allows only that exact update. With `remember: true`, the retention is stored and applied every time the core opens. The purge itself is audited.

## Data classes

A task type's schema may name a class with `"x-halp-data-class": "<class>"`; types without one are `general`. An item has the first non-`general` class of its own type or any ancestor's, so the children of a private note are private too. `em.private_note` is `notes.private`; `ext.issue` and the `dev.*` types are `general`. Agents need the class in `grants.data_classes` (the default grant is `["general"]`); the user is unaffected.

The event log keeps no text of private items: an event whose changes touch one gets the summary "Created a private item" (or Updated, Deleted …) and the args `{"private": true}`; a parked request on one gets the summary "Asked: <command> on a private item" and a `consent.requested` payload without args. The pending entry itself still holds the full args, so the user sees what they approve. `day.state` counts still include private items.

Known limit: `grants.set` accepts only the classes `grants.list` names (`general`, `notes.private`, `email.body`, `secrets`, `audit`). A pack type with any other class string is hidden from every agent until that list grows.

## Undo tiers

- A user's plain undo stays T1: one keystroke, and it is itself redoable.
- A redo that re-applies a T3 change (such as a deleted subtree) is T3 for everyone.
- An agent's undo or redo that deletes any item is T3.
- An agent's `history.undo` steps only its own most recent change.
- The step that gets approved is pinned to its event id, so an approval cannot end up stepping some other event.

## MCP host

The host (`halp_mcp::serve`) runs in the app or in `halpd serve` and shares the process's one `Core`, so parked requests from MCP show up in the app's consent UI. Each socket connection is authenticated by token and gets its own `HalpServer` (rmcp) with `Ctx::agent(id).with_session(..)`. Core events become MCP resource notifications. MCP.md covers the protocol, the socket, the shim and the residual risks.

## Packs and extensions (P5)

Two crates sit beside halp-core (design: docs/hub/P5-EXTENSIONS.md).

- **halp-pack**: the pack format. `halp-pack.json` is the manifest (id, version, `extends`, roles, widgets, task types, prompts, modules, UI widgets) and a `files` index (sha256 and size) that covers every other file. A `.halppack` bundle holds them plus an Ed25519 signature. `verify(bytes, community_keys)` checks the signature, the index and the path rules (`[a-z0-9_./-]` only) and assigns a trust level.
- **halp-ext**: runs a pack's code. Tier B modules run as wasmtime sandboxes; `halp_ext::wiring` connects them to the core.
- Core: `packs.*` commands, the pack registry, and the accessors the shell uses (`ext_ui_widget`, `ext_ui_file`, `pack_prompts`).

Tiers of extension:

| Tier | What | Runs |
| --- | --- | --- |
| A | data: roles, task types, layouts, prompts, `extends` | in the core, as data |
| B | a WASM transform feeding a computed widget; imports nothing, bounded fuel, memory and output | wasmtime, no I/O |
| C | a widget: HTML plus external JS | a sandboxed iframe (`allow-scripts`) served over the `halpext://` scheme with a strict CSP, no inline script, talking over one MessagePort (`ui/initialize`, then JSON-RPC `tools/call`) |

Trust levels, highest first: `builtin`, `first_party` (signed by a key shipped with the app), `community` (signed by a key the user trusted), `unsigned`, `dev` (a local folder, marked in the UI). Only a signed pack from a trusted key may carry modules or UI widgets. Only `first_party` and `community` prompts reach agents over MCP.

Modules and Tier C widgets act as the actor `module:<pack_id>`. It has no grants beyond read-only `general` data, only the calls listed in the manifest and shown in the install review, and every call is audited under that actor. A pack cannot make it act as an agent or the user.

Platform support: Tier A and B work everywhere. Tier C needs the custom scheme and iframe sandbox the shell provides on macOS and Linux; on Windows a Tier C widget shows as unavailable.

The example `hub/examples/packs/acme-sre` (pack `acme.platform-sre`, extends `halp.developer`) uses all three tiers and is signed with the test-only key in `hub/examples/keys`, which must never be trusted. `halp-ext/tests/example_pack.rs` verifies, installs, runs and serves it.
