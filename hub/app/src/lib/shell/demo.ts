// Mock-mode demo: act as an agent (agent:claude) against the MockTransport so
// the activity strip, agent glow, Undo and the consent flow can be tried in a
// plain browser. Not bundled into behaviour of the Tauri build: in a Tauri
// window `__halpMock` is never set and these return false.

import type { Args, HubResponse, Item, Layout } from '../api/types';

interface AgentHandle {
  dispatch(command: string, args?: Args): Promise<HubResponse>;
}
interface MockLike {
  agent(client?: string): AgentHandle;
}

function mock(): MockLike | null {
  const m = (globalThis as { __halpMock?: MockLike }).__halpMock;
  return m && typeof m.agent === 'function' ? m : null;
}

export function demoAvailable(): boolean {
  return mock() !== null;
}

async function openItems(agent: AgentHandle, roleId: string): Promise<Item[]> {
  const res = await agent.dispatch('tasks.list', { status: ['todo', 'in_progress'], role: roleId, parent_id: null });
  return res.ok ? (res.result as { items: Item[] }).items : [];
}

/** Agent swaps the two widgets on the top row (or moves the first one down). */
export async function agentRearrange(): Promise<boolean> {
  const m = mock();
  if (!m) return false;
  const agent = m.agent('claude');
  const res = await agent.dispatch('layout.get', {});
  if (!res.ok) return false;
  const layout = res.result as Layout;
  const sorted = [...layout.widgets].sort((a, b) => a.y - b.y || a.x - b.x);
  const a = sorted[0];
  const b = sorted[1];
  if (!a || !b) return false;
  if (a.y === b.y && a.w === b.w) {
    await agent.dispatch('layout.move_widget', { widget_id: b.id, x: a.x, y: a.y });
  } else {
    await agent.dispatch('layout.move_widget', { widget_id: a.id, x: a.x, y: a.y + b.h });
  }
  return true;
}

/** Agent completes the first open task of the current role. */
export async function agentComplete(roleId: string): Promise<boolean> {
  const m = mock();
  if (!m) return false;
  const agent = m.agent('claude');
  const item = (await openItems(agent, roleId)).find((i) => i.kind === 'task');
  if (!item) return false;
  // cascade so a group (task with subtasks) completes too; harmless on a leaf.
  await agent.dispatch('tasks.complete', { id: item.id, cascade: true });
  return true;
}

/** Agent asks to delete a task: T3, so the consent dialog appears. */
export async function agentRequestDelete(roleId: string): Promise<boolean> {
  const m = mock();
  if (!m) return false;
  const agent = m.agent('claude');
  const items = await openItems(agent, roleId);
  const item = items[items.length - 1];
  if (!item) return false;
  // Resolves only after the human decides.
  void agent.dispatch('tasks.delete', { id: item.id });
  return true;
}

/**
 * Agent asks to delete a task, then adds a subtask under it while the
 * request waits. Approving then finds the request changed since it was
 * shown: the core (and the mock) refuse that approval and ask again under
 * a new id, which the dialog shows in place (CONTRACT v1.4).
 */
export async function agentRequestDeleteThenChange(roleId: string): Promise<boolean> {
  const m = mock();
  if (!m) return false;
  const agent = m.agent('claude');
  const item = (await openItems(agent, roleId)).find((i) => i.kind === 'task');
  if (!item) return false;
  void agent.dispatch('tasks.delete', { id: item.id });
  await agent.dispatch('tasks.create', { title: 'Added while the delete was waiting', parent_id: item.id });
  return true;
}
