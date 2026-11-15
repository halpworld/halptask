<script lang="ts">
  import CalendarCheck from '@lucide/svelte/icons/calendar-check';
  import type { Item } from '../api/types';
  import type { WidgetProps } from './types';
  import { hub } from '../stores/hub.svelte';
  import { query } from '../stores/query.svelte';
  import { addDays, parseISODate } from '../util/dates';
  import TaskRow from './parts/TaskRow.svelte';
  import { listKeydown } from './parts/listnav';
  import EmptyState from '../design/EmptyState.svelte';
  import Skeleton from '../design/Skeleton.svelte';

  let { widget, ambient }: WidgetProps = $props();

  const today = $derived(hub.day?.date ?? '');
  // config.days (the packs use {days: 7}); the core's due_before is exclusive.
  const days = $derived(typeof widget.config.days === 'number' && widget.config.days > 0 ? Math.min(Math.floor(widget.config.days), 60) : 7);
  const horizon = $derived(today ? addDays(today, days + 1) : '');
  const tag = $derived(typeof widget.config.tag === 'string' ? widget.config.tag : undefined);
  const q = query(
    () =>
      hub.dispatch('tasks.list', {
        status: ['todo', 'in_progress'],
        role: hub.roleId,
        ...(tag ? { tag } : {}),
        ...(horizon ? { due_before: horizon } : {}),
      }),
    () => [hub.itemsRev, tag, horizon, hub.roleId],
  );

  interface Bucket {
    key: string;
    label: string;
    tone: 'danger' | 'accent' | 'neutral';
    items: Item[];
  }

  const weekday = new Intl.DateTimeFormat(undefined, { weekday: 'long' });
  const buckets = $derived.by<Bucket[]>(() => {
    const items = (q.data?.items ?? []).filter((i) => i.due);
    const out: Bucket[] = [
      { key: 'overdue', label: 'Overdue', tone: 'danger', items: items.filter((i) => i.due! < today) },
      { key: 'today', label: 'Today', tone: 'accent', items: items.filter((i) => i.due === today) },
    ];
    const later = items.filter((i) => i.due! > today);
    const days = [...new Set(later.map((i) => i.due!))].sort();
    for (const d of days) {
      out.push({ key: d, label: d === addDays(today, 1) ? 'Tomorrow' : weekday.format(parseISODate(d)), tone: 'neutral', items: later.filter((i) => i.due === d) });
    }
    return out.filter((b) => b.items.length);
  });
</script>

{#if q.loading && !q.data}
  <Skeleton lines={6} />
{:else if buckets.length === 0}
  <EmptyState compact icon={CalendarCheck} title="Nothing due this week" hint="A clear runway. Enjoy it." />
{:else}
  <div class="agenda" onkeydown={listKeydown} role="presentation">
    {#each buckets as b (b.key)}
      <section class="bucket {b.tone}" aria-label={b.label}>
        <h3 class="label"><span class="pip"></span>{b.label}<span class="count num">{b.items.length}</span></h3>
        <ul>
          {#each b.items as it (it.id)}
            <TaskRow item={it} showDue={false} dense interactive={!ambient} />
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}

<style>
  .agenda {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: 2px;
  }
  .pip {
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--text-3);
  }
  .danger .pip {
    background: var(--danger);
  }
  .accent .pip {
    background: var(--accent);
  }
  .danger h3 {
    color: var(--danger);
  }
  .accent h3 {
    color: var(--accent-text);
  }
  .count {
    margin-left: auto;
    font-weight: var(--weight-medium);
    color: var(--text-3);
  }
  ul {
    margin: 0;
    padding: 0;
  }
</style>
