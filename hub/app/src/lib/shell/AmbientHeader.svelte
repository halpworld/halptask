<script lang="ts">
  import Minimize2 from '@lucide/svelte/icons/minimize-2';
  import { hub } from '../stores/hub.svelte';
  import { clock } from '../stores/clock.svelte';
  import { formatLongDate } from '../util/dates';
  import { roleMeta } from '../design/roles';

  const time = $derived(new Date(clock.now).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }));
  const meta = $derived(roleMeta(hub.roleId));
  const focus = $derived(hub.day?.focus ?? null);
</script>

<header class="ambient-head">
  <div class="left">
    <time class="clock num" datetime={new Date(clock.now).toISOString()}>{time}</time>
    <div class="meta">
      <span class="date">{hub.day ? formatLongDate(hub.day.date) : ''}</span>
      <span class="role">{hub.role?.title ?? ''} · {meta.tagline}</span>
    </div>
  </div>
  {#if focus}
    <div class="focus">
      <span class="label">Focus</span>
      <span class="ftitle pii">{focus.title}</span>
    </div>
  {/if}
  <button type="button" class="exit" onclick={() => hub.toggleAmbient()} aria-label="Leave ambient mode (A or Esc)">
    <Minimize2 size={16} aria-hidden="true" />
  </button>
</header>

<style>
  .ambient-head {
    display: flex;
    align-items: center;
    gap: var(--space-8);
    padding: var(--space-6) var(--space-2) var(--space-5);
  }
  .left {
    display: flex;
    align-items: baseline;
    gap: var(--space-5);
  }
  .clock {
    font-size: var(--text-5xl);
    font-weight: var(--weight-semibold);
    letter-spacing: -0.03em;
    line-height: 1;
  }
  .meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .date {
    font-size: var(--text-xl);
    font-weight: var(--weight-medium);
  }
  .role {
    color: var(--accent-text);
    font-size: var(--text-md);
    font-family: var(--font-label, var(--font-sans));
  }
  .focus {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-left: var(--space-6);
    border-left: 3px solid var(--accent);
  }
  .ftitle {
    font-size: var(--text-2xl);
    font-weight: var(--weight-semibold);
    line-height: var(--leading-snug);
  }
  .exit {
    margin-left: auto;
    display: grid;
    place-items: center;
    width: 40px;
    height: 40px;
    border-radius: 50%;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-3);
    opacity: 0.5;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .exit:hover,
  .exit:focus-visible {
    opacity: 1;
  }
</style>
