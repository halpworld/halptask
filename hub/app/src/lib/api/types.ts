// Types mirroring the halp-core structs (hub/crates/halp-core). The core's
// `commands.list` output (hub/docs/fixtures/commands.json) is the source of
// truth; tests/contract.test.ts checks the UI's call sites against it.

export type Tier = 'T0' | 'T1' | 'T2' | 'T3';

export type ErrorCode = 'not_found' | 'invalid_args' | 'denied' | 'confirmation_required' | 'conflict' | 'internal';

export type JsonValue = string | number | boolean | null | JsonValue[] | { [key: string]: JsonValue };
export type Args = Record<string, unknown>;

export interface HubRequest {
  command: string;
  args: Args;
  request_id: string;
}

export interface HubError {
  code: ErrorCode;
  message: string;
  confirmation_id?: string;
}

export type HubResponse<T = unknown> = { ok: true; result: T } | { ok: false; error: HubError };

export interface JsonSchema {
  type?: string | string[];
  properties?: Record<string, JsonSchema>;
  required?: string[];
  items?: JsonSchema;
  enum?: JsonValue[];
  description?: string;
  format?: string;
  [key: string]: unknown;
}

export interface CommandDescriptor {
  name: string;
  description: string;
  tier: Tier;
  input_schema: JsonSchema;
  output_schema: JsonSchema;
  /** Added in P1: `general` unless the command touches sensitive data. */
  data_class?: string;
  /** Added in P1: false for consent/grant/client/rule commands. */
  mcp?: boolean;
  /** v1.3: only the user may run it (never an MCP tool). */
  user_only?: boolean;
}

export type Actor = 'user' | 'system' | `agent:${string}` | `module:${string}` | `integration:${string}`;

export interface EntityRef {
  kind: string;
  id: string;
}

export interface HubEvent {
  id: string;
  ts: string;
  actor: Actor;
  command: string;
  entity: EntityRef;
  summary: string;
}

/** A `history.list` row: an event plus its place in the undo log. */
export interface HistoryEntry extends HubEvent {
  seq: number;
  /** For undo/redo rows: the event they reversed or re-applied. */
  causation_id: string | null;
  undoable: boolean;
}

export type ItemKind = 'bullet' | 'task';
export type ItemStatus = 'todo' | 'in_progress' | 'done';

export interface Item {
  id: string;
  parent_id: string | null;
  order: number;
  kind: ItemKind;
  status: ItemStatus;
  title: string;
  notes: string;
  tags: string[];
  /** Ancestor tags not already on the item, nearest ancestor first. */
  inherited_tags: string[];
  due: string | null;
  type_id: string | null;
  fields: Record<string, JsonValue>;
  /** Role ids this item belongs to. Empty: inherit from the nearest ancestor (or all roles). */
  roles: string[];
  /** Who created it: `user`, `agent:<client>`, ... */
  origin: string;
  created_at: string;
  updated_at: string;
  version: number;
}

export interface TaskType {
  id: string;
  title: string;
  icon: string;
  schema: JsonSchema;
  display: { primary_fields: string[]; badge_field?: string };
}

/** `types.list` rows carry where the type came from (a pack id or `user`). */
export interface TaskTypeView extends TaskType {
  origin: string;
}

export interface Widget {
  id: string;
  kind: string;
  title: string;
  x: number;
  y: number;
  w: number;
  h: number;
  config: Record<string, JsonValue>;
}

export interface Layout {
  id: string;
  role_id: string;
  columns: number;
  widgets: Widget[];
}

export type RoleId = 'developer' | 'eng-manager' | 'cso';

export interface Role {
  id: string;
  title: string;
  accent: string;
  layout_id: string;
  task_types: string[];
  widgets: string[];
}

export interface DayCounts {
  overdue: number;
  today: number;
  in_progress: number;
}

export interface DayState {
  date: string;
  role: string;
  focus: Item | null;
  counts: DayCounts;
  agenda: Item[];
  layout: Layout;
}

export interface TaskFilter {
  type_id?: string;
  /** One status or several; implies `kind: task`. */
  status?: ItemStatus | ItemStatus[];
  kind?: ItemKind;
  /** Matches own and inherited tags. */
  tag?: string;
  due_before?: string;
  due_after?: string;
  text?: string;
  /** `null` = roots only. */
  parent_id?: string | null;
  /** Items whose effective roles include this role (or that have none). */
  role?: string;
  limit?: number;
}

export interface ConsentTarget {
  kind: string;
  id: string;
  title: string;
  /** How many items below it the command also touches. */
  descendants: number;
}

/** What `consent.get` / `consent.list` return (halp-core PendingConsent). */
export interface PendingConfirmation {
  /** 64 hex chars. */
  id: string;
  command: string;
  /** The effective tier (an undo that deletes is T3). */
  tier: Tier;
  /** The requesting actor; the approved command runs as this actor. */
  actor: Actor;
  /**
   * Shown with secrets and file contents redacted by the core, and with
   * pinned values (Inbox parent_id, resolved dues) as they will run. May be
   * absent in a slimmer payload (a private item's `consent.requested`).
   */
  args?: Args;
  args_hash?: string;
  /** The request is about a private item (CONTRACT §P3.1). */
  private?: boolean;
  /** Built by the core from resolved targets, e.g. 'Delete "Q3 plan" and 4 subtasks'. */
  summary: string;
  details?: { targets: ConsentTarget[]; effects: string[] };
  /** The agent read external content before asking. */
  tainted?: boolean;
  taint_source?: string | null;
  created_at: string;
  /** After this the core auto-denies. */
  expires_at?: string;
  /**
   * The args are too large to display in full. The dialog then says so and
   * offers Deny only: the user must see everything they approve.
   */
  display_truncated?: boolean;
}

/** `capture.parse` result, and `capture.quick`'s `parsed`. */
export interface ParsedCapture {
  title: string;
  tags: string[];
  due: string | null;
  kind: ItemKind;
  status: ItemStatus;
}

export interface CaptureResult {
  item: Item;
  parsed: ParsedCapture;
}

/** Keys `tasks.update` accepts in `patch`; anything else is invalid_args. */
export interface ItemPatch {
  title?: string;
  notes?: string;
  tags?: string[];
  due?: string | null;
  type_id?: string | null;
  fields?: Record<string, JsonValue>;
  /** Refused for bullets and for items with subtasks (their status is derived). */
  status?: ItemStatus;
  kind?: ItemKind;
  roles?: string[];
}

export interface CreateArgs {
  title: string;
  kind?: ItemKind;
  parent_id?: string;
  index?: number;
  notes?: string;
  tags?: string[];
  due?: string;
  type_id?: string;
  fields?: Record<string, JsonValue>;
  status?: ItemStatus;
  roles?: string[];
}

export interface StepResult {
  /** The event that was undone/redone. */
  event: HubEvent | null;
  /** The new history row recording the undo/redo itself. */
  history_event: HubEvent | null;
}

// ---- Agent administration (halp-core commands/admin.rs, CONTRACT v1.3/v1.4)

/** What a client may do. `t1`: reversible local writes run without asking. */
export interface Grants {
  t1: boolean;
  /** Sorted, de-duplicated. */
  data_classes: string[];
}

/** What the API shows of a paired MCP client (never the token hash). */
export interface ClientView {
  id: string;
  name: string;
  /** `agent:<id>`: the actor its calls run as. */
  actor: string;
  created_at: string;
  revoked_at?: string | null;
  grants: Grants;
  /**
   * v1.8, `clients.list` only: when the policy last decided a call of this
   * client. Kept in memory by the core, so null again after a restart.
   */
  last_seen?: string | null;
  /** v1.8, `clients.list` only: until when it is tainted, or null. */
  tainted_until?: string | null;
}

/** `clients.pair`: the token is shown once; only its SHA-256 is stored. */
export interface PairResult {
  client: ClientView;
  token: string;
}

/** A standing rule: one client may run one T2 command with exactly these top-level args. */
export interface Rule {
  id: string;
  client_id: string;
  command: string;
  args: Record<string, JsonValue>;
  created_at: string;
  created_by: string;
}

/** `allow`, `deny`, `confirm`, `approved`, `failed`, `denied`, `expired`, `superseded`. */
export type AuditDecision = 'allow' | 'deny' | 'confirm' | 'approved' | 'failed' | 'denied' | 'expired' | 'superseded';

export interface AuditEntry {
  id: string;
  ts: string;
  actor: string;
  command: string;
  tier: string;
  args_hash: string;
  decision: string;
  reason: string;
  approver?: string | null;
  confirmation_id?: string | null;
  tainted: boolean;
}

export interface ClearTaintResult {
  client_id: string;
  was_tainted_by?: string | null;
}

export interface PurgeResult {
  redacted: number;
  retention_days?: number | null;
}

export interface ImportReport {
  format: string;
  items: number;
  roots: number;
  skipped_deleted: number;
  warnings: string[];
}

export type IntegrationState = 'pending' | 'syncing' | 'ok' | 'error' | 'needs_auth' | 'disabled' | 'stopped';

export interface IntegrationEntry {
  config: {
    id: string;
    kind: string;
    title: string;
    enabled: boolean;
    interval_secs: number;
    roles: string[];
    settings: unknown;
    created_at: string;
    updated_at: string;
  };
  status: {
    state: IntegrationState;
    items: number;
    has_secret: boolean;
    authorizing: boolean;
    error?: string | null;
    last_sync?: string | null;
    next_sync?: string | null;
  };
}

type NoArgs = Record<string, never>;

/**
 * Typed command map. Unknown commands still work through `dispatchRaw`.
 */
export interface CommandMap extends PacksCommandMap {
  'tasks.create': { args: CreateArgs; result: Item };
  'tasks.get': { args: { id: string }; result: Item };
  'tasks.list': { args: TaskFilter; result: { items: Item[] } };
  'tasks.update': { args: { id: string; patch: ItemPatch; if_version?: number }; result: Item };
  /** `parent_id` absent = stay under the current parent; `null` = move to the root. */
  'tasks.move': { args: { id: string; parent_id?: string | null; index?: number }; result: Item };
  /** A parent without `cascade: true` is invalid_args. */
  'tasks.complete': { args: { id: string; done?: boolean; cascade?: boolean }; result: Item };
  'tasks.delete': { args: { id: string }; result: { deleted: string[] } };
  'history.undo': { args: { event_id?: string }; result: StepResult };
  'history.redo': { args: { event_id?: string }; result: StepResult };
  'history.list': { args: { limit?: number; before?: string; actor_prefix?: string }; result: { events: HistoryEntry[]; can_undo: boolean; can_redo: boolean } };
  'types.list': { args: NoArgs; result: { types: TaskTypeView[] } };
  'types.get': { args: { id: string }; result: TaskType };
  'types.define': { args: TaskType; result: TaskType };
  'layout.get': { args: { role_id?: string }; result: Layout };
  'layout.add_widget': { args: { role_id?: string; kind: string; title?: string; x?: number; y?: number; w?: number; h?: number; config?: Widget['config'] }; result: Layout };
  'layout.update_widget': { args: { role_id?: string; widget_id: string; patch: Partial<Omit<Widget, 'id' | 'kind'>> }; result: Layout };
  'layout.move_widget': { args: { role_id?: string; widget_id: string; x: number; y: number }; result: Layout };
  'layout.remove_widget': { args: { role_id?: string; widget_id: string }; result: Layout };
  'role.list': { args: NoArgs; result: { roles: Role[]; current: string } };
  'role.get': { args: { id?: string }; result: Role };
  'role.switch': { args: { id: string }; result: { role: Role; layout: Layout } };
  'focus.get': { args: NoArgs; result: { focus: Item | null } };
  'focus.set': { args: { id: string }; result: { focus: Item } };
  'focus.clear': { args: NoArgs; result: { focus: null } };
  'day.state': { args: { date?: string }; result: DayState };
  'capture.quick': { args: { text: string; parent_id?: string; tags?: string[]; kind?: ItemKind; top?: boolean }; result: CaptureResult };
  'capture.parse': { args: { text: string; tags?: string[]; kind?: ItemKind }; result: ParsedCapture };
  'commands.list': { args: NoArgs; result: { commands: CommandDescriptor[] } };
  'consent.list': { args: NoArgs; result: { pending: PendingConfirmation[] } };
  'consent.get': { args: { id: string }; result: PendingConfirmation };
  'consent.approve': { args: { id: string }; result: unknown };
  'consent.deny': { args: { id: string }; result: { denied: string } };
  'clients.list': { args: NoArgs; result: { clients: ClientView[] } };
  'clients.pair': { args: { name: string; id?: string }; result: PairResult };
  'clients.revoke': { args: { id: string }; result: ClientView };
  'grants.list': { args: { client_id?: string }; result: { grants: { client_id: string; grants: Grants }[]; data_classes: string[] } };
  'grants.set': { args: { client_id: string; t1?: boolean; data_classes?: string[] }; result: ClientView };
  'policy.clear_taint': { args: { client_id: string }; result: ClearTaintResult };
  'rules.list': { args: { client_id?: string }; result: { rules: Rule[] } };
  'rules.create': { args: { client_id: string; command: string; args: Record<string, JsonValue> }; result: Rule };
  'rules.delete': { args: { id: string }; result: { rules: Rule[] } };
  'audit.list': { args: { limit?: number; before?: string; actor_prefix?: string }; result: { records: AuditEntry[] } };
  'history.purge': { args: { older_than_days?: number; deleted_items?: boolean; remember?: boolean }; result: PurgeResult };
  'import.tui': { args: { content_base64: string; file_name?: string; passphrase?: string; utc_offset_minutes?: number }; result: ImportReport };
  'integrations.list': { args: NoArgs; result: { running: boolean; integrations: IntegrationEntry[] } };
  'integrations.sync_now': { args: { id?: string }; result: { queued: string[] } };
  'integrations.connect': { args: { id: string; kind: string; title?: string; enabled?: boolean; interval_secs?: number; roles?: string[]; settings?: JsonValue; reauthorize?: boolean }; result: IntegrationEntry };
  'integrations.disconnect': { args: { id: string; remove_items?: boolean }; result: { id: string; removed_items: number } };
  'context.capture': { args: { repo_path: string; integration?: string }; result: { integration: string; repo_path: string; queued: boolean } };
}

export type CommandName = keyof CommandMap;
export type ArgsOf<K extends CommandName> = CommandMap[K]['args'];
export type ResultOf<K extends CommandName> = CommandMap[K]['result'];

// ------------------------------------------------------------------ Packs
// P5 extension packs (docs/hub/P5-EXTENSIONS.md §2, §3.4, §4.3, §7). The
// shapes match the core (P5-B, CONTRACT v1.10) and docs/fixtures/commands.json.
// Every string that comes from a pack is untrusted: render it as text only.

/** How far a pack is trusted (§2). `dev` is a dev-mode folder. */
export type PackTrust = 'builtin' | 'first_party' | 'community' | 'unsigned' | 'dev';

export interface PackCapabilities {
  modules: number;
  ui_widgets: number;
  prompts: number;
  annotations: number;
}

/** A `packs.list` row (§7), plus the user-only fields the Packs screen needs. */
export interface PackView {
  id: string;
  version: string;
  title: string;
  role_id: string;
  trust: PackTrust;
  enabled: boolean;
  /** Ids of the packs this one extends, nearest first; empty when none. */
  extends: string[];
  /** Full 64-hex key fingerprint (no spaces), or null (builtin, unsigned, dev). */
  key_fp: string | null;
  capabilities: PackCapabilities;
  /**
   * `error`: the stored bundle failed re-verification or the pack failed to
   * load (its types are retired). `blocked`: a pack it extends is missing
   * or not loaded.
   */
  status: 'ok' | 'disabled' | 'error' | 'blocked';
  /** Untrusted text from the manifest. */
  description?: string;
  publisher?: string;
  /** User only: the version `packs.rollback` would return to, or null. */
  previous_version?: string | null;
  /** User only: why the pack is not loaded (status error or blocked). */
  error?: string;
}

/** A `packs.keys_list` row. */
export interface PackKey {
  key_id: string;
  /** Full fingerprint: SHA-256 of the raw public key, 64 hex. */
  fingerprint: string;
  label: string;
  /** first_party keys are compiled in and cannot be revoked. */
  first_party: boolean;
  revoked: boolean;
  added_at: string | null;
  /** How many installed packs are signed with it. */
  packs: number;
}

export type PermissionKind = 'module' | 'ui_widget_call' | 'annotation' | 'data_class' | 'prompt';

export interface PermissionChange {
  kind: PermissionKind;
  /** Module id, call name, `type_id: annotation`, class name or prompt name (untrusted text). */
  id: string;
  detail?: string;
}

/** What an update adds or drops compared with the installed version. */
export interface PermissionDiff {
  added: PermissionChange[];
  removed: PermissionChange[];
}

/** One `type_overrides` entry as the preview shows it. */
export interface PackOverride {
  type_id: string;
  title: string | null;
  icon: string | null;
  property_descriptions: Record<string, string>;
  added_properties: string[];
  added_enum_values: Record<string, unknown[]>;
}

export interface PackPreview {
  /** Bound to `bundle_sha256`; expires after about 10 minutes. */
  preview_id: string;
  bundle_sha256: string;
  expires_at: string;
  id: string;
  version: string;
  title: string;
  description: string;
  publisher: string;
  trust: PackTrust;
  /**
   * `verified`: a trusted key. `unknown_key`: valid signature by a key the
   * user has not trusted yet: trust is `unsigned` (installed without
   * `trust_key`), or `community` when the pack only loads with the key
   * trusted (then `packs.install` needs `trust_key: true`). `none`: no
   * signature. `invalid`: never sent by the core (a bad signature is an
   * error); kept so the UI fails closed if one ever appears.
   */
  signature: 'verified' | 'unknown_key' | 'none' | 'invalid';
  /** Full fingerprint of the signing key, or null when unsigned. */
  key_fp: string | null;
  installed_version: string | null;
  files: { path: string; size: number }[];
  chain: string[];
  role: { id: string; title: string; accent: string };
  types: { id: string; title: string; data_class?: string | null }[];
  /** Privileged schema annotations (`x-halp-immutable-when`, `x-halp-ext-writable`). */
  annotations: { type_id: string; annotation: string }[];
  data_classes: string[];
  modules: { id: string; title: string; inputs: string[] }[];
  ui_widgets: { kind: string; title: string; calls: string[] }[];
  prompts: { name: string; description: string }[];
  /** Null on a first install. */
  permission_diff: PermissionDiff | null;
  /**
   * Schema overrides this pack applies to types from its parents
   * (CONTRACT v1.10). Unsigned and dev packs may only override types of
   * unsigned or dev packs.
   */
  overrides: PackOverride[];
  migrations: { types: number; items_checked: number; items_migrated: number; items_failed: number; failed_ids: string[] };
  warnings: string[];
}

export interface PacksCommandMap {
  'packs.list': { args: NoArgs; result: { packs: PackView[] } };
  'packs.get': { args: { id: string }; result: PackView };
  'packs.preview': { args: { bundle_base64: string }; result: PackPreview };
  'packs.install': { args: { preview_id: string; trust_key?: boolean }; result: PackView };
  'packs.uninstall': { args: { id: string }; result: { id: string } };
  'packs.enable': { args: { id: string }; result: PackView };
  'packs.disable': { args: { id: string }; result: PackView };
  'packs.rollback': { args: { id: string }; result: PackView };
  'packs.reset_layout': { args: { id: string }; result: { id: string; role_id: string } };
  'packs.dev_set': { args: { path: string | null }; result: { path: string | null; packs: number } };
  'packs.dev_get': { args: NoArgs; result: { path: string | null; packs: PackView[]; errors: { dir: string; message: string }[] } };
  'packs.dev_reload': { args: NoArgs; result: { path: string | null; packs: number } };
  'packs.keys_list': { args: NoArgs; result: { keys: PackKey[] } };
  'packs.keys_add': { args: { pubkey_b64: string; label: string }; result: PackKey };
  'packs.keys_revoke': { args: { key_id: string }; result: { key: PackKey; disabled_packs: string[]; invalid_items: string[] } };
}
