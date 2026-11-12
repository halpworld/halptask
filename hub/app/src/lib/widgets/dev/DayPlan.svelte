<script lang="ts">
  import CalendarDays from '@lucide/svelte/icons/calendar-days';
  import CalendarClock from '@lucide/svelte/icons/calendar-clock';
  import Crosshair from '@lucide/svelte/icons/crosshair';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { query } from '../../stores/query.svelte';
  import { clock } from '../../stores/clock.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { formatTime } from '../../util/dates';
  import { listKeydown } from '../parts/listnav';
  import { ignoreKey, typedItems } from './actions';
  import { dayPlan, DEV_TYPES, meetingsOf, minutesLabel, planTasks, type PlanBlock } from './logic';

  // Computed on the client only: meetings stay put, open tasks for today
  // (focus first) fill the free gaps. Nothing is written back.
  let { widget, ambient }: WidgetProps = $props();

  const HHMM = /^([01]\d|2[0-3]):[0-5]\d$/;
  const dayStart = $derived(typeof widget.config.day_start === 'string' && HHMM.test(widget.config.day_start) ? widget.config.day_start : '09:00');
  const dayEnd = $derived(typeof widget.config.day_end === 'string' && HHMM.test(widget.config.day_end) ? widget.config.day_end : '17:30');
  const defaultMinutes = $derived(typeof widget.config.default_minutes === 'number' && widget.config.default_minutes >= 5 && widget.config.default_minutes <= 240 ? widget.config.default_minutes : 30);

  const today = $derived(hub.day?.date ?? '');
  const now = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const meetingsQ = typedItems(DEV_TYPES.meeting);
  const tasksQ = query(
    () => hub.dispatch('tasks.list', { status: ['todo', 'in_progress'], role: hub.roleId, limit: 1000 }),
    () => [hub.itemsRev, hub.roleId],
  );
  const plan = $derived(
    today
      ? dayPlan({
          date: today,
          now,
          meetings: meetingsOf(meetingsQ.data?.items ?? []),
          tasks: planTasks(tasksQ.data?.items ?? [], today),
          focusId: hub.day?.focus?.id ?? null,
          dayStart,
          dayEnd,
          defaultMinutes,
        })
      : null,
  );
  const placed = $derived(plan ? plan.blocks.filter((b) => b.kind === 'task').length : 0);
  const meetingsToday = $derived(plan ? plan.blocks.filter((b) => b.kind === 'meeting').length : 0);
  // Index of the first block that has not ended: the "now" line goes above it.
  const nowIndex = $derived(plan ? plan.blocks.findIndex((b) => b.end > now) : -1);

  /** Height follows duration, within calm bounds. */
  function rows(b: PlanBlock): string {
    const min = (b.end - b.start) / 60_000;
    return `${Math.round(Math.max(26, Math.min(96, 14 + min * 0.7)))}px`;
  }
  function label(b: PlanBlock): string {
    if (b.kind === 'free') return 'Free';
    return b.item?.title ?? '';
  }
  function onKey(e: KeyboardEvent, b: PlanBlock) {
    if (ambient || ignoreKey(e) || b.kind !== 'task' || !b.item) return;
    if (e.key === 'f' || e.key === 'Enter') {
      e.preventDefault();
      void hub.run('focus.set', { id: b.item.id });
    } else if (e.key === 'x') {
      e.preventDefault();
      void hub.complete(b.item);
    }
  }
</script>

{#if (meetingsQ.loading && !meetingsQ.data) || (tasksQ.loading && !tasksQ.data) || !plan}
  <Skeleton lines={7} />
{:else if (meetingsQ.error && !meetingsQ.data) || (tasksQ.error && !tasksQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={meetingsQ.error ?? tasksQ.error ?? undefined} guide={false} />
{:else if placed === 0 && meetingsToday === 0}
  <EmptyState compact icon={CalendarDays} title="An open day" hint="Meetings come from your calendar; tasks due today fill the gaps between them." />
{:else}
  <p class="summary">
    <span><span class="num">{placed}</span> planned</span>
    <span>· <span class="num">{minutesLabel(plan.freeMs)}</span> free</span>
    {#if plan.unplaced.length}<span class="over">· <span class="num">{plan.unplaced.length}</span> won’t fit</span>{/if}
  </p>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ol class="plan" onkeydown={listKeydown} aria-label="Today, {dayStart} to {dayEnd}">
    {#each plan.blocks as b, i (`${b.kind}-${b.item?.id ?? ''}-${b.start}`)}
      {#if i === nowIndex}<li class="now" aria-label="Now, {formatTime(new Date(now))}"><span class="num">{formatTime(new Date(now))}</span></li>{/if}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="block {b.kind}" class:past={b.past} class:focus={b.focus} class:current={b.start <= now && now < b.end} style:min-height={rows(b)} data-row onkeydown={(e) => onKey(e, b)}>
        <span class="time num">{formatTime(new Date(b.start))}</span>
        <span class="body">
          {#if b.kind === 'task' && !ambient}
            <button class="title" data-row-focus aria-label="{label(b)}, {formatTime(new Date(b.start))} to {formatTime(new Date(b.end))}{b.focus ? ', in focus' : ''}. Press f to focus, x to complete" onclick={() => b.item && hub.run('focus.set', { id: b.item.id })}>
              {#if b.focus}<Crosshair size={12} aria-hidden="true" />{/if}<span class="pii">{label(b)}</span>
            </button>
          {:else}
            <span class="title">
              {#if b.kind === 'meeting'}<CalendarClock size={12} aria-hidden="true" />{:else if b.focus}<Crosshair size={12} aria-hidden="true" />{/if}<span class:pii={b.kind !== 'free'}>{label(b)}</span>
            </span>
          {/if}
          <span class="len num">{minutesLabel(b.end - b.start)}</span>
        </span>
      </li>
    {/each}
  </ol>
  {#if plan.unplaced.length}
    <p class="unplaced">
      <span class="label">Won’t fit today</span>
      <span class="pii">{plan.unplaced.map((t) => t.title).join(' · ')}</span>
    </p>
  {/if}
{/if}

<style>
  .summary {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    margin: 0 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .over {
    color: var(--warning);
  }
  .plan {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .block {
    display: grid;
    grid-template-columns: 3.2em 1fr;
    gap: var(--space-2);
    align-items: stretch;
  }
  .time {
    padding-top: 4px;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .body {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    gap: 2px;
    min-width: 0;
    padding: 4px 8px;
    border-radius: var(--radius-md);
    border-left: 3px solid transparent;
  }
  .meeting .body {
    background: var(--surface-3);
    border-left-color: var(--text-3);
  }
  .task .body {
    background: var(--accent-soft);
    border-left-color: var(--accent);
  }
  .task.focus .body {
    background: color-mix(in oklab, var(--accent-soft) 70%, var(--accent) 12%);
  }
  .free .body {
    border: 1px dashed var(--border);
    border-left-width: 1px;
    color: var(--text-3);
  }
  .past {
    opacity: 0.5;
  }
  .current .body {
    box-shadow: 0 0 0 1px var(--accent);
  }
  .title {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    text-align: left;
    line-height: var(--leading-snug);
    overflow: hidden;
  }
  button.title {
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .title > span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .free .title {
    font-weight: var(--weight-regular);
  }
  .len {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .now {
    position: relative;
    height: 14px;
    display: flex;
    align-items: center;
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    color: var(--danger);
  }
  .now::after {
    content: '';
    flex: 1;
    height: 1.5px;
    margin-left: 6px;
    background: var(--danger);
    border-radius: 1px;
  }
  .unplaced {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: var(--space-3) 0 0;
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
</style>
