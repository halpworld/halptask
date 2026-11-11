# HalpTask Hub over MCP

The Hub is an MCP server. Agents such as Claude Code and Claude Desktop use it to read the day, create, move and complete tasks, and rearrange the layout. Every call goes through the same core and policy as the desktop UI, and every call is audited. A T2 or T3 call waits until you confirm it in the Hub.

## Protocol version

- Built on `rmcp` **3.9.0**, pinned with `=3.9.0` (the official Rust SDK).
- The Hub speaks **MCP 2026-07-28**, which starts with `server/discover` and has no `initialize`.
- Older clients can still use `initialize` with **2025-11-25** or an older revision; the server negotiates down.
- `halp_mcp::PROTOCOL_VERSION` and `PROTOCOL_VERSION_WITH_INITIALIZE` hold these two strings, and tests cover both lifecycles.

## Pieces

```
MCP client --stdio--> halp-mcp (shim) --unix socket--> Hub host (app or `halpd serve`)
                                                        +-- HalpServer (rmcp) per connection
                                                        +-- Core::dispatch (policy, audit, consent)
```

- **Host** (`halp_mcp::serve`): a Unix domain socket, `<data dir>/halp.sock`. The desktop app and `halpd serve` both run it. The MCP server runs inside the Hub, so it shares the one Core and its pending consents.
- **Shim** (`halp-mcp` binary): the command an MCP client launches. It sends its token, then copies bytes between stdio and the socket. It holds no logic and no state.

## Setup

1. Start the Hub: open the app, or run `halpd serve`.
2. Pair a client:
   ```
   halpd clients pair --name "Claude Code"      # optional: --id claude-code
   ```
   This prints the token **once**; the Hub stores only its SHA-256. It also prints ready-to-use config.
3. Save the token in a file only you can read:
   ```
   mkdir -p -m 700 ~/.halptask && (umask 077; cat > ~/.halptask/claude-code.token)
   ```
4. Configure the client.
   - Claude Code:
     ```
     claude mcp add halptask --env HALP_MCP_TOKEN_FILE=$HOME/.halptask/claude-code.token -- halp-mcp
     ```
   - JSON config (Claude Desktop and others):
     ```json
     { "mcpServers": { "halptask": { "command": "halp-mcp",
         "env": { "HALP_MCP_TOKEN_FILE": "/Users/<you>/.halptask/claude-code.token" } } } }
     ```
   When the Hub uses a non-default `--data-dir`, add `HALP_SOCKET=<data dir>/halp.sock`; `halpd clients pair` includes it for you.

Other client commands:

- `halpd clients list` shows each client's id, name, grants, created time and revoked state. While the Hub runs, `list` and `revoke` go through its user channel; otherwise they open the database.
- `halpd clients revoke ID` takes effect on the client's next call, even on a connection that is already open and even when another process does the revoke. It also deletes the client's standing rules. A request from that client that is still pending can no longer be approved.

### Shim options

| Setting | Meaning |
| --- | --- |
| `HALP_MCP_TOKEN` | The token itself. |
| `HALP_MCP_TOKEN_FILE` | A file holding the token. This is preferred: the token stays out of the client's config. The shim warns if group or others can read the file, refuses files over 4 KiB, and refuses tokens that contain whitespace. |
| `HALP_SOCKET` / `--socket PATH` | The socket path. The default is `<platform data dir for halptask-hub>/halp.sock`. |

The shim **never takes a token as an argument**, because arguments show in `ps`. It rejects any argument it does not know without echoing it back. When the client closes stdin, replies still arrive for up to 2 s, and then the shim exits. A call that is still waiting for consent stays pending in the Hub until you decide or it expires.

## Socket and authentication

- **Permissions:**
  - The data directory is forced to `0700`.
  - The socket is `0600`.
  - The host accepts only peers whose uid (`peer_cred`) owns the data directory.
  - There is no TCP listener.
- **One host per data directory:**
  - A live socket makes a second host fail with `AddrInUse`.
  - A stale socket is removed only when connecting to it fails.
  - A non-socket file at that path is an error, not something to delete.
- **Path length:** at most 103 bytes (`sun_path`). A longer data directory is a startup error, not a truncated path.
- **Connection limits:** connections come from separate pools, so neither silent sockets nor one busy agent can lock out the user channel.
  - At most 8 connections may be sending their first line at once; more are closed on accept. The first line must arrive within 2 s and be at most 512 bytes, or the connection is closed.
  - At most 16 agent connections at once, and at most 4 per client. Over either limit the host answers `ERR busy` and closes.
  - 2 slots are kept for the user channel; over that, `ERR busy`.
  - JSON-RPC messages are limited to 2 MiB.
- **Preamble:** the first line is one of these.
  - `HALP-AUTH <client token>\n` gets the reply `OK agent:<id>`. MCP (newline-delimited JSON-RPC, the stdio framing) follows. Each connection is its own session with a random session id. Taint is tracked per client, across all of its sessions (see Policy).
  - `HALP-USER <user token>\n` gets the reply `OK user` and opens the **user channel** (below).
  - Anything else gets `ERR unauthorized` after a 250 ms delay, and the connection closes.
  - Tokens are compared in constant time. A client token never opens the user channel, and the user token never opens an agent session.

### User channel

The user channel exists so that `halpd consent …` and `halpd clients list|revoke`, run in another terminal, can see and refuse requests parked in a running Hub and list or revoke clients. Its token is random and written to `<data dir>/user.token` (mode `0600`, created fresh) every time the host starts, and both files are removed at shutdown. The channel speaks contract JSON lines (`{"command","args"}` in, the response envelope out) with actor `user`.

The channel only takes power away. It accepts `consent.list`, `consent.get`, `consent.deny`, `clients.list` and `clients.revoke`. `consent.approve` is refused with `denied: … approve in the HalpTask Hub window …`, and every other command is `denied`. Any process running as you can read `user.token`, including an agent with a shell, so a token-holder must not be able to approve. Approval exists only in-process: in the Hub window (the webview's `dispatch`) and in the terminal running `halpd serve --interactive`. Denying and revoking only take power away, so the channel stays on by default.

The channel runs as a limited user (`Ctx::user_cli`). A pending request about a private item (any data class other than `general`) is listed with `private: true`, `args: null`, `display_truncated: true`, an empty `args_hash` and details, and the summary `<command> on a private item (open the Hub to see it)`. Otherwise reading `user.token` would show private items' text through other agents' requests. The id, command, tier, actor, taint flag and times stay, so the request can still be denied. The accepted cost: an agent with a shell can deny requests and revoke other clients (never pair one, change a grant or approve).

## What an agent sees

### Tools

Each command an agent may use is a tool whose name has the dots replaced with underscores (`tasks.create` becomes `tasks_create`). Each tool carries:

- the command's input schema, with an output schema and `structuredContent` on results;
- annotations (read-only, destructive, idempotent);
- a note on its tier in the description. For example, T3 says: "High risk: always waits for the user's confirmation in the Hub."

There are 34 tools (P3 added `integrations_list`, `integrations_sync_now` and `context_capture`). These commands are **never exposed**, because they belong to the user:

- `consent.approve` and `consent.deny`
- `clients.*`, `grants.set` and `rules.*`
- `history.purge` and `import.tui`
- `integrations.connect` and `integrations.disconnect`

The integration write path (`ext.upsert`, `ext.status`) is hidden as well; only `integration:<id>` actors may call it (see INTEGRATIONS.md).

`audit_list` is listed but needs the `audit` data class grant. The core still enforces all of this on its own; hiding the tools is only a convenience.

A command error becomes a tool error (`isError: true`) whose text carries the error code and message, for example `denied: client claude-code was revoked`.

### Resources

| URI | Content |
| --- | --- |
| `halp://day/state` | `day.state` |
| `halp://layout/current` | the active role's layout |
| `halp://events` | the newest events; `halp://events?since=<event id>` gives the events after that one, oldest first |
| `halp://task/{id}` | one item (template) |
| `halp://role/{id}` | one role (template) |
| `halp://packs` | `packs.list`: installed packs with the fields an agent may see (id, version, title, trust, enabled, status, what they add). User-only fields (signer keys, file lists, review details) are left out |

URIs are parsed strictly; `halp://task/../x` and other malformed URIs are errors. Clients can subscribe through the 2026-07-28 `listen` filter or the legacy `resources/subscribe`. The server sends `notifications/resources/updated` for the resources each core event touches. A client can subscribe to `halp://task/{id}` only for an item it may read now (it exists and the client has the grant for its data class); otherwise the subscription is refused with `resource_not_found` and nothing is sent about that item, so a subscription does not tell an agent when a private item changes.

### Prompts

Role packs ship prompts as Markdown with front matter in `packs/<role>/prompts/*.md`:

- developer: `morning-triage`, `end-of-day`
- eng-manager: `prep-one-on-one`, `weekly-update`
- cso: `data-chase-round`, `weekly-esg-digest`, `prepare-board-brief`, `evidence-gap-check`

`prompts/list` and `prompts/get` serve them, with their arguments filled in. halp-mcp embeds every `prompts/*.md` of every built-in pack at build time (`build.rs`), so a new prompt file needs no code change; a test checks that none is missing.

#### Pack prompts

Installed packs can also ship prompts (`prompts/*.md`). `prompts/list` appends them to the built-in ones, and `prompts/get` serves them.

- Name: `<pack_id>.<prompt name>`, for example `acme.platform-sre.oncall-handoff`.
- Label: the description starts with `[community: <pack_id>]` or `[first_party: <pack_id>]`, and `prompts/get` prepends the same label plus a line saying the text comes from a pack and is untrusted. Trust is the owning pack's, not that of a pack that extends it.
- Served only for packs that are installed, enabled, live and signed by a trusted key (`first_party` or `community`). Unsigned and dev packs' prompts are never served, and neither are prompts of disabled or uninstalled packs.
- Each file is re-hashed against the pack's file index on every list, must be UTF-8, at most 32 KiB, and must pass the same text checks as other input (no bidi or control characters). A file that fails is skipped.
- Pack prompts take no arguments.
- When a pack is installed, enabled, disabled, updated or removed, the server sends `notifications/prompts/list_changed` (through `listen` for 2026 clients, and from a watcher started in `on_initialized` for legacy clients), and `halp://packs` is reported as updated.

#### What an agent cannot do with packs

Agents can read `packs_list` / `halp://packs` and nothing else. No tool for install, enable, disable, uninstall or trust keys is exposed, and `packs.install` and the other writes are user-only in the core: called as an agent they return `denied`. An agent cannot change which prompts, widgets or modules exist. Pack widgets and modules act as `module:<pack_id>` with read-only `general` data, never as an agent.

## Policy

Every call from every actor goes through `halp_policy::Engine` in `Core::dispatch`; there is no other path.

- **`user` and `system`:** T0 to T2 are allowed. T3 is always confirmed, even for the user.
- **`agent:<client>`:**
  1. The client must be paired and not revoked.
  2. It is rate limited: calls 120 burst at 4/s, writes 60 at 1/s, parked requests 16 at 0.1/s.
  3. It needs a grant for the command's data class: `general` (the default), `notes.private`, `email.body`, `secrets` or `audit`, or a class a loaded task type declares with `x-halp-data-class`.
  4. Then by tier:
     - T0 is allowed.
     - T1 is allowed when the `t1` grant is on (it is on at pairing); otherwise it is confirmed.
     - T2 is confirmed, unless a standing rule for this client, command and exact arguments matches **and** the client is not tainted.
     - T3 is always confirmed.
  5. A circuit breaker denies T2 and T3 for 5 min after 8 T2/T3 requests within 60 s.
- **`module:`:** T0 of class `general` is allowed; everything else is confirmed.
- **`integration:<id>`:** T0 of class `general` and the write path `ext.upsert` / `ext.patch_fields` / `ext.status` (`INTEGRATION_WRITES`) are allowed, the latter rate limited per integration; everything else is confirmed.

Some calls get a higher **effective tier** for their arguments:

- An agent's undo or redo that deletes items is T3.
- Any redo that re-applies a T3 change is T3.
- An agent removing the `activity` widget is T2.

The approved step is pinned to its event id.

**Taint:** when a client reads content another actor wrote, that client is tainted. This covers an item's `origin` and an event's `actor`, since an event summary quotes its actor's text. Taint belongs to the client (`agent:<id>`), not to one connection, so reconnecting does not shed it. A tainted client gets no standing-rule shortcuts, and its consent prompts say where the taint came from.

Taint clears by itself 30 minutes after the client's **last** tainted read (each new tainted read restarts the clock; `Limits::taint_ttl`). The user can also clear it at once with `policy.clear_taint {"client_id": "<id>"}`. That command is user only, hidden from MCP, T1, and recorded as a `policy.clear_taint` event. Taint lives in memory, so a restart of the Hub clears it too.

## Consent flow

1. A call that needs confirmation is checked (its arguments are validated first) and parked. The core records a `consent.requested` audit event. It also pins the request's **scope**: a digest of what it would touch right now. That covers every item named in its arguments with its whole subtree (ids, parents, versions), the exact changes an undo or redo would apply, the events a purge would redact, and the layout a widget command edits.
2. The MCP tool call **stays open**. Every few seconds it sends `notifications/progress` with "Waiting for the user to confirm in the Hub", when the client gave a progress token.
3. The user approves in the app or in `halpd serve --interactive`. They can deny there, or with `halpd consent deny ID` from another terminal.
   - **Approve** first recomputes the scope. If it differs from what was shown (say the agent moved one of your items under the item it asked to delete), nothing runs. The approval gets `conflict: the request changed since you saw it` with a **new** `confirmation_id`. The old request is recorded as `superseded`, and the same request is parked again under the new id with a fresh summary, the same arguments and the original deadline, with a new `consent.requested` event. A waiting tool call follows the new id. If the request can no longer run at all, the approval fails and the request is settled as `failed`.
   - Otherwise approve runs the parked command as the original actor and records the approval together with its outcome. The tool call returns that result.
   - **Deny** turns the call into a tool error `denied: the user denied this request`.
   - **Expiry** after 5 min also gives a `denied` error. The MCP host sweeps expired requests every 15 s, so a request expires on time even when nothing else is dispatched.
   - A request too large to show in full (`display_truncated`) cannot be approved anywhere, the Hub window included: `consent.approve` answers `denied` and the request stays pending until it is denied or expires.
4. If the client was revoked after it asked, the approval is refused and recorded as a denial.

Limits: at most 16 pending requests per actor and 64 in total.

`halpd serve --interactive` prompts on each new request. It fetches the request with `consent.get` and shows it, throws away anything typed before the prompt appeared, and approves only when you type the 4-character code it shows (the last 4 characters of the confirmation id); anything else denies. A request too large to show in full (`display_truncated`) can only be denied, there and in the Hub window. `halpd consent approve` no longer approves; it explains where to approve and suggests `halpd consent deny ID`.

T1 writes are not locked while a request is pending. A lock would need every write's footprint worked out in advance, and it would block the user's own edits too. The approval-time scope check refuses anything that changed what the user saw, which is the property that matters.

## halpd

halpd is the user's own tool and acts as `user`. The old `--actor` flag is gone, and using it gives an error.

```
halpd [--data-dir DIR] [--db PATH] [--db-passphrase-env VAR | --db-passphrase-stdin] <command>
  dispatch '<json>' | -          one request as the user. A parked T3 asks for "yes" on a TTY; otherwise it is denied.
  import <path> [...]            TUI import (import.tui, T3)
  commands | schema              the command list; `schema` prints the sorted descriptors as JSON
                                 (committed as docs/fixtures/commands.json)
  serve [--interactive]          run the MCP host until Ctrl-C
  clients pair|list|revoke       manage MCP clients
  consent list|deny              see or refuse requests parked in the running Hub (over the socket)
  consent approve                refused: approve in the Hub window or `serve --interactive`
```

**One Core per database.** A Core holds an exclusive advisory lock on `<data dir>/hub.lock` (next to `hub.db`) for its whole life. While the app runs, every halpd subcommand that opens the database (`dispatch`, `import`, `clients pair`, `serve`) fails with `HalpTask Hub is already running; use the agent socket (halp-mcp) instead`. Pair clients in the app instead. `consent list`, `consent deny`, `clients list` and `clients revoke` still work because they go through the socket's user channel. As defense in depth, every commit compares each changed row with the `before` value it was computed from, and refuses with `conflict` if someone else changed it in between.

## Residual risks

- **Same-uid processes.** The socket checks keep other users out, not your own processes. Anything running as you can read `user.token`; that now lets it list and deny, not approve. But a same-uid agent with a shell can do whatever your OS account can do. It can read the database key from the keychain (or watch you type the passphrase), open `hub.db` while the Hub is not running, edit files, or drive the GUI with accessibility APIs. **The trust anchor is the approval in the Hub window.** Nothing in the Hub can protect you from a process with your full account rights. Run agents that have a shell in a sandbox, or as another user, if they must not be able to act as you.
- **TTY check.** The terminal checks and approval codes in `halpd serve --interactive` and `halpd dispatch` guard against accidents (a script piping `yes`), not against a hostile agent running as you. Such an agent can fake a terminal with `script` or a pty and read the code from it. It can also quit the Hub (so `hub.lock` is free) and run `halpd dispatch` itself, opening `hub.db` with the key from the keychain, with no Hub window to approve in. For approvals an agent cannot fake, use the app and keep shell agents in a sandbox or under another user.
  - Future hardening: a keychain access-control list that lets only the signed Hub app (not `halpd` or any other binary) read the database key without a prompt, so a headless `halpd` run needs your password.
- **Windows.** There is no named-pipe host yet. `serve` returns `Unsupported`, and the shim says the socket is not available. When it is built, the pipe must get an owner-only DACL and must reject remote clients. Until then, MCP is Unix only.
- **In-memory consents.** Pending requests live in the Hub process. A restart drops them, which counts as a denial. `halpd dispatch` runs its own Core, so it cannot run at all while the app holds `hub.lock`.
- **Revoked clients.** A revoked client's pending request stays listed until it is denied or expires, but it can no longer be approved.
- **Socket path.** The path is limited to 103 bytes, so pick a short `--data-dir`.
- **Transport.** Streamable HTTP is not offered, even over the socket; stdio framing is the only transport.
