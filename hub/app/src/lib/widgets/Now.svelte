<script lang="ts">
  import Crosshair from '@lucide/svelte/icons/crosshair';
  import Check from '@lucide/svelte/icons/check';
  import X from '@lucide/svelte/icons/x';
  import Bot from '@lucide/svelte/icons/bot';
  import type { WidgetProps } from './types';
  import { hub, actorName, isAgent } from '../stores/hub.svelte';
  import { query } from '../stores/query.svelte';
  import { clock } from '../stores/clock.svelte';
  import Button from '../design/Button.svelte';
  import Ring from '../design/Ring.svelte';
  import EmptyState from '../design/EmptyState.svelte';
  import Badge from '../design/Badge.svelte';
  import { relativeDue } from '../util/dates';

  let { ambient }: WidgetProps = $props();

  const focus = $derived(hub.day?.focus ?? null);
  const today = $derived(hub.day?.date ?? '');

  const kids = query(
    () => (focus ? hub.dispatch('tasks.list', { parent_id: focus.id }) : Promise.resolve({ items: [] })),
    () => [hub.itemsRev, focus?.id],
  );
  const sub = $derived.by(() => {
    const items = kids.data?.items ?? [];
    return { done: items.filter((i) => i.status === 'done').length, total: items.length, next: items.find((i) => i.status !== 'done') };
  });

  // When and by whom focus was set, from the event feed. The core only logs
  // focus.set when the focus actually changes, and the feed holds the latest
  // history page, so an older focus simply shows no "for" time or byline.
  const focusEvent = $derived(focus ? hub.events.find((e) => e.command === 'focus.set' && e.entity.id === focus.id) : undefined);
  const focusedFor = $derived.by(() => {
    if (!focusEvent) return '';
    const m = Math.max(0, Math.round((clock.now - new Date(focusEvent.ts).getTime()) / 60000));
    return m < 1 ? 'just now' : m < 60 ? `${m} min` : `${Math.floor(m / 60)}h ${m % 60}m`;
  });

  const candidates = query(
    () => hub.dispatch('tasks.list', { status: 'in_progress', role: hub.roleId, limit: 3 }),
    () => [hub.itemsRev, focus?.id, hub.roleId],
  );
  const suggestions = $derived((candidates.data?.items ?? []).slice(0, 3));

  async function complete() {
    if (!focus) return;
    if (await hub.complete(focus)) await hub.run('focus.clear', {});
  }
</script>

{#if focus}
  <div class="now" class:ambient>
    <div class="top">
      <span class="label">In focus{#if focusedFor}<span class="for"> · {focusedFor}</span>{/if}</span>
      {#if focusEvent && isAgent(focusEvent.actor)}
        <Badge tone="accent" icon={Bot} title="Set by {actorName(focusEvent.actor)}">by {actorName(focusEvent.actor)}</Badge>
      {/if}
      {#if focus.due && today}<Badge tone={focus.due < today ? 'danger' : 'neutral'}>due {relativeDue(focus.due, today)}</Badge>{/if}
    </div>
    <h3 class="title pii">{focus.title}</h3>
    {#if sub.next}
      <p class="next"><span class="label">Next</span> <span class="pii">{sub.next.title}</span></p>
    {:else if focus.notes}
      <p class="notes pii">{focus.notes}</p>
    {/if}
    <div class="bottom">
      {#if sub.total > 0}
        <Ring value={sub.done / sub.total} size={ambient ? 52 : 38} stroke={ambient ? 5 : 4} label="Subtasks done">
          <span class="num">{sub.done}/{sub.total}</span>
        </Ring>
      {/if}
      {#if !ambient}
        <span class="spacer"></span>
        <Button size="sm" variant="ghost" icon={X} onclick={() => hub.run('focus.clear', {})}>Clear</Button>
        <Button size="sm" variant="primary" icon={Check} onclick={complete}>Done</Button>
      {/if}
    </div>
  </div>
{:else}
  <EmptyState compact icon={Crosshair} title="Nothing in focus" hint={suggestions.length ? 'Pick one thing to protect.' : 'Press f on any task to focus it.'}>
    {#if suggestions.length && !ambient}
      <div class="sugg">
        {#each suggestions as s (s.id)}
          <button class="chip" onclick={() => hub.run('focus.set', { id: s.id })}><Crosshair size={12} aria-hidden="true" /><span class="pii truncate">{s.title}</span></button>
        {/each}
      </div>
    {/if}
  </EmptyState>
{/if}

<style>
  .now {
    container-type: size;
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .top .label {
    color: var(--accent-text);
  }
  .for {
    color: var(--text-3);
    font-weight: var(--weight-medium);
  }
  .title {
    font-size: clamp(var(--text-lg), 6.5cqi, var(--text-3xl));
    font-weight: var(--weight-semibold);
    line-height: var(--leading-tight);
    letter-spacing: var(--tracking-tight);
    color: var(--text);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
    text-wrap: balance;
    flex: none;
  }
  .ambient .title {
    font-size: clamp(var(--text-2xl), 7cqi, var(--text-5xl));
  }
  .next,
  .notes {
    color: var(--text-2);
    font-size: var(--text-sm);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .next,
  .notes {
    flex: 0 1 auto;
    min-height: 0;
  }
  /* Short frames: drop the secondary line rather than clip it mid-line. */
  @container (max-height: 180px) {
    .next,
    .notes {
      display: none;
    }
  }
  .next .label {
    margin-right: 4px;
  }
  .bottom {
    flex: none;
    margin-top: auto;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
  .sugg {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    width: 100%;
    max-width: 320px;
  }
  .chip {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    padding: 6px 10px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-2);
    font-size: var(--text-sm);
    text-align: left;
  }
  .chip:hover {
    border-color: var(--accent);
    color: var(--text);
  }
</style>
