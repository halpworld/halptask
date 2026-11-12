import { describe, expect, it } from 'vitest';
import { boxToRect, bottom, changedIds, clampBox, collides, compact, findSpace, metricsFor, moveBox, pxToCells, resizeBox, stack, type Box } from '../src/lib/layout/grid';

const b = (id: string, x: number, y: number, w: number, h: number): Box => ({ id, x, y, w, h });
const overlaps = (boxes: Box[]) => boxes.some((a, i) => boxes.some((c, j) => i < j && collides(a, c)));

describe('grid math', () => {
  it('detects collisions only on real overlap', () => {
    expect(collides(b('a', 0, 0, 2, 2), b('b', 1, 1, 2, 2))).toBe(true);
    expect(collides(b('a', 0, 0, 2, 2), b('b', 2, 0, 2, 2))).toBe(false);
    expect(collides(b('a', 0, 0, 2, 2), b('a', 0, 0, 2, 2))).toBe(false); // same id
  });

  it('clamps boxes into the grid', () => {
    expect(clampBox(b('a', 11, -3, 4, 0), 12)).toMatchObject({ x: 8, y: 0, w: 4 });
    expect(clampBox(b('a', 0, 0, 20, 2), 12).w).toBe(12);
  });

  it('compacts upward and keeps input order', () => {
    const out = compact([b('a', 0, 5, 4, 2), b('b', 4, 9, 4, 2)]);
    expect(out.map((x) => x.id)).toEqual(['a', 'b']);
    expect(out.map((x) => x.y)).toEqual([0, 0]);
  });

  it('move pushes overlapped widgets down and never overlaps', () => {
    const start = [b('a', 0, 0, 6, 3), b('b', 6, 0, 6, 3), b('c', 0, 3, 12, 2)];
    const out = moveBox(start, 'c', 0, 0);
    expect(out.find((x) => x.id === 'c')).toMatchObject({ x: 0, y: 0 });
    expect(overlaps(out)).toBe(false);
    expect(out.find((x) => x.id === 'a')!.y).toBe(2);
  });

  it('move is clamped to the columns', () => {
    const out = moveBox([b('a', 0, 0, 4, 2)], 'a', 20, 0, 12);
    expect(out[0]).toMatchObject({ x: 8 });
  });

  it('resize respects minimum size and pushes neighbours', () => {
    const start = [b('a', 0, 0, 4, 2), b('b', 0, 2, 4, 2)];
    expect(resizeBox(start, 'a', 1, 1)[0]).toMatchObject({ w: 2, h: 2 });
    const grown = resizeBox(start, 'a', 4, 4);
    expect(grown.find((x) => x.id === 'b')!.y).toBe(4);
    expect(overlaps(grown)).toBe(false);
  });

  it('unknown id is a no-op', () => {
    const start = [b('a', 0, 0, 4, 2)];
    expect(moveBox(start, 'zzz', 3, 3)).toEqual(start);
  });

  it('finds free space in reading order', () => {
    expect(findSpace([b('a', 0, 0, 6, 2)], 6, 2)).toEqual({ x: 6, y: 0 });
    expect(findSpace([b('a', 0, 0, 12, 2)], 4, 2)).toEqual({ x: 0, y: 2 });
  });

  it('stacks into one column for narrow windows', () => {
    const out = stack([b('b', 6, 0, 6, 3), b('a', 0, 0, 6, 2)]);
    expect(out.find((x) => x.id === 'a')).toMatchObject({ x: 0, y: 0, w: 12 });
    expect(out.find((x) => x.id === 'b')).toMatchObject({ y: 2 });
    expect(bottom(out)).toBe(5);
  });

  it('converts between cells and pixels', () => {
    const m = metricsFor(12 * 50 + 11 * 10, 1000, 10, { gap: 10, minRow: 10, maxRow: 200 });
    expect(m.colWidth).toBe(50);
    expect(boxToRect(b('a', 1, 1, 2, 2), m)).toEqual({ left: 60, top: m.rowHeight + 10, width: 110, height: m.rowHeight * 2 + 10 });
    expect(pxToCells(64, -2, m)).toEqual({ dx: 1, dy: 0 });
  });

  it('row height is clamped between min and max', () => {
    expect(metricsFor(1200, 100, 20, { minRow: 44 }).rowHeight).toBe(44);
    expect(metricsFor(1200, 5000, 2, { maxRow: 140 }).rowHeight).toBe(140);
  });

  it('reports changed ids', () => {
    expect(changedIds([b('a', 0, 0, 2, 2), b('b', 2, 0, 2, 2)], [b('a', 0, 0, 2, 2), b('b', 4, 0, 2, 2), b('c', 0, 2, 2, 2)])).toEqual(['b', 'c']);
  });
});
