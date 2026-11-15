// Agent administration as plain data, so it can be tested without a DOM:
// argument building for grants.set / rules.create / history.purge /
// import.tui, MCP config snippets for a freshly paired client, and what the
// audit log says about each client. Shapes follow halp-core
// (commands/admin.rs) and hub/docs/fixtures/commands.json.

import type { AuditEntry, ClientView, CommandDescriptor, Grants, JsonValue, TaskTypeView } from '../api/types';

/** The data classes a grant can name (grants.list returns the live list; this is the fallback). */
export const DATA_CLASSES = ['audit', 'email.body', 'general', 'notes.private', 'secrets'] as const;

export const DATA_CLASS_LABELS: Record<string, string> = {
  general: 'Tasks, layout and everyday data',
  'notes.private': 'Private notes (1:1s and the like)',
  'email.body': 'Email bodies',
  secrets: 'Secrets',
  audit: 'The audit log',
};

/** Audit decisions (AuditEntry.decision). `superseded` is v1.4. */
export const AUDIT_DECISIONS = ['allow', 'deny', 'confirm', 'approved', 'failed', 'denied', 'expired', 'superseded'] as const;

/** How long taint lasts after the last tainted read (halp-policy DEFAULT_TAINT_TTL). */
export const TAINT_TTL_MS = 30 * 60_000;

// ---------------------------------------------------------------- Grants

export interface GrantsDraft {
  t1: boolean;
  data_classes: string[];
}

export type GrantsSetArgs = { client_id: string; t1?: boolean; data_classes?: string[] };

const sortedUnique = (xs: readonly string[]) => [...new Set(xs)].sort();

/**
 * `grants.set` args for going from `current` to `next`: only what changed,
 * with `data_classes` as the full sorted list (the core replaces it). Null
 * when nothing changed.
 */
export function grantsSetArgs(clientId: string, current: Grants, next: GrantsDraft): GrantsSetArgs | null {
  const args: GrantsSetArgs = { client_id: clientId };
  if (next.t1 !== current.t1) args.t1 = next.t1;
  const want = sortedUnique(next.data_classes);
  if (want.join('\n') !== sortedUnique(current.data_classes).join('\n')) args.data_classes = want;
  return 't1' in args || 'data_classes' in args ? args : null;
}

export interface GrantableClass {
  id: string;
  label: string;
  /** Built in, or the types (and their pack) that declare it. */
  source: string;
  builtin: boolean;
}

/**
 * Every class `grants.set` accepts: the built-ins (`grants.list.data_classes`)
 * plus the distinct `x-halp-data-class` values of loaded types, each
 * labelled with the types and packs that declare it.
 */
export function grantableClasses(builtins: readonly string[], types: readonly Pick<TaskTypeView, 'id' | 'title' | 'origin' | 'schema'>[]): GrantableClass[] {
  const declared = new Map<string, string[]>();
  for (const t of types) {
    const cls = (t.schema as Record<string, unknown> | null)?.['x-halp-data-class'];
    if (typeof cls !== 'string' || !cls) continue;
    const where = `${t.title || t.id} (${t.origin === 'user' ? 'your type' : `pack ${t.origin}`})`;
    declared.set(cls, [...(declared.get(cls) ?? []), where]);
  }
  const out: GrantableClass[] = builtins.map((id) => ({
    id,
    label: DATA_CLASS_LABELS[id] ?? id,
    source: declared.has(id) ? `built in; also used by ${declared.get(id)!.join(', ')}` : 'built in',
    builtin: true,
  }));
  for (const [id, where] of [...declared].sort(([a], [b]) => a.localeCompare(b))) {
    if (builtins.includes(id)) continue;
    out.push({ id, label: DATA_CLASS_LABELS[id] ?? id, source: `declared by ${where.join(', ')}`, builtin: false });
  }
  return out;
}

/** Toggle one data class in a draft (returns a new draft). */
export function toggleClass(draft: GrantsDraft, cls: string, on: boolean): GrantsDraft {
  const set = new Set(draft.data_classes);
  if (on) set.add(cls);
  else set.delete(cls);
  return { ...draft, data_classes: [...set].sort() };
}

// ----------------------------------------------------------------- Rules

/**
 * Commands a standing rule may name: every agent-callable T2 command in
 * commands.list (T2, not user-only, offered over MCP). The core refuses the
 * rest. Today the core has none; packs and extensions may add some.
 */
export function rulableCommands(commands: readonly CommandDescriptor[]): CommandDescriptor[] {
  return commands.filter((c) => c.tier === 'T2' && !c.user_only && c.mcp !== false).sort((a, b) => a.name.localeCompare(b.name));
}

export type RuleArgs = { client_id: string; command: string; args: Record<string, JsonValue> };

/**
 * Parse the rule form. `argsText` is a JSON object of top-level argument
 * name to the exact value it must have (at least one, at most 16).
 */
export function ruleArgs(clientId: string, command: string, argsText: string, schema?: CommandDescriptor): { ok: true; args: RuleArgs } | { ok: false; error: string } {
  if (!clientId) return { ok: false, error: 'Choose an agent.' };
  if (!command) return { ok: false, error: 'Choose a command.' };
  if (schema && (schema.tier !== 'T2' || schema.user_only)) return { ok: false, error: `${command} cannot have a standing rule: only T2 commands an agent may call can.` };
  let parsed: unknown;
  try {
    parsed = JSON.parse(argsText || '{}');
  } catch {
    return { ok: false, error: 'The arguments are not valid JSON.' };
  }
  if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return { ok: false, error: 'The arguments must be a JSON object, e.g. {"repo": "halptask"}.' };
  const keys = Object.keys(parsed);
  if (keys.length === 0) return { ok: false, error: 'A rule needs at least one argument to match.' };
  if (keys.length > 16) return { ok: false, error: 'A rule can match at most 16 arguments.' };
  const props = schema?.input_schema.properties;
  if (props) {
    const unknown = keys.filter((k) => !(k in props));
    if (unknown.length) return { ok: false, error: `${command} has no argument ${unknown.map((k) => `\`${k}\``).join(', ')}.` };
  }
  return { ok: true, args: { client_id: clientId, command, args: parsed as Record<string, JsonValue> } };
}

/** `key = value` lines for a rule's matched args, as JSON values. */
export function ruleMatchLines(args: Record<string, JsonValue>): string[] {
  return Object.keys(args)
    .sort()
    .map((k) => `${k} = ${JSON.stringify(args[k])}`);
}

// ----------------------------------------------------------------- Audit

export interface AuditFilter {
  /** Matched with audit.list's `actor_prefix` in the core. */
  actor: string;
  /** '' = any. Filtered here: audit.list has no decision filter. */
  decision: string;
}

export function filterAudit(records: readonly AuditEntry[], decision: string): AuditEntry[] {
  return decision ? records.filter((r) => r.decision === decision) : [...records];
}

/** One audit record as a line of text. */
export function auditLine(r: AuditEntry): string {
  const parts = [r.ts, r.decision.padEnd(10), r.tier, r.actor, r.command];
  if (r.approver) parts.push(`by ${r.approver}`);
  if (r.tainted) parts.push('[tainted]');
  if (r.reason) parts.push(`— ${r.reason}`);
  return parts.join('  ');
}

export interface ClientActivity {
  /** When the policy last decided a call of this client (core memory). */
  lastSeen: string | null;
  /** Tainted now: it read untrusted content less than the TTL ago. */
  tainted: boolean;
  /** When the taint clears by itself, while tainted. */
  taintedUntil: string | null;
}

/**
 * A client's activity from `clients.list` (v1.8 `last_seen` and
 * `tainted_until`, both from the policy's memory, not the audit log).
 */
export function clientActivity(c: Pick<ClientView, 'last_seen' | 'tainted_until'>, now: number = Date.now()): ClientActivity {
  const until = c.tainted_until ?? null;
  const t = until ? Date.parse(until) : NaN;
  const tainted = Number.isFinite(t) && t > now;
  return { lastSeen: c.last_seen ?? null, tainted, taintedUntil: tainted ? until : null };
}

export function clientStatus(c: ClientView): 'active' | 'revoked' {
  return c.revoked_at ? 'revoked' : 'active';
}

// ------------------------------------------------------------- MCP config

/** Where we suggest keeping a client's token (owner-only file, as in docs/MCP.md). */
export function tokenPath(clientId: string): string {
  return `~/.halptask/${clientId}.token`;
}

export interface McpSnippets {
  /** Shell: create an owner-only file and read the token from stdin (paste, Enter, Ctrl-D). */
  saveToken: string;
  claudeCode: string;
  claudeDesktop: string;
  generic: string;
}

/**
 * Ready-to-paste config for the `halp-mcp` stdio shim. The token never
 * appears in any of them: it goes in a 0600 file read through
 * `HALP_MCP_TOKEN_FILE`, so it stays out of shell history, `ps` and the
 * client's config. `home` is the absolute home directory for JSON configs
 * (they do not expand `~`); `$HOME` is used when unknown.
 */
export function mcpSnippets(clientId: string, home?: string): McpSnippets {
  const rel = tokenPath(clientId);
  const shellPath = rel.replace(/^~/, '$HOME');
  const absPath = rel.replace(/^~/, home ?? '/Users/<you>');
  const saveToken = `mkdir -p -m 700 ~/.halptask && (umask 077; cat > ${rel})`;
  const claudeCode = `claude mcp add halptask --env HALP_MCP_TOKEN_FILE=${shellPath} -- halp-mcp`;
  const config = { mcpServers: { halptask: { command: 'halp-mcp', env: { HALP_MCP_TOKEN_FILE: absPath } } } };
  const claudeDesktop = JSON.stringify(config, null, 2);
  const generic = `command: halp-mcp   (stdio, no arguments)\nenv:     HALP_MCP_TOKEN_FILE=${absPath}`;
  return { saveToken, claudeCode, claudeDesktop, generic };
}

/** A client id the core will accept (`[A-Za-z0-9_.-]{1,64}`), or an error. */
export function checkClientId(id: string): string | null {
  if (!id) return null;
  return /^[A-Za-z0-9_.-]{1,64}$/.test(id) ? null : 'Use 1–64 letters, digits, dot, dash or underscore.';
}

// ------------------------------------------------------------------- Data

/** `history.purge` args, or an error for the form. */
export function purgeArgs(daysText: string, remember: boolean, deletedItems: boolean): { ok: true; args: { older_than_days?: number; remember?: boolean; deleted_items?: boolean } } | { ok: false; error: string } {
  const t = daysText.trim();
  if (!t && !deletedItems) return { ok: false, error: 'Enter a number of days, or choose to redact deleted items.' };
  const args: { older_than_days?: number; remember?: boolean; deleted_items?: boolean } = {};
  if (t) {
    if (!/^\d+$/.test(t)) return { ok: false, error: 'Days must be a whole number.' };
    const n = Number(t);
    if (n < 1 || n > 36500) return { ok: false, error: 'Days must be between 1 and 36500.' };
    args.older_than_days = n;
    if (remember) args.remember = true;
  }
  if (deletedItems) args.deleted_items = true;
  return { ok: true, args };
}

/** Standard base64 of raw bytes (chunked, so large files do not overflow the call stack). */
export function bytesToBase64(bytes: Uint8Array): string {
  let bin = '';
  const CHUNK = 0x8000;
  for (let i = 0; i < bytes.length; i += CHUNK) bin += String.fromCharCode(...bytes.subarray(i, i + CHUNK));
  return btoa(bin);
}

/**
 * Whether to ask for a passphrase up front. Only a hint: the core decides
 * from the file's contents and answers `invalid_args` when one is missing.
 */
export function looksEncrypted(fileName: string): boolean {
  return /\.(halp|enc|age)$/i.test(fileName);
}

/** The import limit we enforce before reading a file into memory (the IPC message cap is 2 MiB). */
export const IMPORT_MAX_BYTES = 1536 * 1024;

/** The machine's UTC offset in minutes, as import.tui expects (east positive). */
export function utcOffsetMinutes(d: Date = new Date()): number {
  return -d.getTimezoneOffset();
}

// ----------------------------------------------------------- Integrations

/** `owner/name`, as GitHub spells a repository. */
const REPO_RE = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/;

/** Parse the repo allow-list (one per line, or comma separated). Empty means every repo. */
export function parseRepos(text: string): { ok: true; repos: string[] } | { ok: false; error: string } {
  const repos = [...new Set(text.split(/[\s,]+/).map((r) => r.trim()).filter(Boolean))];
  const bad = repos.filter((r) => !REPO_RE.test(r));
  if (bad.length) return { ok: false, error: `Use owner/name: ${bad.slice(0, 3).join(', ')}${bad.length > 3 ? ', …' : ''}` };
  if (repos.length > 100) return { ok: false, error: 'At most 100 repositories.' };
  return { ok: true, repos };
}

export type GithubConnectArgs = {
  id: string;
  kind: 'github';
  title?: string;
  interval_secs?: number;
  settings: { repos: string[]; auth: 'token' };
};

/**
 * `integrations.connect` args for a GitHub integration that authenticates
 * with a personal access token (set afterwards through the shell, never
 * here). Returns an error for the form instead.
 */
export function githubConnectArgs(id: string, title: string, reposText: string, intervalText = ''): { ok: true; args: GithubConnectArgs } | { ok: false; error: string } {
  if (!/^[A-Za-z0-9_.-]{1,64}$/.test(id)) return { ok: false, error: 'The id is 1–64 letters, digits, dot, dash or underscore.' };
  const repos = parseRepos(reposText);
  if (!repos.ok) return repos;
  const args: GithubConnectArgs = { id, kind: 'github', settings: { repos: repos.repos, auth: 'token' } };
  if (title.trim()) args.title = title.trim();
  const t = intervalText.trim();
  if (t) {
    if (!/^\d+$/.test(t)) return { ok: false, error: 'The interval is a whole number of seconds.' };
    const n = Number(t);
    if (n < 60 || n > 86400) return { ok: false, error: 'The interval is 60 to 86400 seconds.' };
    args.interval_secs = n;
  }
  return { ok: true, args };
}

export type CalendarConnectArgs = {
  id: string;
  kind: 'calendar';
  title?: string;
  roles: string[];
  settings: { source: 'ics' };
};

/**
 * The roles a new calendar is connected for by default: the active role,
 * plus developer when that is eng-manager (the calendar writes em.* items
 * only for eng-manager, and the developer view shows the meetings).
 */
export function defaultCalendarRoles(active: string): string[] {
  return active === 'eng-manager' ? ['eng-manager', 'developer'] : [active];
}

/**
 * `integrations.connect` args for an ICS calendar. The feed URL is a secret
 * set afterwards through the shell, never here. At least one role is needed.
 */
export function calendarConnectArgs(id: string, title: string, roles: string[]): { ok: true; args: CalendarConnectArgs } | { ok: false; error: string } {
  if (!/^[A-Za-z0-9_.-]{1,64}$/.test(id)) return { ok: false, error: 'The id is 1–64 letters, digits, dot, dash or underscore.' };
  const picked = [...new Set(roles)];
  if (!picked.length) return { ok: false, error: 'Choose at least one role.' };
  const args: CalendarConnectArgs = { id, kind: 'calendar', roles: picked, settings: { source: 'ics' } };
  if (title.trim()) args.title = title.trim();
  return { ok: true, args };
}

/**
 * The core already redacts tokens from status errors; this is a second
 * guard before display: anything shaped like a credential is masked.
 */
export function redactError(text: string | null | undefined): string {
  if (!text) return '';
  return text
    .replace(/\b(gh[pousr]_[A-Za-z0-9]{8,}|github_pat_[A-Za-z0-9_]{8,}|xox[abprs]-[A-Za-z0-9-]{8,}|lin_api_[A-Za-z0-9]{8,})/g, '[redacted]')
    .replace(/\b(Bearer|token)\s+[A-Za-z0-9._~+/=-]{8,}/gi, '$1 [redacted]')
    .replace(/(https?:\/\/)[^\s/@:]+:[^\s/@]+@/gi, '$1[redacted]@')
    .slice(0, 300);
}

export const INTEGRATION_STATE_LABEL: Record<string, { text: string; tone: 'neutral' | 'success' | 'warning' | 'danger' | 'info' }> = {
  ok: { text: 'OK', tone: 'success' },
  syncing: { text: 'Syncing', tone: 'info' },
  pending: { text: 'Pending', tone: 'neutral' },
  error: { text: 'Error', tone: 'danger' },
  needs_auth: { text: 'Needs a token', tone: 'warning' },
  disabled: { text: 'Disabled', tone: 'neutral' },
  stopped: { text: 'Not running', tone: 'neutral' },
};
