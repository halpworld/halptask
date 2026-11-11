<script lang="ts">
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import Bot from '@lucide/svelte/icons/bot';
  import { fly } from 'svelte/transition';
  import { hub, actorName } from '../stores/hub.svelte';
  import { clock } from '../stores/clock.svelte';
  import { relativeTime } from '../util/dates';
  import { motion } from '../design/motion.svelte';
  import { eventSummary } from '../privacy/private';
  import { privateIds } from '../privacy/index.svelte';

  // Agent changes are always visible and reversible: the last few are listed
  // here with a per-event Undo (history.undo with event_id).
  const recent = $derived(hub.agentEvents.filter((e) => !e.command.startsWith('consent.')).slice(0, 4));
  const working = $derived(hub.agentActiveUntil > clock.now);
  const pending = $derived(hub.consent.length);
  // Private notes never show up here by content.
  const priv = privateIds();
</script>

<footer class="strip" aria-label="Agent activity">
  <span class="who" class:working>
    <span class="pulse" aria-hidden="true"></span>
    <Bot size={15} aria-hidden="true" />
    <span class="state" aria-live="polite">{working ? 'Agent working…' : 'Agents'}</span>
  </span>

  {#if pending}
    <button type="button" class="pending" onclick={() => hub.consent[0] && (hub.ambient = false)}>
      {pending} waiting for your approval
    </button>
  {/if}

  <ol class="events">
    {#each recent as e (e.id)}
      <li in:fly={{ x: -12, duration: motion.ms(260) }}>
        <span class="actor">{actorName(e.actor)}</span>
        <span class="sum truncate pii">{eventSummary(e, priv.ids)}</span>
        <time class="t num" datetime={e.ts}>{relativeTime(e.ts, clock.now)}</time>
        <button type="button" class="undo" onclick={() => hub.undo(e.id)} aria-label="Undo: {eventSummary(e, priv.ids)}">
          <Undo2 size={13} aria-hidden="true" />Undo
        </button>
      </li>
    {:else}
      <li class="quiet">No agent changes yet. Anything an agent does shows up here, with Undo.</li>
    {/each}
  </ol>
</footer>

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    min-height: 40px;
    padding: 0 var(--space-5);
    border-top: 1px solid var(--border);
    background: var(--surface-2);
    font-size: var(--text-sm);
    overflow: hidden;
  }
  .who {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--text-2);
    font-weight: var(--weight-medium);
    flex: none;
  }
  .pulse {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--border-strong);
  }
  .working .pulse {
    background: var(--accent);
    animation: pulse 1.2s var(--ease-in-out) infinite;
  }
  .working {
    color: var(--accent-text);
  }
  @keyframes pulse {
    0%,
    100% {
      box-shadow: 0 0 0 0 var(--accent-glow);
    }
    50% {
      box-shadow: 0 0 0 5px var(--accent-glow);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .working .pulse {
      animation: none;
    }
  }
  .pending {
    flex: none;
    height: 26px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-full);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    background: var(--danger-soft);
    color: var(--danger);
    font-weight: var(--weight-semibold);
    font-size: var(--text-xs);
  }
  .events {
    display: flex;
    gap: var(--space-5);
    list-style: none;
    margin: 0;
    padding: 0;
    min-width: 0;
    flex: 1;
    overflow: hidden;
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-width: 0;
    max-width: 380px;
    flex: 0 1 auto;
  }
  li + li {
    padding-left: var(--space-5);
    border-left: 1px solid var(--border);
  }
  .actor {
    color: var(--accent-text);
    font-weight: var(--weight-semibold);
    flex: none;
  }
  .sum {
    color: var(--text-2);
  }
  .t {
    color: var(--text-3);
    font-size: var(--text-xs);
    flex: none;
  }
  .undo {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    flex: none;
    height: 24px;
    padding: 0 8px;
    border-radius: var(--radius-sm);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-2);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
  }
  .undo:hover {
    color: var(--text);
    border-color: var(--border-strong);
  }
  .quiet {
    color: var(--text-3);
  }
  @media (max-width: 760px) {
    li:not(:first-child) {
      display: none;
    }
  }
</style>
