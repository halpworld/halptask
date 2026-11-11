import { describe, expect, it, vi } from 'vitest';
import { HubApiError, HubClient } from '../src/lib/api/client';
import type { Transport } from '../src/lib/api/transport';
import type { HubEvent, HubRequest, HubResponse } from '../src/lib/api/types';

class FakeTransport implements Transport {
  readonly kind = 'mock' as const;
  sent: HubRequest[] = [];
  listeners = new Set<(e: HubEvent) => void>();
  constructor(private handler: (r: HubRequest) => HubResponse) {}
  async send(r: HubRequest): Promise<HubResponse> {
    this.sent.push(r);
    return this.handler(r);
  }
  subscribe(l: (e: HubEvent) => void) {
    this.listeners.add(l);
    return () => this.listeners.delete(l);
  }
}

const pending = { id: 'c1', command: 'tasks.delete', tier: 'T3' as const, actor: 'user' as const, args: { id: 't1' }, summary: 'delete “x”', created_at: '2026-11-10T00:00:00Z' };

function t3Transport() {
  return new FakeTransport((r) => {
    if (r.command === 'tasks.delete') return { ok: false, error: { code: 'confirmation_required', message: 'needs approval', confirmation_id: 'c1' } };
    if (r.command === 'consent.get') return { ok: true, result: pending };
    if (r.command === 'consent.approve') return { ok: true, result: { deleted: ['t1'] } };
    if (r.command === 'consent.deny') return { ok: true, result: { denied: 'c1' } };
    return { ok: false, error: { code: 'not_found', message: 'nope' } };
  });
}

describe('HubClient', () => {
  it('sends the contract envelope with a unique request_id', async () => {
    const t = new FakeTransport(() => ({ ok: true, result: { roles: [] } }));
    const c = new HubClient(t);
    await c.dispatch('role.list', {});
    await c.dispatch('role.list', {});
    expect(t.sent[0]).toMatchObject({ command: 'role.list', args: {} });
    expect(t.sent[0]!.request_id).toMatch(/^[0-9A-HJKMNP-TV-Z]{26}$/);
    expect(t.sent[0]!.request_id).not.toBe(t.sent[1]!.request_id);
  });

  it('unwraps results and throws HubApiError on failure', async () => {
    const c = new HubClient(new FakeTransport(() => ({ ok: false, error: { code: 'not_found', message: 'No task x' } })));
    const err = await c.dispatch('tasks.get', { id: 'x' }).catch((e: unknown) => e);
    expect(err).toBeInstanceOf(HubApiError);
    expect(err).toMatchObject({ code: 'not_found', command: 'tasks.get', message: 'No task x' });
  });

  it('without a consent handler, confirmation_required surfaces with its id', async () => {
    const c = new HubClient(t3Transport());
    await expect(c.dispatch('tasks.delete', { id: 't1' })).rejects.toMatchObject({ code: 'confirmation_required', confirmationId: 'c1' });
  });

  it('approve: asks the handler with consent.get details, then calls consent.approve', async () => {
    const t = t3Transport();
    const c = new HubClient(t);
    const handler = vi.fn(async () => true);
    c.setConsentHandler(handler);
    await expect(c.dispatch('tasks.delete', { id: 't1' })).resolves.toEqual({ deleted: ['t1'] });
    expect(handler).toHaveBeenCalledWith(pending, false);
    expect(t.sent.map((r) => r.command)).toEqual(['tasks.delete', 'consent.get', 'consent.approve']);
    expect(t.sent[2]!.args).toEqual({ id: 'c1' });
  });

  it('deny: calls consent.deny and throws denied', async () => {
    const t = t3Transport();
    const c = new HubClient(t);
    c.setConsentHandler(async () => false);
    await expect(c.dispatch('tasks.delete', { id: 't1' })).rejects.toMatchObject({ code: 'denied' });
    expect(t.sent.map((r) => r.command)).toEqual(['tasks.delete', 'consent.get', 'consent.deny']);
  });

  it('falls back to local details when the core has no consent.get', async () => {
    const t = new FakeTransport((r) =>
      r.command === 'tasks.delete'
        ? { ok: false, error: { code: 'confirmation_required', message: 'needs approval', confirmation_id: 'c9' } }
        : r.command === 'consent.approve'
          ? { ok: true, result: { deleted: ['t1'] } }
          : { ok: false, error: { code: 'not_found', message: 'no' } },
    );
    const c = new HubClient(t);
    let seen: unknown;
    c.setConsentHandler(async (p) => ((seen = p), true));
    await c.dispatch('tasks.delete', { id: 't1' });
    expect(seen).toMatchObject({ id: 'c9', command: 'tasks.delete', args: { id: 't1' }, tier: 'T3' });
  });

  it('onEvent subscribes and unsubscribes', () => {
    const t = new FakeTransport(() => ({ ok: true, result: null }));
    const c = new HubClient(t);
    const off = c.onEvent(() => {});
    expect(t.listeners.size).toBe(1);
    off();
    expect(t.listeners.size).toBe(0);
  });
});
