// "How to fill" guides: every role-pack card has one, the type ids and
// fields they name exist in the packs, and a connect button only appears
// where Settings > Connections has a form.

import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { TaskType } from '../src/lib/api/types';
import { fillGuide, GUIDED_KINDS, TYPE_META } from '../src/lib/widgets/fill';
import { registry } from '../src/lib/widgets/registry';

interface Pack {
  id: string;
  widgets: string[];
  task_types: TaskType[];
}
const packs = ['developer', 'eng-manager', 'cso'].map(
  (p) => JSON.parse(readFileSync(new URL(`../../packs/${p}/pack.json`, import.meta.url), 'utf8')) as Pack,
);
const types = new Map(packs.flatMap((p) => p.task_types.map((t) => [t.id, t] as const)));
const packOf = new Map(packs.flatMap((p) => p.task_types.map((t) => [t.id, p.id] as const)));
const src = (f: string) => readFileSync(new URL(`../src/lib/widgets/${f}`, import.meta.url), 'utf8');

/** Every enum value anywhere in a JSON schema. */
function enums(s: unknown, out = new Set<string>()): Set<string> {
  if (s && typeof s === 'object') {
    const o = s as Record<string, unknown>;
    if (Array.isArray(o.enum)) for (const v of o.enum) out.add(String(v));
    for (const v of Object.values(o)) enums(v, out);
  }
  return out;
}

describe('fill guides', () => {
  it('cover every widget a role pack ships, except the ones that need no data', () => {
    const selfFilling = new Set(['now', 'quick-capture', 'activity']);
    for (const kind of new Set(packs.flatMap((p) => p.widgets))) {
      if (selfFilling.has(kind)) continue;
      expect(fillGuide(kind), kind).toBeDefined();
    }
  });

  it('only guide kinds the registry knows', () => {
    for (const kind of GUIDED_KINDS) expect(registry.has(kind), kind).toBe(true);
  });

  it('name only type ids that exist, with fields their schema has', () => {
    for (const kind of GUIDED_KINDS) {
      const g = fillGuide(kind)!;
      for (const [, id] of g.agent.matchAll(/type_id "([a-z0-9_.-]+)"/g)) {
        const t = types.get(id!);
        expect(t, `${kind}: ${id}`).toBeDefined();
        const props = Object.keys((t!.schema as { properties?: Record<string, unknown> }).properties ?? {});
        // Field names the prompt quotes as identifiers (snake_case words).
        for (const [, f] of g.agent.replace(/"[^"]*"/g, '').matchAll(/\b([a-z]+_[a-z_]+)\b/g)) {
          if (['type_id', 'tasks_create', 'tasks_list', 'types_get', 'tasks_update', 'layout_get', 'layout_update_widget', 'i_owe', 'waiting_on', 'in_progress'].includes(f!)) continue;
          expect(props, `${kind}: field ${f} of ${id}`).toContain(f);
        }
      }
    }
  });

  it('tell the agent the kind and role each type is stored with', () => {
    for (const kind of GUIDED_KINDS) {
      const a = fillGuide(kind)!.agent;
      for (const [, id] of a.matchAll(/type_id "([a-z0-9_.-]+)"/g)) {
        const m = TYPE_META[id!];
        expect(m, `${kind}: ${id} has no TYPE_META`).toBeDefined();
        expect(m!.role, `${kind}: ${id}`).toBe(packOf.get(id!));
        expect(a, kind).toContain(`kind "${m!.kind}" and roles ["${m!.role}"]`);
      }
    }
    for (const id of Object.keys(TYPE_META)) expect(types.has(id), id).toBe(true);
    // The kinds the GitHub connector writes (crates/halp-integrations/src/github.rs).
    const gh = readFileSync(new URL('../../crates/halp-integrations/src/github.rs', import.meta.url), 'utf8');
    expect(TYPE_META['dev.pull_request']!.kind).toBe('task');
    expect(TYPE_META['dev.ci_run']!.kind).toBe('bullet');
    expect(gh).toMatch(/dev\.ci_run/);
  });

  it('quote only enum values the schema allows', () => {
    // Quoted words with an underscore are enum values (i_owe, waiting_on).
    for (const kind of GUIDED_KINDS) {
      const a = fillGuide(kind)!.agent;
      const ids = [...a.matchAll(/type_id "([a-z0-9_.-]+)"/g)].map((x) => x[1]!);
      const allowed = new Set(ids.flatMap((id) => [...enums(types.get(id)?.schema)]));
      for (const [, v] of a.matchAll(/"([a-z]+_[a-z_]+)"/g)) {
        if (ids.includes(v!)) continue;
        expect(allowed, `${kind}: "${v}"`).toContain(v);
      }
    }
  });

  it('name only config keys the widget reads', () => {
    const files: Record<string, string> = { 'metric-tile': 'MetricTile.svelte', countdown: 'CountdownWidget.svelte', 'ring-roster': 'RingRoster.svelte', timeline: 'Timeline.svelte' };
    for (const [kind, file] of Object.entries(files)) {
      const a = fillGuide(kind)!.agent;
      const code = src(file);
      for (const [, key] of a.matchAll(/config\.([a-z_]+)/g)) expect(code, `${kind}: config.${key}`).toMatch(new RegExp(`config\\.${key}\\b`));
      for (const [, v] of a.matchAll(/source "([a-z.]+)"/g)) expect(code, `${kind}: ${v}`).toContain(v);
    }
  });

  it('offer connect buttons only for connectors with a Settings form', () => {
    for (const kind of GUIDED_KINDS) for (const c of fillGuide(kind)!.connect ?? []) expect(['github', 'calendar']).toContain(c);
  });

  it('tell the agent not to invent data', () => {
    for (const kind of GUIDED_KINDS) {
      const a = fillGuide(kind)!.agent;
      if (a.includes('tasks_create') && a.includes('type_id')) expect(a, kind).toMatch(/Don't invent values/);
    }
  });

  it('is undefined for unknown and prototype keys', () => {
    expect(fillGuide('nope')).toBeUndefined();
    expect(fillGuide('__proto__')).toBeUndefined();
    expect(fillGuide('toString')).toBeUndefined();
  });
});
