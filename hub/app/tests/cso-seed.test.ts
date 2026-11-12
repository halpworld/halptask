// The mock's CSO data against the CSO pack: same types, widgets and layout,
// every seeded item valid (including the pack's if/then rules), and the
// demo story the widgets rely on.

import { readFileSync } from 'node:fs';
import { beforeEach, describe, expect, it } from 'vitest';
import { MockTransport } from '../src/lib/api/mock/mock';
import { FieldError, validateFields } from '../src/lib/api/mock/schema';
import { CSO_TYPES, demoSha } from '../src/lib/api/mock/seed-cso';
import { seedItems, seedLayouts, seedRoles, seedTypes } from '../src/lib/api/mock/seed';
import type { Args, HubResponse, Item, Layout, TaskType } from '../src/lib/api/types';
import { CSO_KINDS, registry, resolveWidget } from '../src/lib/widgets/registry';
import { checkBrief, deadlineRows, readBrief, readTarget, trajectoryModel } from '../src/lib/widgets/cso/logic';

interface Pack {
  widgets: string[];
  task_types: TaskType[];
  layout: Layout;
}
const pack = JSON.parse(readFileSync(new URL('../../packs/cso/pack.json', import.meta.url), 'utf8')) as Pack;
const promptDir = new URL('../../packs/cso/prompts/', import.meta.url);

const NOW = new Date('2026-11-10T09:30:00');
const TODAY = '2026-11-10';
let mock: MockTransport;
let n = 0;
async function call<T = unknown>(command: string, args: Args = {}): Promise<HubResponse<T>> {
  return (await mock.send({ command, args, request_id: `c${n++}` })) as HubResponse<T>;
}
beforeEach(() => {
  mock = new MockTransport({ latency: 0, now: () => NOW, seedHistory: false });
});

const schemas = new Map(seedTypes().map((t) => [t.id, t.schema]));
const validate = (type: string, fields: Record<string, unknown>) => validateFields((id) => schemas.get(id), type, fields);

describe('CSO seed mirrors the pack', () => {
  it('has the pack task types, exactly', () => {
    expect(CSO_TYPES).toEqual(pack.task_types);
  });

  it('gives the cso role the pack widgets and types', () => {
    const role = seedRoles().find((r) => r.id === 'cso')!;
    expect(role.widgets).toEqual(pack.widgets);
    expect(role.task_types).toEqual(['base.task', 'base.note', 'base.link', ...pack.task_types.map((t) => t.id)]);
    for (const k of pack.widgets) expect(registry.has(k), k).toBe(true);
  });

  it('lays out the CSO view like the pack default, configs included', () => {
    const shape = (l: Layout) => l.widgets.map(({ id, kind, title, x, y, w, h, config }) => ({ id, kind, title, x, y, w, h, config }));
    const cso = seedLayouts(TODAY, NOW.toISOString()).find((l) => l.role_id === 'cso')!;
    expect(cso.columns).toBe(pack.layout.columns);
    expect(shape(cso)).toEqual(shape(pack.layout));
  });

  it('pack widget configs pass the widgets’ own config schemas', () => {
    for (const w of pack.layout.widgets) {
      const schema = resolveWidget(w.kind).config;
      if (schema) expect(() => validateFields(() => schema, 'cfg', w.config), w.id).not.toThrow();
    }
  });

  it('registers every CSO kind as a live widget with a closed config schema', () => {
    for (const k of CSO_KINDS) {
      const e = resolveWidget(k);
      expect(e.kind).toBe(k);
      expect(e.placeholder).toBeUndefined();
      expect(e.config).toMatchObject({ type: 'object', additionalProperties: false });
    }
  });
});

describe('the seeded demo company', () => {
  const items = seedItems(TODAY, NOW.toISOString()).filter((i) => i.roles.includes('cso'));
  const typed = items.filter((i) => i.type_id);

  it('is schema-valid, item by item', () => {
    for (const i of typed) expect(() => validate(i.type_id!, i.fields), i.id).not.toThrow();
  });

  it('covers every type with a realistic amount of data', () => {
    const count = (t: string) => items.filter((i) => i.type_id === t).length;
    expect(count('cso.disclosure')).toBeGreaterThanOrEqual(5);
    expect(count('cso.data_request')).toBeGreaterThanOrEqual(8);
    expect(count('cso.evidence')).toBeGreaterThanOrEqual(5);
    expect(count('cso.target')).toBe(2);
    expect(count('cso.reg_change')).toBeGreaterThanOrEqual(4);
    expect(count('cso.board_brief')).toBeGreaterThanOrEqual(2);
    expect(count('cso.energy_reading')).toBeGreaterThanOrEqual(2);
  });

  it('labels itself as demo data', () => {
    const src = readFileSync(new URL('../src/lib/api/mock/seed-cso.ts', import.meta.url), 'utf8');
    expect(src).toMatch(/DEMO DATA/);
    for (const i of items.filter((x) => x.type_id === 'cso.disclosure' && x.fields.certainty_note)) expect(String(i.fields.certainty_note)).toMatch(/Demo data/);
  });

  it('opens on the SB 253 filing day with uncertain dates shown as such', () => {
    const rows = deadlineRows(items, TODAY, { horizonDays: 800 });
    const sb = rows.find((r) => r.d.item.id === 'cso-1')!;
    expect(sb).toMatchObject({ state: 'today', certain: true });
    expect(rows.filter((r) => !r.certain).map((r) => r.d.certainty).sort()).toEqual(['delayed', 'litigated', 'proposed']);
  });

  it('has a trajectory with a real gap and an unsourced point', () => {
    const t = readTarget(items.find((i) => i.id === 'cso-target-s12')!)!;
    const m = trajectoryModel(t, 2026);
    expect(m.gaps.map((g) => g.year)).toEqual([2023]);
    expect(m.points.filter((p) => !p.sourced).map((p) => p.year)).toEqual([2025]);
  });

  it('has a brief section blocked by an unsourced figure and a ready one fully sourced', () => {
    const byId = new Map(items.map((i) => [i.id, i]));
    const checks = items.map(readBrief).filter((b) => !!b).map((b) => checkBrief(b!, byId));
    expect(checks.some((c) => c.b.status === 'draft' && !c.canBeReady)).toBe(true);
    for (const c of checks.filter((x) => x.b.status !== 'draft')) expect(c.canBeReady, c.b.section).toBe(true);
  });

  it('uses well-formed demo hashes', () => {
    expect(demoSha('x')).toMatch(/^[0-9a-f]{64}$/);
    expect(demoSha('x')).toBe(demoSha('x'));
    expect(demoSha('x')).not.toBe(demoSha('y'));
  });
});

describe('the pack’s conditional rules, as the core enforces them', () => {
  const approved = { file_name: 'a.pdf', sha256: 'a'.repeat(64), status: 'approved' };

  it('refuses approved evidence without an approving reviewer', () => {
    expect(() => validate('cso.evidence', approved)).toThrow(FieldError);
    expect(() => validate('cso.evidence', { ...approved, approvals: [{ reviewer: 'A', decision: 'rejected', at: '2026-11-01T10:00:00Z' }] })).toThrow(FieldError);
    expect(() => validate('cso.evidence', { ...approved, approvals: [{ reviewer: 'A', decision: 'approved', at: '2026-11-01T10:00:00Z' }] })).not.toThrow();
    expect(() => validate('cso.evidence', { ...approved, status: 'draft' })).not.toThrow();
  });

  it('refuses a ready or presented brief with an unsourced figure', () => {
    const b = { section: 'S', figures: [{ label: 'x', value: 1, unit: 't' }] };
    expect(() => validate('cso.board_brief', { ...b, status: 'draft' })).not.toThrow();
    expect(() => validate('cso.board_brief', { ...b, status: 'ready' })).toThrow(FieldError);
    expect(() => validate('cso.board_brief', { ...b, status: 'presented' })).toThrow(FieldError);
    expect(() => validate('cso.board_brief', { section: 'S', status: 'ready', figures: [{ label: 'x', value: 1, unit: 't', source_item_id: 'cso-1' }] })).not.toThrow();
  });

  it('refuses non-https sources and malformed hashes', () => {
    expect(() => validate('cso.disclosure', { framework: 'sb253', due: TODAY, regulatory_certainty: 'confirmed', source_url: 'http://example.com/' })).toThrow(FieldError);
    expect(() => validate('cso.evidence', { file_name: 'a.pdf', sha256: 'A'.repeat(64), status: 'draft' })).toThrow(FieldError);
    expect(() => validate('cso.evidence', { file_name: '../a.pdf', sha256: 'a'.repeat(64), status: 'draft' })).toThrow(FieldError);
  });

  it('marks the board brief with the unpublished data class', () => {
    const t = pack.task_types.find((x) => x.id === 'cso.board_brief')!;
    expect(t.schema['x-halp-data-class']).toBe('cso.unpublished');
  });
});

describe('the mock core with the CSO role', () => {
  it('switches to the CSO role with a valid focus item', async () => {
    expect((await call('role.switch', { id: 'cso' })).ok).toBe(true);
    const day = await call<{ focus: Item | null }>('day.state');
    expect(day.ok && day.result.focus?.id).toBe('cso-1');
  });

  it('logs a chase through tasks.update with if_version', async () => {
    const got = await call<Item>('tasks.get', { id: 'cso-req-polymer' });
    expect(got.ok).toBe(true);
    if (!got.ok) return;
    const fields = { ...got.result.fields, status: 'chased', chase_count: 1, last_chased: NOW.toISOString() };
    const res = await call<Item>('tasks.update', { id: got.result.id, patch: { fields }, if_version: got.result.version });
    expect(res.ok && res.result.fields).toMatchObject({ status: 'chased', chase_count: 1 });
    const stale = await call('tasks.update', { id: got.result.id, patch: { fields }, if_version: got.result.version });
    expect(stale.ok).toBe(false);
  });

  it('refuses marking a brief ready while a figure is unsourced', async () => {
    const got = await call<Item>('tasks.get', { id: 'cso-brief-perf' });
    if (!got.ok) throw new Error('missing brief');
    const res = await call('tasks.update', { id: got.result.id, patch: { fields: { ...got.result.fields, status: 'ready' } } });
    expect(res.ok).toBe(false);
  });
});

describe('CSO prompts', () => {
  const names = ['weekly-esg-digest.md', 'data-chase-round.md', 'prepare-board-brief.md', 'evidence-gap-check.md'];
  for (const name of names) {
    it(`${name} forbids invented figures and treats data as untrusted`, () => {
      const body = readFileSync(new URL(name, promptDir), 'utf8');
      expect(body).toMatch(/never invent or estimate figures/i);
      expect(body).toMatch(/untrusted/i);
      expect(body).toMatch(/draft, do not send/i);
      expect(body).toMatch(/gap/i);
    });
  }
});

describe('unpublished board brief figures in the mock core', () => {
  const ids = (r: HubResponse<unknown>) => (r.ok ? (r.result as { items: Item[] }).items.map((i) => i.id) : null);

  it('hides cso.board_brief from agents without the cso.unpublished grant', async () => {
    const user = await call<{ items: Item[] }>('tasks.list', { type_id: 'cso.board_brief' });
    const briefs = user.ok ? user.result.items : [];
    expect(briefs.length).toBeGreaterThanOrEqual(2);
    const first = briefs[0]!;
    const agent = mock.agent('claude');
    expect(ids(await agent.dispatch('tasks.list', { type_id: 'cso.board_brief' }))).toEqual([]);
    expect(ids(await agent.dispatch('tasks.list', { role: 'cso', limit: 1000 }))).not.toContain(first.id);
    expect(await agent.dispatch('tasks.get', { id: first.id })).toMatchObject({ ok: false, error: { code: 'not_found' } });
    // The user still finds them by text: only notes.private is kept out of search.
    expect(ids(await call('tasks.list', { text: first.title }))).toContain(first.id);
  });

  it('shows them once the agent is granted the class', async () => {
    expect((await call('grants.set', { client_id: 'claude', data_classes: ['general', 'cso.unpublished'] })).ok).toBe(true);
    const agent = mock.agent('claude');
    expect(ids(await agent.dispatch('tasks.list', { type_id: 'cso.board_brief' }))!.length).toBeGreaterThanOrEqual(2);
  });
});
