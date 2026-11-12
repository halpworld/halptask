// Which item ids are private, for redacting event summaries. Only the ids are
// kept: the notes themselves are dropped as soon as they arrive.

import { hub } from '../stores/hub.svelte';
import { PRIVATE_TYPE_IDS } from './private';

let ids = $state<ReadonlySet<string>>(new Set());
let lastRev = -1;
let inflight = false;

async function refresh(rev: number): Promise<void> {
  if (inflight || rev === lastRev || !hub.client) return;
  inflight = true;
  try {
    const next = new Set(ids);
    for (const type_id of PRIVATE_TYPE_IDS) {
      // A role without this type is fine: the list just comes back empty or fails.
      const res = await hub.client.dispatchRaw('tasks.list', { type_id, limit: 1000 });
      if (res.ok) for (const it of (res.result as { items: { id: string }[] }).items) next.add(it.id);
    }
    // Ids are only ever added: an event about a deleted note stays redacted.
    if (next.size !== ids.size) ids = next;
  } catch {
    // Not connected or no such type: nothing to redact yet.
  } finally {
    lastRev = rev;
    inflight = false;
  }
  // Items changed while we were reading: read again.
  if (hub.itemsRev !== lastRev) void refresh(hub.itemsRev);
}

/**
 * The set of private item ids, kept fresh as items change. Call during
 * component initialisation.
 */
export function privateIds(): { readonly ids: ReadonlySet<string> } {
  $effect(() => {
    const rev = hub.itemsRev;
    if (hub.status === 'ready') void refresh(rev);
  });
  return {
    get ids() {
      return ids;
    },
  };
}

/** Record a private item the moment it is created, before the next refresh. */
export function markPrivate(id: string): void {
  if (!ids.has(id)) ids = new Set([...ids, id]);
}
