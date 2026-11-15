import { beforeEach, describe, expect, it } from 'vitest';
import { CONSENT_DISPLAY_MAX_BYTES, MockTransport } from '../src/lib/api/mock/mock';
import type { Args, DayState, HistoryEntry, HubEvent, HubResponse, Item, Layout, PendingConfirmation } from '../src/lib/api/types';

const NOW = new Date('2026-11-10T09:30:00');
let clockNow = NOW;
let mock: MockTransport;
let events: HubEvent[];
let n = 0;

async function call<T = unknown>(command: string, args: Args = {}): Promise<HubResponse<T>> {
  return (await mock.send({ command, args, request_id: `r${n++}` })) as HubResponse<T>;
}
async function ok<T = unknown>(command: string, args: Args = {}): Promise<T> {
  const r = await call<T>(command, args);
  if (!r.ok) throw new Error(`${command}: ${r.error.code} ${r.error.message}`);
  return r.result;
}
const list = async (args: Args) => (await ok<{ items: Item[] }>('tasks.list', args)).items;

beforeEach(() => {
  clockNow = NOW;
  mock = new MockTransport({ latency: 0, now: () => clockNow, seedHistory: false });
  events = [];
  mock.subscribe((e) => events.push(e));
});

describe('MockTransport: tasks', () => {
  it('creates, gets, updates and completes a task, emitting events', async () => {
    const t = await ok<Item>('tasks.create', { title: 'Write tests', tags: ['QA'], parent_id: 'dev-1' });
    expect(t).toMatchObject({ title: 'Write tests', status: 'todo', kind: 'task', version: 1, tags: ['qa'], origin: 'user' });
    expect(t.inherited_tags).toEqual(['auth', 'hub']);
    expect(await ok<Item>('tasks.get', { id: t.id })).toMatchObject({ id: t.id });
    const u = await ok<Item>('tasks.update', { id: t.id, patch: { title: 'Write more tests' } });
    expect(u.title).toBe('Write more tests');
    expect(u.version).toBeGreaterThan(t.version);
    expect((await ok<Item>('tasks.complete', { id: t.id })).status).toBe('done');
    expect(events.map((e) => e.command)).toEqual(['tasks.create', 'tasks.update', 'tasks.complete']);
    expect(events.every((e) => e.actor === 'user' && e.entity.kind === 'item')).toBe(true);
  });

  it('a no-op update emits nothing', async () => {
    const cur = await ok<Item>('tasks.get', { id: 'dev-3' });
    await ok('tasks.update', { id: 'dev-3', patch: { title: cur.title } });
    expect(events).toHaveLength(0);
  });

  it('rejects unknown argument and patch keys (deny_unknown_fields)', async () => {
    expect(await call('tasks.get', { id: 'dev-1', extra: 1 })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.update', { id: 'dev-1', patch: { colour: 'red' } })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.create', { title: 42 })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.update', { id: 'dev-1', patch: {}, if_version: 99 })).toMatchObject({ ok: false, error: { code: 'conflict' } });
  });

  it('derives parent status from leaf subtasks and refuses a status patch on it', async () => {
    // dev-1a todo + dev-1b done -> in_progress
    expect((await ok<Item>('tasks.get', { id: 'dev-1' })).status).toBe('in_progress');
    await ok('tasks.complete', { id: 'dev-1a' });
    expect((await ok<Item>('tasks.get', { id: 'dev-1' })).status).toBe('done');
    expect(await call('tasks.update', { id: 'dev-1', patch: { status: 'todo' } })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
  });

  it('a group needs cascade to complete, and cascade sets the subtree', async () => {
    expect(await call('tasks.complete', { id: 'dev-1' })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    const done = await ok<Item>('tasks.complete', { id: 'dev-1', cascade: true });
    expect(done.status).toBe('done');
    expect((await list({ parent_id: 'dev-1' })).every((i) => i.status === 'done')).toBe(true);
    expect(events.at(-1)!.summary).toContain('and its subtasks');
  });

  it('bullets have no status', async () => {
    expect(await call('tasks.create', { title: 'Notes', kind: 'bullet', status: 'done' })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    const b = await ok<Item>('tasks.create', { title: 'Notes', kind: 'bullet' });
    expect(await call('tasks.update', { id: b.id, patch: { status: 'done' } })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
  });

  it('lists in outline pre-order and filters like the core', async () => {
    const all = await list({});
    const i1 = all.findIndex((i) => i.id === 'dev-1');
    expect(all.slice(i1, i1 + 3).map((i) => i.id)).toEqual(['dev-1', 'dev-1a', 'dev-1b']);

    const open = await list({ status: ['todo', 'in_progress'], role: 'developer', parent_id: null });
    expect(open.length).toBeGreaterThan(0);
    expect(open.every((i) => i.status !== 'done' && i.kind === 'task' && i.parent_id === null)).toBe(true);
    expect(open.some((i) => i.id.startsWith('cso-'))).toBe(false);

    // Tag filter matches inherited tags.
    const auth = await list({ tag: 'auth' });
    expect(auth.map((i) => i.id)).toEqual(expect.arrayContaining(['dev-1', 'dev-1a', 'dev-1b']));

    expect((await list({ parent_id: 'dev-1' })).map((i) => i.id)).toEqual(['dev-1a', 'dev-1b']);
    expect((await list({ text: 'PKCE' })).map((i) => i.id)).toContain('dev-1');
    expect(await call('tasks.list', { limit: 0 })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.list', { due_after: '2026-12-01', due_before: '2026-11-01' })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
  });

  it('tasks.move validates the parent and renumbers siblings', async () => {
    expect(await call('tasks.move', { id: 'dev-1', parent_id: 'dev-1a' })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.move', { id: 'dev-1', parent_id: 'missing' })).toMatchObject({ ok: false, error: { code: 'not_found' } });
    await ok('tasks.move', { id: 'dev-1b', index: 0 });
    const kids = await list({ parent_id: 'dev-1' });
    expect(kids.map((i) => [i.id, i.order])).toEqual([
      ['dev-1b', 0],
      ['dev-1a', 1],
    ]);
    await ok('tasks.move', { id: 'dev-3', parent_id: 'dev-1', order: 99 }); // alias of index, clamped
    expect((await list({ parent_id: 'dev-1' })).map((i) => [i.id, i.order])).toEqual([
      ['dev-1b', 0],
      ['dev-1a', 1],
      ['dev-3', 2],
    ]);
  });

  it('day.state counts and scopes to the role', async () => {
    const day = await ok<DayState>('day.state');
    expect(day.date).toBe('2026-11-10');
    expect(day.role).toBe('developer');
    expect(day.focus?.id).toBe('dev-1');
    expect(day.counts.overdue).toBeGreaterThan(0);
    expect(day.layout.role_id).toBe('developer');
    expect(day.agenda.every((i) => !i.id.startsWith('cso-') && i.status !== 'done')).toBe(true);
  });

  it('capture.quick files into an Inbox bullet with the current role', async () => {
    const r = await ok<{ item: Item; parsed: { due: string | null; title: string } }>('capture.quick', { text: 'Ping Ana #followup due:tomorrow' });
    expect(r.parsed).toEqual({ title: 'Ping Ana', tags: ['followup'], due: '2026-11-11', kind: 'task', status: 'todo' });
    expect(r.item).toMatchObject({ title: 'Ping Ana', tags: ['followup'], due: '2026-11-11', roles: ['developer'], origin: 'user' });
    const inbox = await ok<Item>('tasks.get', { id: r.item.parent_id! });
    expect(inbox).toMatchObject({ title: 'Inbox', kind: 'bullet', tags: ['inbox'], parent_id: null, order: 0 });
    // Second capture reuses it; top puts it first.
    const r2 = await ok<{ item: Item }>('capture.quick', { text: 'Second', top: true });
    expect(r2.item.parent_id).toBe(inbox.id);
    expect((await list({ parent_id: inbox.id })).map((i) => i.title)).toEqual(['Second', 'Ping Ana']);
    // An unknown due phrase stays in the title, like the core.
    expect((await ok<{ title: string; due: null }>('capture.parse', { text: 'Call Bo due:tom' })).title).toBe('Call Bo due:tom');
    expect(await call('capture.parse', { text: '  #only ' })).toMatchObject({ ok: false, error: { code: 'invalid_args', message: 'nothing to capture' } });
  });
});

describe('MockTransport: consent', () => {
  it('tasks.delete needs confirmation, even from the user, with a described request', async () => {
    const r = await call('tasks.delete', { id: 'dev-1' });
    expect(r).toMatchObject({ ok: false, error: { code: 'confirmation_required' } });
    const id = !r.ok ? r.error.confirmation_id! : '';
    expect(id).toMatch(/^[0-9a-f]{64}$/);
    expect(events.at(-1)).toMatchObject({ command: 'consent.requested', entity: { kind: 'confirmation', id } });
    const p = await ok<PendingConfirmation>('consent.get', { id });
    expect(p).toMatchObject({ command: 'tasks.delete', tier: 'T3', actor: 'user', args: { id: 'dev-1' }, tainted: false, taint_source: null });
    expect(p.summary).toBe('Delete "Ship OAuth PKCE loopback flow" and 2 nested items');
    expect(p.details?.targets).toEqual([{ kind: 'item', id: 'dev-1', title: 'Ship OAuth PKCE loopback flow', descendants: 2 }]);
    expect(p.args_hash).toMatch(/^[0-9a-f]{64}$/);
    expect(Date.parse(p.expires_at!) - Date.parse(p.created_at)).toBe(5 * 60_000);
    expect((await ok<{ pending: PendingConfirmation[] }>('consent.list')).pending).toHaveLength(1);
    expect(await call('tasks.get', { id: 'dev-1' })).toMatchObject({ ok: true }); // nothing deleted yet
  });

  it('approve runs the command; deny does not', async () => {
    const a = await call('tasks.delete', { id: 'dev-3' });
    await ok('consent.approve', { id: !a.ok ? a.error.confirmation_id : '' });
    expect(await call('tasks.get', { id: 'dev-3' })).toMatchObject({ ok: false, error: { code: 'not_found' } });

    const d = await call('tasks.delete', { id: 'dev-2' });
    expect(await ok('consent.deny', { confirmation_id: !d.ok ? d.error.confirmation_id : '' })).toEqual({ denied: !d.ok ? d.error.confirmation_id : '' });
    expect(await call('tasks.get', { id: 'dev-2' })).toMatchObject({ ok: true });
    expect((await ok<{ pending: unknown[] }>('consent.list')).pending).toHaveLength(0);
    expect(events.map((e) => e.command)).toEqual(['consent.requested', 'consent.approve', 'tasks.delete', 'consent.requested', 'consent.deny']);
  });

  it('an agent cannot approve its own request, and its call resolves after the user decides', async () => {
    const agent = mock.agent('claude');
    const done = agent.dispatch('tasks.delete', { id: 'dev-3' });
    const req = events.find((e) => e.command === 'consent.requested')!;
    expect(req.actor).toBe('agent:claude');

    expect(await agent.dispatch('consent.approve', { id: req.entity.id })).toMatchObject({ ok: false, error: { code: 'denied' } });

    await ok('consent.approve', { id: req.entity.id });
    await expect(done).resolves.toMatchObject({ ok: true });
    expect(events.find((e) => e.command === 'tasks.delete')!.actor).toBe('agent:claude'); // runs as the requester
  });

  it('marks requests from a tainted agent', async () => {
    void mock.agent('claude', { taintSource: 'https://example.com/issue/7' }).dispatch('tasks.delete', { id: 'dev-3' });
    const [p] = (await ok<{ pending: PendingConfirmation[] }>('consent.list')).pending;
    expect(p).toMatchObject({ tainted: true, taint_source: 'https://example.com/issue/7', actor: 'agent:claude' });
  });

  it('denied and expired agent calls resolve with denied', async () => {
    const done = mock.agent('claude').dispatch('tasks.delete', { id: 'dev-3' });
    await ok('consent.deny', { id: events.find((e) => e.command === 'consent.requested')!.entity.id });
    await expect(done).resolves.toMatchObject({ ok: false, error: { code: 'denied' } });

    const late = mock.agent('claude').dispatch('tasks.delete', { id: 'dev-2' });
    clockNow = new Date(NOW.getTime() + 5 * 60_000 + 1);
    expect((await ok<{ pending: unknown[] }>('consent.list')).pending).toHaveLength(0);
    await expect(late).resolves.toMatchObject({ ok: false, error: { code: 'denied' } });
    // Like the core: expiry is its own event, not a user denial.
    expect(events.at(-1)).toMatchObject({ command: 'consent.expired', actor: 'system', entity: { kind: 'confirmation' } });
  });

  it('flags args too large to display in full', async () => {
    void mock.agent('claude').dispatch('tasks.delete', { id: 'dev-2' });
    // No mock T3 command takes large args, so park one directly.
    const park = (mock as unknown as { park: (c: string, d: { tier: string }, a: object, actor: string) => unknown }).park.bind(mock);
    park('tasks.delete', { tier: 'T3' }, { id: 'dev-3', pad: 'x'.repeat(CONSENT_DISPLAY_MAX_BYTES) }, 'agent:claude');
    const pending = (await ok<{ pending: PendingConfirmation[] }>('consent.list')).pending;
    expect(pending.map((p) => p.display_truncated)).toEqual([false, true]);
  });

  it('unknown confirmation ids are not_found', async () => {
    expect(await call('consent.approve', { id: 'nope' })).toMatchObject({ ok: false, error: { code: 'not_found' } });
  });
});

describe('MockTransport: history', () => {
  it('undo and redo the latest change; undone events stay listed', async () => {
    const t = await ok<Item>('tasks.create', { title: 'Temp' });
    const u = await ok<{ event: HubEvent | null; history_event: HubEvent | null }>('history.undo');
    expect(u.event?.command).toBe('tasks.create');
    expect(u.history_event?.command).toBe('history.undo');
    expect(await call('tasks.get', { id: t.id })).toMatchObject({ ok: false });

    const h = await ok<{ events: HistoryEntry[]; can_undo: boolean; can_redo: boolean }>('history.list');
    expect(h.events.map((e) => e.command)).toEqual(['history.undo', 'tasks.create']);
    expect(h.events[0]!.causation_id).toBe(u.event!.id);
    expect(h.events[1]!.undoable).toBe(false);
    expect(h.can_redo).toBe(true);

    await ok('history.redo');
    expect(await call('tasks.get', { id: t.id })).toMatchObject({ ok: true });
  });

  it('nothing to undo returns nulls', async () => {
    expect(await ok('history.undo')).toEqual({ event: null, history_event: null });
  });

  it('undo a specific (agent) event by id without touching later changes', async () => {
    await mock.agent('claude').dispatch('tasks.complete', { id: 'dev-2' });
    await ok('tasks.update', { id: 'dev-3', patch: { title: 'Mine' } });
    const agentEvent = events.find((e) => e.actor === 'agent:claude')!;
    await ok('history.undo', { event_id: agentEvent.id });
    expect((await ok<Item>('tasks.get', { id: 'dev-2' })).status).not.toBe('done');
    expect((await ok<Item>('tasks.get', { id: 'dev-3' })).title).toBe('Mine');
  });

  it('role.switch and focus are logged but not undoable, and only when they change', async () => {
    await ok('focus.set', { id: 'dev-1' }); // already focused: no event
    expect(events).toHaveLength(0);
    await ok('focus.set', { id: 'dev-2' });
    await ok('role.switch', { id: 'cso' });
    await ok('role.switch', { id: 'cso' });
    expect(events.map((e) => e.command)).toEqual(['focus.set', 'role.switch']);
    const h = await ok<{ events: HistoryEntry[]; can_undo: boolean }>('history.list');
    expect(h.events.every((e) => !e.undoable)).toBe(true);
    expect(h.can_undo).toBe(false);
  });

  it('history.list filters by actor prefix', async () => {
    await mock.agent('claude').dispatch('focus.clear', {});
    await ok('focus.set', { id: 'dev-2' });
    const agents = await ok<{ events: HubEvent[] }>('history.list', { actor_prefix: 'agent:' });
    expect(agents.events.length).toBe(1);
    expect(agents.events.every((e) => e.actor.startsWith('agent:'))).toBe(true);
  });

  it('seeds agent history when asked', async () => {
    const seeded = new MockTransport({ latency: 0, now: () => NOW });
    const r = (await seeded.send({ command: 'history.list', args: { actor_prefix: 'agent:' }, request_id: 'x' })) as HubResponse<{ events: HubEvent[] }>;
    expect(r.ok && r.result.events.length).toBe(3);
  });
});

describe('MockTransport: layout and roles', () => {
  it('move_widget applies grid rules and emits a layout event', async () => {
    const before = await ok<Layout>('layout.get');
    const w = before.widgets[1]!;
    const after = await ok<Layout>('layout.move_widget', { widget_id: w.id, x: 0, y: 0 });
    expect(after.widgets.find((x) => x.id === w.id)).toMatchObject({ x: 0, y: 0 });
    expect(events.at(-1)).toMatchObject({ command: 'layout.move_widget', entity: { kind: 'widget', id: w.id } });
    expect(await call('layout.move_widget', { widget_id: w.id, x: 12, y: 0 })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('layout.move_widget', { widget_id: w.id, x: 0, y: 201 })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
  });

  it('add, update and remove widgets', async () => {
    const role = (await ok<{ roles: { id: string; widgets: string[] }[] }>('role.list')).roles.find((r) => r.id === 'developer')!;
    expect(await call('layout.add_widget', { kind: 'no-such-widget' })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    const kind = role.widgets[0]!;
    const added = await ok<Layout>('layout.add_widget', { kind, title: 'Launch', w: 3, h: 2 });
    const nw = added.widgets.find((x) => x.title === 'Launch')!;
    expect(nw.id).toMatch(/^w-[0-9a-z]{10}$/);
    expect(await call('layout.add_widget', { kind, h: 25 })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    const upd = await ok<Layout>('layout.update_widget', { widget_id: nw.id, patch: { w: 6 } });
    expect(upd.widgets.find((x) => x.id === nw.id)!.w).toBe(6);
    const rm = await ok<Layout>('layout.remove_widget', { widget_id: nw.id });
    expect(rm.widgets.some((x) => x.id === nw.id)).toBe(false);
  });

  it('role.switch changes the layout and day; role_id is accepted as an alias', async () => {
    const r = await ok<{ role: { id: string }; layout: Layout }>('role.switch', { id: 'cso' });
    expect(r.role.id).toBe('cso');
    expect(r.layout.role_id).toBe('cso');
    expect((await ok<DayState>('day.state')).role).toBe('cso');
    expect((await ok<{ current: string }>('role.list')).current).toBe('cso');
    await ok('role.switch', { role_id: 'developer' });
    expect(await call('role.switch', { id: 'astronaut' })).toMatchObject({ ok: false, error: { code: 'not_found' } });
    expect(await call('role.switch', { id: 'cso', role_id: 'cso' })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
  });

  it('lists all roles and commands with tiers', async () => {
    const roles = await ok<{ roles: { id: string }[] }>('role.list');
    expect(roles.roles.map((r) => r.id)).toEqual(['developer', 'eng-manager', 'cso']);
    const cmds = await ok<{ commands: { name: string; tier: string }[] }>('commands.list');
    const names = cmds.commands.map((c) => c.name);
    for (const c of ['tasks.create', 'tasks.list', 'history.undo', 'layout.move_widget', 'role.switch', 'focus.set', 'day.state', 'capture.quick', 'capture.parse', 'consent.approve']) expect(names).toContain(c);
    expect(cmds.commands.find((c) => c.name === 'tasks.delete')!.tier).toBe('T3');
  });
});

describe('MockTransport: consent v1.4 and round 2', () => {
  it('re-asks under a new id when the scope changed, emitting superseded then requested', async () => {
    const agent = mock.agent('claude');
    const waiting = agent.dispatch('tasks.delete', { id: 'dev-3' });
    const [first] = (await ok<{ pending: PendingConfirmation[] }>('consent.list')).pending;
    await ok('tasks.update', { id: 'dev-3', patch: { title: 'Renamed meanwhile' } });
    events = [];
    const r = await call('consent.approve', { id: first!.id });
    expect(r).toMatchObject({ ok: false, error: { code: 'conflict' } });
    const next = (r as { error: { confirmation_id: string } }).error.confirmation_id;
    expect(next).not.toBe(first!.id);
    expect(events.map((e) => [e.command, e.entity.id])).toEqual([
      ['consent.superseded', first!.id],
      ['consent.requested', next],
    ]);
    await ok('consent.approve', { id: next });
    expect((await waiting).ok).toBe(true);
    const audit = (await ok<{ records: { decision: string }[] }>('audit.list', {})).records.map((x) => x.decision);
    expect(audit).toContain('superseded');
  });

  it('keeps consent.list under its budget; consent.get still returns the entry in full', async () => {
    await ok('grants.set', { client_id: 'claude', t1: false });
    const agent = mock.agent('claude');
    const notes = 'n'.repeat(60 * 1024);
    for (let i = 0; i < 20; i++) void agent.dispatch('tasks.create', { title: `big ${i}`, notes });
    const pending = (await ok<{ pending: PendingConfirmation[] }>('consent.list')).pending;
    expect(pending).toHaveLength(20);
    const cut = pending.filter((x) => x.display_truncated);
    expect(cut.length).toBeGreaterThan(0);
    expect(pending.filter((x) => !x.display_truncated).every((x) => x.args!.notes === notes)).toBe(true);
    const full = await ok<PendingConfirmation>('consent.get', { id: cut[0]!.id });
    expect(full.display_truncated).toBe(false);
    expect(full.args!.notes).toBe(notes);
  });

  it('pins a parked capture.quick to the Inbox', async () => {
    await ok('capture.quick', { text: 'Make the Inbox exist' });
    const agent = mock.agent('claude');
    // A T1 call parks for an agent without the T1 grant.
    await ok('grants.set', { client_id: 'claude', t1: false });
    void agent.dispatch('capture.quick', { text: 'Pay rent due:fri' });
    const [p] = (await ok<{ pending: PendingConfirmation[] }>('consent.list')).pending;
    expect(p!.args!.parent_id).toEqual(expect.any(String));
  });
});

describe('integrations.connect roles', () => {
  it('stores and echoes the roles it was given, and defaults to developer', async () => {
    const e = await ok<{ config: { roles: string[] } }>('integrations.connect', { id: 'cal', kind: 'calendar', roles: ['eng-manager', 'developer'], settings: { source: 'ics' } });
    expect(e.config.roles).toEqual(['eng-manager', 'developer']);
    const list = await ok<{ integrations: { config: { id: string; roles: string[] } }[] }>('integrations.list');
    expect(list.integrations.find((i) => i.config.id === 'cal')!.config.roles).toEqual(['eng-manager', 'developer']);
    const d = await ok<{ config: { roles: string[] } }>('integrations.connect', { id: 'cal2', kind: 'calendar', settings: { source: 'ics' } });
    expect(d.config.roles).toEqual(['developer']);
  });
});
