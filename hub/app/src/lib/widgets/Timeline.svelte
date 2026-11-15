<script lang="ts">
  import type { WidgetProps } from './types';
  import { clock } from '../stores/clock.svelte';
  import EmptyState from '../design/EmptyState.svelte';

  let { widget }: WidgetProps = $props();

  interface Ev {
    start: string;
    end: string;
    title: string;
    s: number;
    e: number;
  }
  const toMin = (hhmm: string) => {
    const [h, m] = hhmm.split(':').map(Number);
    return (h ?? 0) * 60 + (m ?? 0);
  };
  const events = $derived.by<Ev[]>(() => {
    const raw = widget.config.events;
    if (!Array.isArray(raw)) return [];
    return raw
      .flatMap((r) => {
        if (!r || typeof r !== 'object' || Array.isArray(r)) return [];
        const o = r as Record<string, unknown>;
        if (typeof o.start !== 'string' || typeof o.title !== 'string') return [];
        const end = typeof o.end === 'string' ? o.end : o.start;
        return [{ start: o.start, end, title: o.title, s: toMin(o.start), e: toMin(end) }];
      })
      .sort((a, b) => a.s - b.s);
  });
  const nowMin = $derived.by(() => {
    const d = new Date(clock.now);
    return d.getHours() * 60 + d.getMinutes();
  });
  const state = (ev: Ev) => (nowMin >= ev.e ? 'past' : nowMin >= ev.s ? 'now' : 'future');
  const nextIdx = $derived(events.findIndex((e) => e.s > nowMin));
  const fmtIn = (mins: number) => (mins < 60 ? `in ${mins}m` : `in ${Math.floor(mins / 60)}h ${mins % 60 ? `${mins % 60}m` : ''}`);
</script>

{#if events.length === 0}
  <EmptyState compact title="No events" />
{:else}
  <ol class="tl">
    {#each events as ev, i (ev.start + ev.title)}
      {@const st = state(ev)}
      {#if i === nextIdx && (i === 0 || state(events[i - 1]!) === 'past')}
        <li class="nowline" aria-label="Now"><span></span></li>
      {/if}
      <li class="ev {st}">
        <span class="time num">{ev.start}</span>
        <span class="bar" aria-hidden="true"></span>
        <span class="body">
          <span class="title pii">{ev.title}</span>
          <span class="sub num">
            {#if st === 'now'}until {ev.end}{:else if i === nextIdx}{fmtIn(ev.s - nowMin)}{:else}{ev.e - ev.s} min{/if}
          </span>
        </span>
      </li>
    {/each}
  </ol>
{/if}

<style>
  .tl {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .ev {
    display: grid;
    grid-template-columns: 3.2em 4px 1fr;
    gap: var(--space-3);
    align-items: stretch;
    padding: 5px 0;
  }
  .time {
    font-size: var(--text-xs);
    color: var(--text-3);
    padding-top: 1px;
  }
  .bar {
    border-radius: 4px;
    background: var(--surface-3);
  }
  .now .bar {
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  .body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .title {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .past .title {
    color: var(--text-3);
  }
  .now .title {
    color: var(--accent-text);
  }
  .nowline {
    position: relative;
    height: 10px;
  }
  .nowline span {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    height: 2px;
    background: var(--accent);
    border-radius: 2px;
    opacity: 0.7;
  }
  .nowline span::before {
    content: '';
    position: absolute;
    left: -2px;
    top: -3px;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--accent);
  }
</style>
