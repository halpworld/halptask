// The mock's engineering manager data against the eng-manager pack: same
// types, widgets and layout, every seeded item valid, and private notes kept
// away from agents and search.

import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { MockTransport } from '../src/lib/api/mock/mock';
import { validateFields } from '../src/lib/api/mock/schema';
import { EM_TYPES, isoWeek } from '../src/lib/api/mock/seed-em';
import { seedItems, seedLayouts, seedRoles, seedTypes } from '../src/lib/api/mock/seed';
import type { Args, HubResponse, Item, JsonSchema, Layout, TaskType } from '../src/lib/api/types';
import { EM_KINDS, registry, resolveWidget } from '../src/lib/widgets/registry';
import { isPrivateSchema } from '../src/lib/privacy/private';

interface Pack {
  widgets: string[];
  task_types: TaskType[];
  layout: Layout;
}
const pack = JSON.parse(readFileSync(new URL('../../packs/eng-manager/pack.json', import.meta.url), 'utf8')) as Pack;

const NOW = new Date('2026-11-10T09:30:00');
const TODAY = '2026-11-10';
let mock: MockTransport;
let n = 0;
async function call<T = unknown>(command: string, args: Args = {}): Promise<HubResponse<T>> {
  return (await mock.send({ command, args, request_id: `e${n++}` })) as HubResponse<T>;
}
beforeEach(() => {
  mock = new MockTransport({ latency: 0, now: () => NOW, seedHistory: false });
});

describe('eng-manager seed mirrors the pack', () => {
  it('has the pack task types, exactly', () => {
    expect(EM_TYPES).toEqual(pack.task_types);
  });

  it('gives the eng-manager role the pack widgets and types', () => {
    const role = seedRoles().find((r) => r.id === 'eng-manager')!;
    expect(role.widgets).toEqual(pack.widgets);
    expect(role.task_types).toEqual(['base.task', 'base.note', 'base.link', ...pack.task_types.map((t) => t.id)]);
    for (const k of pack.widgets) expect(registry.has(k), k).toBe(true);
  });

  it('lays out the EM view like the pack default, configs included', () => {
    const shape = (l: Layout) => l.widgets.map(({ id, kind, title, x, y, w, h, config }) => ({ id, kind, title, x, y, w, h, config }));
    const em = seedLayouts(TODAY, NOW.toISOString()).find((l) => l.role_id === 'eng-manager')!;
    expect(em.columns).toBe(pack.layout.columns);
    expect(shape(em)).toEqual(shape(pack.layout));
  });

  it('pack widget configs pass the widgets’ own config schemas', () => {
    for (const w of pack.layout.widgets) {
      const schema = resolveWidget(w.kind).config;
      if (!schema) continue;
      expect(() => validateFields(() => schema, 'cfg', w.config), w.id).not.toThrow();
    }
  });

  it('registers every EM kind as a live widget with a closed config schema', () => {
    for (const k of EM_KINDS) {
      const e = resolveWidget(k);
      expect(e.kind).toBe(k);
      expect(e.placeholder).toBeUndefined();
      expect(e.config).toMatchObject({ type: 'object', additionalProperties: false });
    }
  });

  it('seeds a realistic, schema-valid EM week', () => {
    const types = new Map(seedTypes().map((t) => [t.id, t.schema]));
    const items = seedItems(TODAY, NOW.toISOString()).filter((i) => i.roles.includes('eng-manager'));
    const typed = items.filter((i) => i.type_id);
    for (const i of typed) expect(() => validateFields((id) => types.get(id), i.type_id, i.fields), i.id).not.toThrow();
    const count = (t: string) => items.filter((i) => i.type_id === t).length;
    expect(count('em.person')).toBe(6);
    expect(count('em.meeting')).toBeGreaterThanOrEqual(15);
    expect(count('em.commitment')).toBeGreaterThanOrEqual(6);
    expect(count('em.one_on_one')).toBeGreaterThanOrEqual(4);
    expect(count('em.team_signal')).toBe(6);
    expect(count('em.private_note')).toBeGreaterThanOrEqual(2);
    // Every person reference points at a seeded person.
    const people = new Set(items.filter((i) => i.type_id === 'em.person').map((i) => i.id));
    for (const i of typed) if (typeof i.fields.person_id === 'string') expect(people.has(i.fields.person_id), i.id).toBe(true);
    // Private notes only ever carry the neutral title.
    for (const i of items.filter((x) => x.type_id === 'em.private_note')) expect(i.title).toBe('Private note');
  });

  it('marks the private note type with the private data class', () => {
    expect(isPrivateSchema(EM_TYPES.find((t) => t.id === 'em.private_note')!.schema)).toBe(true);
    for (const t of EM_TYPES.filter((x) => x.id !== 'em.private_note')) expect(isPrivateSchema(t.schema), t.id).toBe(false);
  });

  it('labels ISO weeks like the team signal pattern', () => {
    expect(isoWeek('2026-11-10')).toBe('2026-W46');
    expect(isoWeek('2026-01-01')).toBe('2026-W01');
    expect(isoWeek('2027-01-01')).toBe('2026-W53');
    expect(isoWeek('2024-12-30')).toBe('2025-W01');
  });

  it('switches to the EM role with a valid focus item', async () => {
    const res = await call('role.switch', { id: 'eng-manager' });
    expect(res.ok).toBe(true);
    const day = await call<{ focus: Item | null }>('day.state');
    expect(day.ok && day.result.focus?.id).toBe('em-1');
  });
});

describe('the pack never ranks or scores people', () => {
  const BANNED = /score|rating|rank|sentiment|performance|productivity|velocity|grade/i;
  const props = (s: JsonSchema): string[] => {
    const out: string[] = [];
    const walk = (x: unknown) => {
      if (!x || typeof x !== 'object') return;
      const o = x as Record<string, unknown>;
      if (o.properties && typeof o.properties === 'object') out.push(...Object.keys(o.properties as object));
      for (const v of Object.values(o)) walk(v);
    };
    walk(s);
    return out;
  };

  it('has no per-person metric fields in any type', () => {
    for (const t of pack.task_types) for (const p of props(t.schema)) expect(BANNED.test(p), `${t.id}.${p}`).toBe(false);
  });

  it('keeps delivery signals team-level: no person field on em.team_signal', () => {
    const t = pack.task_types.find((x) => x.id === 'em.team_signal')!;
    for (const p of props(t.schema)) expect(/person|author|user|owner|member|who/.test(p), p).toBe(false);
  });

  it('offers only name and next-1:1 orders in the roster config', () => {
    const sort = (resolveWidget('people-roster').config as { properties: { sort: { enum: string[] } } }).properties.sort;
    expect(sort.enum).toEqual(['name', 'next']);
  });
});

describe('private notes in the mock core', () => {
  it('lists private notes for the user only', async () => {
    const user = await call<{ items: Item[] }>('tasks.list', { type_id: 'em.private_note' });
    expect(user.ok && user.result.items.length).toBeGreaterThanOrEqual(2);
    const agent = await mock.agent('claude').dispatch('tasks.list', { type_id: 'em.private_note' });
    expect(agent.ok && (agent.result as { items: Item[] }).items).toEqual([]);
  });

  it('hides private notes from every agent listing and get', async () => {
    const agent = mock.agent('claude');
    const all = await agent.dispatch('tasks.list', { role: 'eng-manager', limit: 1000 });
    expect(all.ok).toBe(true);
    if (all.ok) expect((all.result as { items: Item[] }).items.some((i) => i.type_id === 'em.private_note')).toBe(false);
    expect(await agent.dispatch('tasks.get', { id: 'em-n-1' })).toMatchObject({ ok: false, error: { code: 'not_found' } });
    expect((await call('tasks.get', { id: 'em-n-1' })).ok).toBe(true);
  });

  it('never matches private notes in text search, even for the user', async () => {
    const created = await call<Item>('tasks.create', { title: 'Private note', kind: 'bullet', type_id: 'em.private_note', fields: { person_id: 'em-p-sam', body: 'zebra' }, notes: 'zebra' });
    expect(created.ok).toBe(true);
    const res = await call<{ items: Item[] }>('tasks.list', { text: 'zebra' });
    expect(res.ok && res.result.items).toEqual([]);
    const byTitle = await call<{ items: Item[] }>('tasks.list', { text: 'private note' });
    expect(byTitle.ok && byTitle.result.items).toEqual([]);
  });
});
