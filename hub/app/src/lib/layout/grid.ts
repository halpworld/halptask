// Pure layout math for the 12-column widget grid. Shared by the grid
// component (drag preview) and the mock transport (authoritative result), so
// what you see while dragging is what the core returns.
//
// halp-domain/src/grid.rs is authoritative; hub/docs/fixtures/grid-cases.json
// pins the two together (tests/grid-fixtures.test.ts).
//
// Rules:
//  0. Requests are validated first (checkPosition/checkSize): integers only,
//     x < columns, y <= MAX_GRID_ROWS, 1 <= w <= columns, 1 <= h <= MAX_WIDGET_H.
//     Invalid input is refused, never clamped or rounded.
//  1. Positions are clamped into the grid: x + w <= columns, h <= MAX_WIDGET_H.
//  2. The widget being moved/resized wins; widgets it overlaps are pushed
//     down, recursively.
//  3. Everything is then compacted upward (gravity), in reading order.

export interface Box {
  id: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export const DEFAULT_COLUMNS = 12;
export const MIN_W = 2;
export const MIN_H = 2;
export const MAX_WIDGET_H = 24;
export const MAX_GRID_ROWS = 200;

/** A refused grid request; the core answers these with `invalid_args`. */
export class GridError extends Error {
  readonly code = 'invalid_args' as const;
}

const isCell = (n: unknown): n is number => typeof n === 'number' && Number.isInteger(n) && n >= 0 && n <= 0xffff_ffff;

/** Mirrors grid::check_position (plus the u32 deserialisation rules). */
export function checkPosition(x: unknown, y: unknown, columns = DEFAULT_COLUMNS): void {
  if (!isCell(x) || !isCell(y)) throw new GridError('x and y must be whole numbers >= 0');
  if (x >= columns) throw new GridError(`x must be below ${columns} (the column count)`);
  if (y > MAX_GRID_ROWS) throw new GridError(`y must be at most ${MAX_GRID_ROWS}`);
}

/** Mirrors grid::check_size (plus the u32 deserialisation rules). */
export function checkSize(w: unknown, h: unknown, columns = DEFAULT_COLUMNS): void {
  if (!isCell(w) || !isCell(h)) throw new GridError('w and h must be whole numbers >= 1');
  if (w < 1 || w > columns) throw new GridError(`w must be 1..=${columns}`);
  if (h < 1 || h > MAX_WIDGET_H) throw new GridError(`h must be 1..=${MAX_WIDGET_H}`);
}

/** Bytewise (UTF-8) order, like Rust's `String::cmp`. Code point order is the same thing. */
export function cmpBytes(a: string, b: string): number {
  const A = Array.from(a, (c) => c.codePointAt(0)!);
  const B = Array.from(b, (c) => c.codePointAt(0)!);
  for (let i = 0; i < Math.min(A.length, B.length); i++) if (A[i] !== B[i]) return A[i]! - B[i]!;
  return A.length - B.length;
}

export function collides(a: Box, b: Box): boolean {
  if (a.id === b.id) return false;
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y;
}

export function bottom(boxes: readonly Box[]): number {
  return boxes.reduce((m, b) => Math.max(m, b.y + b.h), 0);
}

export function clampBox<T extends Box>(box: T, columns = DEFAULT_COLUMNS): T {
  // No rounding: the core works in u32 and refuses fractions up front. The
  // max(0) guards only matter for drag previews, which may compute negatives.
  const w = Math.max(1, Math.min(columns, box.w));
  const h = Math.max(1, Math.min(MAX_WIDGET_H, box.h));
  const x = Math.max(0, Math.min(columns - w, box.x));
  const y = Math.max(0, box.y);
  return { ...box, x, y, w, h };
}

/** Reading order: top-to-bottom, then left-to-right. */
export function readingOrder<T extends Box>(boxes: readonly T[]): T[] {
  return [...boxes].sort((a, b) => a.y - b.y || a.x - b.x || cmpBytes(a.id, b.id));
}

/** Move each box as far up as it can go without overlapping earlier boxes. */
export function compact<T extends Box>(boxes: readonly T[], pinned?: string): T[] {
  const placed: T[] = [];
  const order = readingOrder(boxes);
  // A pinned box keeps its y so the user's drop target is respected when
  // there is room above it only because of the drag itself.
  const pin = pinned ? order.find((b) => b.id === pinned) : undefined;
  if (pin) placed.push({ ...pin });
  for (const box of order) {
    if (box.id === pinned) continue;
    const next = { ...box };
    while (next.y > 0 && !placed.some((p) => collides({ ...next, y: next.y - 1 }, p))) next.y -= 1;
    while (placed.some((p) => collides(next, p))) next.y += 1;
    placed.push(next);
  }
  // Return in the original input order so keyed lists stay stable.
  const byId = new Map(placed.map((b) => [b.id, b]));
  return boxes.map((b) => byId.get(b.id) ?? b);
}

/** Push anything overlapping `moved` downward, cascading. */
function pushDown<T extends Box>(boxes: T[], movedId: string): T[] {
  const result = boxes.map((b) => ({ ...b }));
  const queue = [movedId];
  let guard = 0;
  while (queue.length && guard++ < 10_000) {
    const id = queue.shift()!;
    const mover = result.find((b) => b.id === id);
    if (!mover) continue;
    for (const other of readingOrder(result)) {
      if (other.id === movedId || other.id === id) continue;
      if (collides(mover, other)) {
        other.y = mover.y + mover.h;
        queue.push(other.id);
      }
    }
  }
  return result;
}

function settle<T extends Box>(boxes: readonly T[], id: string, patch: Partial<Box>, columns: number): T[] {
  const target = boxes.find((b) => b.id === id);
  if (!target) return [...boxes];
  const next = clampBox({ ...target, ...patch, id } as T, columns);
  const replaced = boxes.map((b) => (b.id === id ? next : clampBox(b, columns)));
  return compactAround(pushDown(replaced, id), id);
}

/**
 * Compact everything, but give the moved box first claim to its spot: it
 * floats up only as far as its own column span is free.
 */
function compactAround<T extends Box>(boxes: T[], movedId: string): T[] {
  const others = boxes.filter((b) => b.id !== movedId);
  const moved = boxes.find((b) => b.id === movedId);
  if (!moved) return compact(boxes);
  // Float the moved box up over gaps that are empty in its columns.
  const m = { ...moved };
  while (m.y > 0 && !others.some((o) => collides({ ...m, y: m.y - 1 }, o))) m.y -= 1;
  const withMoved = boxes.map((b) => (b.id === movedId ? m : b));
  return compact(withMoved, movedId);
}

export function moveBox<T extends Box>(boxes: readonly T[], id: string, x: number, y: number, columns = DEFAULT_COLUMNS): T[] {
  return settle(boxes, id, { x, y }, columns);
}

export function resizeBox<T extends Box>(boxes: readonly T[], id: string, w: number, h: number, columns = DEFAULT_COLUMNS): T[] {
  return settle(boxes, id, { w: Math.max(MIN_W, w), h: Math.max(MIN_H, h) }, columns);
}

/** Validated move, as layout.move_widget does it. Throws GridError. */
export function checkedMove<T extends Box>(boxes: readonly T[], id: string, x: unknown, y: unknown, columns = DEFAULT_COLUMNS): T[] {
  checkPosition(x, y, columns);
  return moveBox(boxes, id, x as number, y as number, columns);
}

/** Validated resize, as layout.update_widget does it. Throws GridError. */
export function checkedResize<T extends Box>(boxes: readonly T[], id: string, w: unknown, h: unknown, columns = DEFAULT_COLUMNS): T[] {
  checkSize(w, h, columns);
  return resizeBox(boxes, id, w as number, h as number, columns);
}

/** Validated findSpace, as layout.add_widget does it. Throws GridError. */
export function checkedFindSpace(boxes: readonly Box[], w: unknown, h: unknown, columns = DEFAULT_COLUMNS): { x: number; y: number } {
  checkSize(w, h, columns);
  return findSpace(boxes, w as number, h as number, columns);
}

/** First free spot (reading order) that fits a w x h box. */
export function findSpace(boxes: readonly Box[], w: number, h: number, columns = DEFAULT_COLUMNS): { x: number; y: number } {
  const width = Math.min(columns, Math.max(1, w));
  const limit = bottom(boxes) + 1;
  for (let y = 0; y <= limit; y++) {
    for (let x = 0; x + width <= columns; x++) {
      const probe = { id: '\u0000probe', x, y, w: width, h };
      if (!boxes.some((b) => collides(probe, b))) return { x, y };
    }
  }
  return { x: 0, y: limit };
}

/** Single-column stacking for narrow windows. */
export function stack<T extends Box>(boxes: readonly T[], columns = DEFAULT_COLUMNS): T[] {
  let y = 0;
  const byId = new Map<string, T>();
  for (const b of readingOrder(boxes)) {
    byId.set(b.id, { ...b, x: 0, w: columns, y });
    y += b.h;
  }
  return boxes.map((b) => byId.get(b.id) ?? b);
}

export interface GridMetrics {
  columns: number;
  colWidth: number;
  rowHeight: number;
  gap: number;
}

export interface Rect {
  left: number;
  top: number;
  width: number;
  height: number;
}

export function boxToRect(b: Box, m: GridMetrics): Rect {
  return {
    left: b.x * (m.colWidth + m.gap),
    top: b.y * (m.rowHeight + m.gap),
    width: b.w * m.colWidth + (b.w - 1) * m.gap,
    height: b.h * m.rowHeight + (b.h - 1) * m.gap,
  };
}

/** Pixel delta to whole grid cells. */
export function pxToCells(dx: number, dy: number, m: GridMetrics): { dx: number; dy: number } {
  // `+ 0` normalises -0 so callers can compare with ===/Object.is safely.
  return { dx: Math.round(dx / (m.colWidth + m.gap)) + 0, dy: Math.round(dy / (m.rowHeight + m.gap)) + 0 };
}

export function metricsFor(width: number, height: number, rows: number, opts: { columns?: number; gap?: number; minRow?: number; maxRow?: number } = {}): GridMetrics {
  const columns = opts.columns ?? DEFAULT_COLUMNS;
  const gap = opts.gap ?? 16;
  const colWidth = Math.max(0, (width - gap * (columns - 1)) / columns);
  const fit = rows > 0 ? (height - gap * (rows - 1)) / rows : 80;
  const rowHeight = Math.max(opts.minRow ?? 52, Math.min(opts.maxRow ?? 140, fit));
  return { columns, colWidth, rowHeight, gap };
}

/** Ids whose geometry differs between two layouts (for "agent moved this" highlights). */
export function changedIds(before: readonly Box[], after: readonly Box[]): string[] {
  const prev = new Map(before.map((b) => [b.id, b]));
  return after
    .filter((b) => {
      const p = prev.get(b.id);
      return !p || p.x !== b.x || p.y !== b.y || p.w !== b.w || p.h !== b.h;
    })
    .map((b) => b.id);
}
