// Private data (data class `notes.private`): today only the engineering
// manager's private 1:1 notes (em.private_note).
//
// A pack marks a task type private with the schema annotation
// `"x-halp-data-class": "notes.private"`. The rules the UI keeps:
//   - private items render only in the one-on-one widget, behind an explicit
//     reveal, blurred until revealed;
//   - they never appear in search results, capture previews, the activity
//     feed or agent-strip summaries (summaries about them are replaced by a
//     neutral line);
//   - their title is always the neutral PRIVATE_TITLE and the body lives in
//     a field, so text search (title and notes only) never matches it.
// The core is expected to enforce the same for agents (see the P4 report);
// the mock already does.

import type { HubEvent, Item, JsonSchema } from '../api/types';

export const PRIVATE_DATA_CLASS = 'notes.private';
export const PRIVATE_SCHEMA_KEY = 'x-halp-data-class';
/** Types known to be private even before their schema is loaded. */
export const PRIVATE_TYPE_IDS: readonly string[] = ['em.private_note'];
/** The only title a private item is ever given. */
export const PRIVATE_TITLE = 'Private note';
/** What the activity feed and agent strip show for a private item's event. */
export const PRIVATE_SUMMARY = 'Changed a private note';

export function isPrivateSchema(schema: JsonSchema | null | undefined): boolean {
  return !!schema && typeof schema === 'object' && (schema as Record<string, unknown>)[PRIVATE_SCHEMA_KEY] === PRIVATE_DATA_CLASS;
}

export function isPrivateTypeId(typeId: string | null | undefined, extra: ReadonlySet<string> = new Set()): boolean {
  return !!typeId && (PRIVATE_TYPE_IDS.includes(typeId) || extra.has(typeId));
}

export function isPrivateItem(item: Pick<Item, 'type_id'>, extra?: ReadonlySet<string>): boolean {
  return isPrivateTypeId(item.type_id, extra);
}

/** Items safe to show in search, previews, feeds and summaries. */
export function withoutPrivate<T extends Pick<Item, 'type_id'>>(items: readonly T[], extra?: ReadonlySet<string>): T[] {
  return items.filter((i) => !isPrivateItem(i, extra));
}

/**
 * Search over items for a palette or picker: private items never match, and
 * only the title and notes are searched (never typed fields).
 */
export function searchItems<T extends Pick<Item, 'type_id' | 'title' | 'notes'>>(items: readonly T[], text: string, extra?: ReadonlySet<string>): T[] {
  const q = text.trim().toLowerCase();
  if (!q) return [];
  return withoutPrivate(items, extra).filter((i) => i.title.toLowerCase().includes(q) || i.notes.toLowerCase().includes(q));
}

/** The summary to display for an event: neutral when it is about a private item. */
export function eventSummary(e: Pick<HubEvent, 'entity' | 'summary'>, privateIds: ReadonlySet<string>): string {
  return e.entity.kind === 'item' && privateIds.has(e.entity.id) ? PRIVATE_SUMMARY : e.summary;
}

/** Events with private summaries replaced, for feeds and agent strips. */
export function redactEvents<T extends Pick<HubEvent, 'entity' | 'summary'>>(events: readonly T[], privateIds: ReadonlySet<string>): T[] {
  return events.map((e) => (e.entity.kind === 'item' && privateIds.has(e.entity.id) ? { ...e, summary: PRIVATE_SUMMARY } : e));
}
