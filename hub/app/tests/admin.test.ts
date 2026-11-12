// Pure helpers behind the Settings panel (src/lib/settings/admin.ts).

import { describe, expect, it } from 'vitest';
import type { AuditEntry, CommandDescriptor } from '../src/lib/api/types';
import {
  bytesToBase64,
  checkClientId,
  clientActivity,
  filterAudit,
  calendarConnectArgs,
  defaultCalendarRoles,
  githubConnectArgs,
  grantsSetArgs,
  looksEncrypted,
  mcpSnippets,
  parseRepos,
  purgeArgs,
  redactError,
  rulableCommands,
  ruleArgs,
  ruleMatchLines,
  TAINT_TTL_MS,
  toggleClass,
} from '../src/lib/settings/admin';

const desc = (name: string, tier: string, extra: Partial<CommandDescriptor> = {}): CommandDescriptor =>
  ({ name, tier, description: name, input_schema: { type: 'object', properties: { parent_id: {}, title: {} } }, mcp: true, ...extra }) as CommandDescriptor;

const rec = (actor: string, ts: string, tainted = false, decision = 'allow'): AuditEntry =>
  ({ id: `a-${ts}`, ts, actor, command: 'tasks.create', tier: 'T2', args_hash: 'h', decision, reason: '', tainted }) as AuditEntry;

describe('grants.set args', () => {
  const cur = { t1: false, data_classes: ['general'] };
  it('sends only what changed, classes sorted and unique', () => {
    expect(grantsSetArgs('c', cur, { t1: false, data_classes: ['general'] })).toBeNull();
    expect(grantsSetArgs('c', cur, { t1: true, data_classes: ['general'] })).toEqual({ client_id: 'c', t1: true });
    expect(grantsSetArgs('c', cur, { t1: false, data_classes: ['secrets', 'general', 'secrets'] })).toEqual({ client_id: 'c', data_classes: ['general', 'secrets'] });
    expect(grantsSetArgs('c', cur, { t1: false, data_classes: [] })).toEqual({ client_id: 'c', data_classes: [] });
  });
  it('toggles a class without mutating the draft', () => {
    const d = { t1: false, data_classes: ['general'] };
    expect(toggleClass(d, 'audit', true).data_classes).toEqual(['audit', 'general']);
    expect(toggleClass(d, 'general', false).data_classes).toEqual([]);
    expect(d.data_classes).toEqual(['general']);
  });
});

describe('standing rule args', () => {
  const t2 = desc('tasks.create', 'T2');
  it('offers only T2 commands an agent may call', () => {
    const all = [t2, desc('tasks.delete', 'T3'), desc('tasks.get', 'T0'), desc('rules.create', 'T2', { user_only: true, mcp: false })];
    expect(rulableCommands(all).map((c) => c.name)).toEqual(['tasks.create']);
  });
  it('lists every agent-callable T2 command, sorted, and nothing an agent cannot call', () => {
    const packT2 = desc('pack.publish_brief', 'T2', { mcp: true });
    const hidden = desc('pack.internal', 'T2', { mcp: false });
    expect(rulableCommands([t2, hidden, packT2]).map((c) => c.name)).toEqual(['pack.publish_brief', 'tasks.create']);
  });
  it('builds args and refuses bad input', () => {
    expect(ruleArgs('claude', 'tasks.create', '{"parent_id": "dev-1"}', t2)).toEqual({ ok: true, args: { client_id: 'claude', command: 'tasks.create', args: { parent_id: 'dev-1' } } });
    expect(ruleArgs('', 'tasks.create', '{"a":1}', t2).ok).toBe(false);
    expect(ruleArgs('claude', 'tasks.create', '{}', t2)).toMatchObject({ ok: false, error: /at least one/ });
    expect(ruleArgs('claude', 'tasks.create', '[1]', t2).ok).toBe(false);
    expect(ruleArgs('claude', 'tasks.create', '{nope', t2)).toMatchObject({ ok: false, error: /JSON/ });
    expect(ruleArgs('claude', 'tasks.create', '{"colour": "red"}', t2)).toMatchObject({ ok: false, error: /colour/ });
    const many = JSON.stringify(Object.fromEntries(Array.from({ length: 17 }, (_, i) => [`k${i}`, i])));
    expect(ruleArgs('claude', 'tasks.create', many)).toMatchObject({ ok: false, error: /16/ });
    expect(ruleArgs('claude', 'tasks.delete', '{"id":"x"}', desc('tasks.delete', 'T3')).ok).toBe(false);
  });
  it('shows matched args as exact JSON values', () => {
    expect(ruleMatchLines({ title: 'x', parent_id: 'dev-1', n: 2 })).toEqual(['n = 2', 'parent_id = "dev-1"', 'title = "x"']);
  });
});

describe('client activity from clients.list', () => {
  const now = Date.parse('2026-11-10T10:00:00Z');
  it('takes last seen and taint expiry from the policy fields', () => {
    const until = '2026-11-10T10:20:00.000Z';
    expect(clientActivity({ last_seen: '2026-11-10T09:50:00Z', tainted_until: until }, now)).toEqual({ lastSeen: '2026-11-10T09:50:00Z', tainted: true, taintedUntil: until });
    expect(clientActivity({ last_seen: '2026-11-10T09:50:00Z', tainted_until: until }, now + TAINT_TTL_MS)).toMatchObject({ tainted: false, taintedUntil: null });
    expect(clientActivity({}, now)).toEqual({ lastSeen: null, tainted: false, taintedUntil: null });
  });
  it('filters by decision, superseded included', () => {
    const recs = [rec('a', '1', false, 'superseded'), rec('a', '2', false, 'allow')];
    expect(filterAudit(recs, 'superseded')).toHaveLength(1);
    expect(filterAudit(recs, '')).toHaveLength(2);
  });
});

describe('MCP snippets', () => {
  it('point at an owner-only token file and never contain a token', () => {
    const s = mcpSnippets('claude', '/Users/me');
    expect(s.saveToken).toBe('mkdir -p -m 700 ~/.halptask && (umask 077; cat > ~/.halptask/claude.token)');
    expect(s.claudeCode).toContain('HALP_MCP_TOKEN_FILE=$HOME/.halptask/claude.token');
    expect(JSON.parse(s.claudeDesktop)).toEqual({ mcpServers: { halptask: { command: 'halp-mcp', env: { HALP_MCP_TOKEN_FILE: '/Users/me/.halptask/claude.token' } } } });
    expect(Object.values(s).join('\n')).not.toMatch(/halp_[0-9a-f]{8,}/);
  });
  it('checks client ids', () => {
    expect(checkClientId('claude-2')).toBeNull();
    expect(checkClientId('bad id')).toMatch(/letters/);
  });
});

describe('data', () => {
  it('builds purge args', () => {
    expect(purgeArgs('90', true, false)).toEqual({ ok: true, args: { older_than_days: 90, remember: true } });
    expect(purgeArgs('', false, true)).toEqual({ ok: true, args: { deleted_items: true } });
    expect(purgeArgs('', true, false).ok).toBe(false);
    expect(purgeArgs('0', false, false).ok).toBe(false);
    expect(purgeArgs('1.5', false, false).ok).toBe(false);
  });
  it('encodes bytes as standard base64, in chunks', () => {
    expect(bytesToBase64(new TextEncoder().encode('- [ ] hi'))).toBe(btoa('- [ ] hi'));
    const big = new Uint8Array(200_000).map((_, i) => i % 251);
    expect(bytesToBase64(big)).toBe(Buffer.from(big).toString('base64'));
  });
  it('asks for a passphrase only for encrypted exports', () => {
    expect(looksEncrypted('backup.halp')).toBe(true);
    expect(looksEncrypted('tasks.md')).toBe(false);
  });
});

describe('GitHub connect', () => {
  it('parses an owner/name allow-list', () => {
    expect(parseRepos('octo/api\n octo/web, octo/api')).toEqual({ ok: true, repos: ['octo/api', 'octo/web'] });
    expect(parseRepos('just-a-name')).toMatchObject({ ok: false });
    expect(parseRepos('https://github.com/a/b')).toMatchObject({ ok: false });
  });
  it('builds integrations.connect args with token auth and no secret', () => {
    expect(githubConnectArgs('github', ' Work ', 'octo/api', '600')).toEqual({
      ok: true,
      args: { id: 'github', kind: 'github', title: 'Work', interval_secs: 600, settings: { repos: ['octo/api'], auth: 'token' } },
    });
    expect(githubConnectArgs('gh', '', 'octo/api', '30').ok).toBe(false);
    expect(githubConnectArgs('bad id', '', 'octo/api').ok).toBe(false);
  });
  it('builds calendar connect args with the chosen roles', () => {
    expect(defaultCalendarRoles('developer')).toEqual(['developer']);
    expect(defaultCalendarRoles('eng-manager')).toEqual(['eng-manager', 'developer']);
    expect(defaultCalendarRoles('cso')).toEqual(['cso']);
    expect(calendarConnectArgs('calendar', ' Work ', ['eng-manager', 'developer', 'developer'])).toEqual({
      ok: true,
      args: { id: 'calendar', kind: 'calendar', title: 'Work', roles: ['eng-manager', 'developer'], settings: { source: 'ics' } },
    });
    expect(calendarConnectArgs('calendar', '', []).ok).toBe(false);
    expect(calendarConnectArgs('bad id', '', ['developer']).ok).toBe(false);
  });
  it('masks anything shaped like a credential in an error', () => {
    const out = redactError('401 for ghp_abcdefghijklmnop1234 via https://me:secret@api.github.com, Bearer abcdef123456');
    expect(out).not.toMatch(/ghp_abc|secret|abcdef123456/);
    expect(out).toContain('[redacted]');
    expect(redactError('x'.repeat(1000))).toHaveLength(300);
    expect(redactError(null)).toBe('');
  });
});

describe('grantable data classes', () => {
  it('merges the built-ins with classes loaded types declare, labelled by type and pack', async () => {
    const { grantableClasses } = await import('../src/lib/settings/admin');
    const types = [
      { id: 'em.private_note', title: 'Private note', origin: 'eng-manager', schema: { 'x-halp-data-class': 'notes.private' } },
      { id: 'cso.finding', title: 'Finding', origin: 'cso', schema: { 'x-halp-data-class': 'security.findings' } },
      { id: 'mine', title: 'Mine', origin: 'user', schema: { 'x-halp-data-class': 'security.findings' } },
      { id: 'plain', title: 'Plain', origin: 'developer', schema: {} },
    ];
    const out = grantableClasses(['general', 'notes.private'], types as never);
    expect(out.map((c) => [c.id, c.builtin])).toEqual([
      ['general', true],
      ['notes.private', true],
      ['security.findings', false],
    ]);
    expect(out[1]!.source).toContain('Private note (pack eng-manager)');
    expect(out[2]!.source).toBe('declared by Finding (pack cso), Mine (your type)');
  });
});
