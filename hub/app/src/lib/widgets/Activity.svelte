<script lang="ts">
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import History from '@lucide/svelte/icons/history';
  import type { HubEvent } from '../api/types';
  import type { WidgetProps } from './types';
  import { hub, actorName, isAgent } from '../stores/hub.svelte';
  import { clock } from '../stores/clock.svelte';
  import { relativeTime } from '../util/dates';
  import EmptyState from '../design/EmptyState.svelte';
  import ActorAvatar from './parts/ActorAvatar.svelte';
  import { eventSummary } from '../privacy/private';
  import { privateIds } from '../privacy/index.svelte';

  let { widget, ambient }: WidgetProps = $props();

  let onlyAgents = $state(false);
  $effect(() => {
    onlyAgents = widget.config.only_agents === true;
  });
  const NON_UNDOABLE = /^(history\.|consent\.)/;
  const events = $derived(hub.events.filter((e) => (!onlyAgents || isAgent(e.actor)) && e.command !== 'consent.requested').slice(0, 30));
  const undoable = (e: HubEvent) => !NON_UNDOABLE.test(e.command);
  // Private notes never show up here by content.
  const priv = privateIds();
</script>

{#if !ambient}
  <div class="filters" role="group" aria-label="Filter activity">
    <button aria-pressed={!onlyAgents} onclick={() => (onlyAgents = false)}>All</button>
    <button aria-pressed={onlyAgents} onclick={() => (onlyAgents = true)}>Agents</button>
  </div>
{/if}
{#if events.length === 0}
  <EmptyState compact icon={History} title="No activity yet" />
{:else}
  <ol class="feed" aria-label="Recent activity">
    {#each events as e (e.id)}
      <li class:agent={isAgent(e.actor)}>
        <ActorAvatar actor={e.actor} />
        <span class="txt">
          <span class="sum pii">{eventSummary(e, priv.ids)}</span>
          <span class="meta"><span>{actorName(e.actor)}</span> · <time datetime={e.ts}>{relativeTime(e.ts, clock.now)}</time></span>
        </span>
        {#if !ambient && undoable(e)}
          <button class="undo" aria-label="Undo: {eventSummary(e, priv.ids)}" title="Undo" onclick={() => hub.undo(e.id)}><Undo2 size={13} aria-hidden="true" /></button>
        {/if}
      </li>
    {/each}
  </ol>
{/if}

<style>
  .filters {
    display: inline-flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    margin-bottom: var(--space-2);
  }
  .filters button {
    border: 0;
    background: transparent;
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-3);
    padding: 3px 10px;
    border-radius: var(--radius-sm);
  }
  .filters button[aria-pressed='true'] {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .feed {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  li {
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    animation: enter var(--dur-slow) var(--ease-out);
  }
  @keyframes enter {
    from {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
  }
  .sum {
    font-size: var(--text-sm);
    line-height: var(--leading-snug);
    overflow-wrap: anywhere;
  }
  .meta {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .agent .meta span:first-child {
    color: var(--accent-text);
    font-weight: var(--weight-semibold);
  }
  .undo {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-3);
    opacity: 0;
  }
  li:hover .undo,
  .undo:focus-visible {
    opacity: 1;
  }
  .undo:hover {
    background: var(--surface-3);
    color: var(--text);
  }
</style>
