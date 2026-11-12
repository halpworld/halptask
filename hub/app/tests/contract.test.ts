// The UI against the core's contract. hub/docs/fixtures/commands.json is the
// core's `commands.list` output (written by p1-core). This test:
//  1. finds every command the UI dispatches (dispatch / dispatchRaw / run calls
//     with a literal command name) and checks it exists in the core,
//  2. for calls with a literal args object, checks the required args are
//     present and no key is unknown to the input schema,
//  3. checks the mock matches the core (tiers, required args, accepted args)
//     on every command the mock has, and has every command the UI calls,
//     so `pnpm dev` behaves like the real app.
// Skipped (with a warning) while the fixture does not exist.

import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { describe, expect, it } from 'vitest';
import { CORE_NON_COMMAND_EVENTS, EV, EV_PREFIX } from '../src/lib/api/events';
import { MockTransport } from '../src/lib/api/mock/mock';
import type { CommandDescriptor, JsonSchema } from '../src/lib/api/types';

const FIXTURE = process.env.HALP_COMMANDS_JSON ? pathToFileURL(process.env.HALP_COMMANDS_JSON) : new URL('../../docs/fixtures/commands.json', import.meta.url);
const SRC = fileURLToPath(new URL('../src/', import.meta.url));
const present = existsSync(FIXTURE);
if (!present) console.warn(`contract.test: ${fileURLToPath(FIXTURE)} not found; skipping (p1-core writes it).`);

interface CallSite {
  file: string;
  line: number;
  command: string;
  /** Top-level keys of a literal args object; null when not a plain literal. */
  keys: string[] | null;
}

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((f) => {
    const p = join(dir, f);
    if (statSync(p).isDirectory()) return f === 'mock' ? [] : walk(p);
    return /\.(ts|svelte)$/.test(f) ? [p] : [];
  });
}

/** The text of a balanced {...} starting at `i`, or null. */
function braced(src: string, i: number): string | null {
  let depth = 0;
  let quote: string | null = null;
  for (let j = i; j < src.length; j++) {
    const c = src[j]!;
    if (quote) {
      if (c === '\\') j++;
      else if (c === quote) quote = null;
      continue;
    }
    if (c === "'" || c === '"' || c === '`') quote = c;
    else if (c === '{' || c === '(' || c === '[') depth++;
    else if (c === '}' || c === ')' || c === ']') {
      depth--;
      if (depth === 0) return src.slice(i + 1, j);
    }
  }
  return null;
}

function topLevelKeys(body: string): string[] | null {
  const parts: string[] = [];
  let depth = 0;
  let quote: string | null = null;
  let cur = '';
  for (let j = 0; j < body.length; j++) {
    const c = body[j]!;
    if (quote) {
      cur += c;
      if (c === '\\') cur += body[++j] ?? '';
      else if (c === quote) quote = null;
      continue;
    }
    if (c === "'" || c === '"' || c === '`') quote = c;
    if ('{(['.includes(c)) depth++;
    if ('})]'.includes(c)) depth--;
    if (c === ',' && depth === 0) {
      parts.push(cur);
      cur = '';
    } else cur += c;
  }
  if (cur.trim()) parts.push(cur);
  const keys: string[] = [];
  for (const p of parts) {
    const t = p.trim();
    if (t.startsWith('...')) return null; // spread: keys not statically known
    const m = /^([A-Za-z_$][\w$]*)\s*(?::|$)/.exec(t);
    if (!m) return null;
    keys.push(m[1]!);
  }
  return keys;
}

function callSites(): CallSite[] {
  const out: CallSite[] = [];
  const re = /\b(?:dispatch|dispatchRaw|run)\(\s*'([a-z_]+\.[a-z_]+)'\s*(,\s*)?/g;
  for (const file of walk(SRC)) {
    const src = readFileSync(file, 'utf8');
    for (const m of src.matchAll(re)) {
      const after = m.index! + m[0].length;
      let keys: string[] | null = null;
      if (!m[2]) keys = [];
      else if (src[after] === '{') {
        const body = braced(src, after);
        const rest = body === null ? '' : src.slice(after + body.length + 2).trimStart();
        // Only a literal that is the whole argument (not `{...} : x` etc.).
        keys = body !== null && rest.startsWith(')') ? topLevelKeys(body) : null;
      }
      out.push({ file: relative(SRC, file), line: src.slice(0, m.index).split('\n').length, command: m[1]!, keys });
    }
  }
  return out;
}

describe.skipIf(!present)('UI call sites match the core contract', () => {
  const core: CommandDescriptor[] = present ? (JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commands: CommandDescriptor[] }).commands : [];
  const byName = new Map(core.map((c) => [c.name, c]));
  const sites = callSites();

  it('finds the UI call sites', () => {
    expect(sites.length).toBeGreaterThan(10);
  });

  it('every dispatched command exists in the core', () => {
    const missing = sites.filter((s) => !byName.has(s.command)).map((s) => `${s.file}:${s.line} ${s.command}`);
    expect(missing).toEqual([]);
  });

  it('literal args carry the required keys and no unknown keys', () => {
    const problems: string[] = [];
    for (const s of sites) {
      const d = byName.get(s.command);
      if (!d || s.keys === null) continue;
      const schema = d.input_schema as JsonSchema;
      const props = schema.properties ?? {};
      for (const r of schema.required ?? []) if (!s.keys.includes(r)) problems.push(`${s.file}:${s.line} ${s.command} is missing required \`${r}\``);
      if (schema.additionalProperties === false || Object.keys(props).length) {
        for (const k of s.keys) if (!(k in props)) problems.push(`${s.file}:${s.line} ${s.command} passes unknown \`${k}\``);
      }
    }
    expect(problems).toEqual([]);
  });

  // The mock stands in for the core during `pnpm dev`, so it must match the
  // core on every command the UI calls. Core commands the UI never calls
  // (admin, import, ...) need not exist in the mock.
  it('the mock matches the core on every command the UI calls', async () => {
    const mock = new MockTransport({ latency: 0, seedHistory: false });
    const res = await mock.send({ command: 'commands.list', args: {}, request_id: 'c' });
    const mine = res.ok ? (res.result as { commands: CommandDescriptor[] }).commands : [];
    const diffs: string[] = [];
    const used = new Set(sites.map((s) => s.command));
    for (const c of core) {
      const m = mine.find((x) => x.name === c.name);
      if (!m) {
        if (used.has(c.name)) diffs.push(`mock lacks ${c.name}, which the UI calls`);
        continue;
      }
      if (m.tier !== c.tier) diffs.push(`${c.name}: tier ${m.tier} in mock, ${c.tier} in core`);
      const req = (s: JsonSchema) => [...(s.required ?? [])].sort().join(',');
      if (req(m.input_schema) !== req(c.input_schema as JsonSchema)) diffs.push(`${c.name}: required [${req(m.input_schema)}] in mock, [${req(c.input_schema as JsonSchema)}] in core`);
      const coreProps = Object.keys((c.input_schema as JsonSchema).properties ?? {}).sort();
      const mockProps = Object.keys(m.input_schema.properties ?? {}).sort();
      for (const p of mockProps) if (coreProps.length && !coreProps.includes(p)) diffs.push(`${c.name}: mock accepts \`${p}\`, core does not`);
    }
    for (const m of mine) if (!byName.has(m.name)) diffs.push(`mock has ${m.name}, core does not`);
    expect(diffs).toEqual([]);
  });
});

// Events: a core event's `command` is a command name, or one of the consent
// lifecycle names in CORE_NON_COMMAND_EVENTS. The UI must only react to names
// the core emits (consent.expired once slipped through as consent.deny).
describe.skipIf(!present)('UI event handlers match the core', () => {
  const core: CommandDescriptor[] = present ? (JSON.parse(readFileSync(FIXTURE, 'utf8')) as { commands: CommandDescriptor[] }).commands : [];
  const emitted = new Set<string>([...core.map((c) => c.name), ...CORE_NON_COMMAND_EVENTS]);
  const store = readFileSync(new URL('../src/lib/stores/hub.svelte.ts', import.meta.url), 'utf8');

  it('every event name the UI handles is one the core emits', () => {
    expect(Object.values(EV).filter((n) => !emitted.has(n))).toEqual([]);
  });

  it('every event prefix the UI handles matches a core command', () => {
    expect(Object.values(EV_PREFIX).filter((pre) => !core.some((c) => c.name.startsWith(pre)))).toEqual([]);
  });

  it('the store names events only through the EV constants', () => {
    // A literal like `c === 'consent.deny'` or `c.startsWith('tasks.')` would bypass the checks above.
    const literal = /\b(?:c|e\.command)\s*(?:===|!==)\s*'[^']*'|\bc\.startsWith\('[^']*'\)/g;
    const handler = store.slice(store.indexOf('handleEvent('), store.indexOf('private scheduleRefresh'));
    expect(handler.length).toBeGreaterThan(100);
    expect(handler.match(literal) ?? []).toEqual([]);
  });

  it('the mock emits only core event names', async () => {
    const seen = new Set<string>();
    const mock = new MockTransport({ latency: 0, seedHistory: true });
    mock.subscribe((e) => seen.add(e.command));
    const res = await mock.send({ command: 'history.list', args: { limit: 500 }, request_id: 'h' });
    if (res.ok) for (const e of (res.result as { events: { command: string }[] }).events) seen.add(e.command);
    expect([...seen].filter((n) => !emitted.has(n))).toEqual([]);
  });
});
