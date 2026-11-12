// The Settings panel's store against the mock core: pairing (the token is
// shown once and not kept), grants, taint, rules, audit, data (T3 through
// consent, with the file content and passphrase redacted on display), and
// connections (GitHub PAT flow, token errors, archived on remote removal).

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';
import { rulableCommands } from '../src/lib/settings/admin';
import { HubClient } from '../src/lib/api/client';
import { MockTransport } from '../src/lib/api/mock/mock';
import type { Item, PendingConfirmation } from '../src/lib/api/types';
import { hub } from '../src/lib/stores/hub.svelte';
import { settings } from '../src/lib/stores/settings.svelte';
import { toasts } from '../src/lib/design/toast.svelte';
import { TOKEN_ERROR_TEXT } from '../src/lib/api/shell';

let mock: MockTransport;
let asked: PendingConfirmation[];
let approve: boolean;
const g = globalThis as { __halpMock?: unknown };

beforeEach(async () => {
  mock = new MockTransport({ latency: 0, seedHistory: false });
  const client = new HubClient(mock);
  asked = [];
  approve = true;
  client.setConsentHandler(async (p) => {
    asked.push(p);
    return approve;
  });
  (hub as unknown as { client: unknown }).client = client;
  hub.commands = (await client.dispatch('commands.list', {})).commands;
  toasts.items = [];
  settings.reset();
  settings.clients = [];
  settings.integrations = [];
  g.__halpMock = mock;
});
afterEach(() => {
  delete g.__halpMock;
  vi.restoreAllMocks();
});

const allItems = async () => (await hub.dispatch('tasks.list', { limit: 1000 })).items as Item[];

describe('agents', () => {
  it('shows the pairing token once and keeps it nowhere after the dialog closes', async () => {
    const spies = (['log', 'info', 'warn', 'error', 'debug'] as const).map((m) => vi.spyOn(console, m).mockImplementation(() => {}));
    expect(await settings.pair('Review bot')).toBe(true);
    const token = settings.pairing!.token;
    expect(token).toMatch(/^halp_[0-9a-f]+$/);
    expect(settings.pairing!.client.id).toBe('review-bot');
    expect(settings.clients.map((c) => c.id)).toContain('review-bot');
    settings.closePairing();
    expect(settings.pairing).toBeNull();
    const everything = JSON.stringify(Object.fromEntries(Object.entries(settings)));
    expect(everything).not.toContain(token);
    expect(JSON.stringify(toasts.items)).not.toContain(token);
    for (const s of spies) for (const call of s.mock.calls) expect(JSON.stringify(call)).not.toContain(token);
    // The core keeps only a hash: no command returns the token again.
    expect(JSON.stringify(await hub.dispatch('clients.list', {}))).not.toContain(token);
  });

  it('closing the panel drops the token too', async () => {
    await settings.pair('Other');
    settings.reset();
    expect(settings.pairing).toBeNull();
  });

  it('saves only changed grants', async () => {
    await settings.loadAgents();
    const c = settings.clients.find((x) => x.id === 'claude')!;
    expect(await settings.saveGrants(c, { t1: true, data_classes: ['general', 'audit'] })).toBe(true);
    expect(settings.clients.find((x) => x.id === 'claude')!.grants).toEqual({ t1: true, data_classes: ['audit', 'general'] });
  });

  it('reads last seen and taint from clients.list, and trusts again with policy.clear_taint', async () => {
    const agent = mock.agent('claude', { taintSource: 'email' });
    await agent.dispatch('tasks.get', { id: 'dev-1' });
    await settings.loadAgents();
    expect(settings.activity.claude).toMatchObject({ tainted: true });
    expect(settings.activity.claude!.lastSeen).toBeTruthy();
    expect(settings.activity.claude!.taintedUntil).toBeTruthy();
    await settings.clearTaint(settings.clients.find((x) => x.id === 'claude')!);
    expect(settings.activity.claude!.tainted).toBe(false);
    expect(toasts.items.at(-1)?.message).toMatch(/trusted again.*email/);
  });

  it('revokes only after the in-app confirm', async () => {
    await settings.loadAgents();
    const c = settings.clients.find((x) => x.id === 'claude')!;
    const confirm = vi.spyOn(hub, 'confirm').mockResolvedValueOnce(false);
    await settings.revoke(c);
    expect(settings.clients.find((x) => x.id === 'claude')!.revoked_at).toBeFalsy();
    confirm.mockResolvedValueOnce(true);
    await settings.revoke(c);
    expect(settings.clients.find((x) => x.id === 'claude')!.revoked_at).toBeTruthy();
    expect((await mock.agent('claude').dispatch('tasks.get', { id: 'dev-1' })).ok).toBe(false);
  });
});

describe('standing rules', () => {
  it('the core offers no agent-callable T2 command, and refuses rules for user-only ones', async () => {
    expect(hub.commands.filter((c) => c.tier === 'T2' && !c.user_only)).toEqual([]);
    expect(await settings.createRule({ client_id: 'claude', command: 'grants.set', args: { client_id: 'claude' } })).toBe(false);
    expect(settings.rules).toEqual([]);
  });

  it('shows an empty state instead of a form with no commands', () => {
    expect(rulableCommands(hub.commands)).toEqual([]);
    const src = readFileSync(new URL('../src/lib/settings/RulesSection.svelte', import.meta.url), 'utf8');
    const empty = src.slice(src.indexOf('{#if !commands.length}'), src.indexOf('{:else}'));
    expect(empty).toMatch(/T2 commands that agents can call/);
    expect(empty).toMatch(/none yet/);
    expect(empty).toMatch(/Packs and extensions may add/);
  });
});

describe('audit', () => {
  it('pages with before and filters by actor in the core, decision here', async () => {
    const agent = mock.agent('claude');
    for (let i = 0; i < 60; i++) await agent.dispatch('tasks.get', { id: 'dev-1' });
    settings.auditActor = 'agent:claude';
    await settings.loadAudit();
    expect(settings.audit).toHaveLength(50);
    expect(settings.auditMore).toBe(true);
    await settings.loadAudit(true);
    expect(settings.audit).toHaveLength(60);
    expect(new Set(settings.audit.map((r) => r.id)).size).toBe(60);
    settings.auditDecision = 'deny';
    expect(settings.shownAudit).toEqual([]);
    settings.auditDecision = '';
    settings.auditActor = '';
  });
});

describe('data (T3, through consent)', () => {
  it('purges history only after approval', async () => {
    approve = false;
    expect(await settings.purge({ older_than_days: 30 })).toBeNull();
    expect(asked.map((p) => p.command)).toEqual(['history.purge']);
    approve = true;
    expect(await settings.purge({ older_than_days: 30 })).toMatchObject({ redacted: expect.any(Number) });
  });

  it('imports a TUI export; the consent dialog sees neither the content nor the passphrase', async () => {
    const text = '- [ ] First imported\n- [ ] Second imported\n';
    const file = new File([text], 'tasks.md', { type: 'text/markdown' });
    const report = await settings.importFile(file, '');
    expect(report!.items).toBeGreaterThanOrEqual(2);
    const shown = asked.at(-1)!;
    expect(shown.command).toBe('import.tui');
    expect(shown.args!.content_base64).toMatch(/^<\d+ bytes>$/);
    expect(JSON.stringify(shown)).not.toContain(btoa(text));
    await settings.importFile(new File(['x'], 'backup.halp'), 'correct horse');
    expect(asked.at(-1)!.args!.passphrase).toBe('[redacted]');
    expect(JSON.stringify(asked)).not.toContain('correct horse');
  });

  it('refuses files over the size limit before reading them', async () => {
    const big = { name: 'big.md', size: 2 * 1024 * 1024, arrayBuffer: vi.fn() } as unknown as File;
    expect(await settings.importFile(big, '')).toBeNull();
    expect(big.arrayBuffer).not.toHaveBeenCalled();
    expect(asked).toEqual([]);
  });
});

describe('connections', () => {
  it('connects GitHub, needs a token, syncs, and archives what was removed remotely', async () => {
    expect(await settings.connectGithub({ id: 'github', kind: 'github', settings: { repos: ['octo/api'], auth: 'token' } })).toBe(true);
    expect(settings.integrations[0]!.status).toMatchObject({ state: 'needs_auth', has_secret: false });
    expect(await settings.setToken('github', '')).toBe(TOKEN_ERROR_TEXT.empty);
    expect(await settings.setToken('github', 'ghp_bäd')).toBe(TOKEN_ERROR_TEXT.not_printable_ascii);
    expect(await settings.setToken('nope', 'ghp_x')).toBe(TOKEN_ERROR_TEXT.unknown_integration);
    expect(await settings.setToken('github', 'ghp_abcdefghijklmnop')).toBeNull();
    expect(settings.integrations[0]!.status).toMatchObject({ state: 'ok', has_secret: true, items: 3 });
    expect(JSON.stringify(settings.integrations)).not.toContain('ghp_abcdefghijklmnop');
    const synced = async () => (await allItems()).filter((i) => i.origin === 'integration:github' && i.type_id === 'dev.pull_request');
    expect((await synced()).filter((i) => i.tags.includes('archived'))).toEqual([]);
    await settings.syncNow('github');
    const archived = (await synced()).filter((i) => i.tags.includes('archived'));
    expect(archived).toHaveLength(1);
    expect(archived[0]!.status).toBe('done');
  });

  it('takes a calendar feed URL, refuses non-URLs and shows only a hint', async () => {
    await hub.dispatch('integrations.connect', { id: 'cal', kind: 'calendar', settings: { source: 'ics', me: ['me@acme.com'] } });
    await settings.loadIntegrations();
    expect(await settings.setToken('cal', 'not a url')).toBe(TOKEN_ERROR_TEXT.invalid_url);
    expect(await settings.setToken('cal', 'http://cal.example/feeds/SECRET1234/basic.ics')).toBe(TOKEN_ERROR_TEXT.invalid_url);
    expect(settings.secretHints.cal ?? null).toBeNull();
    expect(await settings.setToken('cal', 'webcal://cal.example/feeds/SECRETabcd/basic.ics')).toBeNull();
    expect(settings.secretHints.cal).toBe('cal.example …abcd');
    expect(JSON.stringify(settings.secretHints)).not.toContain('SECRET');
    expect(JSON.stringify(settings.integrations)).not.toContain('SECRET');
  });

  it('reports not_running when no shell takes the token', async () => {
    delete g.__halpMock;
    expect(await settings.setToken('github', 'ghp_abcdefghijklmnop')).toBe(TOKEN_ERROR_TEXT.not_running);
  });

  it('disconnects through consent, optionally deleting synced items', async () => {
    await settings.connectGithub({ id: 'github', kind: 'github', settings: { repos: ['octo/api'], auth: 'token' } });
    await settings.setToken('github', 'ghp_abcdefghijklmnop');
    approve = false;
    await settings.disconnect('github', true);
    expect(asked.at(-1)!.command).toBe('integrations.disconnect');
    expect(settings.integrations).toHaveLength(1);
    approve = true;
    await settings.disconnect('github', true);
    expect(settings.integrations).toEqual([]);
    expect((await allItems()).filter((i) => i.origin === 'integration:github')).toEqual([]);
  });
});

describe('grants for type-declared data classes', () => {
  it('offers and accepts a class a loaded type declares', async () => {
    await hub.dispatch('types.define', {
      id: 'x.secret_doc',
      title: 'Secret doc',
      icon: 'lock',
      schema: { $schema: 'https://json-schema.org/draft/2020-12/schema', 'x-halp-data-class': 'legal.privileged', type: 'object', properties: {}, additionalProperties: false },
      display: { primary_fields: [] },
    } as never);
    await settings.loadAgents();
    expect(settings.dataClasses.find((d) => d.id === 'legal.privileged')).toMatchObject({ builtin: false });
    const c = settings.clients.find((x) => x.id === 'claude')!;
    expect(await settings.saveGrants(c, { t1: c.grants.t1, data_classes: ['general', 'legal.privileged'] })).toBe(true);
    expect(settings.clients.find((x) => x.id === 'claude')!.grants.data_classes).toEqual(['general', 'legal.privileged']);
  });
});
