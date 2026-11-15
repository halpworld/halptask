// In-memory stand-in for halp-core so the UI runs in a plain browser with
// `pnpm dev` and in vitest. It follows the core's rules where the UI can
// observe them (hub/crates/halp-core, halp-domain):
//  - the envelope, error codes and deny_unknown_fields argument checking,
//  - the policy: T3 needs confirmation from every actor (the user included),
//    T2 from everyone but the user; consent.approve/deny are user-only and
//    the approved command runs as the original actor,
//  - the outline: pre-order listing, sibling renumbering, derived parent
//    status, inherited tags and roles, cascade rules for groups,
//  - the event log: per-event undo/redo, undone events stay in history,
//    role.switch / focus.* are logged but not undoable, and a write that
//    changes nothing emits no event,
//  - capture through the exact port in capture/parse.ts, grid math through
//    layout/grid.ts (both pinned to the Rust code by tests).
// Agents (`agent('claude')`) are simulated; their parked calls resolve when
// the user decides.

import type { Transport } from '../transport';
import type {
  Actor,
  Args,
  AuditEntry,
  ClientView,
  CommandDescriptor,
  IntegrationEntry,
  Grants,
  ConsentTarget,
  DayState,
  EntityRef,
  ErrorCode,
  HistoryEntry,
  HubEvent,
  HubRequest,
  HubResponse,
  Item,
  ItemStatus,
  JsonSchema,
  JsonValue,
  Layout,
  PendingConfirmation,
  Role,
  Rule,
  TaskTypeView,
  Tier,
  Widget,
} from '../types';
import { ulid } from '../../util/ulid';
import { addDays, today as todayStr } from '../../util/dates';
import { CaptureError, normalizeTag, parseDue, resolveCapture } from '../../capture/parse';
import { checkPosition, checkSize, clampBox, compact, findSpace, GridError, moveBox, resizeBox } from '../../layout/grid';
import { seedItems, seedLayouts, seedRoles, seedTypes } from './seed';
import { FieldError, validateFields } from './schema';
import { MockPacks } from './packs';
import { isPrivateTypeId, PRIVATE_DATA_CLASS, PRIVATE_SCHEMA_KEY } from '../../privacy/private';

class CommandError extends Error {
  constructor(
    readonly code: ErrorCode,
    message: string,
  ) {
    super(message);
  }
}
const invalid = (m: string) => new CommandError('invalid_args', m);

type ItemDiff = [id: string, before: Item | null, after: Item | null][];
type Change = { kind: 'items'; diff: ItemDiff } | { kind: 'layout'; before: Layout; after: Layout } | { kind: 'type'; id: string; before: TaskTypeView | null; after: TaskTypeView };

interface LogRow {
  event: HubEvent;
  seq: number;
  causation_id: string | null;
  change?: Change;
  undone: boolean;
}

interface Ctx {
  actor: Actor;
}

interface CommandDef {
  tier: Tier;
  description: string;
  input: JsonSchema;
  /** Accepted alternative names, alias -> canonical (serde `alias`). */
  aliases?: Record<string, string>;
  /** Only the user may run it; hidden from MCP (CONTRACT v1.3). */
  user_only?: boolean;
  data_class?: string;
  run(args: Args, ctx: Ctx): unknown;
}

interface Pending extends PendingConfirmation {
  args: Args;
  resolve?: (res: HubResponse) => void;
  /** What the request would touch, recomputed at approval (core scope_digest). */
  scope?: string;
  /** The short form shown when a listing runs over its budget. */
  short_args?: Args;
  /** The real arguments (`args` is what is shown: file content and passphrases replaced). */
  call_args?: Args;
}

interface IntegrationRec {
  config: IntegrationEntry['config'];
  status: IntegrationEntry['status'];
  /** Mock remote: which PR numbers GitHub still reports, per repo. */
  remote: Map<string, number[]>;
  syncs: number;
}

interface ClientRec {
  id: string;
  name: string;
  token_hash: string;
  created_at: string;
  revoked_at: string | null;
  grants: Grants;
}

export interface MockOptions {
  /** Artificial latency in ms (randomised up to this value). 0 in tests. */
  latency?: number;
  /** Fixed "now" for deterministic tests. */
  now?: () => Date;
  /** Seed a few earlier agent events so the activity strip has history. */
  seedHistory?: boolean;
}

export interface AgentOptions {
  /** Simulate an agent that read external content (taint tracking in the core). */
  taintSource?: string;
}

const CONSENT_TTL_MS = 5 * 60_000;
/** consent.list keeps the args of all entries under this (core MAX_LIST_ARGS_BYTES). */
export const CONSENT_LIST_BUDGET_BYTES = 1024 * 1024;
export const DATA_CLASSES = ['audit', 'email.body', 'general', 'notes.private', 'secrets'];
const TAINT_TTL_MS = 30 * 60_000;
/** Args larger than this (JSON bytes) are flagged display_truncated. A stand-in for the core's limit. */
export const CONSENT_DISPLAY_MAX_BYTES = 64 * 1024;
const DEFAULT_LIST_LIMIT = 200;
const MAX_LIST_LIMIT = 1000;
const AGENDA_DAYS = 7;
const AGENDA_MAX = 100;
const INBOX_TITLE = 'Inbox';
const INBOX_TAG = 'inbox';
const STATUSES: ItemStatus[] = ['todo', 'in_progress', 'done'];

// ---- schema helpers (also what commands.list reports) -----------------------

const S = {
  str: (description?: string): JsonSchema => ({ type: 'string', ...(description ? { description } : {}) }),
  uint: (description?: string): JsonSchema => ({ type: 'integer', minimum: 0, ...(description ? { description } : {}) }),
  bool: (): JsonSchema => ({ type: 'boolean' }),
  strs: (): JsonSchema => ({ type: 'array', items: { type: 'string' } }),
  nstr: (): JsonSchema => ({ type: ['string', 'null'] }),
  any: (): JsonSchema => ({ type: 'object' }),
  obj: (properties: Record<string, JsonSchema> = {}, required: string[] = []): JsonSchema => ({ type: 'object', properties, required, additionalProperties: false }),
};
const NO_ARGS = S.obj();

function typeOk(t: string, v: unknown): boolean {
  switch (t) {
    case 'string':
      return typeof v === 'string';
    case 'integer':
      return typeof v === 'number' && Number.isInteger(v);
    case 'number':
      return typeof v === 'number' && Number.isFinite(v);
    case 'boolean':
      return typeof v === 'boolean';
    case 'array':
      return Array.isArray(v);
    case 'object':
      return typeof v === 'object' && v !== null && !Array.isArray(v);
    case 'null':
      return v === null;
    default:
      return true;
  }
}

/** Minimal serde-like check: types, required keys, deny_unknown_fields. */
function validate(schema: JsonSchema, v: unknown, path: string): void {
  const types = schema.type === undefined ? [] : Array.isArray(schema.type) ? schema.type : [schema.type];
  if (types.length && !types.some((t) => typeOk(t, v))) throw invalid(`${path}: expected ${types.join(' or ')}`);
  if (typeof v === 'number' && typeof schema.minimum === 'number' && v < schema.minimum) throw invalid(`${path}: must be >= ${schema.minimum}`);
  if (schema.enum && !schema.enum.includes(v as never) && !(Array.isArray(v) && v.every((x) => schema.enum!.includes(x)))) throw invalid(`${path}: unknown variant`);
  if (Array.isArray(v) && schema.items) v.forEach((x, i) => validate(schema.items!, x, `${path}[${i}]`));
  if (schema.properties && typeOk('object', v)) {
    const o = v as Record<string, unknown>;
    for (const k of Object.keys(o)) {
      const p = schema.properties[k];
      if (!p) {
        if (schema.additionalProperties === false) throw invalid(`${path}: unknown field \`${k}\``);
        continue;
      }
      validate(p, o[k], `${path}.${k}`);
    }
    for (const r of schema.required ?? []) if (!(r in o)) throw invalid(`${path}: missing field \`${r}\``);
  }
}

const clone = <T>(v: T): T => structuredClone(v);
const short = (s: string) => (Array.from(s).length > 60 ? `${Array.from(s).slice(0, 57).join('')}...` : s);
const randomHex = (bytes: number) => {
  const a = new Uint8Array(bytes);
  crypto.getRandomValues(a);
  return Array.from(a, (b) => b.toString(16).padStart(2, '0')).join('');
};
/** Not a real sha256; the mock only needs a stable fingerprint of the right shape. */
function fakeHash(s: string): string {
  let out = '';
  for (let round = 0; round < 8; round++) {
    let h = 0x811c9dc5 ^ round;
    for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 0x01000193);
    out += (h >>> 0).toString(16).padStart(8, '0');
  }
  return out;
}
const sameJson = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const jsonBytes = (v: unknown) => new TextEncoder().encode(JSON.stringify(v)).length;

/** Long values cut, nested values as their size (a stand-in for the core's audit_args). */
function shortArgs(args: Args): Args {
  const out: Args = {};
  for (const [k, v] of Object.entries(args)) {
    if (typeof v === 'string') out[k] = Array.from(v).length > 120 ? `${Array.from(v).slice(0, 80).join('')}… (${jsonBytes(v)} bytes)` : v;
    else if (v !== null && typeof v === 'object') out[k] = jsonBytes(v) > 120 ? `<${Array.isArray(v) ? 'array' : 'object'}, ${jsonBytes(v)} bytes>` : v;
    else out[k] = v;
  }
  return out;
}

/** What a consent prompt shows (core display_args): import content and passphrases replaced; over the cap the short form. */
function displayArgs(command: string, args: Args): { args: Args; short: Args; truncated: boolean } {
  const a = clone(args);
  if (command === 'import.tui') {
    if (typeof a.content_base64 === 'string') a.content_base64 = `<${Math.floor(a.content_base64.length / 4) * 3} bytes>`;
    if (a.passphrase !== undefined && a.passphrase !== null) a.passphrase = '[redacted]';
  }
  const short = shortArgs(a);
  return jsonBytes(a) > CONSENT_DISPLAY_MAX_BYTES ? { args: short, short, truncated: true } : { args: a, short, truncated: false };
}

export class MockTransport implements Transport {
  readonly kind = 'mock' as const;

  private items = new Map<string, Item>();
  private types = new Map<string, TaskTypeView>();
  private roles: Role[] = [];
  private layouts = new Map<string, Layout>();
  private roleId = 'developer';
  private focusByRole = new Map<string, string | null>();
  private log: LogRow[] = [];
  private seq = 0;
  private redoStack: LogRow[] = [];
  private pending = new Map<string, Pending>();
  private listeners = new Set<(e: HubEvent) => void>();
  private readonly commands: Record<string, CommandDef>;
  private readonly latency: number;
  private readonly now: () => Date;
  private taint = new Map<Actor, string>();
  private clients = new Map<string, ClientRec>();
  private rules: Rule[] = [];
  private auditLog: AuditEntry[] = [];
  private taintAt = new Map<Actor, number>();
  /** When each agent's latest call reached the policy (core: in memory). */
  private lastSeen = new Map<Actor, number>();
  /** What the shell would show of stored secrets (never the secrets). */
  private secretHints = new Map<string, string | null>();
  private integrations = new Map<string, IntegrationRec>();
  private readonly packs: MockPacks;

  constructor(opts: MockOptions = {}) {
    this.latency = opts.latency ?? 90;
    this.now = opts.now ?? (() => new Date());
    this.packs = new MockPacks({ now: () => this.now(), fail: (c, m) => new CommandError(c, m), event: (ctx, c, e, m) => this.adminEvent(ctx, c, e, m) });
    this.commands = this.defineCommands();
    this.reset();
    if (opts.seedHistory ?? true) this.seedHistory();
  }

  // ---------------------------------------------------------------- Transport

  async send(request: HubRequest): Promise<HubResponse> {
    if (this.latency > 0) await new Promise((r) => setTimeout(r, 20 + Math.random() * this.latency));
    // Like the Tauri shell: the webview is always the user.
    return this.exec(request.command, request.args ?? {}, 'user');
  }

  subscribe(listener: (event: HubEvent) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  // ------------------------------------------------------------ Mock helpers

  /** A handle that dispatches as an agent client, e.g. `agent('claude')`. */
  agent(client = 'claude', opts: AgentOptions = {}) {
    const actor: Actor = `agent:${client}`;
    if (opts.taintSource) {
      this.taint.set(actor, opts.taintSource);
      this.taintAt.set(actor, this.now().getTime());
    }
    return {
      dispatch: (command: string, args: Args = {}): Promise<HubResponse> => {
        const res = this.exec(command, args, actor);
        // An agent's parked call resolves once the user decides.
        if (!res.ok && res.error.code === 'confirmation_required' && res.error.confirmation_id) {
          const p = this.pending.get(res.error.confirmation_id);
          if (p) return new Promise((resolve) => (p.resolve = resolve));
        }
        return Promise.resolve(res);
      },
    };
  }

  reset(): void {
    const t = todayStr(this.now());
    const iso = this.now().toISOString();
    this.items = new Map(seedItems(t, iso).map((i) => [i.id, i]));
    this.types = new Map(seedTypes().map((ty) => [ty.id, ty]));
    this.roles = seedRoles();
    this.layouts = new Map(seedLayouts(t, iso).map((l) => [l.id, l]));
    this.roleId = 'developer';
    this.focusByRole = new Map([
      ['developer', 'dev-1'],
      ['eng-manager', 'em-1'],
      ['cso', 'cso-1'],
    ]);
    this.log = [];
    this.redoStack = [];
    this.pending.clear();
    this.taint.clear();
    this.clients = new Map([
      [
        'claude',
        {
          id: 'claude',
          name: 'Claude Code',
          token_hash: fakeHash('seed-token'),
          created_at: new Date(this.now().getTime() - 3 * 86_400_000).toISOString(),
          revoked_at: null,
          grants: { t1: true, data_classes: ['general'] },
        },
      ],
    ]);
    this.rules = [];
    this.auditLog = [];
    this.taintAt.clear();
    this.lastSeen.clear();
    this.secretHints.clear();
    this.integrations.clear();
    this.packs.reset();
  }

  // --------------------------------------------------------------- Execution

  exec(command: string, rawArgs: Args, actor: Actor): HubResponse {
    this.expireConsents();
    const def = this.commands[command];
    if (!def) return this.fail('not_found', `unknown command ${command}`);
    if (typeof rawArgs !== 'object' || rawArgs === null || Array.isArray(rawArgs)) return this.fail('invalid_args', 'args must be an object');
    if (command === 'consent.approve' || command === 'consent.deny') return this.guard(() => this.decide(command, this.parseArgs(def, rawArgs), actor));

    const hash = fakeHash(JSON.stringify(rawArgs));
    if (actor !== 'user' && !actor.startsWith('integration:')) {
      const client = this.clientOf(actor);
      const refuse = (why: string) => {
        this.audit(actor, command, def.tier, hash, 'deny', why);
        return this.fail('denied', why);
      };
      if (client?.revoked_at) return refuse(`client ${client.id} is revoked`);
      if (client) this.lastSeen.set(actor, this.now().getTime());
      if (def.user_only) return refuse(`${command} is only for the user`);
      if (client && def.data_class && def.data_class !== 'general' && !client.grants.data_classes.includes(def.data_class)) return refuse(`${command} needs the ${def.data_class} grant`);
    }
    if (command !== 'consent.list' && command !== 'consent.get') {
      const why = this.confirmReason(command, def, rawArgs, actor);
      // The core decodes the arguments before asking anyone.
      if (why)
        return this.guard(() => {
          this.parseArgs(def, rawArgs);
          return this.park(command, def, rawArgs, actor, why);
        });
    }
    const res = this.run(def, rawArgs, actor);
    if (actor !== 'user' ? !command.startsWith('consent.') : def.tier !== 'T0' && !command.startsWith('consent.')) {
      this.audit(actor, command, def.tier, hash, res.ok ? 'allow' : 'failed', res.ok ? (actor === 'user' ? 'the user' : 'allowed by policy') : res.error.message);
    }
    return res;
  }

  /** Why this call must be confirmed, or null (halp-policy, simplified). */
  private confirmReason(command: string, def: CommandDef, args: Args, actor: Actor): string | null {
    if (def.tier === 'T3') return 'T3 always needs your approval';
    if (actor === 'user' || actor.startsWith('integration:')) return null;
    const client = this.clientOf(actor);
    if (def.tier === 'T1') return client && !client.grants.t1 ? 'this agent has no T1 grant' : null;
    if (def.tier !== 'T2') return null;
    if (this.isTainted(actor)) return `tainted by ${this.taint.get(actor) ?? 'external content'}: standing rules do not apply`;
    const id = actor.slice('agent:'.length);
    const ok = this.rules.some((r) => r.client_id === id && r.command === command && Object.entries(r.args).every(([k, v]) => sameJson(args[k], v)));
    return ok ? null : 'T2 needs your approval';
  }

  private clientOf(actor: Actor): ClientRec | undefined {
    return actor.startsWith('agent:') ? this.clients.get(actor.slice('agent:'.length)) : undefined;
  }

  private isTainted(actor: Actor): boolean {
    if (!this.taint.has(actor)) return false;
    const at = this.taintAt.get(actor) ?? this.now().getTime();
    if (this.now().getTime() - at >= TAINT_TTL_MS) {
      this.taint.delete(actor);
      this.taintAt.delete(actor);
      return false;
    }
    return true;
  }

  private audit(actor: Actor, command: string, tier: Tier, args_hash: string, decision: string, reason: string, confirmation_id: string | null = null, approver: string | null = null, tainted?: boolean): void {
    this.auditLog.push({ id: ulid(), ts: this.now().toISOString(), actor, command, tier, args_hash, decision, reason: short(reason), approver, confirmation_id, tainted: tainted ?? this.isTainted(actor) });
  }

  private parseArgs(def: CommandDef, raw: Args): Args {
    const args: Args = { ...raw };
    for (const [alias, canon] of Object.entries(def.aliases ?? {})) {
      if (alias in args) {
        if (canon in args) throw invalid(`duplicate field \`${canon}\``);
        args[canon] = args[alias];
        delete args[alias];
      }
    }
    validate(def.input, args, 'args');
    return args;
  }

  private run(def: CommandDef, rawArgs: Args, actor: Actor): HubResponse {
    return this.guard(() => clone(def.run(this.parseArgs(def, rawArgs), { actor })));
  }

  private guard(f: () => unknown): HubResponse {
    try {
      const r = f();
      if (r && typeof r === 'object' && 'ok' in r && typeof (r as HubResponse).ok === 'boolean') return r as HubResponse;
      return { ok: true, result: r };
    } catch (err) {
      if (err instanceof CommandError) return this.fail(err.code, err.message);
      if (err instanceof GridError || err instanceof CaptureError) return this.fail('invalid_args', err.message);
      return this.fail('internal', err instanceof Error ? err.message : String(err));
    }
  }

  /** Typed-item fields must match their type's schema, as in the core. */
  private checkFields(typeId: string | null, fields: Item['fields']): void {
    try {
      validateFields((id) => this.types.get(id)?.schema, typeId, fields);
    } catch (e) {
      if (e instanceof FieldError) throw invalid(e.message);
      throw e;
    }
  }

  private fail(code: ErrorCode, message: string): HubResponse {
    return { ok: false, error: { code, message } };
  }

  private emit(event: HubEvent): void {
    for (const l of this.listeners) {
      try {
        l(clone(event));
      } catch (err) {
        console.error('event listener failed', err);
      }
    }
  }

  private event(actor: Actor, command: string, entity: EntityRef, summary: string): HubEvent {
    return { id: ulid(), ts: this.now().toISOString(), actor, command, entity, summary };
  }

  /** Append to the log and emit. `change` makes it undoable. */
  private record(ev: HubEvent, change?: Change, causation: string | null = null): HubEvent {
    this.log.push({ event: ev, seq: ++this.seq, causation_id: causation, change, undone: false });
    if (change) this.redoStack = [];
    this.emit(ev);
    return ev;
  }

  // ------------------------------------------------------------------ Consent

  private park(command: string, def: CommandDef, args: Args, actor: Actor, reason = 'needs your approval'): HubResponse {
    // Like the core: a parked capture.quick is pinned to the Inbox, so the
    // user approves exactly where it will land.
    if (command === 'capture.quick' && (args.parent_id === undefined || args.parent_id === null)) {
      const inbox = [...this.items.values()].find((it) => it.parent_id === null && it.title === INBOX_TITLE && it.tags.includes(INBOX_TAG));
      if (inbox) args = { ...args, parent_id: inbox.id };
    }
    const p = this.enqueue(command, def.tier, args, actor, new Date(this.now().getTime() + CONSENT_TTL_MS).toISOString());
    this.audit(actor, command, def.tier, p.args_hash ?? '', 'confirm', reason, p.id, null, p.tainted);
    return { ok: false, error: { code: 'confirmation_required', message: p.summary, confirmation_id: p.id } };
  }

  private enqueue(command: string, tier: Tier, args: Args, actor: Actor, expiresAt: string, resolve?: Pending['resolve']): Pending {
    const { summary, targets, effects } = this.describe(command, args);
    const id = randomHex(32);
    const source = this.isTainted(actor) ? (this.taint.get(actor) ?? '') : null;
    const shown = displayArgs(command, args);
    const p: Pending = {
      id,
      command,
      tier,
      actor,
      args: shown.args,
      args_hash: fakeHash(JSON.stringify(args)),
      summary,
      details: { targets, effects },
      tainted: source !== null,
      taint_source: source || null,
      created_at: this.now().toISOString(),
      expires_at: expiresAt,
      display_truncated: shown.truncated,
      call_args: clone(args),
      short_args: shown.short,
      scope: this.scopeOf(args),
      resolve,
    };
    this.pending.set(id, p);
    this.record(this.event(actor, 'consent.requested', { kind: 'confirmation', id }, `Asked to run ${command}`));
    return p;
  }

  /** Everything the request would touch, as it is now (core scope_digest, items only). */
  private scopeOf(args: Args): string {
    const names: string[] = [];
    for (const k of ['id', 'item_id']) if (typeof args[k] === 'string') names.push(args[k] as string);
    if (Array.isArray(args.ids)) names.push(...args.ids.filter((x): x is string => typeof x === 'string'));
    const out: Record<string, unknown> = {};
    for (const n of names) {
      if (!this.items.has(n)) {
        out[n] = null;
        continue;
      }
      for (const s of this.subtree(n)) {
        const it = this.items.get(s);
        out[s] = it ? [it.parent_id, it.version, it.updated_at] : null;
      }
    }
    if (typeof args.parent_id === 'string') {
      const it = this.items.get(args.parent_id);
      out[`parent:${args.parent_id}`] = it ? [it.parent_id, it.version, it.updated_at] : null;
    }
    return JSON.stringify(Object.keys(out).sort().map((k) => [k, out[k]]));
  }

  private describe(command: string, args: Args): { summary: string; targets: ConsentTarget[]; effects: string[] } {
    if (command === 'tasks.delete' && typeof args.id === 'string') {
      const it = this.items.get(args.id);
      if (it) {
        const n = this.subtree(it.id).length - 1;
        return {
          summary: `Delete "${short(it.title)}"${n ? ` and ${n} nested item${n > 1 ? 's' : ''}` : ''}`,
          targets: [{ kind: 'item', id: it.id, title: it.title, descendants: n }],
          effects: [`Removes ${n + 1} item${n ? 's' : ''} from the outline. You can undo it from history.`],
        };
      }
      return { summary: `Delete item ${args.id}`, targets: [], effects: [] };
    }
    return { summary: `Run ${command}`, targets: [], effects: [] };
  }

  private expireConsents(): void {
    const now = this.now().getTime();
    for (const p of [...this.pending.values()]) {
      if (p.expires_at && Date.parse(p.expires_at) <= now) {
        this.pending.delete(p.id);
        this.audit(p.actor, p.command, p.tier, p.args_hash ?? '', 'expired', 'nobody answered in time', p.id, null, p.tainted);
        this.record(this.event('system', 'consent.expired', { kind: 'confirmation', id: p.id }, `Expired: ${p.command}`));
        p.resolve?.({ ok: false, error: { code: 'denied', message: 'The request expired before anyone answered it' } });
      }
    }
  }

  private decide(command: 'consent.approve' | 'consent.deny', args: Args, actor: Actor): HubResponse {
    const verb = command === 'consent.approve' ? 'approve' : 'deny';
    if (actor !== 'user') return this.fail('denied', `only the user can ${verb} a pending request`);
    const p = this.pending.get(args.id as string);
    if (!p) return this.fail('not_found', `no pending confirmation ${String(args.id)}`);
    const callArgs = p.call_args ?? p.args;
    if (command === 'consent.deny') {
      this.pending.delete(p.id);
      this.audit(p.actor, p.command, p.tier, p.args_hash ?? '', 'denied', 'declined by the user', p.id, 'user', p.tainted);
      this.record(this.event('user', 'consent.deny', { kind: 'confirmation', id: p.id }, `Declined: ${p.command}`));
      p.resolve?.({ ok: false, error: { code: 'denied', message: 'Declined by the user' } });
      return { ok: true, result: { denied: p.id } };
    }
    if (p.display_truncated) return this.fail('invalid_args', 'this request is too large to show in full, so it cannot be approved; deny it');
    const def = this.commands[p.command];
    if (!def) return this.fail('not_found', `unknown command ${p.command}`);
    // What the user saw is what runs: if the request would now touch
    // something else, ask again under a new id (CONTRACT v1.4).
    const scope = this.scopeOf(callArgs);
    if (p.scope !== undefined && scope !== p.scope) {
      const why = 'the request changed since you saw it';
      this.pending.delete(p.id);
      this.audit(p.actor, p.command, p.tier, p.args_hash ?? '', 'superseded', why, p.id, 'user', p.tainted);
      this.record(this.event('user', 'consent.superseded', { kind: 'confirmation', id: p.id }, `Changed since asked: ${short(p.summary)}`));
      const next = this.enqueue(p.command, p.tier, callArgs, p.actor, p.expires_at ?? new Date(this.now().getTime() + CONSENT_TTL_MS).toISOString(), p.resolve);
      return { ok: false, error: { code: 'conflict', message: `${why}: review it again and decide (it is now ${next.id})`, confirmation_id: next.id } };
    }
    this.pending.delete(p.id);
    this.record(this.event('user', 'consent.approve', { kind: 'confirmation', id: p.id }, `Approved: ${p.command}`));
    // Runs as the requester, without asking the policy again.
    const res = this.run(def, callArgs, p.actor);
    this.audit(p.actor, p.command, p.tier, p.args_hash ?? '', res.ok ? 'approved' : 'failed', res.ok ? 'approved by the user' : `approved; failed: ${res.error.message}`, p.id, 'user', p.tainted);
    p.resolve?.(res);
    return res;
  }

  private publicPending(p: Pending): PendingConfirmation {
    const { resolve: _r, call_args: _c, short_args: _s, scope: _x, ...rest } = p;
    return rest;
  }

  // ------------------------------------------------------------------ Outline

  private today(): string {
    return todayStr(this.now());
  }

  private require(id: unknown): Item {
    const it = typeof id === 'string' ? this.items.get(id) : undefined;
    if (!it) throw new CommandError('not_found', `no item ${String(id)}`);
    return it;
  }

  private children(parent: string | null): Item[] {
    return [...this.items.values()].filter((i) => i.parent_id === parent).sort((a, b) => a.order - b.order);
  }

  private renumber(parent: string | null): void {
    this.children(parent).forEach((c, i) => (c.order = i));
  }

  private preorder(from: string | null = null): string[] {
    const out: string[] = [];
    const walk = (p: string | null) => {
      for (const c of this.children(p)) {
        out.push(c.id);
        walk(c.id);
      }
    };
    walk(from);
    return out;
  }

  /** The item and all its descendants, pre-order. */
  private subtree(id: string): string[] {
    return [id, ...this.preorder(id)];
  }

  private ancestors(id: string): Item[] {
    const out: Item[] = [];
    let p = this.items.get(id)?.parent_id ?? null;
    while (p) {
      const it = this.items.get(p);
      if (!it) break;
      out.push(it);
      p = it.parent_id;
    }
    return out;
  }

  private hasSubtasks(id: string): boolean {
    return this.preorder(id).some((d) => this.items.get(d)?.kind === 'task');
  }

  /** Outline::effective_status: tasks with subtasks derive from their leaf tasks. */
  private effectiveStatus(it: Item): ItemStatus {
    if (it.kind !== 'task') return it.status;
    let active = false;
    let todo = false;
    let done = false;
    for (const d of this.preorder(it.id)) {
      const x = this.items.get(d)!;
      if (x.kind !== 'task' || this.hasSubtasks(d)) continue;
      if (x.status === 'in_progress') active = true;
      else if (x.status === 'todo') todo = true;
      else done = true;
    }
    if (active || (todo && done)) return 'in_progress';
    if (done) return 'done';
    if (todo) return 'todo';
    return it.status;
  }

  private effectiveRoles(id: string): string[] {
    const it = this.items.get(id);
    if (!it) return [];
    if (it.roles.length) return it.roles;
    return this.ancestors(id).find((a) => a.roles.length)?.roles ?? [];
  }

  private inRole(id: string, role: string): boolean {
    const r = this.effectiveRoles(id);
    return r.length === 0 || r.includes(role);
  }

  private view(id: string): Item {
    const it = this.require(id);
    const own = new Set(it.tags);
    const inherited: string[] = [];
    for (const a of this.ancestors(id)) for (const t of a.tags) if (!own.has(t)) (own.add(t), inherited.push(t));
    return { ...clone(it), status: this.effectiveStatus(it), inherited_tags: inherited };
  }

  /** Runs an outline mutation, records exactly what changed, and emits once. */
  private writeItems<R>(ctx: Ctx, command: string, entity: EntityRef, summary: string | (() => string), mutate: () => R): R {
    const before = new Map([...this.items].map(([k, v]) => [k, clone(v)]));
    const result = mutate();
    const diff: ItemDiff = [];
    for (const id of new Set([...before.keys(), ...this.items.keys()])) {
      const b = before.get(id) ?? null;
      const a = this.items.get(id) ?? null;
      if (!sameJson(b, a)) diff.push([id, b, a && clone(a)]);
    }
    if (diff.length) this.record(this.event(ctx.actor, command, entity, typeof summary === 'function' ? summary() : summary), { kind: 'items', diff });
    return result;
  }

  private touch(it: Item): void {
    it.updated_at = this.now().toISOString();
    it.version += 1;
  }

  private insert(it: Item, parent: string | null, index?: number): void {
    const sibs = this.children(parent);
    const idx = Math.min(index ?? sibs.length, sibs.length);
    sibs.splice(idx, 0, it);
    this.items.set(it.id, it);
    sibs.forEach((s, i) => (s.order = i));
  }

  private checkTitle(raw: string): string {
    const t = raw.trim();
    if (!t) throw invalid('title must not be empty');
    if (Array.from(t).length > 2000) throw invalid('title exceeds 2000 characters');
    if (/\p{Cc}/u.test(t)) throw invalid('title must not contain control characters or line breaks');
    return t;
  }

  private cleanTags(raw: string[]): string[] {
    const out: string[] = [];
    for (const r of raw) {
      const t = normalizeTag(r);
      if (t === null) throw invalid(`tag ${JSON.stringify(r)} must start with a letter and may only contain letters, digits, '_', '-' and '/'`);
      if (!out.includes(t)) out.push(t);
    }
    if (out.length > 32) throw invalid('at most 32 tags per item');
    return out;
  }

  private checkDue(raw: string): string {
    const d = parseDue(raw, this.today());
    if (!d) throw invalid(`invalid date format: ${JSON.stringify(raw)}`);
    return d;
  }

  private checkDate(raw: unknown, name: string): string {
    if (typeof raw !== 'string' || !/^\d{4}-\d{2}-\d{2}$/.test(raw) || parseDue(raw, this.today()) !== raw) throw invalid(`${name} must be YYYY-MM-DD`);
    return raw;
  }

  private checkRoles(raw: string[]): string[] {
    for (const r of raw) if (!this.roles.some((x) => x.id === r)) throw new CommandError('not_found', `no role ${r}`);
    return [...new Set(raw)];
  }

  /** The data class of a task type: its schema's `x-halp-data-class`, else general. */
  private typeClass(typeId: string | null | undefined): string {
    if (!typeId) return 'general';
    if (isPrivateTypeId(typeId)) return PRIVATE_DATA_CLASS;
    const c = (this.types.get(typeId)?.schema as Record<string, unknown> | undefined)?.[PRIVATE_SCHEMA_KEY];
    return typeof c === 'string' && c ? c : 'general';
  }

  /** Non-general data classes of an item and its ancestors (the core's item_class). */
  private itemClasses(it: Item): string[] {
    const out: string[] = [];
    for (let cur: Item | undefined = it; cur; cur = cur.parent_id ? this.items.get(cur.parent_id) : undefined) {
      const c = this.typeClass(cur.type_id);
      if (c !== 'general') out.push(c);
    }
    return out;
  }

  /**
   * Whether an item is hidden from this actor. Stands in for the core's grant
   * check: any type whose schema names a data class other than `general`
   * (for example `notes.private`, `cso.unpublished`) is hidden from agents
   * whose client lacks that class's grant, and from integrations and modules.
   * The user sees everything, but text search still skips `notes.private`.
   */
  private isPrivate(it: Item, actor: Actor = 'user', search = false): boolean {
    const classes = this.itemClasses(it);
    if (classes.length === 0) return false;
    if (actor === 'user' || actor === 'system') return search && classes.includes(PRIVATE_DATA_CLASS);
    const granted = this.clientOf(actor)?.grants.data_classes ?? [];
    return classes.some((c) => !granted.includes(c));
  }

  private list(a: Args, actor: Actor = 'user'): Item[] {
    const statuses = a.status === undefined ? null : ((Array.isArray(a.status) ? a.status : [a.status]) as ItemStatus[]);
    const kind = statuses ? 'task' : (a.kind as Item['kind'] | undefined);
    const tag = a.tag === undefined ? null : normalizeTag(a.tag as string);
    if (a.tag !== undefined && tag === null) throw invalid('tag is not valid');
    const text = typeof a.text === 'string' ? a.text.trim().toLowerCase() : '';
    if (Array.from(text).length > 200) throw invalid('text exceeds 200 characters');
    const due_before = a.due_before === undefined ? null : this.checkDate(a.due_before, 'due_before');
    const due_after = a.due_after === undefined ? null : this.checkDate(a.due_after, 'due_after');
    if (due_before && due_after && due_after > due_before) throw invalid('due_after is later than due_before');
    if (a.limit === 0) throw invalid('limit must be at least 1');
    const limit = typeof a.limit === 'number' ? Math.min(a.limit, MAX_LIST_LIMIT) : DEFAULT_LIST_LIMIT;
    let candidates: string[];
    if (a.parent_id === undefined) candidates = this.preorder();
    else if (a.parent_id === null) candidates = this.children(null).map((c) => c.id);
    else candidates = this.children(this.require(a.parent_id).id).map((c) => c.id);

    const out: Item[] = [];
    for (const id of candidates) {
      if (out.length >= limit) break;
      const it = this.items.get(id)!;
      if (kind && it.kind !== kind) continue;
      if (this.isPrivate(it, actor, !!text)) continue;
      const status = this.effectiveStatus(it);
      if (statuses && !statuses.includes(status)) continue;
      if (a.type_id !== undefined && it.type_id !== a.type_id) continue;
      if (due_before && (!it.due || it.due >= due_before)) continue;
      if (due_after && (!it.due || it.due <= due_after)) continue;
      if (typeof a.role === 'string' && !this.inRole(id, a.role)) continue;
      if (text && !it.title.toLowerCase().includes(text) && !it.notes.toLowerCase().includes(text)) continue;
      const v = this.view(id);
      if (tag && !v.tags.includes(tag) && !v.inherited_tags.includes(tag)) continue;
      out.push(v);
    }
    return out;
  }

  private dayState(date?: string): DayState {
    const d = date ?? this.today();
    const horizon = addDays(d, AGENDA_DAYS);
    const counts = { overdue: 0, today: 0, in_progress: 0 };
    let agenda: Item[] = [];
    for (const id of this.preorder()) {
      const it = this.items.get(id)!;
      if (it.kind !== 'task' || !this.inRole(id, this.roleId)) continue;
      const status = this.effectiveStatus(it);
      if (status === 'done') continue;
      if (status === 'in_progress') counts.in_progress++;
      if (it.due) {
        if (it.due < d) counts.overdue++;
        else if (it.due === d) counts.today++;
        if (it.due <= horizon) agenda.push(this.view(id));
      }
    }
    // Stable sort keeps outline order within a day.
    agenda = agenda.sort((x, y) => (x.due! < y.due! ? -1 : x.due! > y.due! ? 1 : 0)).slice(0, AGENDA_MAX);
    const fid = this.focusByRole.get(this.roleId) ?? null;
    return { date: d, role: this.roleId, focus: fid && this.items.has(fid) ? this.view(fid) : null, counts, agenda, layout: this.layoutFor() };
  }

  // ------------------------------------------------------------------- Layout

  private role(id?: unknown): Role {
    const rid = typeof id === 'string' ? id : this.roleId;
    const r = this.roles.find((x) => x.id === rid);
    if (!r) throw new CommandError('not_found', `no role ${rid}`);
    return r;
  }

  private layoutFor(roleId?: unknown): Layout {
    const l = this.layouts.get(this.role(roleId).layout_id);
    if (!l) throw new CommandError('internal', 'no layout for role');
    return l;
  }

  private widget(layout: Layout, id: unknown): Widget {
    const w = layout.widgets.find((x) => x.id === id);
    if (!w) throw new CommandError('not_found', `no widget ${String(id)}`);
    return w;
  }

  private layoutChange(ctx: Ctx, roleId: unknown, command: string, summary: string, widgetId: string, mutate: (l: Layout) => void): Layout {
    const l = this.layoutFor(roleId);
    const before = clone(l);
    mutate(l);
    if (!sameJson(before, l)) this.record(this.event(ctx.actor, command, { kind: 'widget', id: widgetId }, summary), { kind: 'layout', before, after: clone(l) });
    return l;
  }

  // ------------------------------------------------------------------ History

  private apply(change: Change, which: 'before' | 'after'): void {
    switch (change.kind) {
      case 'items':
        for (const [id, b, a] of change.diff) {
          const v = which === 'before' ? b : a;
          if (v) this.items.set(id, clone(v));
          else this.items.delete(id);
        }
        for (const [r, f] of this.focusByRole) if (f && !this.items.has(f)) this.focusByRole.set(r, null);
        break;
      case 'layout': {
        const v = which === 'before' ? change.before : change.after;
        this.layouts.set(v.id, clone(v));
        break;
      }
      case 'type':
        if (which === 'before' && !change.before) this.types.delete(change.id);
        else this.types.set(change.id, clone(which === 'before' ? change.before! : change.after));
        break;
    }
  }

  private step(ctx: Ctx, undo: boolean, eventId: unknown) {
    let row: LogRow | undefined;
    if (undo) {
      row = typeof eventId === 'string' ? this.log.find((r) => r.event.id === eventId) : [...this.log].reverse().find((r) => r.change && !r.undone);
      if (typeof eventId === 'string' && (!row || !row.change || row.undone)) throw new CommandError('not_found', `nothing to undo for event ${eventId}`);
    } else {
      row = typeof eventId === 'string' ? this.redoStack.find((r) => r.event.id === eventId) : this.redoStack.at(-1);
      if (typeof eventId === 'string' && !row) throw new CommandError('not_found', `nothing to redo for event ${eventId}`);
    }
    if (!row || !row.change) return { event: null, history_event: null };
    this.apply(row.change, undo ? 'before' : 'after');
    row.undone = undo;
    if (undo) this.redoStack.push(row);
    else this.redoStack = this.redoStack.filter((r) => r !== row);
    const ev = this.event(ctx.actor, undo ? 'history.undo' : 'history.redo', row.event.entity, `${undo ? 'Undid' : 'Redid'}: ${row.event.summary}`);
    this.log.push({ event: ev, seq: ++this.seq, causation_id: row.event.id, undone: false });
    this.emit(ev);
    return { event: clone(row.event), history_event: ev };
  }

  // ----------------------------------------------------------------- Commands

  private defineCommands(): Record<string, CommandDef> {
    const roleArg = { role_id: S.str('Default: the current role') };
    const statusSchema: JsonSchema = { type: ['string', 'array'], enum: STATUSES };
    const kindSchema: JsonSchema = { type: 'string', enum: ['task', 'bullet'] };

    return {
      'commands.list': { tier: 'T0', description: 'Every command with its tier and schemas.', input: NO_ARGS, run: () => ({ commands: this.descriptors() }) },

      'tasks.create': {
        tier: 'T1',
        description: 'Create a task or bullet.',
        input: S.obj({ title: S.str(), kind: kindSchema, parent_id: S.str(), index: S.uint(), notes: S.str(), tags: S.strs(), due: S.str('Date or phrase: 2026-09-01, friday, +3d'), type_id: S.str(), fields: S.any(), status: { type: 'string', enum: STATUSES }, roles: S.strs() }, ['title']),
        run: (a, ctx) => {
          const kind = (a.kind as Item['kind']) ?? 'task';
          const iso = this.now().toISOString();
          if (a.status !== undefined && kind === 'bullet') throw invalid('bullets have no status');
          const parent = typeof a.parent_id === 'string' ? this.require(a.parent_id).id : null;
          if (typeof a.type_id === 'string' && !this.types.has(a.type_id)) throw new CommandError('not_found', `no task type ${a.type_id}`);
          const it: Item = {
            id: ulid(),
            parent_id: parent,
            order: 0,
            kind,
            status: (a.status as ItemStatus) ?? 'todo',
            title: this.checkTitle(a.title as string),
            notes: (a.notes as string) ?? '',
            tags: a.tags ? this.cleanTags(a.tags as string[]) : [],
            inherited_tags: [],
            due: typeof a.due === 'string' ? this.checkDue(a.due) : null,
            type_id: (a.type_id as string) ?? null,
            fields: clone((a.fields as Item['fields']) ?? {}),
            roles: a.roles ? this.checkRoles(a.roles as string[]) : [],
            origin: ctx.actor,
            created_at: iso,
            updated_at: iso,
            version: 1,
          };
          this.checkFields(it.type_id, it.fields);
          const summary = `Created ${kind} "${short(it.title)}"`;
          this.writeItems(ctx, 'tasks.create', { kind: 'item', id: it.id }, summary, () => this.insert(it, parent, a.index as number | undefined));
          return this.view(it.id);
        },
      },

      'tasks.get': { tier: 'T0', description: 'Get one item by id.', input: S.obj({ id: S.str() }, ['id']), run: (a, ctx) => {
          const it = this.require(a.id);
          if (this.isPrivate(it, ctx.actor)) throw new CommandError('not_found', `no item ${String(a.id)}`);
          return this.view(it.id);
        },
      },

      'tasks.list': {
        tier: 'T0',
        description: 'List items matching a filter, in outline order.',
        input: S.obj({ type_id: S.str(), status: statusSchema, kind: kindSchema, tag: S.str(), due_before: S.str('YYYY-MM-DD, exclusive'), due_after: S.str('YYYY-MM-DD, exclusive'), text: S.str(), parent_id: S.nstr(), role: S.str(), limit: S.uint() }),
        run: (a, ctx) => ({ items: this.list(a, ctx.actor) }),
      },

      'tasks.update': {
        tier: 'T1',
        description: 'Change fields on an item.',
        input: S.obj(
          {
            id: S.str(),
            patch: S.obj({ title: S.str(), notes: S.str(), tags: S.strs(), due: S.nstr(), type_id: S.nstr(), fields: S.any(), status: { type: 'string', enum: STATUSES }, kind: kindSchema, roles: S.strs() }),
            if_version: S.uint(),
          },
          ['id', 'patch'],
        ),
        run: (a, ctx) => {
          const cur = this.require(a.id);
          if (typeof a.if_version === 'number' && a.if_version !== cur.version) throw new CommandError('conflict', `item is at version ${cur.version}, not ${a.if_version}`);
          const p = a.patch as Record<string, unknown>;
          const next = clone(cur);
          if (p.roles !== undefined) next.roles = this.checkRoles(p.roles as string[]);
          if (p.title !== undefined) next.title = this.checkTitle(p.title as string);
          if (p.notes !== undefined) next.notes = p.notes as string;
          if (p.tags !== undefined) next.tags = this.cleanTags(p.tags as string[]);
          if (p.due !== undefined) next.due = p.due === null ? null : this.checkDue(p.due as string);
          if (p.type_id !== undefined) {
            if (p.type_id !== null && !this.types.has(p.type_id as string)) throw new CommandError('not_found', `no task type ${String(p.type_id)}`);
            next.type_id = p.type_id as string | null;
            if (p.type_id === null && p.fields === undefined) next.fields = {};
          }
          if (p.fields !== undefined) next.fields = clone(p.fields as Item['fields']);
          if (p.kind !== undefined) next.kind = p.kind as Item['kind'];
          if (p.status !== undefined) {
            if (next.kind === 'bullet') throw invalid('bullets have no status');
            if (this.hasSubtasks(cur.id)) throw invalid("this item's status comes from its subtasks; complete it with tasks.complete and cascade=true");
            next.status = p.status as ItemStatus;
          }
          if (next.kind === 'bullet') next.status = 'todo';
          this.checkFields(next.type_id, next.fields);
          if (sameJson(next, cur)) return this.view(cur.id);
          const what = Object.keys(p).join(', ');
          this.writeItems(ctx, 'tasks.update', { kind: 'item', id: cur.id }, `Updated ${what} on "${short(next.title)}"`, () => {
            this.touch(next);
            this.items.set(cur.id, next);
          });
          return this.view(cur.id);
        },
      },

      'tasks.move': {
        tier: 'T1',
        description: 'Move an item under a new parent and/or to a new position.',
        aliases: { order: 'index' },
        input: S.obj({ id: S.str(), parent_id: { ...S.nstr(), description: 'Absent: keep the current parent. null: move to the root.' }, index: S.uint() }, ['id']),
        run: (a, ctx) => {
          const cur = this.require(a.id);
          const parent = a.parent_id === undefined ? cur.parent_id : (a.parent_id as string | null);
          if (parent !== null) {
            this.require(parent);
            if (this.subtree(cur.id).includes(parent)) throw invalid('cannot move an item under itself or its own descendant');
          }
          this.writeItems(ctx, 'tasks.move', { kind: 'item', id: cur.id }, `Moved "${short(cur.title)}"`, () => {
            const old = cur.parent_id;
            cur.parent_id = '\u0000moving';
            this.renumber(old);
            cur.parent_id = parent;
            const sibs = this.children(parent).filter((s) => s.id !== cur.id);
            const idx = Math.min((a.index as number | undefined) ?? sibs.length, sibs.length);
            sibs.splice(idx, 0, cur);
            sibs.forEach((s, i) => (s.order = i));
            this.touch(cur);
          });
          return this.view(cur.id);
        },
      },

      'tasks.complete': {
        tier: 'T1',
        description: 'Mark a task done (done=false reopens it). Groups need cascade=true.',
        input: S.obj({ id: S.str(), done: S.bool(), cascade: S.bool() }, ['id']),
        run: (a, ctx) => {
          const cur = this.require(a.id);
          const group = cur.kind === 'bullet' || this.hasSubtasks(cur.id);
          if (group && a.cascade !== true) throw invalid('this item has subtasks or is a bullet; pass cascade=true to change the whole group');
          const target: ItemStatus = a.done === false ? 'todo' : 'done';
          const verb = a.done === false ? 'Reopened' : 'Completed';
          const summary = group ? `${verb} "${short(cur.title)}" and its subtasks` : `${verb} "${short(cur.title)}"`;
          this.writeItems(ctx, 'tasks.complete', { kind: 'item', id: cur.id }, summary, () => {
            const ids = a.cascade === true ? this.subtree(cur.id) : [cur.id];
            for (const id of ids) {
              const it = this.items.get(id)!;
              if (it.kind === 'task' && it.status !== target) {
                it.status = target;
                this.touch(it);
              }
            }
          });
          return this.view(cur.id);
        },
      },

      'tasks.delete': {
        tier: 'T3',
        description: 'Delete an item and everything under it.',
        input: S.obj({ id: S.str() }, ['id']),
        run: (a, ctx) => {
          const cur = this.require(a.id);
          const ids = this.subtree(cur.id);
          const summary = ids.length > 1 ? `Deleted "${short(cur.title)}" and ${ids.length - 1} nested items` : `Deleted "${short(cur.title)}"`;
          this.writeItems(ctx, 'tasks.delete', { kind: 'item', id: cur.id }, summary, () => {
            for (const id of ids) this.items.delete(id);
            this.renumber(cur.parent_id);
          });
          for (const [r, f] of this.focusByRole) if (f && ids.includes(f)) this.focusByRole.set(r, null);
          return { deleted: ids };
        },
      },

      'history.list': {
        tier: 'T0',
        description: 'Recent events, newest first, including undone ones.',
        input: S.obj({ limit: S.uint(), before: S.str('Event id: list events older than this one'), actor_prefix: S.str() }),
        run: (a) => {
          if (a.limit === 0) throw invalid('limit must be at least 1');
          const limit = Math.min((a.limit as number | undefined) ?? 50, 500);
          const prefix = (a.actor_prefix as string | undefined) ?? '';
          let rows = [...this.log].reverse();
          if (typeof a.before === 'string') {
            const i = rows.findIndex((r) => r.event.id === a.before);
            rows = i < 0 ? [] : rows.slice(i + 1);
          }
          const events: HistoryEntry[] = rows
            .filter((r) => r.event.actor.startsWith(prefix))
            .slice(0, limit)
            .map((r) => ({ ...r.event, seq: r.seq, causation_id: r.causation_id, undoable: !!r.change && !r.undone }));
          return { events, can_undo: this.log.some((r) => r.change && !r.undone), can_redo: this.redoStack.length > 0 };
        },
      },
      'history.undo': { tier: 'T1', description: 'Undo the latest change, or one event by id.', input: S.obj({ event_id: S.str() }), run: (a, ctx) => this.step(ctx, true, a.event_id) },
      'history.redo': { tier: 'T1', description: 'Redo the latest undone change, or one event by id.', input: S.obj({ event_id: S.str() }), run: (a, ctx) => this.step(ctx, false, a.event_id) },

      'types.list': { tier: 'T0', description: 'List task types.', input: NO_ARGS, run: () => ({ types: [...this.types.values()] }) },
      'types.get': {
        tier: 'T0',
        description: 'Get one task type.',
        input: S.obj({ id: S.str() }, ['id']),
        run: (a) => {
          const t = this.types.get(a.id as string);
          if (!t) throw new CommandError('not_found', `no task type ${String(a.id)}`);
          const { origin: _o, ...rest } = t;
          return rest;
        },
      },
      'types.define': {
        tier: 'T1',
        description: 'Create or replace a task type.',
        input: S.obj({ id: S.str(), title: S.str(), icon: S.str(), schema: S.any(), display: S.any() }, ['id', 'title', 'schema']),
        run: (a, ctx) => {
          if (!/^[a-z0-9_-]+(\.[a-z0-9_-]+)+$/.test(a.id as string)) throw invalid('id must be namespaced, e.g. dev.pull_request');
          const prev = this.types.get(a.id as string) ?? null;
          const t: TaskTypeView = { id: a.id as string, title: a.title as string, icon: (a.icon as string | undefined) ?? '', schema: clone(a.schema as JsonSchema), display: clone((a.display as TaskTypeView['display'] | undefined) ?? { primary_fields: [] }), origin: 'user' };
          this.types.set(t.id, t);
          this.record(this.event(ctx.actor, 'types.define', { kind: 'type', id: t.id }, `Defined type ${t.id}`), { kind: 'type', id: t.id, before: prev && clone(prev), after: clone(t) });
          const { origin: _o, ...rest } = t;
          return rest;
        },
      },

      'layout.get': { tier: 'T0', description: "Get the current (or a role's) layout.", input: S.obj(roleArg), run: (a) => this.layoutFor(a.role_id) },
      'layout.add_widget': {
        tier: 'T1',
        description: "Add a widget (one of the role's widget kinds).",
        input: S.obj({ ...roleArg, kind: S.str(), title: S.str(), x: S.uint(), y: S.uint(), w: S.uint(), h: S.uint(), config: S.any() }, ['kind']),
        run: (a, ctx) => {
          const role = this.role(a.role_id);
          const kind = a.kind as string;
          if (!role.widgets.includes(kind)) throw invalid(`widget kind ${kind} is not offered by role ${role.id}`);
          const ww = (a.w as number | undefined) ?? 4;
          const hh = (a.h as number | undefined) ?? 3;
          const l = this.layoutFor(role.id);
          checkSize(ww, hh, l.columns);
          if ((a.x === undefined) !== (a.y === undefined)) throw invalid('give both x and y, or neither');
          if (a.x !== undefined) checkPosition(a.x, a.y, l.columns);
          const id = `w-${ulid().slice(-10).toLowerCase()}`;
          return this.layoutChange(ctx, role.id, 'layout.add_widget', `Added ${kind} widget`, id, (lay) => {
            const spot = a.x !== undefined ? { x: a.x as number, y: a.y as number } : findSpace(lay.widgets, ww, hh, lay.columns);
            const widget: Widget = clampBox({ id, kind, title: (a.title as string | undefined) ?? kind, x: spot.x, y: spot.y, w: ww, h: hh, config: clone((a.config as Widget['config']) ?? {}) }, lay.columns);
            lay.widgets = moveBox([...lay.widgets, widget], id, widget.x, widget.y, lay.columns);
          });
        },
      },
      'layout.update_widget': {
        tier: 'T1',
        description: "Change a widget's title, config, size or position.",
        input: S.obj({ ...roleArg, widget_id: S.str(), patch: S.obj({ title: S.str(), config: S.any(), x: S.uint(), y: S.uint(), w: S.uint(), h: S.uint() }) }, ['widget_id', 'patch']),
        run: (a, ctx) => {
          const l = this.layoutFor(a.role_id);
          const w = this.widget(l, a.widget_id);
          const p = a.patch as Partial<Widget>;
          if (p.x !== undefined || p.y !== undefined) checkPosition(p.x ?? w.x, p.y ?? w.y, l.columns);
          if (p.w !== undefined || p.h !== undefined) checkSize(p.w ?? w.w, p.h ?? w.h, l.columns);
          return this.layoutChange(ctx, a.role_id, 'layout.update_widget', `Updated "${short(w.title)}"`, w.id, (lay) => {
            const t = this.widget(lay, w.id);
            if (p.title !== undefined) t.title = p.title;
            if (p.config !== undefined) t.config = clone(p.config);
            if (p.w !== undefined || p.h !== undefined) lay.widgets = resizeBox(lay.widgets, w.id, p.w ?? t.w, p.h ?? t.h, lay.columns);
            if (p.x !== undefined || p.y !== undefined) {
              const t2 = this.widget(lay, w.id);
              lay.widgets = moveBox(lay.widgets, w.id, p.x ?? t2.x, p.y ?? t2.y, lay.columns);
            }
          });
        },
      },
      'layout.move_widget': {
        tier: 'T1',
        description: 'Move a widget to grid position x, y.',
        input: S.obj({ ...roleArg, widget_id: S.str(), x: S.uint(), y: S.uint() }, ['widget_id', 'x', 'y']),
        run: (a, ctx) => {
          const l = this.layoutFor(a.role_id);
          const w = this.widget(l, a.widget_id);
          checkPosition(a.x, a.y, l.columns);
          return this.layoutChange(ctx, a.role_id, 'layout.move_widget', `Moved "${short(w.title)}"`, w.id, (lay) => {
            lay.widgets = moveBox(lay.widgets, w.id, a.x as number, a.y as number, lay.columns);
          });
        },
      },
      'layout.remove_widget': {
        tier: 'T1',
        description: 'Remove a widget.',
        input: S.obj({ ...roleArg, widget_id: S.str() }, ['widget_id']),
        run: (a, ctx) => {
          const l = this.layoutFor(a.role_id);
          const w = this.widget(l, a.widget_id);
          return this.layoutChange(ctx, a.role_id, 'layout.remove_widget', `Removed "${short(w.title)}"`, w.id, (lay) => {
            lay.widgets = compact(lay.widgets.filter((x) => x.id !== w.id));
          });
        },
      },

      'role.list': { tier: 'T0', description: 'List roles and the current one.', input: NO_ARGS, run: () => ({ roles: this.roles, current: this.roleId }) },
      'role.get': { tier: 'T0', description: 'Get a role (default: the current one).', aliases: { role_id: 'id' }, input: S.obj({ id: S.str() }), run: (a) => this.role(a.id) },
      'role.switch': {
        tier: 'T1',
        description: 'Switch the active role (layout and accent). Not undoable.',
        aliases: { role_id: 'id' },
        input: S.obj({ id: S.str() }, ['id']),
        run: (a, ctx) => {
          const r = this.role(a.id);
          if (r.id !== this.roleId) {
            this.roleId = r.id;
            this.record(this.event(ctx.actor, 'role.switch', { kind: 'role', id: r.id }, `Switched to ${r.title}`));
          }
          return { role: r, layout: this.layoutFor() };
        },
      },

      'focus.get': { tier: 'T0', description: 'The focused item of the current role.', input: NO_ARGS, run: () => ({ focus: this.dayState().focus }) },
      'focus.set': {
        tier: 'T1',
        description: 'Focus an item.',
        aliases: { item_id: 'id' },
        input: S.obj({ id: S.str() }, ['id']),
        run: (a, ctx) => {
          const it = this.require(a.id);
          if (this.focusByRole.get(this.roleId) !== it.id) {
            this.focusByRole.set(this.roleId, it.id);
            this.record(this.event(ctx.actor, 'focus.set', { kind: 'item', id: it.id }, `Focused "${short(it.title)}"`));
          }
          return { focus: this.view(it.id) };
        },
      },
      'focus.clear': {
        tier: 'T1',
        description: 'Clear the focus.',
        input: NO_ARGS,
        run: (_a, ctx) => {
          if (this.focusByRole.get(this.roleId)) {
            this.focusByRole.set(this.roleId, null);
            this.record(this.event(ctx.actor, 'focus.clear', { kind: 'role', id: this.roleId }, 'Cleared focus'));
          }
          return { focus: null };
        },
      },

      'day.state': { tier: 'T0', description: 'The whole picture of today, in one call.', input: S.obj({ date: S.str() }), run: (a) => this.dayState(a.date === undefined ? undefined : this.checkDate(a.date, 'date')) },

      'capture.quick': {
        tier: 'T1',
        description: 'Create an item from one line, e.g. "Fix login #bug due:friday". Goes to the Inbox by default.',
        input: S.obj({ text: S.str(), parent_id: S.str(), tags: S.strs(), kind: kindSchema, top: S.bool() }, ['text']),
        run: (a, ctx) => {
          const parsed = resolveCapture(a.text as string, this.today(), (a.tags as string[] | undefined) ?? [], (a.kind as Item['kind'] | undefined) ?? 'task');
          if (typeof a.parent_id === 'string') this.require(a.parent_id);
          const iso = this.now().toISOString();
          const it: Item = {
            id: ulid(),
            parent_id: null,
            order: 0,
            kind: parsed.kind,
            status: parsed.status,
            title: parsed.title,
            notes: '',
            tags: parsed.tags,
            inherited_tags: [],
            due: parsed.due,
            type_id: null,
            fields: {},
            roles: [this.roleId],
            origin: ctx.actor,
            created_at: iso,
            updated_at: iso,
            version: 1,
          };
          this.writeItems(ctx, 'capture.quick', { kind: 'item', id: it.id }, `Captured "${short(it.title)}"`, () => {
            let parent = a.parent_id as string | undefined;
            if (!parent) {
              parent = this.children(null).find((r) => r.kind === 'bullet' && r.title.toLowerCase() === INBOX_TITLE.toLowerCase())?.id;
              if (!parent) {
                const inbox: Item = { ...it, id: ulid(), kind: 'bullet', status: 'todo', title: INBOX_TITLE, tags: [INBOX_TAG], due: null, roles: [], origin: ctx.actor };
                this.insert(inbox, null, 0);
                parent = inbox.id;
              }
            }
            it.parent_id = parent;
            this.insert(it, parent, a.top === true ? 0 : undefined);
          });
          return { item: this.view(it.id), parsed };
        },
      },
      'capture.parse': {
        tier: 'T0',
        description: 'Parse capture text exactly as capture.quick would, without writing anything.',
        input: S.obj({ text: S.str(), tags: S.strs(), kind: kindSchema }, ['text']),
        run: (a) => resolveCapture(a.text as string, this.today(), (a.tags as string[] | undefined) ?? [], (a.kind as Item['kind'] | undefined) ?? 'task'),
      },

      ...this.adminCommands(),
      ...this.integrationCommands(),
      ...this.packs.commands(),

      'consent.list': {
        tier: 'T0',
        description: 'Requests waiting for the user, oldest first.',
        input: NO_ARGS,
        run: (_a, ctx) => {
          // Past the budget an entry carries its short form; consent.get still returns it in full.
          let used = 0;
          const pending = [...this.pending.values()]
            .filter((p) => ctx.actor === 'user' || p.actor === ctx.actor)
            .map((p) => {
              const pub = this.publicPending(p);
              const n = jsonBytes(pub.args);
              if (used + n > CONSENT_LIST_BUDGET_BYTES) return { ...pub, args: clone(p.short_args ?? shortArgs(p.args)), display_truncated: true };
              used += n;
              return pub;
            });
          return { pending };
        },
      },
      'consent.get': {
        tier: 'T0',
        description: 'One pending request.',
        aliases: { confirmation_id: 'id' },
        input: S.obj({ id: S.str() }, ['id']),
        run: (a, ctx) => {
          const p = this.pending.get(a.id as string);
          if (!p || (ctx.actor !== 'user' && p.actor !== ctx.actor)) throw new CommandError('not_found', `no pending confirmation ${String(a.id)}`);
          return this.publicPending(p);
        },
      },
      'consent.approve': { tier: 'T0', user_only: true, description: 'Approve a pending request (user only).', aliases: { confirmation_id: 'id' }, input: S.obj({ id: S.str() }, ['id']), run: () => undefined },
      'consent.deny': { tier: 'T0', user_only: true, description: 'Deny a pending request (user only).', aliases: { confirmation_id: 'id' }, input: S.obj({ id: S.str() }, ['id']), run: () => undefined },
    };
  }

  // -------------------------------------------------- Agents and policy (v1.3)

  private clientView(c: ClientRec): ClientView {
    const actor: Actor = `agent:${c.id}`;
    const seen = this.lastSeen.get(actor);
    const taintAt = this.isTainted(actor) ? this.taintAt.get(actor) : undefined;
    return {
      id: c.id,
      name: c.name,
      actor,
      created_at: c.created_at,
      revoked_at: c.revoked_at,
      grants: clone(c.grants),
      last_seen: seen === undefined ? null : new Date(seen).toISOString(),
      tainted_until: taintAt === undefined ? null : new Date(taintAt + TAINT_TTL_MS).toISOString(),
    };
  }

  private loadClient(id: unknown): ClientRec {
    const c = typeof id === 'string' ? this.clients.get(id) : undefined;
    if (!c) throw new CommandError('not_found', `client ${String(id)} not found`);
    return c;
  }

  private adminEvent(ctx: Ctx, command: string, entity: EntityRef, summary: string): void {
    this.record(this.event(ctx.actor, command, entity, summary));
  }

  private adminCommands(): Record<string, CommandDef> {
    const idSchema = S.str();
    return {
      'clients.list': { tier: 'T0', user_only: true, description: 'Paired MCP clients.', input: NO_ARGS, run: () => ({ clients: [...this.clients.values()].map((c) => this.clientView(c)) }) },
      'clients.pair': {
        tier: 'T2',
        user_only: true,
        description: 'Pair a new MCP client. The token is shown once.',
        input: S.obj({ name: S.str(), id: S.nstr() }, ['name']),
        run: (a, ctx) => {
          const name = (a.name as string).trim();
          if (!name) throw invalid('name must not be empty');
          if (Array.from(name).length > 100) throw invalid('name exceeds 100 characters');
          let id: string;
          if (typeof a.id === 'string') {
            if (!/^[A-Za-z0-9_.-]{1,64}$/.test(a.id)) throw invalid('client id must be 1-64 of [A-Za-z0-9_.-]');
            if (this.clients.has(a.id)) throw new CommandError('conflict', `client ${a.id} already exists`);
            id = a.id;
          } else {
            const base = name.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 56) || 'client';
            id = base;
            for (let n = 2; this.clients.has(id); n++) id = `${base}-${n}`;
          }
          const token = `halp_${randomHex(32)}`;
          const c: ClientRec = { id, name, token_hash: fakeHash(token), created_at: this.now().toISOString(), revoked_at: null, grants: { t1: false, data_classes: ['general'] } };
          this.clients.set(id, c);
          this.adminEvent(ctx, 'clients.pair', { kind: 'client', id }, `Paired MCP client ${short(name)}`);
          return { client: this.clientView(c), token };
        },
      },
      'clients.revoke': {
        tier: 'T2',
        user_only: true,
        description: 'Revoke a client: its token stops working and its standing rules go.',
        input: S.obj({ id: idSchema }, ['id']),
        run: (a, ctx) => {
          const c = this.loadClient(a.id);
          if (c.revoked_at) return this.clientView(c);
          c.revoked_at = this.now().toISOString();
          this.rules = this.rules.filter((r) => r.client_id !== c.id);
          this.adminEvent(ctx, 'clients.revoke', { kind: 'client', id: c.id }, `Revoked MCP client ${short(c.name)}`);
          return this.clientView(c);
        },
      },
      'grants.list': {
        tier: 'T0',
        user_only: true,
        description: 'What each client may do, and the data classes there are.',
        input: S.obj({ client_id: S.nstr() }),
        run: (a) => {
          if (typeof a.client_id === 'string') this.loadClient(a.client_id);
          const cs = [...this.clients.values()].filter((c) => typeof a.client_id !== 'string' || c.id === a.client_id);
          return { grants: cs.map((c) => ({ client_id: c.id, grants: clone(c.grants) })), data_classes: [...DATA_CLASSES] };
        },
      },
      'grants.set': {
        tier: 'T2',
        user_only: true,
        description: "Change a client's T1 grant and data classes.",
        input: S.obj({ client_id: S.str(), t1: { type: ['boolean', 'null'] }, data_classes: { type: ['array', 'null'], items: { type: 'string' } } }, ['client_id']),
        run: (a, ctx) => {
          const c = this.loadClient(a.client_id);
          const next: Grants = clone(c.grants);
          if (typeof a.t1 === 'boolean') next.t1 = a.t1;
          if (Array.isArray(a.data_classes)) {
            // Built-ins plus any class a loaded type declares (x-halp-data-class).
            const known = new Set([...DATA_CLASSES, ...[...this.types.values()].map((t) => (t.schema as Record<string, unknown>)['x-halp-data-class']).filter((x): x is string => typeof x === 'string')]);
            for (const d of a.data_classes as string[]) if (!known.has(d)) throw invalid(`unknown data class ${d}`);
            next.data_classes = [...new Set(a.data_classes as string[])].sort();
          }
          if (!sameJson(next, c.grants)) {
            c.grants = next;
            this.adminEvent(ctx, 'grants.set', { kind: 'client', id: c.id }, `Changed grants of ${short(c.name)}`);
          }
          return this.clientView(c);
        },
      },
      'policy.clear_taint': {
        tier: 'T1',
        user_only: true,
        description: "Vouch for what a client has read, so standing rules apply to it again.",
        input: S.obj({ client_id: S.str() }, ['client_id']),
        run: (a, ctx) => {
          const c = this.loadClient(a.client_id);
          const actor: Actor = `agent:${c.id}`;
          const was = this.isTainted(actor) ? (this.taint.get(actor) ?? null) : null;
          this.taint.delete(actor);
          this.taintAt.delete(actor);
          this.adminEvent(ctx, 'policy.clear_taint', { kind: 'client', id: c.id }, `Cleared the taint of MCP client ${short(c.name)}`);
          return { client_id: c.id, was_tainted_by: was };
        },
      },
      'rules.list': {
        tier: 'T0',
        user_only: true,
        description: 'Standing rules.',
        input: S.obj({ client_id: S.nstr() }),
        run: (a) => ({ rules: this.rules.filter((r) => typeof a.client_id !== 'string' || r.client_id === a.client_id) }),
      },
      'rules.create': {
        tier: 'T2',
        user_only: true,
        description: 'Let one client run one T2 command with exactly these arguments without asking.',
        input: S.obj({ client_id: S.str(), command: S.str(), args: S.any() }, ['client_id', 'command', 'args']),
        run: (a, ctx) => {
          const c = this.loadClient(a.client_id);
          if (c.revoked_at) throw invalid(`client ${c.id} is revoked`);
          const def = this.commands[a.command as string];
          if (!def) throw new CommandError('not_found', `unknown command ${String(a.command)}`);
          if (def.tier !== 'T2' || def.user_only) throw invalid(`${String(a.command)} is not a T2 command an agent may run`);
          const keys = Object.keys(a.args as object);
          if (keys.length === 0) throw invalid('a rule needs at least one argument predicate');
          if (keys.length > 16) throw invalid('at most 16 argument predicates');
          const rule: Rule = { id: ulid(), client_id: c.id, command: a.command as string, args: clone(a.args as Record<string, JsonValue>), created_at: this.now().toISOString(), created_by: ctx.actor };
          this.rules.push(rule);
          this.adminEvent(ctx, 'rules.create', { kind: 'rule', id: rule.id }, `Standing rule: ${rule.client_id} may run ${rule.command}`);
          return rule;
        },
      },
      'rules.delete': {
        tier: 'T2',
        user_only: true,
        description: 'Delete a standing rule.',
        input: S.obj({ id: S.str() }, ['id']),
        run: (a, ctx) => {
          if (!this.rules.some((r) => r.id === a.id)) throw new CommandError('not_found', `rule ${String(a.id)} not found`);
          this.rules = this.rules.filter((r) => r.id !== a.id);
          this.adminEvent(ctx, 'rules.delete', { kind: 'rule', id: a.id as string }, 'Deleted a standing rule');
          return { rules: this.rules };
        },
      },
      'audit.list': {
        tier: 'T0',
        data_class: 'audit',
        description: 'Policy decisions, newest first.',
        input: S.obj({ limit: { type: ['integer', 'null'], minimum: 0 }, before: S.nstr(), actor_prefix: S.nstr() }),
        run: (a) => {
          if (a.limit === 0) throw invalid('limit must be at least 1');
          const limit = Math.min((a.limit as number | null | undefined) ?? 100, 1000);
          let rows = [...this.auditLog].reverse();
          if (typeof a.before === 'string') {
            const i = rows.findIndex((r) => r.id === a.before);
            rows = i < 0 ? [] : rows.slice(i + 1);
          }
          const prefix = typeof a.actor_prefix === 'string' ? a.actor_prefix : '';
          return { records: rows.filter((r) => r.actor.startsWith(prefix)).slice(0, limit) };
        },
      },
      'history.purge': {
        tier: 'T3',
        user_only: true,
        description: 'Redact old history (and optionally what deleted items left behind). Cannot be undone.',
        input: S.obj({ older_than_days: { type: ['integer', 'null'], minimum: 0 }, deleted_items: S.bool(), remember: S.bool() }),
        run: (a, ctx) => {
          const days = a.older_than_days as number | null | undefined;
          if (days == null && a.deleted_items !== true) throw invalid('give older_than_days, deleted_items or both');
          if (days != null && (days < 1 || days > 36500)) throw invalid('older_than_days must be 1..=36500');
          const cutoff = days == null ? -Infinity : this.now().getTime() - days * 86_400_000;
          let redacted = 0;
          for (const r of this.log) {
            if (Date.parse(r.event.ts) < cutoff && r.event.summary !== '[redacted]') {
              r.event.summary = '[redacted]';
              r.change = undefined;
              redacted++;
            }
          }
          this.adminEvent(ctx, 'history.purge', { kind: 'history', id: 'purge' }, `Redacted ${redacted} event${redacted === 1 ? '' : 's'}`);
          return { redacted, retention_days: a.remember === true && days != null ? days : null };
        },
      },
      'import.tui': {
        tier: 'T3',
        user_only: true,
        description: 'Import a halptask TUI export (markdown, or an encrypted .halp file).',
        input: S.obj({ content_base64: S.str(), file_name: S.nstr(), passphrase: S.nstr(), utc_offset_minutes: { type: ['integer', 'null'] } }, ['content_base64']),
        run: (a, ctx) => {
          let text: string;
          try {
            text = new TextDecoder('utf-8', { fatal: true }).decode(Uint8Array.from(atob(a.content_base64 as string), (ch) => ch.charCodeAt(0)));
          } catch {
            throw invalid('content_base64 is not base64 of a UTF-8 export');
          }
          const encrypted = typeof a.file_name === 'string' && /\.halp$/i.test(a.file_name);
          if (encrypted && typeof a.passphrase !== 'string') throw invalid('this export is encrypted: give its passphrase');
          const titles = text
            .split(/\r?\n/)
            .map((l) => /^\s*[-*]\s+(?:\[[ xX]\]\s+)?(.+)$/.exec(l)?.[1]?.trim())
            .filter((t): t is string => !!t && !/\p{Cc}/u.test(t))
            .slice(0, 1000);
          const iso = this.now().toISOString();
          const rootId = ulid();
          const base = { order: 0, notes: '', due: null, type_id: null, origin: ctx.actor, created_at: iso, updated_at: iso, version: 1 };
          this.writeItems(ctx, 'import.tui', { kind: 'item', id: rootId }, `Imported ${titles.length} item${titles.length === 1 ? '' : 's'} from the TUI`, () => {
            this.insert({ ...base, id: rootId, parent_id: null, kind: 'bullet', status: 'todo', title: 'Imported from the TUI', tags: ['import'], inherited_tags: [], fields: {}, roles: [this.roleId] }, null);
            for (const t of titles) this.insert({ ...base, id: ulid(), parent_id: rootId, kind: 'task', status: 'todo', title: short(t).slice(0, 500), tags: [], inherited_tags: [], roles: [this.roleId], fields: {} }, rootId);
          });
          return { format: encrypted ? 'encrypted' : 'markdown', items: titles.length + 1, roots: 1, skipped_deleted: 0, warnings: titles.length ? [] : ['no tasks found in the file'] };
        },
      },
    };
  }

  // ------------------------------------------------------- Integrations (P3)

  private integrationView(r: IntegrationRec): IntegrationEntry {
    return { config: clone(r.config), status: clone(r.status) };
  }

  private setStatus(r: IntegrationRec, state: IntegrationEntry['status']['state'], error: string | null = null): void {
    if (r.status.state === state && (r.status.error ?? null) === error) return;
    r.status.state = state;
    r.status.error = error;
    this.record(this.event(`integration:${r.config.id}`, 'integrations.status', { kind: 'integration', id: r.config.id }, `${short(r.config.title)}: ${state.replace('_', ' ')}`));
  }

  /** The mock GitHub: open PRs per repo; the second sync "merges" the last one, so it is archived. */
  private syncGithub(r: IntegrationRec): void {
    const id = r.config.id;
    const actor: Actor = `integration:${id}`;
    const repos = ((r.config.settings as { repos?: string[] } | null)?.repos ?? []).slice(0, 5);
    const list = repos.length ? repos : ['halptask/hub'];
    r.syncs++;
    for (const [i, repo] of list.entries()) {
      if (!r.remote.has(repo)) r.remote.set(repo, [301 + i * 10, 302 + i * 10, 303 + i * 10]);
      if (r.syncs === 2 && i === 0) r.remote.get(repo)!.pop();
    }
    const iso = this.now().toISOString();
    const counts = { created: 0, updated: 0, archived: 0 };
    let total = 0;
    const summary = () => `Synced ${short(r.config.title)}: ${counts.created} new, ${counts.updated} updated, ${counts.archived} archived`;
    this.writeItems({ actor }, 'ext.upsert', { kind: 'integration', id }, summary, () => {
      let root = [...this.items.values()].find((it) => it.origin === actor && it.parent_id === null && it.kind === 'bullet');
      if (!root) {
        root = { id: ulid(), parent_id: null, order: 0, kind: 'bullet', status: 'todo', title: r.config.title, notes: '', tags: [], inherited_tags: [], due: null, type_id: null, fields: {}, roles: [...r.config.roles], origin: actor, created_at: iso, updated_at: iso, version: 1 };
        this.insert(root, null);
      }
      const rootId = root.id;
      const seen = new Set<string>();
      for (const [repo, nums] of r.remote) {
        for (const n of nums) {
          total++;
          const ext = `github:${repo}#${n}`;
          seen.add(ext);
          const existing = [...this.items.values()].find((it) => it.origin === actor && it.fields.external_id === ext);
          if (existing) {
            if (existing.tags.includes('archived')) {
              existing.tags = existing.tags.filter((t) => t !== 'archived');
              existing.version++;
              counts.updated++;
            }
            continue;
          }
          counts.created++;
          this.insert(
            {
              id: ulid(),
              parent_id: rootId,
              order: 0,
              kind: 'task',
              status: 'todo',
              title: `${['Retry webhook delivery', 'Tidy the release workflow', 'Cache the ETag lookups'][n % 3]} (${repo}#${n})`,
              notes: '',
              tags: ['review'],
              inherited_tags: [],
              due: null,
              type_id: 'dev.pull_request',
              fields: { external_id: ext, repo, number: n, state: 'review_requested', ci: n % 2 ? 'passing' : 'pending', url: `https://github.com/${repo}/pull/${n}`, author: 'octo', reviewers: ['you'], requested_from_me: true, authored_by_me: false, updated_at: iso, additions: 40 + n, deletions: 12, branch: `octo/pr-${n}`, mergeable: 'clean' },
              roles: [...r.config.roles],
              origin: actor,
              created_at: iso,
              updated_at: iso,
              version: 1,
            },
            rootId,
          );
        }
      }
      for (const it of this.items.values()) {
        const ext = it.fields.external_id;
        if (it.origin !== actor || typeof ext !== 'string' || seen.has(ext) || it.tags.includes('archived')) continue;
        it.tags = [...it.tags, 'archived'];
        if (it.kind === 'task' && !this.hasSubtasks(it.id)) it.status = 'done';
        it.version++;
        it.updated_at = iso;
        counts.archived++;
      }
    });
    r.status.items = total;
    r.status.last_sync = iso;
    r.status.next_sync = new Date(this.now().getTime() + r.config.interval_secs * 1000).toISOString();
    this.setStatus(r, 'ok');
  }

  private syncOne(r: IntegrationRec): void {
    if (!r.config.enabled) return this.setStatus(r, 'disabled');
    if (r.config.kind === 'github') {
      if (!r.status.has_secret) return this.setStatus(r, 'needs_auth');
      this.syncGithub(r);
    } else if (r.config.kind === 'git') {
      this.setStatus(r, 'ok');
    } else if (r.config.kind === 'calendar' && (r.config.settings as { source?: unknown } | null)?.source !== 'mcp') {
      // No feed is fetched in the browser: a stored URL just reads as synced.
      this.setStatus(r, r.status.has_secret ? 'ok' : 'needs_auth');
    } else {
      this.setStatus(r, 'error', 'the mock does not run MCP servers');
    }
  }

  /**
   * The browser stand-in for the Tauri command `integrations_set_token`:
   * rejects with the same error codes, never stores the token itself.
   */
  async setIntegrationToken(id: string, token: string): Promise<void> {
    const r = this.integrations.get(id);
    if (!r) throw 'unknown_integration';
    if (!token) throw 'empty';
    if (token.length > 4096) throw 'too_long';
    if (!/^[\x20-\x7e]+$/.test(token)) throw 'not_printable_ascii';
    const feed = r.config.kind === 'calendar' && (r.config.settings as { source?: unknown } | null)?.source !== 'mcp';
    let hint: string | null = null;
    if (feed) {
      let u: URL;
      try {
        u = new URL(token.replace(/^webcal:/i, 'https:'));
      } catch {
        throw 'invalid_url';
      }
      if (u.protocol !== 'https:' || !u.hostname || u.username || u.password) throw 'invalid_url';
      // Like halp-integrations url_hint: the host and the last 4 characters.
      const src = u.search.length > 4 ? u.search.slice(1) : u.pathname.replace(/\/$/, '').replace(/(\/basic|\/full|\/calendar)?\.ics$/, '');
      hint = src.length < 8 ? u.hostname : `${u.hostname} …${src.slice(-4)}`;
    }
    // Only the hint is kept; the token itself is dropped here.
    this.secretHints.set(id, hint);
    r.status.has_secret = true;
    this.syncOne(r);
  }

  /** The browser stand-in for the Tauri command `integrations_secret_hint`. */
  async secretHint(id: string): Promise<string | null> {
    return this.integrations.get(id)?.status.has_secret ? (this.secretHints.get(id) ?? null) : null;
  }

  private integrationCommands(): Record<string, CommandDef> {
    return {
      'integrations.list': {
        tier: 'T0',
        description: 'Configured integrations and their status.',
        input: NO_ARGS,
        run: () => ({ running: true, integrations: [...this.integrations.values()].map((r) => this.integrationView(r)) }),
      },
      'integrations.sync_now': {
        tier: 'T1',
        description: 'Sync one enabled integration now, or all of them.',
        input: S.obj({ id: S.nstr() }),
        run: (a) => {
          let rs = [...this.integrations.values()];
          if (typeof a.id === 'string') {
            const r = this.integrations.get(a.id);
            if (!r) throw new CommandError('not_found', `integration ${a.id} not found`);
            rs = [r];
          }
          const queued = rs.filter((r) => r.config.enabled && r.config.kind !== 'git');
          for (const r of queued) this.syncOne(r);
          return { queued: queued.map((r) => r.config.id) };
        },
      },
      'integrations.connect': {
        tier: 'T2',
        user_only: true,
        description: 'Add or change an integration. Tokens are set separately, never here.',
        input: S.obj(
          { id: S.str(), kind: S.str(), title: S.nstr(), enabled: { type: ['boolean', 'null'] }, interval_secs: { type: ['integer', 'null'], minimum: 0 }, roles: { type: ['array', 'null'], items: { type: 'string' } }, settings: {}, reauthorize: { type: ['boolean', 'null'] } },
          ['id', 'kind'],
        ),
        run: (a, ctx) => {
          const id = a.id as string;
          const kind = a.kind as string;
          if (!/^[A-Za-z0-9_.-]{1,64}$/.test(id)) throw invalid('integration id must be 1-64 of [A-Za-z0-9_.-]');
          if (!['github', 'mcp', 'git', 'calendar'].includes(kind)) throw invalid(`unknown integration kind ${kind}`);
          const prev = this.integrations.get(id);
          if (prev && prev.config.kind !== kind) throw invalid(`integration ${id} is a ${prev.config.kind} integration; its kind cannot change`);
          const interval = (a.interval_secs as number | null | undefined) ?? prev?.config.interval_secs ?? 300;
          if (interval < 60 || interval > 86400) throw invalid('interval_secs must be 60..=86400');
          const settings = a.settings === undefined ? (prev?.config.settings ?? null) : a.settings;
          if (jsonBytes(settings) > 16 * 1024) throw invalid('settings exceed 16 KiB');
          if (kind === 'github') {
            const repos = (settings as { repos?: unknown } | null)?.repos ?? [];
            if (!Array.isArray(repos) || !repos.every((x) => typeof x === 'string' && /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(x))) throw invalid('settings.repos must be a list of owner/name');
          }
          const iso = this.now().toISOString();
          const config: IntegrationEntry['config'] = {
            id,
            kind,
            title: (typeof a.title === 'string' && a.title.trim()) || prev?.config.title || ({ github: 'GitHub', mcp: 'MCP', git: 'Git', calendar: 'Calendar' } as Record<string, string>)[kind]!,
            enabled: typeof a.enabled === 'boolean' ? a.enabled : (prev?.config.enabled ?? true),
            interval_secs: interval,
            roles: Array.isArray(a.roles) ? (a.roles as string[]) : (prev?.config.roles ?? ['developer']),
            settings,
            created_at: prev?.config.created_at ?? iso,
            updated_at: iso,
          };
          const r: IntegrationRec = prev ?? { config, status: { state: 'pending', items: 0, has_secret: false, authorizing: false, error: null, last_sync: null, next_sync: null }, remote: new Map(), syncs: 0 };
          r.config = config;
          this.integrations.set(id, r);
          this.adminEvent(ctx, 'integrations.connect', { kind: 'integration', id }, `${prev ? 'Changed' : 'Connected'} ${short(config.title)}`);
          this.syncOne(r);
          return this.integrationView(r);
        },
      },
      'integrations.disconnect': {
        tier: 'T3',
        user_only: true,
        description: 'Remove an integration and its token; optionally delete what it synced.',
        input: S.obj({ id: S.str(), remove_items: { type: ['boolean', 'null'] } }, ['id']),
        run: (a, ctx) => {
          const r = this.integrations.get(a.id as string);
          if (!r) throw new CommandError('not_found', `integration ${String(a.id)} not found`);
          this.integrations.delete(r.config.id);
          let removed = 0;
          if (a.remove_items === true) {
            const actor = `integration:${r.config.id}`;
            this.writeItems(ctx, 'integrations.disconnect', { kind: 'integration', id: r.config.id }, `Disconnected ${short(r.config.title)} and removed its items`, () => {
              for (const it of [...this.items.values()]) {
                if (it.origin === actor && this.items.has(it.id)) {
                  for (const sid of this.subtree(it.id)) if (this.items.delete(sid)) removed++;
                }
              }
            });
          } else {
            this.adminEvent(ctx, 'integrations.disconnect', { kind: 'integration', id: r.config.id }, `Disconnected ${short(r.config.title)}`);
          }
          return { id: r.config.id, removed_items: removed };
        },
      },
      'context.capture': {
        tier: 'T1',
        description: 'Snapshot a git repo under a configured root as a dev.context item.',
        input: S.obj({ repo_path: S.str(), integration: S.nstr() }, ['repo_path']),
        run: (a) => {
          const gits = [...this.integrations.values()].filter((r) => r.config.kind === 'git' && r.config.enabled && (typeof a.integration !== 'string' || r.config.id === a.integration));
          const r = gits[0];
          if (!r) throw new CommandError('not_found', 'no enabled git integration');
          const path = a.repo_path as string;
          if (!path.startsWith('/')) throw invalid('repo_path must be absolute');
          const roots = ((r.config.settings as { roots?: string[] } | null)?.roots ?? []).map((x) => x.replace(/\/+$/, ''));
          if (!roots.some((root) => path === root || path.startsWith(`${root}/`))) throw invalid('repo_path is not under a configured root');
          return { integration: r.config.id, repo_path: path, queued: true };
        },
      },
    };
  }

  private descriptors(): CommandDescriptor[] {
    return Object.entries(this.commands)
      .map(([name, d]) => ({
      name,
      description: d.description,
      tier: d.tier,
      input_schema: d.input,
      output_schema: { type: 'object' },
      data_class: d.data_class ?? 'general',
      mcp: !d.user_only,
      ...(d.user_only ? { user_only: true } : {}),
    }));
  }

  private seedHistory(): void {
    const agent = this.agent('claude');
    const base = this.now().getTime();
    void agent.dispatch('tasks.create', { title: 'Review #479: keyring fallback on Linux follow-up', tags: ['review'], roles: ['developer'], due: addDays(this.today(), 1) });
    void agent.dispatch('tasks.update', { id: 'dev-5', patch: { notes: 'Jonas is free after 14:00.' } });
    void agent.dispatch('focus.set', { id: 'dev-2' });
    // Backdate so the strip reads "12m ago" etc.
    const ages = [42, 18, 6];
    this.log.forEach((r, i) => {
      r.event.ts = new Date(base - (ages[i] ?? 1) * 60_000).toISOString();
    });
  }
}
