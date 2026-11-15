import { afterEach, describe, expect, it, vi } from 'vitest';
import { COMPUTED_ENTRY, FALLBACK, isKnownKind, resolveWidget } from '../src/lib/widgets/registry';
import {
  COMPUTED_CONFIG,
  Debouncer,
  MAX_LIST_ITEMS,
  MAX_ROWS,
  REFRESH_DEBOUNCE_MS,
  clean,
  isWidgetId,
  parseComputedConfig,
  parseOutput,
  readTransformEnvelope,
  runTransform,
  unavailableTransformText,
} from '../src/lib/widgets/ext/computed';

describe('the computed kind in the registry', () => {
  it('resolves `computed` to the Tier B entry with a config schema', () => {
    expect(resolveWidget('computed')).toBe(COMPUTED_ENTRY);
    expect(isKnownKind('computed')).toBe(true);
    expect(COMPUTED_ENTRY.config).toBe(COMPUTED_CONFIG);
    expect(COMPUTED_CONFIG).toMatchObject({ type: 'object', additionalProperties: false, required: ['transform'] });
    for (const k of ['Computed', 'computed ', 'x.computed', 'computed.x']) expect(resolveWidget(k), k).toBe(FALLBACK);
  });
});

describe('computed widget config', () => {
  it('accepts <pack_id>/<name> and a known view', () => {
    expect(parseComputedConfig({ transform: 'acme.sre/burn', view: 'list' })).toEqual({ packId: 'acme.sre', name: 'burn', view: 'list' });
    expect(parseComputedConfig({ transform: 'dev.acme.sre/burn-2' })).toEqual({ packId: 'dev.acme.sre', name: 'burn-2', view: 'metric' });
    const pattern = new RegExp((COMPUTED_CONFIG.properties as Record<string, { pattern: string } | undefined>).transform?.pattern ?? '^$');
    expect(pattern.test('acme.sre/burn')).toBe(true);
  });

  it('refuses anything else', () => {
    for (const c of [
      null,
      [],
      'acme.sre/burn',
      {},
      { transform: 1 },
      { transform: 'acme/burn' },
      { transform: 'acme.sre' },
      { transform: 'acme.sre/Burn' },
      { transform: 'acme.sre/../x' },
      { transform: '../acme.sre/burn' },
      { transform: 'acme.sre/burn/x' },
      { transform: 'a.b.c.d.e.f/burn' },
      { transform: `${'a'.repeat(32)}.${'b'.repeat(32)}/burn` },
      { transform: 'acme.sre/burn', view: 'html' },
      { transform: 'acme.sre/burn', view: 'metric', style: 'x' },
      { transform: 'acme.sre/burn‮' },
    ]) {
      expect('error' in parseComputedConfig(c), JSON.stringify(c)).toBe(true);
    }
  });

  it('checks widget ids before they reach the shell', () => {
    expect(isWidgetId('w-0123abcd')).toBe(true);
    for (const id of ['', 'w 1', 'w/1', '../w', 'a'.repeat(65), 7]) expect(isWidgetId(id), String(id)).toBe(false);
  });
});

describe('ext_run_transform answers', () => {
  it('reads a success with trust and publisher as text', () => {
    const a = readTransformEnvelope({ ok: true, result: { output: { value: 3 }, trust: 'community', title: 'Acme SRE', publisher: 'Ac‮me', log: 'x' } });
    expect(a).toEqual({ state: 'ok', result: { output: { value: 3 }, trust: 'community', title: 'Acme SRE', publisher: 'Ac\\u202Eme', log: 'x' } });
    const odd = readTransformEnvelope({ ok: true, result: { output: 1, trust: '<b>', title: 5 } });
    expect(odd.state === 'ok' && odd.result.trust).toBe('unknown');
    expect(odd.state === 'ok' && odd.result.title).toBe('');
  });

  it('maps errors to states', () => {
    const err = (code: string, extra = {}) => readTransformEnvelope({ ok: false, error: { code, message: 'm', ...extra } });
    expect(err('denied').state).toBe('unavailable');
    expect(err('busy').state).toBe('busy');
    expect(err('module_failed', { reason: 'timeout' })).toEqual({ state: 'failed', reason: 'timeout', message: 'm' });
    expect(readTransformEnvelope({ ok: false, error: { code: 'internal', message: 'the Hub is locked; unlock it first' } }).state).toBe('locked');
    expect(err('internal').state).toBe('failed');
    expect(readTransformEnvelope(null).state).toBe('failed');
    expect(readTransformEnvelope('x').state).toBe('failed');
  });

  it('is a browser answer outside the desktop app', async () => {
    expect(await runTransform('acme.sre', 'burn', 'w-1')).toEqual({ state: 'browser' });
  });

  it('has text for every state that shows a card', () => {
    for (const s of ['unavailable', 'locked', 'browser', 'failed'] as const) expect(unavailableTransformText(s).title.length).toBeGreaterThan(0);
    expect(unavailableTransformText('failed', 'timeout').hint).toMatch(/too long/);
  });
});

describe('computed views', () => {
  it('metric', () => {
    expect(parseOutput('metric', { value: 1234, label: 'Burn', unit: 'h', delta: -2 })).toEqual({ view: 'metric', value: '1,234', label: 'Burn', unit: 'h', delta: '-2', trend: 'down' });
    expect(parseOutput('metric', 7)).toEqual({ view: 'metric', value: '7' });
    expect(parseOutput('metric', { value: '<img src=x onerror=alert(1)>' })).toMatchObject({ value: '<img src=x onerror=alert(1)>' });
    for (const o of [null, { value: null }, { value: {} }, { value: Number.NaN }, []]) expect('error' in parseOutput('metric', o), JSON.stringify(o)).toBe(true);
  });

  it('list', () => {
    const v = parseOutput('list', { items: [{ label: 'a', value: 2, hint: 'h' }, 'b', 3, { nope: 1 }, null] });
    expect(v).toEqual({ view: 'list', items: [{ label: 'a', value: '2', hint: 'h' }, { label: 'b' }, { label: '3' }], more: 0 });
    const many = parseOutput('list', Array.from({ length: MAX_LIST_ITEMS + 5 }, (_, i) => `i${i}`));
    expect(many).toMatchObject({ more: 5 });
    expect(!('error' in many) && many.view === 'list' && many.items.length).toBe(MAX_LIST_ITEMS);
    expect('error' in parseOutput('list', { items: 'x' })).toBe(true);
  });

  it('series', () => {
    const v = parseOutput('series', { label: 'Open', points: [{ x: 'Mon', y: 3 }, { x: 'Tue', y: 5 }, { y: 'bad' }, 4] });
    expect(v).toEqual({ view: 'series', label: 'Open', points: [{ x: 'Mon', y: 3 }, { x: 'Tue', y: 5 }, { x: '4', y: 4 }], firstX: 'Mon', lastX: '4', lastY: '4', min: 0, max: 5, more: 0 });
    expect(parseOutput('series', [2, 2])).toMatchObject({ min: 0, max: 2 });
    expect(parseOutput('series', [-1, -1])).toMatchObject({ min: -1, max: 0 });
    expect('error' in parseOutput('series', { points: [] })).toBe(true);
    expect('error' in parseOutput('series', 'x')).toBe(true);
  });

  it('table', () => {
    const v = parseOutput('table', { columns: ['Svc', 'Open'], rows: [['api', 3, 'extra'], ['db'], 'bad', [true, null]] });
    expect(v).toEqual({ view: 'table', columns: ['Svc', 'Open'], rows: [['api', '3'], ['db', ''], ['yes', '']], more: 0 });
    const headless = parseOutput('table', { rows: [['a', 'b']] });
    expect(headless).toMatchObject({ columns: ['', ''], rows: [['a', 'b']] });
    const big = parseOutput('table', { columns: ['a'], rows: Array.from({ length: MAX_ROWS + 1 }, () => ['x']) });
    expect(big).toMatchObject({ more: 1 });
    expect('error' in parseOutput('table', { columns: [], rows: [] })).toBe(true);
    expect('error' in parseOutput('table', [['a']])).toBe(true);
  });

  it('cuts long text and escapes hidden characters', () => {
    expect(clean('a'.repeat(300), 10)).toBe('a'.repeat(10) + '…');
    expect(clean('x​y\u0007')).toBe('x\\u200By\\u0007');
    const v = parseOutput('list', [{ label: 'evil‮exe.txt' }]);
    expect(!('error' in v) && v.view === 'list' && v.items[0]?.label).toBe('evil\\u202Eexe.txt');
  });
});

describe('refresh debounce', () => {
  afterEach(() => vi.useRealTimers());

  it('runs at once, then at most once per window, coalescing requests', async () => {
    vi.useFakeTimers();
    let runs = 0;
    const d = new Debouncer(async () => void runs++, () => Date.now());
    d.request();
    await vi.advanceTimersByTimeAsync(0);
    expect(runs).toBe(1);
    for (let i = 0; i < 10; i++) d.request();
    await vi.advanceTimersByTimeAsync(REFRESH_DEBOUNCE_MS - 1);
    expect(runs).toBe(1);
    await vi.advanceTimersByTimeAsync(1);
    expect(runs).toBe(2);
    await vi.advanceTimersByTimeAsync(REFRESH_DEBOUNCE_MS * 3);
    expect(runs).toBe(2);
    d.close();
    d.request();
    await vi.advanceTimersByTimeAsync(REFRESH_DEBOUNCE_MS * 2);
    expect(runs).toBe(2);
  });

  it('never overlaps a slow run', async () => {
    vi.useFakeTimers();
    let active = 0;
    let maxActive = 0;
    let runs = 0;
    const d = new Debouncer(async () => {
      active++;
      runs++;
      maxActive = Math.max(maxActive, active);
      await new Promise((r) => setTimeout(r, 5000));
      active--;
    });
    d.request();
    await vi.advanceTimersByTimeAsync(10);
    d.request();
    d.request();
    await vi.advanceTimersByTimeAsync(20000);
    expect(maxActive).toBe(1);
    expect(runs).toBe(2);
    d.close();
  });
});
