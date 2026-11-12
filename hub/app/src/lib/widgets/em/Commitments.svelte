<script lang="ts">
  import Handshake from '@lucide/svelte/icons/handshake';
  import Check from '@lucide/svelte/icons/check';
  import BellRing from '@lucide/svelte/icons/bell-ring';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { toasts } from '../../design/toast.svelte';
  import { toISODate } from '../../util/dates';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import IconButton from '../../design/IconButton.svelte';
  import Kbd from '../../design/Kbd.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { ignoreKey, peopleById, typedItems } from './actions';
  import { agingLabel, commitmentColumns, EM_TYPES, nudgeTitle, type CommitmentRow } from './logic';

  let { ambient }: WidgetProps = $props();

  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const today = $derived(hub.day?.date ?? toISODate(new Date(minute)));
  const commitQ = typedItems(EM_TYPES.commitment);
  const peopleQ = typedItems(EM_TYPES.person);
  const people = $derived(peopleById(peopleQ.data?.items));
  const cols = $derived(commitmentColumns(commitQ.data?.items ?? [], people, today));
  const total = $derived(cols.iOwe.length + cols.waitingOn.length);

  const SOURCE: Record<string, string> = { meeting: 'from a meeting', slack: 'from Slack', email: 'from email', manual: 'added by you' };

  function done(r: CommitmentRow) {
    void hub.complete(r.c.item);
  }
  /** Drafts a reminder task for you. Nothing is sent to anyone. */
  async function nudge(r: CommitmentRow) {
    const res = await hub.run('tasks.create', { title: nudgeTitle(r), due: today, tags: ['nudge'], roles: [hub.roleId] });
    if (res) toasts.show(`Added “${res.title}”. Nothing was sent.`, { tone: 'success', timeout: 3500 });
  }
  function onRowKey(e: KeyboardEvent, r: CommitmentRow) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 'x') {
      e.preventDefault();
      done(r);
    } else if (e.key === 'n' && r.c.direction === 'waiting_on') {
      e.preventDefault();
      void nudge(r);
    }
  }
  const tone = (r: CommitmentRow) => (r.overdueDays ? 'warning' : r.dueInDays === 0 ? 'accent' : 'neutral');
</script>

{#snippet column(title: string, rows: CommitmentRow[], empty: string, label: string)}
  <section class="col" aria-label={label}>
    <h3 class="head">{title} <span class="num count">{rows.length}</span></h3>
    {#if rows.length === 0}
      <p class="none">{empty}</p>
    {:else}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <ul class="rows" onkeydown={listKeydown}>
        {#each rows as r (r.c.item.id)}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <li class="row" class:late={!!r.overdueDays} data-row onkeydown={(e) => onRowKey(e, r)}>
            <span class="txt" tabindex="-1" data-row-focus aria-label="{r.c.what}, {r.c.direction === 'i_owe' ? 'for' : 'from'} {r.who}, {agingLabel(r)}, {SOURCE[r.c.source]}">
              <span class="what pii">{r.c.what}</span>
              <span class="who"><span class="pii">{r.who}</span> · {SOURCE[r.c.source]}</span>
            </span>
            {#if agingLabel(r)}<Badge tone={tone(r)} title={r.ageDays !== null ? `Promised ${r.ageDays} days ago` : undefined}>{agingLabel(r)}</Badge>{/if}
            {#if !ambient}
              <span class="acts">
                {#if r.c.direction === 'waiting_on'}
                  <IconButton size="sm" icon={BellRing} label="Draft a nudge task for {r.who} (n)" onclick={() => nudge(r)} />
                {/if}
                <IconButton size="sm" icon={Check} label="Mark done (x)" onclick={() => done(r)} />
              </span>
            {/if}
          </li>
        {/each}
      </ul>
    {/if}
  </section>
{/snippet}

{#if commitQ.loading && !commitQ.data}
  <Skeleton lines={5} />
{:else if (commitQ.error && !commitQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={commitQ.error} guide={false} />
{:else if total === 0}
  <EmptyState compact icon={Handshake} title="No open commitments" hint="Promises you made, and promises made to you, appear here." />
{:else}
  <div class="cols">
    {@render column('I owe', cols.iOwe, 'Nothing you promised is open.', 'Commitments you owe, most urgent first')}
    {@render column('Waiting on', cols.waitingOn, 'Nobody owes you anything.', 'Commitments you are waiting on, most urgent first')}
  </div>
  {#if !ambient}<p class="keys"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="n" /> nudge task <Kbd keys="x" /> done · nothing is ever sent</p>{/if}
{/if}

<style>
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(160px, 1fr));
    gap: var(--space-3);
  }
  .col {
    min-width: 0;
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-1);
    font-family: var(--font-label);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    color: var(--text-3);
  }
  .count {
    color: var(--text-2);
  }
  .none {
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 5px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border-radius: var(--radius-md);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .row:hover,
  .row:focus-within {
    background: var(--surface-2);
  }
  .row.late::before {
    content: '';
    position: absolute;
    left: 0;
    top: 6px;
    bottom: 6px;
    width: 2px;
    border-radius: var(--radius-full);
    background: var(--warning);
  }
  .txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
    outline: none;
    border-radius: var(--radius-sm);
  }
  .txt:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .what {
    font-size: var(--text-sm);
    color: var(--text);
    line-height: var(--leading-snug);
  }
  .who {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .acts {
    display: inline-flex;
    gap: 2px;
    opacity: 0.55;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .row:hover .acts,
  .row:focus-within .acts {
    opacity: 1;
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
