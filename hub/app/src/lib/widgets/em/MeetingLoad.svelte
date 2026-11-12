<script lang="ts">
  import CalendarClock from '@lucide/svelte/icons/calendar-clock';
  import Link2 from '@lucide/svelte/icons/link-2';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { toISODate } from '../../util/dates';
  import EmptyState from '../../design/EmptyState.svelte';
  import Meter from '../../design/Meter.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { typedItems } from './actions';
  import { CATEGORIES, CATEGORY_LABEL, EM_TYPES, hoursLabel, meetingLoad, type Category } from './logic';
  import { clockLabel, weekdayShort } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const today = $derived(hub.day?.date ?? toISODate(new Date(minute)));
  const q = typedItems(EM_TYPES.meeting);
  const budgetHours = $derived.by(() => {
    const v = widget.config.budget_hours;
    return typeof v === 'number' && v >= 1 && v <= 80 ? v : 16;
  });
  const load = $derived(
    meetingLoad(q.data?.items ?? [], {
      today,
      now: minute,
      budgetHours,
      dayStart: typeof widget.config.day_start === 'string' ? widget.config.day_start : '09:00',
      dayEnd: typeof widget.config.day_end === 'string' ? widget.config.day_end : '17:00',
    }),
  );
  const shown = $derived(CATEGORIES.filter((c) => c !== 'focus' && load.byCategory[c] > 0));
  const peak = $derived(Math.max(1, ...shown.map((c) => load.byCategory[c])));
  const over = $derived(load.totalMinutes > load.budgetMinutes);
  const hasAny = $derived(load.totalMinutes > 0 || load.byCategory.focus > 0);
  const hourLabels = $derived(Array.from({ length: load.days[0]?.hours.length ?? 0 }, (_, h) => Math.floor((load.dayStartMin + h * 60) / 60)));
  const level = (m: number) => (m <= 0 ? 0 : m < 20 ? 1 : m < 40 ? 2 : m < 60 ? 3 : 4);
  const chain = $derived(load.chains[0]);
  const catClass: Record<Category, string> = { '1on1': 'c1', team: 'c2', 'cross-team': 'c3', external: 'c4', focus: 'c5' };
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if !hasAny}
  <EmptyState compact icon={CalendarClock} title="No meetings this week" hint="Your week fills in here once your calendar is connected for this role." />
{:else}
  <div class="head">
    <p class="big">
      <span class="num v" class:over>{hoursLabel(load.totalMinutes)}</span>
      <span class="of">of {hoursLabel(load.budgetMinutes)} budget</span>
    </p>
    <Meter value={load.totalMinutes} max={load.budgetMinutes} label="Meeting hours this week against your budget" warnAt={0.85} dangerAt={1.01} display="{hoursLabel(load.totalMinutes)} of {hoursLabel(load.budgetMinutes)}" />
    <p class="maker"><span class="num">{hoursLabel(load.makerMinutesLeft)}</span> maker time left this week <span class="muted">(free blocks of an hour or more)</span></p>
  </div>

  <ul class="cats" aria-label="Meeting hours by category">
    {#each shown as c (c)}
      <li>
        <span class="lbl">{CATEGORY_LABEL[c]}</span>
        <span class="track" aria-hidden="true"><span class="fill {catClass[c]}" style:width="{(load.byCategory[c] / peak) * 100}%"></span></span>
        <span class="num h">{hoursLabel(load.byCategory[c])}</span>
      </li>
    {/each}
    {#if load.byCategory.focus > 0}
      <li class="focus"><span class="lbl">Focus blocks</span><span class="muted">{hoursLabel(load.byCategory.focus)} protected, not counted</span></li>
    {/if}
  </ul>

  <figure class="heat" aria-label="Meeting minutes per working hour, Monday to Friday">
    <table>
      <thead>
        <tr><th scope="col"><span class="visually-hidden">Day</span></th>{#each hourLabels as h, i (i)}<th scope="col" class="hr">{i % 2 === 0 ? h : ''}<span class="visually-hidden">{h}:00</span></th>{/each}</tr>
      </thead>
      <tbody>
        {#each load.days as d (d.date)}
          <tr class:today={d.date === today}>
            <th scope="row" class="day">{weekdayShort(d.date)}</th>
            {#each d.hours as m, i (i)}
              <td class="cell l{level(m)}" title="{weekdayShort(d.date)} {hourLabels[i]}:00 · {m} min"><span class="visually-hidden">{m} minutes</span></td>
            {/each}
          </tr>
        {/each}
      </tbody>
    </table>
  </figure>

  {#if chain && !ambient}
    <p class="chain" class:long={chain.count >= 3}>
      <Link2 size={13} aria-hidden="true" />
      {#if chain.count >= 3}Longest back-to-back run{:else}Back to back{/if}:
      <span class="num">{weekdayShort(chain.date)} {clockLabel(chain.start)}–{clockLabel(chain.end)}</span>, {chain.count} meetings{#if load.chains.length > 1}<span class="muted"> · {load.chains.length} runs this week</span>{/if}
    </p>
  {/if}
{/if}

<style>
  .head {
    display: grid;
    gap: var(--space-1);
    margin-bottom: var(--space-3);
  }
  .big {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin: 0;
  }
  .v {
    font-family: var(--font-display);
    font-size: var(--text-2xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--text);
  }
  .v.over {
    color: var(--warning);
  }
  .of,
  .muted {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .maker {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .cats {
    margin: 0 0 var(--space-3);
    padding: 0;
    list-style: none;
    display: grid;
    gap: 4px;
  }
  .cats li {
    display: grid;
    grid-template-columns: 6.5em 1fr auto;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .cats li.focus {
    grid-template-columns: 6.5em 1fr;
  }
  .track {
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--surface-inset);
    overflow: hidden;
  }
  .fill {
    display: block;
    height: 100%;
    border-radius: inherit;
    background: var(--accent);
    transition: width var(--dur-slow) var(--ease-out);
  }
  .fill.c2 {
    background: color-mix(in srgb, var(--accent) 70%, var(--surface));
  }
  .fill.c3 {
    background: color-mix(in srgb, var(--accent) 48%, var(--surface));
  }
  .fill.c4 {
    background: var(--info);
  }
  .h {
    color: var(--text);
  }
  .heat {
    margin: 0 0 var(--space-2);
  }
  table {
    width: 100%;
    border-collapse: separate;
    border-spacing: 2px;
    table-layout: fixed;
  }
  th {
    font-weight: var(--weight-regular);
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .day {
    width: 2.6em;
    text-align: left;
  }
  .hr {
    text-align: left;
  }
  tr.today .day {
    color: var(--accent-text);
    font-weight: var(--weight-semibold);
  }
  .cell {
    height: 12px;
    border-radius: 2px;
    background: var(--surface-inset);
  }
  .cell.l1 {
    background: color-mix(in srgb, var(--accent) 22%, var(--surface-inset));
  }
  .cell.l2 {
    background: color-mix(in srgb, var(--accent) 45%, var(--surface-inset));
  }
  .cell.l3 {
    background: color-mix(in srgb, var(--accent) 70%, var(--surface-inset));
  }
  .cell.l4 {
    background: var(--accent);
  }
  .chain {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .chain.long {
    color: var(--text);
  }
</style>
