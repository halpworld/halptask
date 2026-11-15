<script lang="ts">
  import Activity from '@lucide/svelte/icons/activity';
  import UsersRound from '@lucide/svelte/icons/users-round';
  import type { WidgetProps } from '../types';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import Sparkline from '../../design/Sparkline.svelte';
  import { typedItems } from './actions';
  import { EM_TYPES, teamPulse, type PulseMetric } from './logic';

  let { widget, ambient }: WidgetProps = $props();

  const weeks = $derived.by(() => {
    const v = widget.config.weeks;
    return typeof v === 'number' && Number.isInteger(v) && v >= 2 && v <= 26 ? v : 6;
  });
  const q = typedItems(EM_TYPES.signal);
  const teams = $derived(teamPulse(q.data?.items ?? [], weeks));

  const fmt = (m: PulseMetric) => (m.latest === undefined ? '–' : `${m.latest >= 10 ? Math.round(m.latest) : Math.round(m.latest * 10) / 10}${m.unit}`);
  /** Neutral tones: a trend is context for a conversation, not a grade. */
  function tone(m: PulseMetric): 'accent' | 'muted' {
    if (m.values.length < 2 || m.latest === undefined) return 'muted';
    const first = m.values[0]!;
    if (first === 0) return 'muted';
    const worse = m.lowerIsBetter ? m.latest > first * 1.25 : m.latest < first * 0.75;
    return worse ? 'accent' : 'muted';
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if teams.length === 0}
  <EmptyState compact icon={Activity} title="No team signals yet" hint="Weekly numbers per team appear here: merged PRs, cycle time, review wait and incidents." />
{:else}
  <p class="scope"><Badge tone="neutral" icon={UsersRound} title="These numbers describe the team as a whole. There are no per-person metrics in the Hub.">Team-level only</Badge></p>
  {#each teams as t (t.team)}
    <section class="team" aria-label="{t.team}, last {t.periods.length} weeks">
      {#if teams.length > 1}<h3 class="name">{t.team}</h3>{/if}
      <ul class="metrics">
        {#each t.metrics.filter((m) => m.values.length > 0) as m (m.key)}
          <li>
            <span class="lbl">{m.label}</span>
            <span class="num val">{fmt(m)}</span>
            <span class="spark"><Sparkline values={m.values} label="{m.label} for {t.team}, {t.periods[0]} to {t.periods[t.periods.length - 1]}: {m.values.join(', ')}" height={ambient ? 30 : 22} tone={tone(m)} fill={m.key !== 'incidents'} min={0} /></span>
          </li>
        {/each}
      </ul>
      {#if t.context.length}
        <ul class="ctx">{#each t.context as line (line)}<li>{line}</li>{/each}</ul>
      {/if}
    </section>
  {/each}
{/if}

<style>
  .scope {
    margin: 0 0 var(--space-2);
  }
  .team + .team {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--border);
  }
  .name {
    margin: 0 0 var(--space-1);
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .metrics {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: 6px;
  }
  .metrics li {
    display: grid;
    grid-template-columns: 1fr auto 72px;
    align-items: center;
    gap: var(--space-2);
  }
  .lbl {
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .val {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .spark {
    display: block;
    min-width: 0;
  }
  .ctx {
    margin: var(--space-2) 0 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-xs);
    color: var(--text-2);
    line-height: var(--leading-snug);
  }
  .ctx li + li {
    margin-top: 2px;
  }
</style>
