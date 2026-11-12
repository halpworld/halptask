<script lang="ts">
  import Users from '@lucide/svelte/icons/users';
  import CalendarClock from '@lucide/svelte/icons/calendar-clock';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { toISODate } from '../../util/dates';
  import EmptyState from '../../design/EmptyState.svelte';
  import Kbd from '../../design/Kbd.svelte';
  import Ring from '../../design/Ring.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { saveConfig, typedItems } from './actions';
  import { EM_TYPES, readCommitment, readOneOnOnes, readPeople, readSort, roster, type Commitment, type RosterRow, type RosterSort } from './logic';
  import { whenLabel } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const today = $derived(hub.day?.date ?? toISODate(new Date(minute)));
  const peopleQ = typedItems(EM_TYPES.person);
  const onesQ = typedItems(EM_TYPES.oneOnOne);
  const commitQ = typedItems(EM_TYPES.commitment);

  let sort = $state<RosterSort>('next');
  $effect(() => {
    sort = readSort(widget.config.sort);
  });

  const rows = $derived(
    roster(
      readPeople(peopleQ.data?.items ?? []),
      readOneOnOnes(onesQ.data?.items ?? []),
      (commitQ.data?.items ?? []).map(readCommitment).filter((c): c is Commitment => !!c),
      today,
      minute,
      sort,
    ),
  );
  const overdue = $derived(rows.filter((r) => r.cadence.state === 'overdue' || r.cadence.state === 'never').length);

  function setSort(s: RosterSort) {
    if (s === sort) return;
    sort = s;
    if (!ambient) void saveConfig(widget.id, { ...widget.config, sort: s });
  }

  /** Open this person in the one-on-one widget on the same page. */
  function prep(r: RosterRow) {
    const target = hub.layout?.widgets.find((w) => w.kind === 'one-on-one');
    if (!target) return;
    void saveConfig(target.id, { ...target.config, person_id: r.person.id });
  }

  function cadenceText(r: RosterRow): string {
    const c = r.cadence;
    if (c.state === 'never') return 'No one-on-one yet';
    const since = c.daysSince === 0 ? 'today' : `${c.daysSince}d ago`;
    return `Last 1:1 ${since} · every ${c.cadence}d`;
  }
  const tone = (r: RosterRow) => (r.cadence.state === 'ok' ? 'muted' : 'accent');
</script>

{#if peopleQ.loading && !peopleQ.data}
  <Skeleton lines={4} />
{:else if (peopleQ.error && !peopleQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={peopleQ.error} guide={false} />
{:else if rows.length === 0}
  <EmptyState compact icon={Users} title="No people yet" hint="Add the people on your team to see when you last met each one." />
{:else}
  <div class="bar">
    <p class="summary">
      <span class="num">{rows.length}</span> people{#if overdue}<span> · <span class="num">{overdue}</span> past their 1:1 cadence</span>{/if}
    </p>
    {#if !ambient}
      <div class="seg" role="group" aria-label="Sort people">
        <button type="button" aria-pressed={sort === 'next'} onclick={() => setSort('next')}>Next 1:1</button>
        <button type="button" aria-pressed={sort === 'name'} onclick={() => setSort('name')}>A–Z</button>
      </div>
    {/if}
  </div>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul class="cards" onkeydown={listKeydown} aria-label={sort === 'next' ? 'Your people, by next one-on-one' : 'Your people, alphabetical'}>
    {#each rows as r (r.person.id)}
      <li class="card {r.cadence.state}" data-row>
        <Ring value={r.cadence.ratio} size={ambient ? 48 : 40} stroke={4} tone={tone(r)} label="{r.person.name}: {cadenceText(r)}">
          <span class="num days">{r.cadence.daysSince ?? '–'}</span>
        </Ring>
        <button
          type="button"
          class="main"
          data-row-focus
          disabled={ambient}
          onclick={() => prep(r)}
          aria-label="{r.person.name}. {cadenceText(r)}{r.cadence.state === 'overdue' ? ', past cadence' : ''}. Next one-on-one {whenLabel(r.next?.scheduledFor, minute)}. You owe {r.iOwe}, waiting on {r.waitingOn}. Prepare one-on-one"
        >
          <span class="name pii truncate">{r.person.name}</span>
          <span class="sub truncate">{#if r.person.role}<span class="pii">{r.person.role}</span>{/if}{#if r.person.team}<span class="pii"> · {r.person.team}</span>{/if}</span>
          <span class="meta">
            <span class="next" title="Next one-on-one"><CalendarClock size={12} aria-hidden="true" />{whenLabel(r.next?.scheduledFor, minute)}</span>
            {#if r.iOwe || r.waitingOn}
              <span class="owe" title="Open commitments: you owe {r.iOwe}, waiting on {r.waitingOn}"><span class="num">{r.iOwe}</span>↑ <span class="num">{r.waitingOn}</span>↓</span>
            {/if}
          </span>
        </button>
      </li>
    {/each}
  </ul>
  {#if !ambient}<p class="keys"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="Enter" /> prepare 1:1 · ↑ you owe · ↓ waiting on</p>{/if}
{/if}

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .summary {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .seg {
    margin-left: auto;
    display: inline-flex;
    padding: 2px;
    border-radius: var(--radius-md);
    background: var(--surface-inset);
  }
  .seg button {
    font: inherit;
    font-size: var(--text-2xs);
    padding: 2px var(--space-2);
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-2);
    cursor: pointer;
  }
  .seg button[aria-pressed='true'] {
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .cards {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(190px, 1fr));
    gap: var(--space-2);
  }
  .card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
    padding: var(--space-2) var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    background: var(--surface);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out);
  }
  .card:hover,
  .card:focus-within {
    border-color: var(--border-strong);
  }
  /* Overdue is a soft accent, never an alarm: it is about your calendar, not them. */
  .card.overdue,
  .card.never {
    background: var(--accent-soft);
    border-color: transparent;
  }
  .days {
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
  }
  .main {
    all: unset;
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    flex: 1;
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .main:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .main:disabled {
    cursor: default;
  }
  .name {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .sub {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 2px;
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .next {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .owe {
    margin-left: auto;
    color: var(--text-3);
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 3px;
    margin: var(--space-2) 0 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
</style>
