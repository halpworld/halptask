// The hub store's consent queue: settled requests leave the queue (expired
// included), a request too large to show cannot be approved, and an approval
// that hits `conflict` says so and re-reads the queue.

import { beforeEach, describe, expect, it, vi } from 'vitest';
import { hub, SUPERSEDED_WAIT_MS } from '../src/lib/stores/hub.svelte';
import { toasts } from '../src/lib/design/toast.svelte';
import type { Args, HubEvent, HubResponse, PendingConfirmation } from '../src/lib/api/types';
import type { ConsentOutcome } from '../src/lib/api/client';

const p = (id: string, extra: Partial<PendingConfirmation> = {}): PendingConfirmation => ({
  id,
  command: 'tasks.delete',
  tier: 'T3',
  actor: 'agent:claude',
  args: { id: 'dev-1' },
  summary: 'Delete "x"',
  created_at: '2026-11-10T09:00:00.000Z',
  ...extra,
});
const ev = (command: string, id: string): HubEvent =>
  ({ id: `e-${command}-${id}`, ts: '2026-11-10T09:01:00.000Z', actor: 'system', command, entity: { kind: 'confirmation', id }, summary: '' });

let calls: { command: string; args: Args }[] = [];
let replies: Record<string, HubResponse> = {};
beforeEach(() => {
  calls = [];
  replies = {};
  toasts.items = [];
  hub.consent = [{ pending: p('a') }, { pending: p('b') }];
  (hub as unknown as { client: unknown }).client = {
    dispatchRaw: async (command: string, args: Args) => {
      calls.push({ command, args });
      return replies[command] ?? { ok: true, result: {} };
    },
  };
});

describe('hub consent queue', () => {
  it.each(['consent.approve', 'consent.deny', 'consent.expired'])('drops a request on %s', (c) => {
    hub.handleEvent(ev(c, 'a'));
    expect(hub.consent.map((x) => x.pending.id)).toEqual(['b']);
  });

  it('tells the user when a request expired, and fails our own waiting call as denied', () => {
    let answer: ConsentOutcome | undefined;
    hub.consent = [{ pending: p('a'), resolve: (o) => (answer = o) }];
    hub.handleEvent(ev('consent.expired', 'a'));
    expect(hub.consent).toEqual([]);
    expect(answer?.kind).toBe('denied');
    expect(toasts.items.at(-1)?.message).toMatch(/expired/);
  });

  it('never approves a request too large to show', async () => {
    hub.consent = [{ pending: p('a', { display_truncated: true }) }];
    await hub.decideConsent('a', true);
    expect(calls).toEqual([]);
    expect(hub.consent).toHaveLength(1);
    await hub.decideConsent('a', false);
    expect(calls).toEqual([{ command: 'consent.deny', args: { id: 'a' } }]);
  });

  it('on conflict says the request changed and re-reads the queue', async () => {
    replies['consent.approve'] = { ok: false, error: { code: 'conflict', message: 'args changed' } };
    replies['consent.list'] = { ok: true, result: { pending: [p('a', { summary: 'Delete "y"' }), p('b')] } };
    await hub.decideConsent('a', true);
    expect(calls.map((c) => c.command)).toEqual(['consent.approve', 'consent.list']);
    expect(toasts.items.at(-1)?.message).toMatch(/changed since you saw it/);
    expect(hub.consent.map((x) => [x.pending.id, x.pending.summary])).toEqual([
      ['a', 'Delete "y"'],
      ['b', 'Delete "x"'],
    ]);
  });
});

describe('hub consent queue, v1.4', () => {
  it('a conflict re-ask with a new id stays in the dialog, marked changed, keeping our waiting call', async () => {
    let answer: ConsentOutcome | undefined;
    hub.consent = [{ pending: p('a'), resolve: (o) => (answer = o) }, { pending: p('b') }];
    replies['consent.approve'] = { ok: false, error: { code: 'conflict', message: 'changed', confirmation_id: 'a2' } };
    replies['consent.get'] = { ok: true, result: p('a2', { summary: 'Delete "x" (changed)' }) };
    await hub.decideConsent('a', true);
    expect(calls.map((c) => c.command)).toEqual(['consent.approve', 'consent.get']);
    expect(hub.consent.map((c) => [c.pending.id, !!c.changed])).toEqual([
      ['a2', true],
      ['b', false],
    ]);
    expect(answer).toBeUndefined();
    expect(hub.consent[0]!.resolve).toBeTypeOf('function');
  });

  it('fetches truncated entries with consent.get, at most 4 at a time', async () => {
    let inFlight = 0;
    let peak = 0;
    (hub as unknown as { client: unknown }).client = {
      dispatchRaw: async (command: string, args: Args) => {
        calls.push({ command, args });
        inFlight++;
        peak = Math.max(peak, inFlight);
        await new Promise((r) => setTimeout(r, 2));
        inFlight--;
        return { ok: true, result: p(String(args.id), { summary: 'full' }) };
      },
    };
    const list = Array.from({ length: 10 }, (_, i) => p(`t${i}`, { display_truncated: i % 5 !== 0 }));
    const out = await hub.completePending(list);
    expect(calls.filter((c) => c.command === 'consent.get')).toHaveLength(8);
    expect(peak).toBeLessThanOrEqual(4);
    expect(out.map((x) => x.id)).toEqual(list.map((x) => x.id));
    expect(out.filter((x) => x.summary === 'full')).toHaveLength(8);
  });

  it('follows our request when it is superseded elsewhere and asked again under a new id', async () => {
    let answer: ConsentOutcome | undefined;
    hub.consent = [{ pending: p('a', { args_hash: 'h1' }), resolve: (o) => (answer = o) }];
    hub.handleEvent(ev('consent.superseded', 'a'));
    expect(hub.consent).toEqual([]);
    replies['consent.get'] = { ok: true, result: p('a2', { args_hash: 'h1' }) };
    hub.handleEvent(ev('consent.requested', 'a2'));
    await new Promise((r) => setTimeout(r, 0));
    expect(hub.consent.map((c) => [c.pending.id, !!c.changed, !!c.resolve])).toEqual([['a2', true, true]]);
    expect(answer).toBeUndefined();
  });

  it('fails the waiting call with conflict when a superseded request is never asked again', async () => {
    vi.useFakeTimers();
    try {
      let answer: ConsentOutcome | undefined;
      hub.consent = [{ pending: p('a', { args_hash: 'h9' }), resolve: (o) => (answer = o) }];
      hub.handleEvent(ev('consent.superseded', 'a'));
      vi.advanceTimersByTime(SUPERSEDED_WAIT_MS + 1);
      expect(answer).toMatchObject({ kind: 'error', code: 'conflict' });
    } finally {
      vi.useRealTimers();
    }
  });

  it('settles our waiting call as denied when it is denied elsewhere', () => {
    let answer: ConsentOutcome | undefined;
    hub.consent = [{ pending: p('a'), resolve: (o) => (answer = o) }];
    hub.handleEvent(ev('consent.deny', 'a'));
    expect(answer?.kind).toBe('denied');
  });

  it('ignores events about its own approval in flight', () => {
    let answer: ConsentOutcome | undefined;
    hub.consent = [{ pending: p('a'), resolve: (o) => (answer = o), busy: true }];
    hub.handleEvent(ev('consent.approve', 'a'));
    expect(hub.consent).toHaveLength(1);
    expect(answer).toBeUndefined();
  });
});
