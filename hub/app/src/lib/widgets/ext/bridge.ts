// The host side of a Tier C widget frame (docs/hub/P5-EXTENSIONS.md §6.3,
// §6.4). Pure logic, no DOM: ExtFrame.svelte owns the iframe and the
// MessageChannel and feeds messages through here.
//
// Rules this file enforces:
// - Window `message` events carry no authority. The only one acted on is a
//   handshake retry whose `source` is this frame's own contentWindow, and
//   all it gets is a fresh port to the same frame (a spoof is
//   indistinguishable from a retry, §6.4 condition 6).
// - Everything else arrives on the host's MessagePort and is JSON-RPC 2.0:
//   ≤ 256 KiB, ≤ 20 calls/s, ≤ 8 calls in flight, `tools/call` names only
//   from the widget's allow-list (manifest `calls` ∩ the host list, as Rust
//   answered it). Rust re-checks all of it against the installed manifest.
// - Any `load` after the first is a navigation: tear the frame down
//   (§6.4 condition 5).

import type { HubResponse } from '../../api/types';

/** MCP tool names the host relays (all T0 reads; writes are deferred, §10). */
export const HOST_CALLS = ['tasks_list', 'tasks_get', 'types_list', 'types_get', 'day_state', 'focus_get'] as const;
export type HostCall = (typeof HOST_CALLS)[number];

export const MAX_MESSAGE_BYTES = 256 * 1024;
export const MAX_CALLS_PER_SEC = 20;
export const MAX_IN_FLIGHT = 8;
export const MAX_HANDSHAKE_RETRIES = 3;
/** One `ui/open-item` per this many ms, so a widget cannot spam the host. */
export const OPEN_ITEM_EVERY_MS = 2000;

// JSON-RPC error codes.
export const PARSE_ERROR = -32700;
export const INVALID_REQUEST = -32600;
export const METHOD_NOT_FOUND = -32601;
export const INVALID_PARAMS = -32602;
export const CALL_FAILED = -32000;
export const RATE_LIMITED = -32001;

const SEGMENT = '[a-z][a-z0-9-]{0,31}';
const PACK_ID = new RegExp(`^${SEGMENT}(\\.${SEGMENT}){1,4}$`);
const WIDGET_NAME = new RegExp(`^${SEGMENT}$`);
const ITEM_ID = /^[A-Za-z0-9_.:-]{1,64}$/;

export interface ExtKind {
  kind: string;
  packId: string;
  widget: string;
}

/**
 * Parses a Tier C kind `x.<pack_id>.<name>` (§3.1; a dev pack's id is
 * `dev.<id>`). Anything else, including prototype keys, is null.
 */
export function parseExtKind(kind: unknown): ExtKind | null {
  if (typeof kind !== 'string' || kind.length > 64 || !kind.startsWith('x.')) return null;
  const rest = kind.slice(2);
  const dot = rest.lastIndexOf('.');
  if (dot < 0) return null;
  const packId = rest.slice(0, dot);
  const widget = rest.slice(dot + 1);
  if (packId.length > 64 || !PACK_ID.test(packId) || !WIDGET_NAME.test(widget)) return null;
  return { kind, packId, widget };
}

/** The calls the host will relay for a widget: its list ∩ HOST_CALLS. */
export function effectiveCalls(calls: unknown): HostCall[] {
  if (!Array.isArray(calls)) return [];
  const out: HostCall[] = [];
  for (const c of calls) {
    if (typeof c === 'string' && (HOST_CALLS as readonly string[]).includes(c) && !out.includes(c as HostCall)) out.push(c as HostCall);
  }
  return out;
}

/** The URL a frame may load: `halpext://<pack_id>/<widget>/…` for this widget only. */
export function isFrameSrc(src: unknown, k: ExtKind): src is string {
  if (typeof src !== 'string' || src.length > 1024) return false;
  const prefix = `halpext://${k.packId}/${k.widget}/`;
  if (!src.startsWith(prefix)) return false;
  return /^[a-z0-9_./-]+(\?ticket=[0-9a-f]{32})?$/.test(src.slice(prefix.length));
}

/** Bytes of a message as JSON, or null when it is not plain JSON data. */
export function jsonSize(data: unknown): number | null {
  try {
    const s = JSON.stringify(data);
    if (typeof s !== 'string') return null;
    return new TextEncoder().encode(s).length;
  } catch {
    return null;
  }
}

type Id = string | number | null;

export interface RpcResponse {
  jsonrpc: '2.0';
  id: Id;
  result?: unknown;
  error?: { code: number; message: string; data?: unknown };
}

export interface BridgeDeps {
  /** Relay to Rust `ext_call` (which runs as `module:<pack_id>`). */
  call(name: HostCall, args: Record<string, unknown>): Promise<HubResponse<unknown>>;
  /** The host shows the item in its own UI. */
  openItem(id: string): void;
  /** The frame asks for a height in px; the host clamps it. */
  sizeChanged(h: number): void;
  now(): number;
}

const rpcError = (id: Id, code: number, message: string, data?: unknown): RpcResponse => ({
  jsonrpc: '2.0',
  id,
  error: data === undefined ? { code, message } : { code, message, data },
});

function isPlainObject(v: unknown): v is Record<string, unknown> {
  if (typeof v !== 'object' || v === null || Array.isArray(v)) return false;
  const p = Object.getPrototypeOf(v);
  return p === Object.prototype || p === null;
}

function validId(v: unknown): v is string | number {
  return (typeof v === 'string' && v.length <= 128) || (typeof v === 'number' && Number.isFinite(v));
}

/** Host side of one mounted frame's port. */
export class ExtBridge {
  private readonly allowed: ReadonlySet<string>;
  private calls: number[] = [];
  private inFlight = 0;
  private lastOpen = -Infinity;
  private closed = false;

  constructor(
    private readonly deps: BridgeDeps,
    allowedCalls: readonly string[],
  ) {
    this.allowed = new Set(effectiveCalls(allowedCalls));
  }

  /** Stop answering (the frame was torn down). */
  close(): void {
    this.closed = true;
  }

  /**
   * Handles one message from the port. Returns the JSON-RPC response to
   * post back, or null for notifications and dropped messages.
   */
  async handle(data: unknown): Promise<RpcResponse | null> {
    if (this.closed) return null;
    const size = jsonSize(data);
    if (size === null) return rpcError(null, PARSE_ERROR, 'not JSON data');
    if (size > MAX_MESSAGE_BYTES) return rpcError(null, INVALID_REQUEST, 'message too large');
    if (!isPlainObject(data) || data.jsonrpc !== '2.0' || typeof data.method !== 'string') {
      const id = isPlainObject(data) && validId(data.id) ? data.id : null;
      return rpcError(id, INVALID_REQUEST, 'not a JSON-RPC 2.0 request');
    }
    const hasId = 'id' in data && data.id !== undefined;
    if (hasId && !validId(data.id)) return rpcError(null, INVALID_REQUEST, 'bad id');
    const id: Id = hasId ? (data.id as string | number) : null;
    const params = data.params === undefined ? {} : data.params;
    if (!isPlainObject(params)) return hasId ? rpcError(id, INVALID_PARAMS, 'params must be an object') : null;

    switch (data.method) {
      case 'tools/call':
        if (!hasId) return null;
        return this.toolsCall(id, params);
      case 'ui/open-item': {
        const item = params.id;
        if (typeof item !== 'string' || !ITEM_ID.test(item)) return hasId ? rpcError(id, INVALID_PARAMS, 'bad item id') : null;
        const now = this.deps.now();
        if (now - this.lastOpen < OPEN_ITEM_EVERY_MS) return hasId ? rpcError(id, RATE_LIMITED, 'too many requests') : null;
        this.lastOpen = now;
        this.deps.openItem(item);
        return hasId ? { jsonrpc: '2.0', id, result: {} } : null;
      }
      case 'ui/size-changed':
      case 'ui/notifications/size-changed': {
        const h = params.h ?? params.height;
        if (typeof h === 'number' && Number.isFinite(h) && h >= 0) this.deps.sizeChanged(h);
        return hasId ? { jsonrpc: '2.0', id, result: {} } : null;
      }
      default:
        return hasId ? rpcError(id, METHOD_NOT_FOUND, 'method not found') : null;
    }
  }

  private async toolsCall(id: Id, params: Record<string, unknown>): Promise<RpcResponse> {
    const { name } = params;
    const args = params.arguments === undefined ? {} : params.arguments;
    if (typeof name !== 'string' || !this.allowed.has(name)) return rpcError(id, METHOD_NOT_FOUND, 'this call is not available to this widget');
    if (!isPlainObject(args)) return rpcError(id, INVALID_PARAMS, 'arguments must be an object');
    const now = this.deps.now();
    this.calls = this.calls.filter((t) => now - t < 1000);
    if (this.calls.length >= MAX_CALLS_PER_SEC || this.inFlight >= MAX_IN_FLIGHT) return rpcError(id, RATE_LIMITED, 'too many calls');
    this.calls.push(now);
    this.inFlight++;
    let res: HubResponse<unknown>;
    try {
      res = await this.deps.call(name as HostCall, args);
    } catch {
      res = { ok: false, error: { code: 'internal', message: 'call failed' } };
    } finally {
      this.inFlight--;
    }
    if (res.ok) {
      return { jsonrpc: '2.0', id, result: { structuredContent: res.result, content: [{ type: 'text', text: JSON.stringify(res.result) }] } };
    }
    const code = typeof res.error?.code === 'string' ? res.error.code : 'internal';
    const message = typeof res.error?.message === 'string' ? res.error.message.slice(0, 500) : 'call failed';
    return rpcError(id, CALL_FAILED, message, { code });
  }
}

/**
 * Whether a window `message` event is a handshake retry from this frame:
 * `source` must be the frame's own contentWindow and the data a
 * `ui/initialize` request. Nothing else from the window is ever handled.
 */
export function isHandshakeRetry(ev: { source: unknown; data: unknown }, frameWindow: unknown): boolean {
  if (frameWindow == null || ev.source !== frameWindow) return false;
  const d = ev.data;
  return isPlainObject(d) && d.jsonrpc === '2.0' && d.method === 'ui/initialize';
}

/** Counts frame `load` events; any after the first means tear down. */
export class LoadGuard {
  private loads = 0;
  /** Returns 'init' on the first load and 'teardown' on any later one. */
  onLoad(): 'init' | 'teardown' {
    this.loads++;
    return this.loads === 1 ? 'init' : 'teardown';
  }
}

/** The windows UA check, as a second guard next to Rust's `unsupported`. */
export function isWindowsUA(ua: string | undefined): boolean {
  return typeof ua === 'string' && /Windows/i.test(ua);
}

/** CSS custom properties passed to the frame as theme tokens. */
export const THEME_TOKENS = ['--bg', '--surface', '--surface-2', '--border', '--text', '--text-2', '--text-3', '--accent', '--accent-text', '--accent-soft', '--text-on-accent'] as const;

/** Keeps only short, inert token values (colours and the like). */
export function safeTokens(read: (name: string) => string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const t of THEME_TOKENS) {
    const v = read(t).trim();
    if (v.length > 0 && v.length <= 64 && /^[#a-zA-Z0-9(),.%\s/-]+$/.test(v)) out[t] = v;
  }
  return out;
}
