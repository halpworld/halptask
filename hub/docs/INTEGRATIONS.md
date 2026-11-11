# Integrations (P3 Developer pack, P4 calendar, energy)

Integrations pull outside data into the outline as ordinary items: GitHub pull requests and CI runs, Slack asks and Linear or Jira issues read through an MCP server, snapshots of local git repositories, calendar meetings (P4), and energy readings for the CSO pack (grid carbon intensity from Electricity Maps, site power from an MQTT meter). They only read from the outside world. They never write back to GitHub, Slack or a tracker.

Two crates are involved:

- **halp-core** `commands/integrations.rs` holds the commands, the config entities and the write path (`ext.upsert`, `ext.status`).
- **halp-integrations** holds the runtime that does the network and process work. It is attached to the Core as a `HostHandle` extension (`Core::set_extension`), and only the app process starts it. In halpd and in tests without a runtime, `integrations.list` reports `running: false` with state `stopped`, and the commands that need the runtime (`sync_now`, `context.capture`, and token entry) fail.

## Commands

| Command | Tier | Who | What |
|---|---|---|---|
| `integrations.list` `{}` | T0 | anyone, MCP tool | `{running, integrations: [{config, status}]}` |
| `integrations.sync_now` `{id?}` | T1 | anyone, MCP tool | `{queued: [ids]}`. Wakes one enabled integration, or all of them. |
| `context.capture` `{repo_path, integration?}` | T1 | anyone, MCP tool | Snapshots a git repo under a configured root as a `dev.context` item, then returns a `CaptureTicket`. |
| `integrations.connect` `{id, kind, title?, enabled?, interval_secs?, roles?, settings?, reauthorize?}` | T2 | user only | Adds or changes a config. `kind` is fixed once the id exists. The settings are validated by the connector. |
| `integrations.disconnect` `{id, remove_items?}` | T3 | user only | Removes the config, its links and its keychain token. With `remove_items` it also deletes the synced items. |
| `ext.upsert` `{scope, complete, items}` | T1 | integrations only, hidden | The only way synced items are written. |
| `ext.status` `{state, error?}` | T1 | integrations only, hidden | Records a state change as an `integrations.status` audit event. |
| `ext.patch_fields` `{id, type_id, if_version?, fields}` | T1 | integrations only, hidden | Changes the whitelisted fields (`x-halp-ext-writable` on a built-in or pack type) of an item the integration owns: its `origin` is `integration:<id>` and it has that integration's external link. `general` items only; anything else is `not_found`. See CONTRACT.md v1.8. |

Limits:

- `interval_secs` is 60 to 86400 (default 300).
- `settings` is at most 16 KiB.
- One upsert carries at most 500 items.
- An external id is at most 300 characters.
- `roles` defaults to `["developer"]`.

Actor and policy: the runtime dispatches as `integration:<id>`. The policy lets that actor write only through `ext.upsert` and `ext.status`, with its own rate bucket `integration:<id>`. An upsert refused for its rate becomes a transient error with a 60 s retry.

## How synced items behave

- Each integration gets a root bullet (titled after the integration). Its items go under it, and the bullet is recreated if the user deletes it.
- Every item has `origin: "integration:<id>"`. Because external origin marks content as untrusted, an agent that reads it becomes tainted (per actor, 30-minute TTL).
- Synced types stay in the default `general` data class.
- Items are matched by `(integration, external_id)` through `external_link` entities. A sync that produces the same normalised content (by SHA-256) changes nothing and writes no history event.
- On a change, the remote-owned parts are overwritten: title, notes, due, type and fields. Tags are unioned, so user tags survive. The status of a leaf task follows the remote.
- When a batch is `complete`, items of that `scope` that are missing from it are **archived**, never deleted: they get the `archived` tag and leaf tasks become `done`. If an item comes back, the tag is removed. Widgets should hide items tagged `archived`.
- Before writing, titles and text are sanitised: control characters and bidi overrides are removed, and lengths are capped.

## Kinds and settings

### `github`

```json
{ "repos": ["owner/name"], "api_url": null, "auth": "token", "oauth": null, "ci": true, "max_prs": 30 }
```

- **Auth:** `auth` is `token` (a personal access token, the default) or `oauth`. For `oauth`, set `oauth: {authorize_url, token_url, client_id, scopes}`. Classic GitHub OAuth apps need a client secret, so a PAT is the practical default.
- **`api_url`:** use this for GitHub Enterprise. It must be https.
- **What one sync fetches:**
  1. Two searches: `is:pr is:open archived:false review-requested:@me` and `author:@me`, filtered locally to `repos` (an empty list means all repos).
  2. For each PR: its detail, its reviews, and its Actions runs for the head SHA. GitHub lists reviews oldest first, 100 per page, so every page is read up to the last (`Link: rel="last"`); past 10 pages, the first page and the last 9 are read, so the newest decisions always count.
- **Item types:**
  - `dev.pull_request` task: external id `github:o/r#n`, title `T (o/r#n)`. Fields: repo, number, state, requested_from_me, authored_by_me, ci, url, author, reviewers, updated_at, additions, deletions, branch, mergeable.
  - `dev.ci_run` bullet: external id `github:o/r@branch:wf<id>`. Fields: repo, branch, workflow, status, url, started_at, finished_at, pr_number.
  - Scopes are `prs` and `ci`.
  - A batch is not `complete` when the search reports incomplete results or more PRs than `max_prs`, so nothing is archived on a partial view.
- **HTTP behaviour:**
  - Responses are cached by ETag (up to 1000 entries and 32 MiB; bodies over 1 MiB are not cached), and a 304 reuses the cached body.
  - 401 → `needs_auth`.
  - 403 or 429 with rate-limit headers → `error`, retried no earlier than the reset time.
  - A plain 403 or a 404 → `error` (a configuration problem).

### `mcp`

Reads one tool of an MCP server and maps the result.

```json
{ "transport": "stdio", "command": "/abs/path/server", "args": [], "env": {}, "token_env": "LINEAR_API_KEY",
  "adapter": "linear", "tool": "list_my_issues", "tool_args": {} }
```

```json
{ "transport": "http", "url": "https://mcp.example.com/mcp", "auth": true, "adapter": "slack", "tool": "...", "tool_args": {} }
```

- **Adapters:**
  - `slack` produces `dev.ask` tasks (scope `asks`).
  - `linear` and `jira` produce `ext.issue` tasks (scope `issues`). `ext.issue` is a built-in type.
  - The tool result is read from `structuredContent`, or from its text parsed as JSON. Records are the array itself, or the first array under a familiar key such as `issues`, `messages` or `items`, at most 200.
- **stdio launch rules:**
  - `command` must be an absolute path to a file. No shell is involved.
  - At most 32 args and 32 env entries.
  - The environment is cleared. The server gets only:
    - `PATH=/usr/bin:/bin:/usr/sbin:/sbin`
    - `HOME` and `TMPDIR` pointing at a private directory in the integration data dir
    - `LANG=C.UTF-8`
    - the configured `env`
    - the token in `token_env`
  - Env names must match `^[A-Z][A-Z0-9_]{0,63}$`. Variables that change the runtime, loader, shell, TLS trust or proxy rather than configure the server are refused (any case): `PATH`, `HOME`, `SHELL`, `IFS`, `ENV`, `BASH_ENV`, `LANG`, `TMPDIR`, `NODE_OPTIONS`, `PYTHONSTARTUP`, `PYTHONPATH`, `PERL5OPT`, `RUBYOPT`, `JAVA_TOOL_OPTIONS`, `_JAVA_OPTIONS`, `JDK_JAVA_OPTIONS`, `CLASSPATH`, `SSL_CERT_FILE`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`, `NODE_EXTRA_CA_CERTS`, `HTTP_PROXY`, `HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`, and every name starting with `LD_`, `DYLD_`, `NODE_`, `PYTHON`, `PERL`, `RUBY`, `JAVA_`, `GIT_`, `SSH_`, `NPM_`, `LC_`, `XDG_`, `DOTNET_`, `COR` or `GCONV`. Only the user can set `env` (`integrations.connect` is user-only).
  - Other process limits:
    - stderr is discarded.
    - stdout is capped at 8 MiB.
    - The process is killed when the session ends.
    - A call times out after 30 s.
- **HTTP transport:** the URL must be https and the token is sent as a bearer token when `auth` is true. A server-sent event is capped at 4 MiB and a JSON response body at 16 MiB (the `rmcp` limit), within the 30-second call timeout; a larger answer fails the sync. It is not yet tested against a live server.

### `git`

```json
{ "roots": ["/Users/me/code"] }
```

- Never polls. It writes only on `context.capture`.
- macOS and Linux only for now. On Windows no git binary is found, so validation fails; the stdio MCP and git tests are `cfg(unix)`.
- `repo_path` must be absolute. It is canonicalised, so symlinks are resolved, and it must lie under a canonical root. It must also be the top of a work tree. The filesystem root cannot be a root.
- **How git is run:**
  - git comes from a fixed system location (`/usr/bin`, `/opt/homebrew/bin`, `/usr/local/bin`, `/bin`).
  - The environment is cleared: global and system config are ignored, there is no terminal prompt, and optional locks are not taken.
  - The repository's own `.git/config` is treated as hostile (a repo can come from an archive, a fixture or an agent). Only plumbing that cannot reach a configured program runs: `rev-parse`, `symbolic-ref`, `config --get`, `cat-file commit HEAD` (the subject; no signature check, unlike `git log`), `diff-index --cached` (staged), `diff-files` (changed) and `ls-files --others --exclude-standard` (untracked), all with submodules ignored and no diff text. `git status` and `git log` are not used.
  - Config that names a program is overridden from the environment (`GIT_CONFIG_*`, which beats the repo's config): fsmonitor off, hooks `/dev/null`, `core.attributesFile=/dev/null`, `diff.external`, `core.sshCommand`, `core.askPass`, `credential.helper` and `core.pager` emptied, `gpg.*program=false`, `log.showSignature=false`, `protocol.allow=never`, no lazy fetch. Every filter driver the repo's config defines (`filter.<name>.clean|smudge|process`) is read first and switched off, because comparing a racily clean file re-hashes it through its clean or process filter. A repo with more than 32 drivers is refused.
  - Since nothing refreshes the index, a file whose timestamps changed but whose content did not may be listed as changed.
  - The captured text is the repository's own, written as the integration (`origin: integration:<id>`), so it is untrusted content that taints an agent that reads it.
  - Each run has a 10 s timeout and a 1 MiB output cap (output past the cap is cut at the last whole name and git is stopped).
- Produces a `dev.context` bullet: external id `git:<hash of path>#<branch>`, title `repo @ branch`. Fields: repo (`owner/name` from origin, or `local/<dir>`), branch, captured_at, files (up to 100 changed paths), note (last commit subject).

### `calendar` (P4)

Reads your calendar for the window **today − 7 days to today + 14 days** (local midnights in the calendar's zone) and writes meetings as bullets. It never writes back to the calendar.

```json
{ "source": "ics", "item_types": ["dev.meeting", "em.meeting"], "me": ["me@acme.com"], "domains": ["acme.com"],
  "timezone": null, "include_description": false, "skip_declined": true, "link_one_on_ones": true }
```

```json
{ "source": "mcp", "mcp": { "transport": "stdio", "command": "/abs/path/calendar-server", "tool": "list_events", "tool_args": {} },
  "item_types": ["em.meeting"], "me": ["me@acme.com"] }
```

- **Sources:**
  - `ics`: a published iCalendar feed (Google "secret address in iCal format", Outlook "publish calendar" ICS link, Apple, Fastmail, …). **The feed URL is the secret.** It is not in `settings`; it is set like a token (`integrations_set_token`) and kept in the keychain. Only `https://` is accepted (`webcal://` is turned into `https://`); anything else is refused with `invalid_url`. Redirects are refused, like every connector request.
  - `mcp`: one tool of a calendar MCP server, with the same launch rules as the `mcp` kind (`adapter` is forced to `calendar`; leave it out). Records are read from `items`, `events`, `value` and the other familiar keys, at most 2000. Google Calendar (`summary`, `start.dateTime|date`, `hangoutLink`, `eventType`, `transparency`, `attendees[].self|resource|responseStatus`), Microsoft Graph (`subject`, `start.dateTime` + `timeZone` with Windows zone names, `showAs`, `isAllDay`, `onlineMeeting.joinUrl`, `sensitivity`) and plain `{id, title, start, end}` shapes are understood. Cancelled, `workingLocation` and series-master records are dropped.
  - CalDAV is not supported.
- **The secret URL stays secret.** It never appears in settings, events, history, the audit log, status errors or logs (error texts are redacted with it, and parser errors, which quote the input, are replaced by a fixed text). The app shows only a hint, the feed's host (for example `calendar.google.com`), from `Runtime::secret_hint(id)`; no part of the path or query reaches the webview.
- **ICS parsing** (crates `icalendar` with its parser, `rrule`, `chrono-tz`):
  - `RRULE` (up to 4 per event, `UNTIL` normalised to UTC; `HOURLY` and finer, and rules asking for more than 24 times a day through `BYHOUR` × `BYMINUTE` × `BYSECOND`, are not expanded), `RDATE`, `EXDATE`, and `RECURRENCE-ID` overrides (moved or cancelled instances). At most 500 instances per series and 20 000 in all are expanded, and expansion of one feed stops after 2 seconds. A rule that cannot be read, a fifth or later `RRULE`, or a stopped expansion makes the batch not complete, so nothing is archived on a partial view.
  - The same `UID` more than once (or the same `UID` and `RECURRENCE-ID` for an override): the newest revision wins, by `SEQUENCE`, then the later of `LAST-MODIFIED` and `DTSTAMP`, whatever the order in the feed.
  - `TZID` by IANA name, Windows name (`W. Europe Standard Time`) or vendor-prefixed path (`/mozilla.org/…/Europe/Stockholm`); unknown zones and floating times use `timezone`, else the feed's `X-WR-TIMEZONE`, else the computer's zone. Times in a DST gap move forward one hour.
  - All-day events (`VALUE=DATE`) run from local midnight to local midnight in that zone and are not busy unless marked `OPAQUE`.
  - `STATUS:CANCELLED` events are left out, so an event cancelled after it was synced is archived.
  - `CLASS:PRIVATE` / `CONFIDENTIAL` events become `Private event`, with no location, link or description.
  - Limits: the body is at most 5 MiB (larger is refused while downloading), at most 20 000 `VEVENT`s, nesting at most 6 deep. A body that does not start with `BEGIN:VCALENDAR` is refused with `the feed URL did not return an iCalendar feed`. At most 2000 instances in the window are kept (nearest to now); then the batch is not complete, so nothing is archived.
  - Text is cleaned of control, bidi and zero-width characters and capped: title 300, location 200, description 2000, attendees 100.
- **HTTP:** 401, 403, 404 or 410 → `needs_auth` (`the calendar feed URL was refused (HTTP n); set it again`). 429 and 5xx → `error`, retried no earlier than `Retry-After`.
- **Item types** (`item_types`, default both):
  - `dev.meeting` bullet: external id `ics:dev:<key>` (or `mcp:dev:<key>`). Fields: starts_at, ends_at (RFC 3339 UTC), location, url, busy.
  - `em.meeting` bullet: external id `ics:em:<key>`. The same fields, plus attendees_count, organizer_is_me, recurring and category.
  - Both carry the tag `calendar`. Notes hold the description only with `include_description`.
  - `<key>` is the UID for a single event and `<UID>@<instance start in UTC, or the date for all-day>` for an instance of a series (a moved instance keeps its original key). UIDs over 200 characters are hashed.
  - The scope is the type id, and a batch is `complete` unless capped (500 per type, nearest to now), so events that leave the window or the feed are archived.
  - Events the user declined (an attendee in `me` with `PARTSTAT=DECLINED`) are left out with `skip_declined`.
  - Each item is shown only in its pack's role (`dev.meeting` → `developer`, `em.*` → `eng-manager`), so each pack sees one copy. A type whose role the integration is not connected for (default roles `["developer"]`; no roles means every role) is not written at all: connect with `roles: ["eng-manager"]` (or both) to get `em.meeting` items and 1:1 linking. An `ext.upsert` item may carry `roles`, a non-empty subset of the integration's roles.
- **`em.meeting` category**, the first that fits:
  1. `focus`: a focus-time or out-of-office event, a title with focus, deep work, no meetings, out of office, OOO, vacation, holiday or PTO, or nobody else invited (unless the title is a team ritual).
  2. `1on1`: exactly one other person.
  3. `external`: the organizer's domain is not in `domains` (default: the domains of `me`).
  4. `team`: standup, daily, retro, planning, sprint, refinement or grooming in the title.
  5. `cross-team` otherwise.
- **1:1 linking** (`link_one_on_ones`, default true):
  - The connector reads `em.person` and `em.one_on_one` items (as the integration, T0 `tasks.list`, at most 1000 each). A failed read fails the sync (it is not taken as "nobody"). When 1000 1:1s come back, some may be missing, so no new 1:1 is created that sync (moves still happen). An event is a 1:1 with a person when it has exactly two participants (resources left out, the organizer counted), one is you (`me`) and the other matches exactly one person: by `fields.email`, else by display name, else by the address's local part against the name (`alice.andersson@` ↔ `Alice Andersson`). All-day, private and focus/OOO events are never 1:1s.
  - For each person, the next 1:1 from now on counts. When the person has no open `em.one_on_one` (status absent or `scheduled`, not started, not archived), one is created: external id `1on1:<person id>:ics:<key>`, title `1:1 with <name>`, fields `{person_id, scheduled_for, status: "scheduled"}`, tag `calendar`, scope `one_on_ones` (never complete, so nothing is archived).
  - When an open one exists and its `scheduled_for` differs, the connector moves it with `ext.patch_fields` (see CONTRACT.md v1.8), which changes only `scheduled_for`. It moves only 1:1s it created itself (`origin` is this integration); when the person's next open 1:1 is the user's own or another integration's, it neither moves it nor creates a second one. A failed move is skipped and retried on the next sync.
  - The connector never writes `agenda`, `status` after creation, or any `em.private_note`. An item it created is never re-sent with different content, so its agenda is safe.

### `energy`

Writes one `cso.energy_reading` item per grid zone or meter (the CSO pack's energy widget reads them). It never writes anything back.

```json
{ "source": "electricity_maps", "zones": ["DE", "SE-SE3"], "zone_labels": { "SE-SE3": "Stockholm" },
  "history": true, "renewable": true }
{ "source": "mqtt", "mqtt": { "host": "broker.example.com", "port": 8883, "username": "hub", "wait_secs": 5,
  "meters": [ { "topic": "site/hq/power", "zone": "hq-main", "site": "HQ", "pointer": "/data/w", "at_pointer": "/data/ts", "unit": "W" },
              { "topic": "site/lab/kw", "zone": "lab" } ] } }
```

- **Items.** External id `emaps:<zone>` or `mqtt:<zone>`, kind `bullet`, tag `energy`, title `Grid carbon <label or zone>` or `Site power <site or zone>`, scope `readings`. With the integration connected for the `cso` role, items carry `roles: ["cso"]`. Fields, exactly as in `packs/cso/pack.json`:
  - `source` (`electricity_maps` or `mqtt`), `zone`, `zone_label` (Electricity Maps), `site` (MQTT), `at` (the reading's own time, RFC 3339, whole seconds).
  - `gco2_per_kwh` (0–3000, one decimal) and `renewable_pct` (0–100, one decimal) from Electricity Maps; `power_kw` (±1,000,000, three decimals) from MQTT.
  - `series`: a rolling list of `{at, gco2_per_kwh | power_kw}`, oldest first, one sample per time, **at most 48**. Each sync merges the item's previous series (read back with `tasks.list` as the integration) with the new samples; a failed read skips the write so the series is never cut short.
- **Never invented, so stale shows.** A zone or meter without a usable reading this sync is left out of the batch, so its item keeps its old `at` and the widget shows it as stale (older than `staleMinutes`, 30 by default). A reading older than the item's `at` never replaces it (even when that `at` is ahead of the clock), and times more than 5 minutes in the future are refused. The previous items are read with `tasks.list` (at most 1000); when the page is full, only readings whose item was found are written and the batch is not complete. The batch is complete (and so archives items of removed zones or meters) only when every zone or meter produced a reading. When none did, the sync fails (`error`, or `needs_auth` below) and nothing is written.
- **Numbers.** Values are JSON numbers or numeric strings of at most 64 characters, finite, and inside the ranges above; anything else (`NaN`, `Infinity`, `1e999`, 3001 g, out-of-range power) is dropped like a missing reading. Text (`zone_labels`, `site`) is cleaned like calendar text: control, bidi and format characters removed, whitespace collapsed, at most 120 characters. Zones are 1–64 of `A–Z a–z 0–9 _ . / : -`, at most 10 zones or meters.
- **`electricity_maps`** (API v3, `https://api.electricitymaps.com/`; `api_url` overrides it, https only, no credentials or query). **The API token is the secret**: it is set with `integrations_set_token`, kept in the keychain and sent only in the `auth-token` header (marked sensitive). Per zone, one sync makes up to three GETs: `v3/carbon-intensity/latest?zone=` (required: `carbonIntensity`, `datetime`; capped at 64 KiB), `v3/carbon-intensity/history?zone=` with `history` (the last 24 h, at most 200 records, 512 KiB), and `v3/power-breakdown/latest?zone=` with `renewable` (`renewablePercentage`, used only when its `datetime` is within 90 minutes of the carbon reading). An answer for another zone is ignored. The answer's `isEstimated` is kept as the item's `estimated` field (`true` for a modelled value, `false` for a measured one, absent when the answer does not say). HTTP 401 → `needs_auth`; 429 and 5xx → `error`, retried no earlier than `Retry-After`; 400, 403 and 404 fail only that zone (403 for every zone → `needs_auth`, "check the token's plan"). The shapes were taken from the public API description and are parsed defensively; they are tested against fixtures, not the live service.
- **`mqtt`** (MQTT 3.1.1). One sync connects, subscribes to every meter topic at QoS 0 with a clean session, reads for at most `wait_secs` (1–30, default 5, or until every meter has a value), sends DISCONNECT and closes.
  - `host` is a host name or IP, no scheme. **TLS is required** unless the host is `localhost` or a loopback address (`tls: false` is refused otherwise); the server certificate is checked against the OS trust store (rustls with the platform verifier, as for HTTPS). `port` defaults to 8883 with TLS, 1883 without.
  - **The password is the secret**: with `username` set, the password is set with `integrations_set_token` (no username, no secret). A refused login (CONNACK 4 or 5) → `needs_auth`.
  - Topics are exact (no `+` or `#`), at most 256 bytes, unique. A payload is a plain number, or JSON read at `pointer` (a JSON pointer like `/data/w`); `unit` is `kW` (default) or `W`. Payloads over 4 KiB, packets over 8 KiB and more than 1 MiB per sync are not read.
  - **Time:** a live message is stamped when it arrives. A retained message may be days old, so it counts only when the payload carries its own time at `at_pointer` (RFC 3339 or epoch seconds or milliseconds). The latest value per meter wins.
  - No usable value from any meter → `error` (`no usable meter reading arrived within n s`). Errors name only the kind of failure, never payloads, the username or the password.
  - The packet codec is `rumqttc`'s (`mqttbytes::v4`, default features off); the client loop and TLS are ours. Tested against an in-process broker on 127.0.0.1 (plain TCP); **the TLS path is not tested end to end.**
- **Secrets.** Neither the token nor the password is ever in settings (unknown keys such as `token` or `password` are refused), events, history, the audit log, `integrations.list`, items, status errors or logs; sync errors are redacted with the secret. There is no `secret_hint` for `energy`.

## Tokens

Tokens never travel through `dispatch`, command arguments, events, history or the audit log.

- The app has a separate Tauri command, called from the UI as `invoke("integrations_set_token", { id, token })`. It resolves to `undefined`, or rejects with one of these error codes:

  | Code | Meaning |
  |---|---|
  | `not_running` | the integration runtime is not running |
  | `unknown_integration` | `id` does not match a configured integration |
  | `empty` | the token is empty |
  | `too_long` | the token is over 4 KiB |
  | `not_printable_ascii` | the token contains characters other than printable ASCII |
| `invalid_url` | `calendar` with source `ics`: the feed URL is not a valid https (or `webcal`) URL |
  | `keychain_error` | the keychain write failed |
  | `forbidden` | called from a window other than `main` |
  | `internal` | anything else |

- The token is checked (at most 4 KiB, printable ASCII, `id` an existing config), held in zeroizing memory, and written to the OS keychain (service `halptask-hub`, account `integration:<id>`). It is never echoed or logged.
- Setting a token marks `has_secret` and wakes the sync loop. `integrations.disconnect` deletes the token.
- **A token follows its destination.** When `integrations.connect` changes a config's kind, or where its secret is sent, the stored token is deleted and `has_secret` becomes false, so a token meant for one service is never sent to another. "Where" is: the GitHub `api_url` origin and auth mode (with OAuth, the token URL's origin and client id); an `mcp` server's URL origin, or its stdio command and `token_env`; a calendar's source (`ics` / `mcp`) and its MCP server; an energy source, `api_url`, or MQTT host and port. Other edits keep it. If the keychain delete fails, the old token is not used: every sync and authorization tries the delete again first and reports an error until it succeeds.
- `integrations_set_token` and `integrations_secret_hint` are granted to the `main` window only (capability `hub-main`), and the commands also refuse other windows with `forbidden`.
- Error texts from a sync are redacted with the token (and any refresh token) before they reach status, events or logs.

### OAuth (authorization code with PKCE)

- When a config has an OAuth provider and no token (or `reauthorize: true`), the runtime starts a one-shot loopback listener on `127.0.0.1:<random>/callback`. It then opens the authorize URL in the system browser. The opener accepts only https URLs whose host matches the provider, without credentials, passed as a single argument to `open` or `xdg-open`.
- `state` is 32 random bytes and is compared in constant time. A callback with the wrong (or no) `state` gets 400 and is ignored; the listener keeps waiting for the real one until the timeout (then the flow fails with "authorization timed out after a callback with the wrong state" if a wrong one was seen, else a plain timeout). The callback with the right `state` ends the listener, with success or the provider's error. Connections are served concurrently (at most 16 at once), each with a 5-second read timeout, so a stalled connection cannot block the real callback. Paths other than `/callback` get 404.
- The flow times out after 5 minutes.
- The code is exchanged with the S256 verifier. `expires_in` is capped at 10 years (a larger or overflowing value cannot make the token look never-expiring or crash). Tokens are refreshed when they expire within 60 s, or once after a 401. The old refresh token is kept if the provider does not return a new one.
- The OAuth flow is unit-tested against a mock provider. The full app flow is not tested end to end.

## Sync loop and status

- One task runs per integration, and at most 4 syncs run at once.
- **States:**
  - `pending`: configured, not synced yet.
  - `syncing`
  - `ok`: the next sync is after `interval_secs` ±10%.
  - `error`: retried with exponential backoff, `interval·2^(n-1)` capped at 1 h, ±20% and no earlier than a server's retry-after.
  - `needs_auth`: no timer runs; the loop waits for a token.
  - `disabled`
  - `stopped`: no runtime in this process.
- **Status and events:**
  - `integrations.list` returns `last_sync`, `next_sync`, `error`, `items`, `has_secret` and `authorizing` for each integration.
  - A state change is reported through `ext.status`, but only when `(state, error)` changes. The UI sees it as an `integrations.status` event.
  - `ext.upsert` commits a normal history event with a summary such as `Synced GitHub: 2 new, 1 updated, 0 archived`.
- A `connect` restarts that integration's loop with the new settings, and a `disconnect` cancels it. On app exit the runtime is stopped.
