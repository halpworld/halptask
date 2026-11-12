// Pins grid.ts to the Rust grid (halp-domain/src/grid.rs) through the shared
// fixture file written by the core. Rust is authoritative.
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import { checkedFindSpace, checkedMove, checkedResize, compact, GridError, type Box } from '../src/lib/layout/grid';

const FIXTURE = fileURLToPath(new URL('../../docs/fixtures/grid-cases.json', import.meta.url));

interface Case {
  name: string;
  op: 'move' | 'resize' | 'compact' | 'find_space';
  columns: number;
  widgets: Box[];
  args: Record<string, unknown>;
  expect: Box[] | { x: number; y: number } | { error: string };
}

function run(c: Case): unknown {
  try {
    switch (c.op) {
      case 'move':
        return checkedMove(c.widgets, c.args.id as string, c.args.x, c.args.y, c.columns);
      case 'resize':
        return checkedResize(c.widgets, c.args.id as string, c.args.w, c.args.h, c.columns);
      case 'compact':
        return compact(c.widgets);
      case 'find_space':
        return checkedFindSpace(c.widgets, c.args.w, c.args.h, c.columns);
    }
  } catch (e) {
    if (e instanceof GridError) return { error: e.code };
    throw e;
  }
}

const present = existsSync(FIXTURE);
if (!present) console.warn(`grid fixtures: ${FIXTURE} is missing; skipping. The core (p1-core) writes it.`);

describe.skipIf(!present)('grid.ts matches hub/docs/fixtures/grid-cases.json', () => {
  const cases: Case[] = present ? JSON.parse(readFileSync(FIXTURE, 'utf8')) : [];
  it('has cases', () => expect(cases.length).toBeGreaterThan(0));
  it.each(cases.map((c) => [c.name, c] as const))('%s', (_name, c) => {
    expect(run(c)).toEqual(c.expect);
  });
});
