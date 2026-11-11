<script lang="ts">
  import ListChecks from '@lucide/svelte/icons/list-checks';
  import type { WidgetProps } from './types';
  import { hub } from '../stores/hub.svelte';
  import { query } from '../stores/query.svelte';
  import { filterFor, toRows } from './presets';
  import TaskRow from './parts/TaskRow.svelte';
  import { listKeydown } from './parts/listnav';
  import EmptyState from '../design/EmptyState.svelte';
  import Skeleton from '../design/Skeleton.svelte';
  import Meter from '../design/Meter.svelte';

  let { widget, ambient }: WidgetProps = $props();

  const today = $derived(hub.day?.date ?? '');
  const filter = $derived(filterFor(widget.config, today, hub.roleId));
  const q = query(
    () => hub.dispatch('tasks.list', filter),
    () => [hub.itemsRev, JSON.stringify(filter)],
  );
  const showDone = $derived(widget.config.show_done !== false);
  const rows = $derived(q.data ? toRows(q.data.items, { showDone, today }) : []);
  const roots = $derived(rows.filter((r) => !r.indent));
  const doneCount = $derived(roots.filter((r) => r.item.status === 'done').length);
  const openCount = $derived(roots.length - doneCount);
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} height="14px" />
{:else if q.error}
  <EmptyState compact title="Couldn’t load tasks" hint={q.error} guide={false} />
{:else if rows.length === 0}
  <EmptyState compact icon={ListChecks} title="All clear" hint="Nothing here. Capture something with N." />
{:else}
  <div class="summary">
    <span class="num"><strong>{openCount}</strong> open{#if doneCount}<span class="sep">·</span>{doneCount} done{/if}</span>
    {#if doneCount + openCount > 0}
      <span class="m"><Meter value={doneCount} max={doneCount + openCount} label="Done today" warnAt={2} dangerAt={2} /></span>
    {/if}
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions (j/k roving focus between rows) -->
  <ul class="list" onkeydown={listKeydown} aria-label="{widget.title} tasks">
    {#each rows as r (r.item.id)}
      <TaskRow item={r.item} sub={r.sub} indent={r.indent} interactive={!ambient} />
    {/each}
  </ul>
{/if}

<style>
  .summary {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    color: var(--text-3);
    font-size: var(--text-xs);
    margin-bottom: var(--space-2);
  }
  .summary strong {
    color: var(--text);
    font-weight: var(--weight-semibold);
  }
  .sep {
    margin: 0 6px;
  }
  .m {
    flex: 1;
    max-width: 120px;
    margin-left: auto;
  }
  .list {
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
</style>
