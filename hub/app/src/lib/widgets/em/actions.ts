// Shared data hooks and actions for the engineering manager widgets.

import type { Item, Widget } from '../../api/types';
import { hub } from '../../stores/hub.svelte';
import { readPeople, type Person } from './logic';

export { ignoreKey, openLink, typedItems } from '../dev/actions';

export function peopleById(items: Item[] | undefined): Map<string, Person> {
  return new Map(readPeople(items ?? []).map((p) => [p.id, p]));
}

/** Persist one widget config key (like the focus protector's session). */
export async function saveConfig(widgetId: string, config: Widget['config']): Promise<void> {
  const res = await hub.run('layout.update_widget', { widget_id: widgetId, patch: { config } });
  if (res) hub.layout = res;
}
