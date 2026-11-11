<script lang="ts">
  import Play from '@lucide/svelte/icons/play';
  import Square from '@lucide/svelte/icons/square';
  import Plus from '@lucide/svelte/icons/plus';
  import CalendarClock from '@lucide/svelte/icons/calendar-clock';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import Button from '../../design/Button.svelte';
  import Ring from '../../design/Ring.svelte';
  import { typedItems } from './actions';
  import { clockLabel, DEV_TYPES, meetingsOf, meetingWindow, minutesLabel, readSession, sessionProgress, type FocusSession } from './logic';
  import { formatTime } from '../../util/dates';

  // The session lives in this widget's config (layout.update_widget), so it
  // survives a restart and an agent can see "in deep work until 11:20".
  let { widget, ambient }: WidgetProps = $props();

  const length = $derived.by(() => {
    const v = widget.config.minutes;
    return typeof v === 'number' && v >= 5 && v <= 240 ? Math.round(v) : 50;
  });
  const session = $derived(readSession(widget.config.session));
  const progress = $derived(session ? sessionProgress(session, clock.now) : null);
  const focus = $derived(hub.day?.focus ?? null);
  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);

  const meetingsQ = typedItems(DEV_TYPES.meeting);
  const win = $derived(meetingWindow(meetingsOf(meetingsQ.data?.items ?? []), minute));
  const untilNext = $derived(win.next ? win.next.start - minute : null);
  // A full session would run into the next meeting: offer one that fits.
  const fits = $derived(untilNext === null || untilNext >= length * 60_000);
  const fitMinutes = $derived(untilNext !== null ? Math.floor(untilNext / 60_000 / 5) * 5 : length);
  const endsAt = $derived(session && progress ? new Date(Date.parse(session.started_at) + session.minutes * 60_000) : null);
  const boundTitle = $derived.by(() => {
    if (!session?.item_id) return null;
    return focus && focus.id === session.item_id ? focus.title : null;
  });

  let saving = $state(false);
  async function save(next: FocusSession | null) {
    if (saving) return;
    saving = true;
    const { session: _drop, ...rest } = widget.config;
    const config = next ? { ...rest, session: { item_id: next.item_id, started_at: next.started_at, minutes: next.minutes } } : rest;
    const res = await hub.run('layout.update_widget', { widget_id: widget.id, patch: { config } });
    if (res) hub.layout = res;
    saving = false;
  }
  function start(minutes: number) {
    void save({ item_id: focus?.id ?? null, started_at: new Date().toISOString(), minutes });
  }
  function extend() {
    if (session) void save({ ...session, minutes: Math.min(480, session.minutes + 10) });
  }
</script>

<div class="fp" class:ambient>
  {#if session && progress}
    <div class="timer">
      <Ring value={progress.fraction} size={ambient ? 116 : 88} stroke={ambient ? 7 : 6} tone={progress.done ? 'success' : 'accent'} label="Deep work session progress">
        <span class="clock num" role="timer" aria-label="{minutesLabel(progress.remainingMs)} left">{progress.done ? 'Done' : clockLabel(progress.remainingMs)}</span>
      </Ring>
      <div class="info">
        <span class="label">{progress.done ? 'Session complete' : 'Deep work'}</span>
        {#if boundTitle}
          <span class="item pii">{boundTitle}</span>
        {:else if session.item_id}
          <span class="item muted">Focus has moved on</span>
        {:else}
          <span class="item muted">Not tied to a task</span>
        {/if}
        {#if endsAt && !progress.done}<span class="ends">until <span class="num">{formatTime(endsAt)}</span></span>{/if}
      </div>
    </div>
    <p class="visually-hidden" aria-live="polite">{progress.done ? 'Deep work session complete.' : ''}</p>
    {#if !ambient}
      <div class="actions">
        {#if progress.done}
          <Button size="sm" variant="ghost" disabled={saving} onclick={() => save(null)}>Dismiss</Button>
          <Button size="sm" variant="primary" icon={Play} disabled={saving} onclick={() => start(length)}>Again</Button>
        {:else}
          <Button size="sm" variant="ghost" icon={Plus} disabled={saving} onclick={extend} aria-label="Add 10 minutes">10 min</Button>
          <Button size="sm" variant="secondary" icon={Square} disabled={saving} onclick={() => save(null)}>Stop</Button>
        {/if}
      </div>
    {/if}
  {:else}
    <div class="idle">
      <span class="label">Ready for deep work</span>
      {#if focus}
        <span class="item pii">{focus.title}</span>
      {:else}
        <span class="item muted">No focus set. Press f on a task to pick one.</span>
      {/if}
    </div>
    {#if !ambient}
      <div class="actions">
        {#if fits}
          <Button size="sm" variant="primary" icon={Play} disabled={saving} onclick={() => start(length)}>Start {length} min</Button>
        {:else if fitMinutes >= 10}
          <Button size="sm" variant="primary" icon={Play} disabled={saving} onclick={() => start(fitMinutes)}>Start {fitMinutes} min</Button>
          <Button size="sm" variant="ghost" disabled={saving} onclick={() => start(length)}>{length} min</Button>
        {:else}
          <Button size="sm" variant="secondary" icon={Play} disabled={saving} onclick={() => start(length)}>Start {length} min</Button>
        {/if}
      </div>
    {/if}
  {/if}

  <p class="meeting" class:soon={untilNext !== null && untilNext < 15 * 60_000}>
    <CalendarClock size={13} aria-hidden="true" />
    {#if win.current}
      <span>In <span class="pii">{win.current.item.title}</span> until <span class="num">{formatTime(new Date(win.current.end))}</span></span>
    {:else if win.next && untilNext !== null}
      <span><span class="pii">{win.next.item.title}</span> in <span class="num">{minutesLabel(untilNext)}</span></span>
    {:else if meetingsQ.data}
      <span>No more meetings today{meetingsQ.data.items.length === 0 ? ' (connect a calendar)' : ''}</span>
    {/if}
  </p>
</div>

<style>
  .fp {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    height: 100%;
  }
  .timer {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .clock {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
  }
  .ambient .clock {
    font-size: var(--text-2xl);
  }
  .info,
  .idle {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }
  .item {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    line-height: var(--leading-snug);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .muted {
    color: var(--text-3);
    font-weight: var(--weight-regular);
  }
  .ends {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .actions {
    display: flex;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .meeting {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: auto 0 0;
    padding-top: var(--space-2);
    border-top: 1px solid var(--border);
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .meeting.soon {
    color: var(--warning);
  }
  .meeting :global(svg) {
    flex: none;
  }
</style>
