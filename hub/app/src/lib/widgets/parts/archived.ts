// Items an integration no longer sees remotely are tagged `archived` (and
// leaf tasks set done), never deleted (INTEGRATIONS.md, CONTRACT v1.5 §2).
// Every dev/ and em/ widget hides them by default; this is the one test.

import type { Item } from '../../api/types';

export const ARCHIVED_TAG = 'archived';

type Tagged = Pick<Item, 'tags'> & { inherited_tags?: readonly string[] };

/** Archived itself, or under an archived parent (the tag is inherited). */
export function isArchived(item: Tagged): boolean {
  return item.tags.includes(ARCHIVED_TAG) || !!item.inherited_tags?.includes(ARCHIVED_TAG);
}

export function hideArchived<T extends Tagged>(items: readonly T[]): T[] {
  return items.filter((i) => !isArchived(i));
}
