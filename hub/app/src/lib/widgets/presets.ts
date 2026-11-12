import type { Item, TaskFilter, Widget } from '../api/types';
import { addDays, endOfWeek } from '../util/dates';

export type Preset = 'today' | 'overdue' | 'open' | 'in_progress' | 'week' | 'all';

/** Resolve a widget's config into a contract `tasks.list` filter. */
export function filterFor(config: Widget['config'], today: string, role?: string): TaskFilter {
  const base: TaskFilter = role ? { role } : {};
  const raw = config.filter;
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) Object.assign(base, raw as TaskFilter);
  if (typeof config.tag === 'string') base.tag = config.tag;
  const preset = (typeof config.preset === 'string' ? config.preset : 'open') as Preset;
  switch (preset) {
    case 'today':
      return today ? { ...base, due_before: addDays(today, 1) } : base;
    case 'overdue':
      return today ? { ...base, due_before: today, status: ['todo', 'in_progress'] } : { ...base, status: ['todo', 'in_progress'] };
    case 'in_progress':
      return { ...base, status: 'in_progress' };
    case 'week':
      return today ? { ...base, due_before: addDays(endOfWeek(today), 1), status: ['todo', 'in_progress'] } : { ...base, status: ['todo', 'in_progress'] };
    case 'all':
      return base;
    case 'open':
    default:
      return { ...base, status: ['todo', 'in_progress'] };
  }
}

export interface Row {
  item: Item;
  indent: boolean;
  sub?: { done: number; total: number };
}

/**
 * Nest children under parents that are also in view, sort open before done,
 * and today's done items stay visible (struck through) for a sense of progress.
 */
export function toRows(items: Item[], opts: { showDone: boolean; today: string }): Row[] {
  const visible = items.filter((i) => opts.showDone || i.status !== 'done');
  const ids = new Set(visible.map((i) => i.id));
  const children = new Map<string, Item[]>();
  for (const it of visible) {
    if (it.parent_id && ids.has(it.parent_id)) {
      const arr = children.get(it.parent_id) ?? [];
      arr.push(it);
      children.set(it.parent_id, arr);
    }
  }
  const allKids = new Map<string, Item[]>();
  for (const it of items) {
    if (it.parent_id) {
      const arr = allKids.get(it.parent_id) ?? [];
      arr.push(it);
      allKids.set(it.parent_id, arr);
    }
  }
  const rank = (i: Item) => (i.status === 'done' ? 2 : i.due && i.due < opts.today ? 0 : 1);
  const roots = visible.filter((i) => !i.parent_id || !ids.has(i.parent_id)).sort((a, b) => rank(a) - rank(b) || (a.due ?? '9').localeCompare(b.due ?? '9') || a.order - b.order);
  const rows: Row[] = [];
  for (const r of roots) {
    const kids = allKids.get(r.id) ?? [];
    rows.push({ item: r, indent: false, sub: kids.length ? { done: kids.filter((k) => k.status === 'done').length, total: kids.length } : undefined });
    for (const k of (children.get(r.id) ?? []).sort((a, b) => rank(a) - rank(b) || a.order - b.order)) rows.push({ item: k, indent: true });
  }
  return rows;
}
