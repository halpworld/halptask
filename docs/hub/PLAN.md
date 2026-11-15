# HalpTask Hub — Draft Plan (v0.1, 2026-11-10)

Status: **DRAFT for owner review.** No code written yet. Research claims marked [VERIFY] came from secondary sources.

## 0. The idea in one paragraph

HalpTask becomes a desktop "hub of your day" for Mac, Linux and Windows. Each job role gets an opinionated **Role Pack**: a full composition of widgets, task types, data sources, automations, agent playbooks, layouts and themes, built for that job. An AI agent drives the app most of the time through MCP, and the agent can do everything the UI can, including rearranging the UI itself. Every part of a pack can be forked and extended by the community through a sandboxed module system. **The Hub is a new, all-Rust Tauri v2 app living in this monorepo under `hub/`.** The existing Go TUI is frozen: it stays at the repo root and keeps working and shipping as it does today, and no new work goes into it. The Hub reuses the TUI's *ideas*, not its code: the outline tree, natural-language dates, quick capture, encrypted storage and the plain-text Markdown export format.

## 1. What research told us (condensed)

| | Developer ("The Cockpit") | Eng Manager ("The Command Deck") | CSO ("The Sustainability Wall") |
|---|---|---|---|
| **Core object** | Code work in flight: PRs, CI, tickets, context | **People and commitments**, not tasks | **Datapoints, deadlines and evidence** |
| **#1 pain** | Fragmentation: 59% of daily tasks switched, 29% of interrupted tasks never resumed | Things delegated get lost; 1:1 continuity; about 18 h/week in meetings | Data scattered and owned by others; Scope 3 confidence low; regulation shifting weekly |
| **Killer modules** | Context-restore cards (branch, dirty files, last cmd, next step), PR review queue with staleness heat, CI radar, focus-block protector, Slack-ask catcher, agent-run supervision panel | People roster with 1:1 cadence rings, delegation ledger ("waiting on"), commitments-I-made ledger, 1:1 agenda builder, meeting load meter, brag-doc collector, weekly update generator | Disclosure countdown wall (CSRD, SB 253, CDP), data request chase board, evidence locker with hash audit trail, SBTi trajectory vs actual, live grid-carbon and energy monitor, regulatory radar, questionnaire answer bank, board brief generator |
| **Top integrations** | GitHub/GitLab, Linear/Jira, Slack, CI, calendar, PagerDuty/Sentry, **local git and shell** | Calendar, Slack/Teams, Jira/Linear, GitHub, email, HRIS, ATS, PagerDuty | Carbon platforms (Watershed and others), ERP and procurement, email and Teams, SharePoint, meters and utility data, Electricity Maps, regulatory feeds |
| **Hard "never"** | No activity surveillance; no notification firehose; no silent sends | No per-person productivity scores; no auto-nudging reports; no sentiment scoring; privacy blur mode for an ambient screen | **AI never invents a number that reaches a disclosure.** Every figure has a source row and a named human approver. Uncertain regulation is shown with a confidence flag. |

Cross-role findings:
1. **Draft, never send.** All three roles need the agent to prepare outbound actions (reviews, nudges, supplier emails) and a human to send them.
2. **Today is the CSO's deadline day.** The CARB SB 253 Scope 1/2 deadline is 2026-11-10 [VERIFY]. That shows why a "regulatory certainty" field on deadlines matters.

Full role reports and task-type schemas are in the research appendix (§9).

## 2. Architecture

```
   Tauri v2 window (Svelte 5 + TS UI)            External agents (Claude Code, Claude Desktop, …)
        │ Tauri commands (capability-scoped IPC)         │ MCP: `halp-mcp` stdio shim · streamable HTTP on UDS/named pipe
        └───────────────────────┬────────────────────────┘
                     halp-core  (Rust — owns everything; runs in the Tauri process,
                                 and can also run headless as `halpd`)
  ┌────────────┬─────────────┬──────────────┬────────────────┬────────────────┐
  Domain        Event log +   Policy engine   Integration      Module host
  (items, typed bus (undo,    (tiers T0–T3,   runtime (MCP     (manifests, WASM via
  tasks, views, audit, sync   consent, taint, client via rmcp, wasmtime, sandboxed
  layout)       feed)         audit)          OAuth, keyring)  widget iframes)
        └─ SQLite via rusqlite + SQLCipher (full-file encryption, key in OS keychain) + Markdown export
```

Monorepo layout. The Go app stays at the root and is untouched:

```
hub/
  Cargo.toml            # Rust workspace
  crates/
    halp-domain/        # items, typed tasks, JSON-Schema validation, dates, views, layout model
    halp-store/         # SQLite/SQLCipher, migrations, event log, Markdown import/export (incl. TUI data import)
    halp-policy/        # consent tiers, grants, taint tracking, audit
    halp-mcp/           # MCP server projection + `halp-mcp` stdio shim binary
    halp-integrations/  # MCP client host, OAuth (PKCE loopback), keyring, native connectors
    halp-modules/       # manifest schema, signing, wasmtime host
    halp-core/          # command bus wiring all of the above; `halpd` headless binary
  app/                  # Tauri v2 shell (src-tauri/) + frontend (Svelte 5, TS, design tokens, widget kit)
  packs/                # first-party role packs: developer, eng-manager, cso
```

Key choices, each with its reason:
- **One command API.** The UI and agents both call the same commands, for example `layout.move_widget`, `focus.set` and `tasks.create`. The MCP server is a projection of that API. As a result, the agent can do anything the UI can, and a UI action becomes an agent action without a separate implementation. This is what makes the MCP support "deep".
- **All Rust, Tauri v2 (your call).**
  - Tauri's capability ACLs, isolation pattern and CSP give a mature sandbox.
  - The official Rust MCP SDK is `rmcp`. We will check that its support matches spec 2026-07-28 at the start of P1 [VERIFY].
  - The Go code is a reference implementation, not a dependency.
- **SQLite with SQLCipher.** Rust makes this easy: rusqlite's `bundled-sqlcipher` feature builds it in, with no cgo-style cross-build pain. The whole database file is encrypted, and the key is held in the OS keychain (`keyring` crate). A passphrase is an optional extra. We still need SQLite instead of a whole-file write because integrations will produce thousands of items, several clients will connect at once, and we need an event log.
- **Import from the TUI.** The Hub can import the TUI's Markdown and protobuf data file, so existing users can move over.
- **Typed tasks.** `Item` stays the universal node, so the outline still works. A node gains `type_id` and a `fields` document validated against a JSON Schema, for example `dev.pull_request`, `em.delegated_item` or `cso.data_request`. Relations go in an edges table (blocks, sourced-from, evidence-for).

## 3. MCP, at the deepest level

**HalpTask as an MCP server:**
- **Tools:** every domain command, namespaced (tasks, types, views, layout, role, focus, notify, agenda, modules, theme), each with an `outputSchema`.
- **Resources:** `halp://task/{id}`, `halp://layout/current`, `halp://role/{id}`, `halp://events?since=`. They push live changes through `subscriptions/listen`, so the agent sees what the user does.
- **Prompts:** role playbooks shipped by packs, for example "morning triage", "prep 1:1 with Sam" and "weekly ESG digest".
- **Tasks extension:** long-running jobs such as a full sync, a board brief or a re-plan of the week.
- **MCP Apps (`ui://`):** HalpTask widgets render inside Claude Desktop and other hosts, and HalpTask renders other servers' MCP Apps inside its own dashboards.
- **`get_day_state`:** one cheap call that returns the whole picture for an agent.

**HalpTask as an MCP client/host:**
- Integrations are mainly external MCP servers (GitHub, Atlassian, Slack, Linear, Gmail/Graph, Sentry).
- A module manifest maps a server's tool results to HalpTask task types, for example GitHub PR → `dev.pull_request`.
- Bespoke Go connectors are built only where no good MCP server exists: CalDAV, IMAP, MQTT/Modbus meters, weather and grid-carbon APIs.

**Consent tiers:** enforced in the daemon. Tool annotations are only hints.

| Tier | Examples | Default |
|---|---|---|
| T0 read | list tasks, read layout | auto, scoped per client and per data class (email bodies and 1:1 notes are separate classes) |
| T1 reversible write | create or edit a task, move widgets, set focus | auto after a one-time grant, with an "Agent did X — Undo" toast |
| T2 external side effect | comment on a PR, post to Slack, email a supplier | confirm every time, or a narrow standing rule |
| T3 destructive or privileged | delete, install a module, change grants, export, read secrets | always confirmed in app chrome that the agent cannot render or click |

**Prompt-injection defence:**
- All content from integrations is taint-labelled.
- Once an agent call chain has read tainted content, T2 and T3 actions escalate to human confirmation.
- Network egress is limited to the hosts each manifest declares.
- Every call is written to an Agent Activity timeline with one-click revert.

## 4. The module system (how the community extends roles)

| Kind | What it is | Trust tier |
|---|---|---|
| **Role Pack** | Declarative composition: modules, layouts per screen size, task types, prompts, automations, theme | A: declarative, auto-validated |
| Theme / Layout / View / Task-type schema | Design tokens, grid layout, saved query plus renderer, JSON Schema | A |
| Data widget | First-party component kit (list, kanban, rings, countdown, chart, gauge, timeline, map, matrix) bound to a query | A |
| Logic module | Data transforms, automations, custom field logic in **WASM (wasmtime with fuel and memory limits, WASI off by default)**. No ambient access; host functions check the manifest. | B: sandboxed code |
| Custom UI widget | HTML/JS in an opaque-origin sandboxed iframe with a strict CSP, talking postMessage in the MCP Apps protocol. The same widget also works in Claude Desktop. | C: sandboxed UI |
| Native capability | IMAP, keychain, MQTT, local git and shell hooks | D: first-party only |

Customization goes deep because a Role Pack is plain data. A user or the community can fork `halp.role.developer`, swap one widget, change a threshold or add a task-type field, and publish the result as `acme.role.platform-sre`, which inherits from the original. Packages are signed (cosign or minisign). Installing one shows a permission prompt, and an update that adds permissions prompts again. A registry review process and a revocation list come in a later phase.

## 5. Design direction ("a joy to have up all day")

- **Ambient-first.** Every role has a calm wall mode, readable from 2 m away, with slow motion and no red badges spamming. Interactive mode is one keypress away.
- **Keyboard-first**, inherited from the vim DNA: a command palette, a leader key, and all actions also exposed as MCP tools.
- **One design-token system.** Light and dark, plus per-role accents: developer is cool and terminal-adjacent, EM is warm and people-centric, CSO is earthy with live data.
- **Privacy blur.** One key hides names and sensitive notes when someone walks by or you share your screen.
- **Agent presence.** A subtle "agent is working" strip and the activity timeline. You can always see what the AI changed.

## 6. Build phases

| Phase | Deliverable | Done when |
|---|---|---|
| **P0 Core** | Set up the `hub/` Cargo workspace and CI covering macOS, Linux and Windows. Build the domain crate (items, typed tasks with JSON Schema validation, the outline tree, natural-language dates) and the store crate (SQLCipher, migrations, event log with undo/redo, keychain-held key). Add an importer for the TUI data file. | `cargo test` passes on all 3 OSes; a TUI data file imports losslessly; the Go app and its CI are unchanged. |
| **P1 MCP server** | `rmcp` server with a stdio shim and HTTP over a Unix socket or named pipe. Expose the command set as tools and resources with subscriptions. Build the policy engine (T0–T3), the audit log, and a headless consent prompt. | Claude Code can create, move and complete tasks and rearrange the layout; T2/T3 calls block until a human confirms. |
| **P2 Desktop shell** | Tauri v2 app with a locked-down capabilities file and CSP, the design system, grid layout as data, the declarative widget kit, role switcher, ambient mode, privacy blur, consent dialogs in app chrome, and the Agent Activity timeline. | A demo layout runs on all 3 OSes; the agent rearranges it live. |
| **P3 Developer pack** | MCP client host plus OAuth (PKCE loopback, keychain). GitHub, Linear/Jira and Slack via their MCP servers; local git context capture. Widgets: Now, context-restore, PR queue, my PRs, CI radar, focus protector, ask catcher, day plan. | A developer can run a day from it with real GitHub data. |
| **P4 Eng Manager pack** | Calendar integration. Widgets: people roster, delegation and commitments ledgers, 1:1 agenda builder and private notes (encrypted, separate data class), meeting load, team delivery pulse (team-level only), weekly update draft. | An EM can prep and run 1:1s and track "waiting on" items. |
| **P5 Extension platform** | Manifest signing, WASM Tier B, iframe/MCP Apps Tier C, pack inheritance and forking, a local module dev mode. | A third party can fork a pack and ship a sandboxed widget. |
| **P6 CSO pack** | Data request chase board, evidence locker (hash and approval chain), disclosure countdowns with a regulatory-certainty flag, SBTi trajectory, Electricity Maps and MQTT energy monitor, regulatory radar, board brief (every number linked to its source). | A demo with a seeded dataset; no AI-generated figures. |
| Later | Registry and review, E2E sync, embedded email client, more roles. | — |

Every phase ends with an **adversarial review** and a **security review** on the final code, and a phase is done only when both reviews pass.

## 7. Defaults I'll take unless you object

- Frontend: Svelte 5 with TypeScript and Vite, using our own design-token component kit instead of a heavy UI library.
- The Hub gets its own binary name (`halptask-hub`), its own version line starting at 0.1.0, and its own release pipeline. Signing and notarization come at the end of P2. Until then builds are unsigned dev builds.
- SQLCipher for full-file encryption, with the key in the OS keychain. A passphrase is optional, and Argon2id derives the key when one is used.
- Local-first: **no HalpTask cloud** in v1. That means no OAuth relay, so v1 sticks to providers that allow public PKCE clients or have MCP servers with their own OAuth.
- Email in v1 is **triage only** (Gmail/Graph via MCP, rendered in a sandboxed iframe). A full IMAP client comes later.
- The EM pack never ranks or scores individuals. 1:1 notes are visible only to the user by default and never go to an LLM without per-note opt-in.
- The CSO pack is an **overlay**: carbon platforms and ERP stay the system of record. Covers EU (CSRD/ESRS as amended by Omnibus) and US/California.
- License stays as is (repo LICENSE), and community modules carry their own licenses.

## 8. Decisions (owner, 2026-11-10)

1. Stack: **all Rust, with Tauri v2**, as its own app in this monorepo.
2. Agent: **MCP only** in v1. A built-in agent comes later.
3. TUI: **frozen**. It stays as it is and gets no new work.
4. Scope: **deep slice P0–P4** (core, MCP, shell, developer pack, EM pack), then the extension platform, then CSO.

### Team per phase
- Implementers: Rust core, sonnet; policy, crypto and MCP plumbing, opus.
- Frontend implementer: sonnet.
- Mandatory **adversarial** and **security** reviewers on opus, run independently on every phase's final code.

## 8a. Course corrections (lead)

- **2026-11-10, after the P0–P2 review:** a socket connection can no longer approve a request. The user channel on the agent socket now allows only list, get and deny. Only the Hub window, or `halpd serve --interactive` in its own process, can approve. Why: an agent with a shell running as the same OS user could read `user.token` and approve its own destructive requests.
- **2026-11-10:** one Core per data dir. The Core holds an exclusive lock on `<data_dir>/hub.lock`, and other tools use the agent socket instead of opening the database. Why: two Cores on one database silently lost updates.
- **2026-11-10:** a parked request is tied to the state the user saw. If the target set changes before approval, the approval is refused with `conflict`. Why: an agent could move user items under a pending delete.
- **2026-11-10:** `halpd` is its own crate, not a binary inside halp-core, so that halp-core and halp-mcp don't depend on each other.
- **2026-11-10:** `grants.set` accepts a data class if it is built in, or if a loaded pack type declares it with `x-halp-data-class`. Class names must have a valid format. Why: with a fixed list, a community pack's private class could never be granted. That is safe, but it breaks the community-pack promise.
- **2026-11-10:** the CSO pack's pack and widget work (P6) starts before the extension platform (P5). Why: P5 changes the core heavily while P3/P4 follow-ups are still working in it, and the P6 UI lives in its own files. P5 follows, and the CSO energy connector comes after the core follow-ups.
- **2026-11-11, after the round-3 security review:** a request too large to show in full can no longer be approved, not even in the Hub window. It can only be denied (or left to expire). Before, MCP.md said to approve it in the Hub window. Why: the user must never approve what they cannot see. Also, the socket's user channel shows a request about a private item without its args or summary.
- **2026-11-11:** standing rules stay as they are (option a). They apply only to agent-callable T2 commands, and the core has none today. Settings > Rules says so in an empty state, and the rule form lists any T2 command that packs or extensions add later. Why: removing rules would break the contract for later packs, and a rule for T1 or T3 would weaken the policy.
- **2026-11-11:** `x-halp-immutable-when` (on `cso.evidence`) is enforced by halp-core for every write path, including undo and `ext.upsert`. An approved item cannot be un-approved by anyone, and only the user may approve. Why: an approval that can be undone, or an agent that can approve, would make the evidence locker meaningless.
- **2026-11-11, P5 design (docs/hub/P5-EXTENSIONS.md):** packages are signed with plain Ed25519 (`ring`, already in the lockfile) instead of cosign or minisign; a minisign import can come later. Extension code runs as the existing `module:<pack_id>` actor, not a new actor kind. Before any community pack can load, pack types get a trust level (so unsigned packs cannot use `x-halp-immutable-when` or `x-halp-ext-writable`), and removed pack types become *retired* rather than unknown. Why: without that, a community pack would get first-party lock powers, and uninstalling a pack would make its private items visible to agents. Tier C (iframes) is gated on an isolation spike; if the spike fails, Tier C uses a separate webview with no capabilities.
- **2026-11-11, after the P5-S spike:** Tier C widgets (sandboxed iframes) ship on macOS and Linux only. On Windows they show a placeholder until a child-webview version is verified on WebView2. Why: on Windows the frame would get the Tauri internals and the invoke key, which would give a community widget the main window's full capability.
- **2026-11-11, P5-B wiring:** a pack's whole bundle is stored as one encrypted blob and re-verified (hash and signature against today's keys) on every open, so a revoked key or a tampered blob retires that pack's types instead of loading them. Retired types keep their original trust, so locks and data classes still hold after uninstall. A disabled parent pack keeps serving the packs that extend it and only hides its own role. A bad signature is always an error, never shown as unsigned. Tier C serves HTML only with a single-use ticket and refuses `..` before URL parsing. Why: these close the uninstall, tamper and frame-swap gaps found while building.
- **2026-11-11, P5-E2:** MCP serves pack prompts only from built-in, first-party and community packs, each labelled with its trust and pack id. Unsigned and dev pack prompts never reach an MCP client (the design table had allowed unsigned ones with a label). The core accessor `pack_prompts()` was missing from P5-B and is added with P5-E2. Why: prompts are a prompt-injection channel into agents, and an unsigned pack has no accountable publisher.
- **2026-11-11, P5 final review:** an installed pack's item can never rely on a dev override: while dev mode is on, item writes also validate against the registry without dev packs (what a restart gives, since dev mode is never saved), installed packs and keys cannot be changed while dev mode is on (key revocation excepted), and turning dev mode off or reloading it is refused as a backstop (dev packs' own types still change freely). Tier C serves no SVG at all and a widget entry must be `.html`, because SVG can run script as a document and only `.html` gets a ticket. Tier C call results go through the same second visibility filter as Tier B inputs. Why: the final adversarial and security reviews found these gaps.
- **2026-11-11, P5 fresh review:** a dev pack can never take over a retired type of a real pack, and a retired real-pack row is never replaced by a dev one. Redefining a user type is refused while its items would stop validating. Known limit (pre-existing, not dev-mode related): undo/redo does not re-validate item fields, so undoing an edit or a type redefinition can leave an item that no longer validates.
- **2026-11-11, release prep:** undo/redo now refuses a step that would leave an item invalid (CONTRACT note 19), closing the known limit above. Items of a type that fails to load now count as `secrets` instead of `general` (note 20). The short key id stays as the row key; a colliding key is refused with an exact message and the 64-bit second-preimage cost is accepted (note 22). A security review found that undo/redo of a type change ignored the class of the type's items and quoted field values in its conflict; both are fixed, and `types.define` now also carries the class of every item of the type, ancestors included (note 21). The adversarial review found that a pack retype into a new type was logged as a private write (the price of fail-closed), that focus summaries and `day.state` counts revealed private items, and that the key-collision message asked for a removal that does not exist; all fixed (notes 22, 23). First-party signing keys stay owner-held: the app ships with an empty `FIRST_PARTY_KEYS` until the owner generates and publishes one (see `hub/docs/RELEASE.md`).
- **2026-11-11, card guidance (owner request):** empty cards only had a one-line hint, and several hints named integrations that don't exist (HR roster, incidents, carbon platform, Slack). Every role-pack card now has a "how to fill" guide (`app/src/lib/widgets/fill.ts`): a short version in the empty state and a full one behind a **?** in the card header. A Connect button appears only for GitHub and the calendar feed, the two connectors with a Settings form; every other source says plainly that it has no connector yet and offers a copyable agent prompt that names the item type, tells the agent to read its fields with `types_get` and not to invent values. Follow-up worth doing: Settings forms for the local git, energy and MCP-bridge (Slack, Linear, Jira) connectors, which exist in the runtime but can only be connected by the user today with no UI. Also from the last review round: `history.list` now redacts, for agents, the events of deleted items (CONTRACT note 23).

## 9. Research appendix

Schemas and module lists from the four research reports (developer, EM, CSO, architecture) are kept in the session and will be committed as `docs/hub/research/*.md` once the plan is accepted.
