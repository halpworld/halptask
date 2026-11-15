// The mock's developer data against the developer pack: same types, same
// widgets and layout, every seeded item valid, and the mock refusing
// schema-invalid typed items like the core does.

import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { MockTransport } from '../src/lib/api/mock/mock';
import { validateFields } from '../src/lib/api/mock/schema';
import { BASE_TYPES, DEV_TYPES, seedItems, seedLayouts, seedRoles, seedTypes } from '../src/lib/api/mock/seed';
import type { Args, HubResponse, Item, Layout, TaskType } from '../src/lib/api/types';
import { registry } from '../src/lib/widgets/registry';

interface Pack {
  widgets: string[];
  task_types: TaskType[];
  layout: Layout;
}
const pack = JSON.parse(readFileSync(new URL('../../packs/developer/pack.json', import.meta.url), 'utf8')) as Pack;

const NOW = new Date('2026-11-10T09:30:00');
let mock: MockTransport;
let n = 0;
async function call<T = unknown>(command: string, args: Args = {}): Promise<HubResponse<T>> {
  return (await mock.send({ command, args, request_id: `d${n++}` })) as HubResponse<T>;
}
beforeEach(() => {
  mock = new MockTransport({ latency: 0, now: () => NOW, seedHistory: false });
});

describe('developer seed mirrors the pack', () => {
  it('has the pack task types, exactly', () => {
    expect(DEV_TYPES).toEqual(pack.task_types);
  });

  it('gives the developer role the pack widgets and types', () => {
    const role = seedRoles().find((r) => r.id === 'developer')!;
    expect(role.widgets).toEqual(pack.widgets);
    expect(role.task_types).toEqual(['base.task', 'base.note', 'base.link', ...pack.task_types.map((t) => t.id)]);
    for (const k of pack.widgets) expect(registry.has(k), k).toBe(true);
  });

  it('lays out the developer view like the pack default', () => {
    const shape = (l: Layout) => l.widgets.map(({ id, kind, title, x, y, w, h }) => ({ id, kind, title, x, y, w, h }));
    const dev = seedLayouts('2026-11-10', NOW.toISOString()).find((l) => l.role_id === 'developer')!;
    expect(dev.columns).toBe(pack.layout.columns);
    expect(shape(dev)).toEqual(shape(pack.layout));
  });

  it('seeds only schema-valid typed items', () => {
    const types = new Map(seedTypes().map((t) => [t.id, t.schema]));
    const items = seedItems('2026-11-10', NOW.toISOString());
    const dev = items.filter((i) => i.type_id?.startsWith('dev.') || i.type_id?.startsWith('base.'));
    expect(dev.length).toBeGreaterThan(20);
    for (const i of dev) expect(() => validateFields((id) => types.get(id), i.type_id, i.fields), i.id).not.toThrow();
  });

  it('keeps the base types in step with the core', () => {
    expect(BASE_TYPES.map((t) => t.id)).toEqual(['base.task', 'base.note', 'base.link']);
  });
});

describe('mock validates typed fields like the core', () => {
  const pr = { repo: 'halptask/hub', number: 7 };

  it('accepts valid fields', async () => {
    const r = await call<Item>('tasks.create', { title: 'Review', type_id: 'dev.pull_request', fields: { ...pr, url: 'https://github.com/halptask/hub/pull/7', ci: 'passing', requested_at: '2026-11-10T08:00:00Z' } });
    expect(r.ok).toBe(true);
  });

  it.each([
    ['an unknown field', { ...pr, age_hours: 3 }],
    ['a missing required field', { repo: 'halptask/hub' }],
    ['a wrong type', { ...pr, number: '7' }],
    ['a value outside the enum', { ...pr, ci: 'green' }],
    ['a bad pattern', { ...pr, repo: 'not a repo' }],
    ['a non-URI url', { ...pr, url: 'not a url' }],
    ['a bad date-time', { ...pr, requested_at: 'yesterday' }],
    ['a number out of range', { ...pr, number: 0 }],
  ])('refuses %s', async (_name, fields) => {
    expect(await call('tasks.create', { title: 'Review', type_id: 'dev.pull_request', fields })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
  });

  it('refuses fields on an untyped item and checks updates too', async () => {
    expect(await call('tasks.create', { title: 'Loose', fields: { a: 1 } })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.update', { id: 'dev-2', patch: { fields: { repo: 'halptask/hub', number: 482, state: 'shipped' } } })).toMatchObject({ ok: false, error: { code: 'invalid_args' } });
    expect(await call('tasks.update', { id: 'dev-2', patch: { fields: { repo: 'halptask/hub', number: 482, state: 'approved' } } })).toMatchObject({ ok: true });
  });

  it('checks string limits in code points and arrays', async () => {
    const ask = { source: 'slack', from: 'x'.repeat(201), received_at: '2026-11-10T08:00:00Z' };
    expect(await call('tasks.create', { title: 'Ask', type_id: 'dev.ask', fields: ask })).toMatchObject({ ok: false });
    expect(await call('tasks.create', { title: 'Ask', type_id: 'dev.ask', fields: { ...ask, from: '😀'.repeat(200) } })).toMatchObject({ ok: true });
    const ctx = { repo: 'a/b', branch: 'main', captured_at: '2026-11-10T08:00:00Z', open_prs: [1, 'two'] };
    expect(await call('tasks.create', { title: 'Ctx', kind: 'bullet', type_id: 'dev.context', fields: ctx })).toMatchObject({ ok: false });
  });
});
