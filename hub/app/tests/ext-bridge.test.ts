import { describe, expect, it, vi } from 'vitest';
import {
  CALL_FAILED,
  ExtBridge,
  INVALID_PARAMS,
  INVALID_REQUEST,
  LoadGuard,
  MAX_CALLS_PER_SEC,
  MAX_MESSAGE_BYTES,
  METHOD_NOT_FOUND,
  PARSE_ERROR,
  RATE_LIMITED,
  effectiveCalls,
  isFrameSrc,
  isHandshakeRetry,
  isWindowsUA,
  parseExtKind,
  safeTokens,
  type BridgeDeps,
} from '../src/lib/widgets/ext/bridge';
import { unavailableText } from '../src/lib/widgets/ext/text';

function setup(calls: string[] = ['tasks_list', 'tasks_get']) {
  let t = 0;
  const deps = {
    call: vi.fn(async (name: string, args: Record<string, unknown>) => ({ ok: true as const, result: { name, args } })),
    openItem: vi.fn(),
    sizeChanged: vi.fn(),
    now: () => t,
  };
  const b = new ExtBridge(deps as unknown as BridgeDeps, calls);
  return { b, deps, tick: (ms: number) => (t += ms) };
}

const req = (method: string, params?: unknown, id: unknown = 1) => ({ jsonrpc: '2.0', id, method, params });

describe('parseExtKind', () => {
  it('parses pack and widget, including dev packs', () => {
    expect(parseExtKind('x.acme.sre.oncall')).toEqual({ kind: 'x.acme.sre.oncall', packId: 'acme.sre', widget: 'oncall' });
    expect(parseExtKind('x.dev.acme.sre.oncall')).toMatchObject({ packId: 'dev.acme.sre', widget: 'oncall' });
    expect(parseExtKind('x.a.b.c.d.e.w')).toMatchObject({ packId: 'a.b.c.d.e', widget: 'w' });
  });

  it('rejects anything else', () => {
    for (const k of [
      'x.acme.oncall', // pack id needs two segments
      'x.Acme.sre.w',
      'x.acme.sre.',
      'x..sre.w',
      'acme.sre.w',
      'x.acme.sre.w/../y',
      'x.acme.sre.<img>',
      'x.a.b.c.d.e.f.w',
      `x.acme.sre.${'w'.repeat(60)}`,
      '__proto__',
      'constructor',
      '',
      42,
      null,
      undefined,
      {},
    ]) {
      expect(parseExtKind(k), String(k)).toBeNull();
    }
  });
});

describe('frame src and calls', () => {
  const k = parseExtKind('x.acme.sre.oncall')!;
  it('accepts only this widget’s own halpext URL', () => {
    expect(isFrameSrc(`halpext://acme.sre/oncall/index.html?ticket=${'a'.repeat(32)}`, k)).toBe(true);
    for (const s of [
      'halpext://acme.sre/other/index.html',
      'halpext://acme.srex/oncall/index.html',
      'halpext://other.pack/oncall/index.html',
      'tauri://localhost/index.html',
      'javascript:alert(1)',
      'halpext://acme.sre/oncall/index.html?x=1',
      'halpext://acme.sre/oncall/%2e%2e/x.html',
      'halpext://acme.sre/oncall/a.html#f',
      42,
    ]) {
      expect(isFrameSrc(s, k), String(s)).toBe(false);
    }
  });

  it('intersects manifest calls with the read-only host list', () => {
    expect(effectiveCalls(['tasks_create', 'tasks_list', 'tasks_list', 'focus_get', 7])).toEqual(['tasks_list', 'focus_get']);
    expect(effectiveCalls('tasks_list')).toEqual([]);
  });
});

describe('ExtBridge', () => {
  it('relays an allowed tools/call and wraps the result', async () => {
    const { b, deps } = setup();
    const r = await b.handle(req('tools/call', { name: 'tasks_list', arguments: { limit: 3 } }, 'a'));
    expect(deps.call).toHaveBeenCalledWith('tasks_list', { limit: 3 });
    expect(r).toMatchObject({ jsonrpc: '2.0', id: 'a', result: { structuredContent: { name: 'tasks_list', args: { limit: 3 } } } });
  });

  it('refuses calls outside the allow-list without relaying', async () => {
    const { b, deps } = setup(['tasks_list', 'tasks_create']);
    for (const name of ['tasks_create', 'types_list', 'tasks.list', 'dispatch', '__proto__', 'constructor', 42, undefined]) {
      const r = await b.handle(req('tools/call', { name }));
      expect(r?.error?.code, String(name)).toBe(METHOD_NOT_FOUND);
    }
    expect(deps.call).not.toHaveBeenCalled();
  });

  it('maps a core error to a JSON-RPC error', async () => {
    const { b, deps } = setup();
    deps.call.mockResolvedValueOnce({ ok: false, error: { code: 'denied', message: 'nope' } } as never);
    const r = await b.handle(req('tools/call', { name: 'tasks_list' }));
    expect(r?.error).toEqual({ code: CALL_FAILED, message: 'nope', data: { code: 'denied' } });
  });

  it('rejects malformed messages', async () => {
    const { b, deps } = setup();
    expect((await b.handle('hello'))?.error?.code).toBe(INVALID_REQUEST);
    expect((await b.handle(null))?.error?.code).toBe(INVALID_REQUEST);
    expect((await b.handle([req('tools/call')]))?.error?.code).toBe(INVALID_REQUEST);
    expect((await b.handle({ id: 1, method: 'tools/call' }))?.error?.code).toBe(INVALID_REQUEST);
    expect((await b.handle({ jsonrpc: '2.0', id: { x: 1 }, method: 'tools/call' }))?.error?.code).toBe(INVALID_REQUEST);
    expect((await b.handle(req('tools/call', [1])))?.error?.code).toBe(INVALID_PARAMS);
    expect((await b.handle(req('tools/call', { name: 'tasks_list', arguments: [1] })))?.error?.code).toBe(INVALID_PARAMS);
    const cyclic: Record<string, unknown> = { jsonrpc: '2.0', id: 1, method: 'tools/call' };
    cyclic.self = cyclic;
    expect((await b.handle(cyclic))?.error?.code).toBe(PARSE_ERROR);
    expect((await b.handle({ jsonrpc: '2.0', id: 1, method: 'x', big: 10n }))?.error?.code).toBe(PARSE_ERROR);
    // A class instance is not plain JSON data.
    class Evil {
      jsonrpc = '2.0';
      id = 1;
      method = 'tools/call';
    }
    expect((await b.handle(new Evil()))?.error?.code).toBe(INVALID_REQUEST);
    expect(deps.call).not.toHaveBeenCalled();
  });

  it('bounds message size', async () => {
    const { b, deps } = setup();
    const r = await b.handle(req('tools/call', { name: 'tasks_list', arguments: { q: 'x'.repeat(MAX_MESSAGE_BYTES) } }));
    expect(r?.error?.code).toBe(INVALID_REQUEST);
    expect(deps.call).not.toHaveBeenCalled();
  });

  it('limits calls per second', async () => {
    const { b, deps, tick } = setup();
    for (let i = 0; i < MAX_CALLS_PER_SEC; i++) {
      expect((await b.handle(req('tools/call', { name: 'tasks_list' }, i)))?.result).toBeDefined();
    }
    expect((await b.handle(req('tools/call', { name: 'tasks_list' })))?.error?.code).toBe(RATE_LIMITED);
    tick(1000);
    expect((await b.handle(req('tools/call', { name: 'tasks_list' })))?.result).toBeDefined();
    expect(deps.call).toHaveBeenCalledTimes(MAX_CALLS_PER_SEC + 1);
  });

  it('answers unknown methods with -32601 and ignores notifications', async () => {
    const { b } = setup();
    expect((await b.handle(req('ui/open-link', { url: 'https://evil' })))?.error?.code).toBe(METHOD_NOT_FOUND);
    expect((await b.handle(req('ui/initialize', {})))?.error?.code).toBe(METHOD_NOT_FOUND);
    expect(await b.handle({ jsonrpc: '2.0', method: 'whatever' })).toBeNull();
    expect(await b.handle({ jsonrpc: '2.0', method: 'tools/call', params: { name: 'tasks_list' } })).toBeNull();
  });

  it('handles open-item (rate limited) and size changes', async () => {
    const { b, deps, tick } = setup();
    expect((await b.handle(req('ui/open-item', { id: 'tsk_1' })))?.result).toEqual({});
    expect((await b.handle(req('ui/open-item', { id: 'tsk_2' })))?.error?.code).toBe(RATE_LIMITED);
    tick(2000);
    expect((await b.handle(req('ui/open-item', { id: '<script>' })))?.error?.code).toBe(INVALID_PARAMS);
    expect(deps.openItem).toHaveBeenCalledTimes(1);
    await b.handle({ jsonrpc: '2.0', method: 'ui/size-changed', params: { h: 240 } });
    await b.handle({ jsonrpc: '2.0', method: 'ui/notifications/size-changed', params: { height: 120 } });
    await b.handle({ jsonrpc: '2.0', method: 'ui/size-changed', params: { h: Infinity } });
    expect(deps.sizeChanged.mock.calls).toEqual([[240], [120]]);
  });

  it('stops answering once closed', async () => {
    const { b, deps } = setup();
    b.close();
    expect(await b.handle(req('tools/call', { name: 'tasks_list' }))).toBeNull();
    expect(deps.call).not.toHaveBeenCalled();
  });
});

describe('window messages and loads', () => {
  it('treats only a ui/initialize from the frame itself as a handshake retry', () => {
    const frameWin = {};
    const init = { jsonrpc: '2.0', method: 'ui/initialize' };
    expect(isHandshakeRetry({ source: frameWin, data: init }, frameWin)).toBe(true);
    expect(isHandshakeRetry({ source: {}, data: init }, frameWin)).toBe(false);
    expect(isHandshakeRetry({ source: null, data: init }, null)).toBe(false);
    expect(isHandshakeRetry({ source: frameWin, data: { jsonrpc: '2.0', method: 'tools/call', id: 1 } }, frameWin)).toBe(false);
    expect(isHandshakeRetry({ source: frameWin, data: 'ui/initialize' }, frameWin)).toBe(false);
  });

  it('tears down on any load after the first', () => {
    const g = new LoadGuard();
    expect(g.onLoad()).toBe('init');
    expect(g.onLoad()).toBe('teardown');
    expect(g.onLoad()).toBe('teardown');
  });
});

describe('platform and theme', () => {
  it('detects Windows', () => {
    expect(isWindowsUA('Mozilla/5.0 (Windows NT 10.0; Win64; x64) Edg/120')).toBe(true);
    expect(isWindowsUA('Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15')).toBe(false);
    expect(unavailableText('unsupported').title).toBe('Custom widgets are not yet supported on Windows');
  });

  it('passes only inert theme token values', () => {
    const vals: Record<string, string> = { '--bg': ' #f3f2ee ', '--text': 'url(https://evil/x)', '--accent': 'rgb(1, 2, 3)', '--surface': 'red;}', '--border': '' };
    expect(safeTokens((n) => vals[n] ?? '')).toEqual({ '--bg': '#f3f2ee', '--accent': 'rgb(1, 2, 3)' });
  });
});
