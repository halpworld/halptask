// Shared actions and data hooks for the developer widgets.
//
// Opening links: the shell has no openExternal hook (src/lib/api/shell.ts),
// and the webview may not navigate away from the app (nav.rs). So "open"
// copies the https link to the clipboard and says so. It only ever runs from
// a click or a keypress, and safeUrl() refuses anything that is not https.

import type { Item } from '../../api/types';
import { hub } from '../../stores/hub.svelte';
import { query, type Query } from '../../stores/query.svelte';
import { toasts } from '../../design/toast.svelte';
import { safeUrl } from './logic';

export async function openLink(raw: unknown, what: string): Promise<void> {
  const url = safeUrl(raw);
  if (!url) {
    toasts.show(`${what} has no https link.`);
    return;
  }
  try {
    await navigator.clipboard.writeText(url);
    toasts.show(`Link to ${what} copied. Paste it in your browser.`, { tone: 'success', timeout: 3500 });
  } catch {
    toasts.show(`Could not copy. The link is ${url}`, { timeout: 9000 });
  }
}

/**
 * Items of one pack type in the active role, re-read whenever items change.
 * Call during component initialisation (it sets up an effect).
 */
export function typedItems(typeId: string): Query<{ items: Item[] }> {
  return query(
    () => hub.dispatch('tasks.list', { type_id: typeId, role: hub.roleId, limit: 500 }),
    () => [hub.itemsRev, hub.roleId],
  );
}

/** True when a key event should be left alone (modifiers, typing in a field). */
export function ignoreKey(e: KeyboardEvent): boolean {
  if (e.metaKey || e.ctrlKey || e.altKey) return true;
  const t = e.target as HTMLElement | null;
  return !!t && (t.tagName === 'INPUT' || t.tagName === 'TEXTAREA' || t.isContentEditable);
}
