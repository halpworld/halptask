<script lang="ts">
  import Crosshair from '@lucide/svelte/icons/crosshair';
  import Check from '@lucide/svelte/icons/check';
  import X from '@lucide/svelte/icons/x';
  import { hub } from '../stores/hub.svelte';
  import { formatLongDate } from '../util/dates';
  import IconButton from '../design/IconButton.svelte';

  const day = $derived(hub.day);
  const focus = $derived(day?.focus ?? null);
  const hasNowWidget = $derived(hub.layout?.widgets.some((w) => w.kind === 'now') ?? false);

  async function done() {
    if (!focus) return;
    if (await hub.complete(focus)) await hub.run('focus.clear', {});
  }
</script>

{#if day}
  <div class="dayhead">
    <div class="date">
      <h1>{formatLongDate(day.date)}</h1>
      <p class="counts" aria-label="Today at a glance">
        <span class:alert={day.counts.overdue > 0}><b class="num">{day.counts.overdue}</b> overdue</span>
        <span><b class="num">{day.counts.today}</b> due today</span>
        <span><b class="num">{day.counts.in_progress}</b> in progress</span>
      </p>
    </div>

    <!-- Focus banner: always visible, compact when the Now widget is on the board. -->
    <div class="focus" class:empty={!focus} class:slim={hasNowWidget} role="status" aria-live="polite">
      <Crosshair size={15} aria-hidden="true" />
      {#if focus}
        <span class="label">Focus</span>
        <span class="ftitle truncate pii">{focus.title}</span>
        <IconButton size="sm" icon={Check} label="Mark focus done" onclick={done} />
        <IconButton size="sm" icon={X} label="Clear focus" onclick={() => hub.run('focus.clear', {})} />
      {:else}
        <span class="none">No focus set. Press <b>f</b> on a task to focus it.</span>
      {/if}
    </div>
  </div>
{/if}

<style>
  .dayhead {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-5);
    padding: var(--space-4) var(--space-1) var(--space-3);
    flex-wrap: wrap;
  }
  h1 {
    font-size: var(--text-2xl);
    font-weight: var(--weight-bold);
    letter-spacing: var(--tracking-tight);
    line-height: var(--leading-tight);
  }
  .counts {
    display: flex;
    gap: var(--space-4);
    margin-top: 4px;
    color: var(--text-3);
    font-size: var(--text-sm);
  }
  .counts b {
    color: var(--text);
    font-weight: var(--weight-semibold);
  }
  .counts .alert,
  .counts .alert b {
    color: var(--danger);
  }
  .focus {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    max-width: min(560px, 100%);
    min-width: 0;
    height: 38px;
    padding: 0 6px 0 var(--space-3);
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-text);
    border: 1px solid color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .focus.empty {
    background: var(--surface-2);
    color: var(--text-3);
    border-color: var(--border);
    padding-right: var(--space-4);
  }
  .focus .label {
    color: var(--accent-text);
  }
  .ftitle {
    color: var(--text);
    font-weight: var(--weight-medium);
  }
  .none {
    font-size: var(--text-sm);
  }
  .none b {
    font-family: var(--font-mono);
  }
</style>
