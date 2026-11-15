// Tier B `computed` widgets (docs/hub/P5-EXTENSIONS.md §5): the config, the
// shell command and the view shapes, as plain data so they can be tested
// without a DOM.
//
// A transform's output was already checked by the host (size, depth, text,
// its own output_schema), but it is still a pack's data: every view here is
// parsed again into a small fixed shape, every string is cut and has its
// hidden characters escaped, and the component renders text only (no
// {@html}, no URLs, no styles from the output).

import { isTauri } from '../../api/transport';
import type { JsonSchema } from '../../api/types';
import { escapeHidden } from '../../util/format-chars';

export const VIEWS = ['metric', 'list', 'series', 'table'] as const;
export type View = (typeof VIEWS)[number];

/** One id segment (`halp-pack` ids.rs / check_name). */
const SEG = '[a-z][a-z0-9-]{0,31}';
/** `<pack_id>/<name>`: a pack id of 2 to 5 segments, then a module name. */
const TRANSFORM_RE = new RegExp(`^(${SEG}(?:\\.${SEG}){1,4})/(${SEG})$`);
const WIDGET_ID_RE = /^[A-Za-z0-9_-]{1,64}$/;

/** The widget config's JSON Schema (also checked by `parseComputedConfig`). */
export const COMPUTED_CONFIG: JsonSchema = {
  type: 'object',
  properties: {
    transform: { type: 'string', pattern: TRANSFORM_RE.source, maxLength: 97, description: 'The pack transform to run, as <pack_id>/<name>.' },
    view: { enum: [...VIEWS], description: 'How to show its output.' },
  },
  required: ['transform'],
  additionalProperties: false,
};

export interface ComputedConfig {
  packId: string;
  name: string;
  view: View;
}

/** The checked config, or why it is not usable. */
export function parseComputedConfig(config: unknown): ComputedConfig | { error: string } {
  if (!config || typeof config !== 'object' || Array.isArray(config)) return { error: 'This widget has no transform set.' };
  const c = config as Record<string, unknown>;
  for (const k of Object.keys(c)) if (k !== 'transform' && k !== 'view') return { error: 'This widget’s config has unknown settings.' };
  const t = c.transform;
  if (typeof t !== 'string') return { error: 'This widget has no transform set.' };
  const m = t.length <= 97 ? TRANSFORM_RE.exec(t) : null;
  const packId = m?.[1];
  const name = m?.[2];
  if (!packId || !name || packId.length > 64) return { error: 'This widget’s transform name is not valid.' };
  const view = c.view === undefined ? 'metric' : c.view;
  if (typeof view !== 'string' || !(VIEWS as readonly string[]).includes(view)) return { error: 'This widget’s view is not one of metric, list, series or table.' };
  return { packId, name, view: view as View };
}

export function isWidgetId(id: unknown): id is string {
  return typeof id === 'string' && WIDGET_ID_RE.test(id);
}

// ---- the shell command ----------------------------------------------------------

export interface TransformResult {
  output: unknown;
  trust: string;
  title: string;
  publisher: string;
  /** `halp.log` text, dev packs only. */
  log?: string;
}

export type TransformAnswer =
  | { state: 'ok'; result: TransformResult }
  /** The pack is not installed, disabled, may not run code, or the widget does not match. */
  | { state: 'unavailable'; message: string }
  /** Rate-limited or queue full: keep the last output and try later. */
  | { state: 'busy' }
  | { state: 'failed'; reason: string; message: string }
  | { state: 'locked' }
  /** Not the desktop app (browser preview / mock). */
  | { state: 'browser' };

const str = (v: unknown, max: number): string => (typeof v === 'string' ? clean(v, max) : '');

/** Reads `ext_run_transform`'s envelope into an answer. */
export function readTransformEnvelope(v: unknown): TransformAnswer {
  if (!v || typeof v !== 'object') return { state: 'failed', reason: 'internal', message: 'No answer from the Hub.' };
  const e = v as { ok?: unknown; result?: unknown; error?: unknown };
  if (e.ok === true && e.result && typeof e.result === 'object') {
    const r = e.result as Record<string, unknown>;
    return {
      state: 'ok',
      result: {
        output: r.output,
        trust: typeof r.trust === 'string' && /^[a-z_]{1,16}$/.test(r.trust) ? r.trust : 'unknown',
        title: str(r.title, 80),
        publisher: str(r.publisher, 80),
        ...(typeof r.log === 'string' ? { log: str(r.log, 4000) } : {}),
      },
    };
  }
  const err = (e.error && typeof e.error === 'object' ? e.error : {}) as Record<string, unknown>;
  const message = str(err.message, 300);
  switch (err.code) {
    case 'denied':
      return { state: 'unavailable', message };
    case 'busy':
      return { state: 'busy' };
    case 'internal':
      return /locked/i.test(message) ? { state: 'locked' } : { state: 'failed', reason: 'internal', message };
    case 'module_failed':
      return { state: 'failed', reason: str(err.reason, 32) || 'module', message };
    default:
      return { state: 'failed', reason: 'internal', message };
  }
}

export async function runTransform(packId: string, name: string, widgetId: string): Promise<TransformAnswer> {
  if (!isTauri()) return { state: 'browser' };
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return readTransformEnvelope(await invoke<unknown>('ext_run_transform', { packId, name, widgetId }));
  } catch (err) {
    return { state: 'failed', reason: 'internal', message: clean(String(err), 300) };
  }
}

// ---- refresh debounce -------------------------------------------------------------

/** At most one run per widget in this window (§5.3). */
export const REFRESH_DEBOUNCE_MS = 2000;

/**
 * Coalesces refresh requests: the first runs at once, later ones wait until
 * `REFRESH_DEBOUNCE_MS` after the previous run started, and any number of
 * requests in that window become one trailing run. A run never overlaps the
 * previous one.
 */
export class Debouncer {
  private last = -Infinity;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private running = false;
  private again = false;
  private closed = false;

  constructor(
    private readonly run: () => Promise<void>,
    private readonly now: () => number = () => Date.now(),
    private readonly gap = REFRESH_DEBOUNCE_MS,
  ) {}

  request(): void {
    if (this.closed) return;
    if (this.running) {
      this.again = true;
      return;
    }
    if (this.timer) return;
    const wait = Math.max(0, this.last + this.gap - this.now());
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.fire();
    }, wait);
  }

  private async fire(): Promise<void> {
    if (this.closed) return;
    this.running = true;
    this.last = this.now();
    try {
      await this.run();
    } finally {
      this.running = false;
      if (this.again) {
        this.again = false;
        this.request();
      }
    }
  }

  close(): void {
    this.closed = true;
    if (this.timer) clearTimeout(this.timer);
    this.timer = null;
  }
}

// ---- views ----------------------------------------------------------------------

export const MAX_TEXT = 200;
export const MAX_LIST_ITEMS = 50;
export const MAX_POINTS = 200;
export const MAX_COLUMNS = 8;
export const MAX_ROWS = 50;

/** Cut to `max` characters (by code point) and escape hidden characters. */
export function clean(text: string, max = MAX_TEXT): string {
  const chars = Array.from(text);
  const cut = chars.length > max ? chars.slice(0, max).join('') + '…' : text;
  return escapeHidden(cut);
}

/** A number or string as display text; anything else is `null`. */
function scalar(v: unknown): string | null {
  if (typeof v === 'string') return clean(v);
  if (typeof v === 'number' && Number.isFinite(v)) return formatNumber(v);
  if (typeof v === 'boolean') return v ? 'yes' : 'no';
  return null;
}

export function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return '';
  if (Number.isInteger(n) && Math.abs(n) < 1e15) return n.toLocaleString('en-US');
  return n.toLocaleString('en-US', { maximumFractionDigits: 2 });
}

const opt = (v: unknown, max = MAX_TEXT): string | undefined => (typeof v === 'string' && v !== '' ? clean(v, max) : undefined);
const isObj = (v: unknown): v is Record<string, unknown> => !!v && typeof v === 'object' && !Array.isArray(v);

export interface MetricView {
  view: 'metric';
  value: string;
  label?: string;
  unit?: string;
  hint?: string;
  /** A signed change, shown with its sign. */
  delta?: string;
  trend?: 'up' | 'down' | 'flat';
}
export interface ListView {
  view: 'list';
  items: { label: string; value?: string; hint?: string }[];
  more: number;
}
export interface SeriesView {
  view: 'series';
  label?: string;
  unit?: string;
  points: { x: string; y: number }[];
  /** The first and last x labels and the last y, for the axis line. */
  firstX: string;
  lastX: string;
  lastY: string;
  min: number;
  max: number;
  more: number;
}
export interface TableView {
  view: 'table';
  columns: string[];
  rows: string[][];
  more: number;
}
export type ParsedView = MetricView | ListView | SeriesView | TableView;

/** The output parsed for `view`, or why it does not fit. */
export function parseOutput(view: View, output: unknown): ParsedView | { error: string } {
  switch (view) {
    case 'metric':
      return parseMetric(output);
    case 'list':
      return parseList(output);
    case 'series':
      return parseSeries(output);
    case 'table':
      return parseTable(output);
  }
}

const misfit = (view: string) => ({ error: `The transform’s output does not fit the ${view} view.` });

function parseMetric(o: unknown): MetricView | { error: string } {
  const src = isObj(o) ? o : { value: o };
  const value = scalar(src.value);
  if (value === null) return misfit('metric');
  const out: MetricView = { view: 'metric', value: Array.from(value).length > 40 ? clean(value, 40) : value };
  const label = opt(src.label, 80);
  const unit = opt(src.unit, 16);
  const hint = opt(src.hint, MAX_TEXT);
  if (label) out.label = label;
  if (unit) out.unit = unit;
  if (hint) out.hint = hint;
  if (typeof src.delta === 'number' && Number.isFinite(src.delta)) {
    out.delta = (src.delta > 0 ? '+' : '') + formatNumber(src.delta);
    out.trend = src.delta > 0 ? 'up' : src.delta < 0 ? 'down' : 'flat';
  }
  return out;
}

function parseList(o: unknown): ListView | { error: string } {
  const arr = Array.isArray(o) ? o : isObj(o) && Array.isArray(o.items) ? o.items : null;
  if (!arr) return misfit('list');
  const items: ListView['items'] = [];
  for (const it of arr.slice(0, MAX_LIST_ITEMS)) {
    if (isObj(it)) {
      const label = scalar(it.label);
      if (label === null) continue;
      const row: ListView['items'][number] = { label };
      const value = scalar(it.value);
      if (value !== null) row.value = clean(value, 40);
      const hint = opt(it.hint);
      if (hint) row.hint = hint;
      items.push(row);
    } else {
      const label = scalar(it);
      if (label !== null) items.push({ label });
    }
  }
  return { view: 'list', items, more: Math.max(0, arr.length - MAX_LIST_ITEMS) };
}

function parseSeries(o: unknown): SeriesView | { error: string } {
  const src = Array.isArray(o) ? { points: o } : isObj(o) ? o : null;
  if (!src || !Array.isArray(src.points)) return misfit('series');
  const raw = src.points as unknown[];
  const points: SeriesView['points'] = [];
  raw.slice(0, MAX_POINTS).forEach((p, i) => {
    if (typeof p === 'number' && Number.isFinite(p)) points.push({ x: String(i + 1), y: p });
    else if (isObj(p) && typeof p.y === 'number' && Number.isFinite(p.y)) points.push({ x: scalar(p.x) ?? String(i + 1), y: p.y });
  });
  const first = points[0];
  const last = points[points.length - 1];
  if (!first || !last) return misfit('series');
  let min = Math.min(...points.map((p) => p.y));
  let max = Math.max(...points.map((p) => p.y));
  if (min > 0) min = 0;
  if (max === min) max = min + 1;
  const out: SeriesView = { view: 'series', points, firstX: first.x, lastX: last.x, lastY: formatNumber(last.y), min, max, more: Math.max(0, raw.length - MAX_POINTS) };
  const label = opt(src.label, 80);
  const unit = opt(src.unit, 16);
  if (label) out.label = label;
  if (unit) out.unit = unit;
  return out;
}

function parseTable(o: unknown): TableView | { error: string } {
  if (!isObj(o) || !Array.isArray(o.rows)) return misfit('table');
  const rawRows = o.rows as unknown[];
  let columns: string[];
  if (Array.isArray(o.columns)) columns = (o.columns as unknown[]).slice(0, MAX_COLUMNS).map((c) => clean(scalar(c) ?? '', 40));
  else {
    const first = rawRows.find((r) => Array.isArray(r)) as unknown[] | undefined;
    columns = Array.from({ length: Math.min(first?.length ?? 0, MAX_COLUMNS) }, () => '');
  }
  if (columns.length === 0) return misfit('table');
  const rows: string[][] = [];
  for (const r of rawRows.slice(0, MAX_ROWS)) {
    if (!Array.isArray(r)) continue;
    rows.push(columns.map((_, i) => clean(scalar(r[i]) ?? '', 80)));
  }
  return { view: 'table', columns, rows, more: Math.max(0, rawRows.length - MAX_ROWS) };
}

/** The chrome's trust label. */
export function trustLabel(trust: string): string {
  return trust === 'dev' ? 'DEV' : trust.replace('_', ' ');
}

/** What the card says when the transform cannot run. */
export function unavailableTransformText(state: 'unavailable' | 'locked' | 'browser' | 'failed', reason = ''): { title: string; hint: string } {
  switch (state) {
    case 'browser':
      return { title: 'Computed widget', hint: 'Pack transforms run only in the desktop app.' };
    case 'locked':
      return { title: 'Computed widget', hint: 'Unlock the Hub to show this widget.' };
    case 'unavailable':
      return { title: 'Computed widget unavailable', hint: 'Its pack is not installed, is disabled, or is not trusted to run code.' };
    default:
      return { title: 'The transform failed', hint: failureHint(reason) };
  }
}

function failureHint(reason: string): string {
  switch (reason) {
    case 'timeout':
    case 'fuel_exhausted':
      return 'It took too long and was stopped.';
    case 'memory_limit':
      return 'It used too much memory and was stopped.';
    case 'schema_mismatch':
    case 'invalid_output':
      return 'Its output was not valid.';
    case 'input':
      return 'It could not read its inputs.';
    case 'internal':
      return 'The extension host is not running.';
    default:
      return 'The pack’s module did not run.';
  }
}
