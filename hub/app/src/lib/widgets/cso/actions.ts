// Shared data hooks and writes for the CSO widgets. Every write goes through
// hub.run (the command dispatcher), with if_version so a concurrent change by
// an integration or agent is refused rather than overwritten.

import type { Item, JsonValue, Widget } from '../../api/types';
import { hub } from '../../stores/hub.svelte';
import { toasts } from '../../design/toast.svelte';
import { briefStatusPatch, chasePatch, statusPatch, type BriefCheck, type BriefStatus, type DataRequest, type RequestStatus } from './logic';

export { ignoreKey, openLink, typedItems } from '../dev/actions';

type Fields = Record<string, JsonValue>;

function update(item: Item, fields: Fields) {
  return hub.run('tasks.update', { id: item.id, patch: { fields }, if_version: item.version });
}

/** Record one chase: chase_count + 1, last_chased = now. Nothing is sent. */
export async function chase(r: DataRequest, nowIso: string): Promise<boolean> {
  const fields = chasePatch(r.item.fields, nowIso);
  if (!fields) return false;
  const res = await update(r.item, fields);
  if (res) toasts.show(`Chase ${fields.chase_count} logged for ${r.supplier}. Nothing was sent.`, { tone: 'success', timeout: 3500 });
  return !!res;
}

export async function setRequestStatus(r: DataRequest, status: RequestStatus): Promise<boolean> {
  if (r.status === status) return false;
  return !!(await update(r.item, statusPatch(r.item.fields, status)));
}

/** Refuses "ready" / "presented" while any figure lacks a source (the core refuses it too). */
export async function setBriefStatus(check: BriefCheck, status: BriefStatus): Promise<boolean> {
  const fields = briefStatusPatch(check, status);
  if (!fields) {
    toasts.show(`${check.blocking} figure${check.blocking === 1 ? '' : 's'} need a source before this section can be ${status}.`, { tone: 'danger', timeout: 5000 });
    return false;
  }
  return !!(await update(check.b.item, fields));
}

export async function copyText(text: string, what: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
    toasts.show(`${what} copied.`, { tone: 'success', timeout: 2500 });
  } catch {
    toasts.show(`Could not copy. ${what}: ${text}`, { timeout: 9000 });
  }
}

/** Pin a source item as the role's focus, so it can be opened from Now. */
export async function focusItem(id: string): Promise<void> {
  const res = await hub.run('focus.set', { id });
  if (res) toasts.show(`Focused “${res.focus.title}”.`, { tone: 'success', timeout: 2500 });
}

export async function saveConfig(widgetId: string, config: Widget['config']): Promise<void> {
  const res = await hub.run('layout.update_widget', { widget_id: widgetId, patch: { config } });
  if (res) hub.layout = res;
}

/** All items of one role keyed by id, for resolving figure sources. */
export function byId(...lists: (Item[] | undefined)[]): Map<string, Item> {
  const m = new Map<string, Item>();
  for (const l of lists) for (const it of l ?? []) m.set(it.id, it);
  return m;
}
