<script lang="ts">
  import ChevronLeft from '@lucide/svelte/icons/chevron-left';
  import ChevronRight from '@lucide/svelte/icons/chevron-right';
  import type { Item, ItemStatus } from '../api/types';
  import type { WidgetProps } from './types';
  import { hub } from '../stores/hub.svelte';
  import { query } from '../stores/query.svelte';
  import { relativeDue } from '../util/dates';
  import Skeleton from '../design/Skeleton.svelte';
  import { toasts } from '../design/toast.svelte';

  let { widget, ambient }: WidgetProps = $props();

  const COLUMNS: { status: ItemStatus; label: string }[] = [
    { status: 'todo', label: 'To do' },
    { status: 'in_progress', label: 'Doing' },
    { status: 'done', label: 'Done' },
  ];

  const tag = $derived(typeof widget.config.tag === 'string' ? widget.config.tag : undefined);
  const typeId = $derived(typeof widget.config.type_id === 'string' ? widget.config.type_id : undefined);
  const q = query(
    () => hub.dispatch('tasks.list', { kind: 'task', role: hub.roleId, ...(tag ? { tag } : {}), ...(typeId ? { type_id: typeId } : {}) }),
    () => [hub.itemsRev, tag, typeId, hub.roleId],
  );
  const today = $derived(hub.day?.date ?? '');

  // Optimistic status while a move is in flight, so cards don't flicker back.
  let optimistic = $state<Record<string, ItemStatus>>({});
  const items = $derived((q.data?.items ?? []).filter((i) => !i.parent_id).map((i) => (optimistic[i.id] ? { ...i, status: optimistic[i.id]! } : i)));
  const byStatus = (s: ItemStatus): Item[] => {
    const list = items.filter((i) => i.status === s);
    return s === 'done' ? list.slice(-4) : list;
  };

  let dragId = $state<string | null>(null);
  let over = $state<ItemStatus | null>(null);

  // A card with subtasks takes its status from them (the core refuses a
  // status patch), so only Done/To do apply, as a confirmed cascade.
  async function moveTo(item: Item, status: ItemStatus) {
    if (item.status === status) return;
    const kids = await hub.run('tasks.list', { parent_id: item.id, limit: 1 });
    if (!kids) return;
    if (kids.items.length > 0) {
      if (status === 'in_progress') {
        toasts.show('This card follows its subtasks. Start one of them instead.');
        return;
      }
      await hub.complete(item, status === 'done');
      return;
    }
    optimistic = { ...optimistic, [item.id]: status };
    await hub.run('tasks.update', { id: item.id, patch: { status } });
    const { [item.id]: _drop, ...rest } = optimistic;
    optimistic = rest;
  }
  function step(item: Item, dir: -1 | 1) {
    const idx = COLUMNS.findIndex((c) => c.status === item.status);
    const next = COLUMNS[idx + dir];
    if (next) void moveTo(item, next.status);
  }
  function onDrop(status: ItemStatus) {
    const it = items.find((i) => i.id === dragId);
    dragId = null;
    over = null;
    if (it) void moveTo(it, status);
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={4} />
{:else}
  <div class="board">
    {#each COLUMNS as col, ci (col.status)}
      {@const cards = byStatus(col.status)}
      <section
        class="col"
        class:over={over === col.status}
        aria-label="{col.label}, {cards.length} cards"
        ondragover={(e) => {
          e.preventDefault();
          over = col.status;
        }}
        ondragleave={() => (over = over === col.status ? null : over)}
        ondrop={(e) => {
          e.preventDefault();
          onDrop(col.status);
        }}
      >
        <h3 class="label">{col.label}<span class="n num">{cards.length}</span></h3>
        <ul>
          {#each cards as it (it.id)}
            <li
              class="card"
              class:done={it.status === 'done'}
              class:dragging={dragId === it.id}
              draggable={!ambient}
              ondragstart={(e) => {
                dragId = it.id;
                e.dataTransfer?.setData('text/plain', it.id);
              }}
              ondragend={() => (dragId = null)}
            >
              <span class="t pii">{it.title}</span>
              <span class="foot">
                {#if it.due && today}<span class="due num" class:late={it.due < today && it.status !== 'done'}>{relativeDue(it.due, today)}</span>{/if}
                {#if !ambient}
                  <span class="moves">
                    <button aria-label="Move “{it.title}” to {COLUMNS[ci - 1]?.label}" disabled={ci === 0} onclick={() => step(it, -1)}><ChevronLeft size={13} aria-hidden="true" /></button>
                    <button aria-label="Move “{it.title}” to {COLUMNS[ci + 1]?.label}" disabled={ci === COLUMNS.length - 1} onclick={() => step(it, 1)}><ChevronRight size={13} aria-hidden="true" /></button>
                  </span>
                {/if}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}

<style>
  .board {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: var(--space-3);
    height: 100%;
  }
  .col {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    border: 1px dashed transparent;
    transition:
      border-color var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
  }
  .col.over {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-1) var(--space-2);
  }
  .n {
    margin-left: auto;
    font-weight: var(--weight-medium);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    overflow: auto;
    min-height: 0;
  }
  .card {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-sm);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--elev-1);
    font-size: var(--text-sm);
    cursor: grab;
    transition:
      transform var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out),
      opacity var(--dur-fast) var(--ease-out);
  }
  .card:hover {
    box-shadow: var(--elev-2);
  }
  .card.dragging {
    opacity: 0.4;
  }
  .card.done .t {
    color: var(--text-3);
    text-decoration: line-through;
  }
  .t {
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    line-height: var(--leading-snug);
  }
  .foot {
    display: flex;
    align-items: center;
    margin-top: 4px;
    min-height: 20px;
  }
  .due {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .due.late {
    color: var(--danger);
  }
  .moves {
    margin-left: auto;
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .card:hover .moves,
  .card:focus-within .moves {
    opacity: 1;
  }
  .moves button {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border: 0;
    border-radius: var(--radius-xs);
    background: transparent;
    color: var(--text-3);
  }
  .moves button:hover:not(:disabled) {
    background: var(--surface-3);
    color: var(--text);
  }
  .moves button:disabled {
    opacity: 0.3;
  }
</style>
