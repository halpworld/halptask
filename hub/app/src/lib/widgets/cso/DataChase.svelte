<script lang="ts">
  import Send from '@lucide/svelte/icons/send';
  import BellRing from '@lucide/svelte/icons/bell-ring';
  import Inbox from '@lucide/svelte/icons/inbox';
  import BadgeCheck from '@lucide/svelte/icons/badge-check';
  import Ban from '@lucide/svelte/icons/ban';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { toISODate } from '../../util/dates';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import IconButton from '../../design/IconButton.svelte';
  import Kbd from '../../design/Kbd.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { chase, ignoreKey, setRequestStatus, typedItems } from './actions';
  import { chaseBoard, CSO_TYPES, DEFAULT_CHASE_AFTER, intIn, isOpenRequest, scopeLabel, type ChaseCard, type RequestStatus } from './logic';
  import { dateShort, daysLabel } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const today = $derived(hub.day?.date ?? toISODate(new Date(minute)));
  const chaseAfter = $derived(intIn(widget.config.chase_after_days, 1, 90, DEFAULT_CHASE_AFTER));
  const q = typedItems(CSO_TYPES.dataRequest);
  const board = $derived(chaseBoard(q.data?.items ?? [], today, minute, chaseAfter));
  const total = $derived(board.reduce((n, c) => n + c.cards.length, 0));
  const overdue = $derived(board.reduce((n, c) => n + c.overdue, 0));

  const TITLE: Record<RequestStatus, string> = { requested: 'Requested', chased: 'Chased', received: 'Received', validated: 'Validated', rejected: 'Rejected' };
  let busy = $state<string | null>(null);

  async function act(c: ChaseCard, what: 'chase' | RequestStatus) {
    if (busy) return;
    busy = c.r.item.id;
    try {
      if (what === 'chase') await chase(c.r, new Date(clock.now).toISOString());
      else await setRequestStatus(c.r, what);
    } finally {
      busy = null;
    }
  }
  function onCardKey(e: KeyboardEvent, c: ChaseCard) {
    if (ambient || ignoreKey(e)) return;
    const open = isOpenRequest(c.r);
    const map: Record<string, 'chase' | RequestStatus | undefined> = {
      c: open ? 'chase' : undefined,
      r: open ? 'received' : undefined,
      v: c.r.status === 'received' ? 'validated' : undefined,
      x: c.r.status === 'received' ? 'rejected' : undefined,
    };
    const what = map[e.key];
    if (what) {
      e.preventDefault();
      void act(c, what);
    }
  }
  function aria(c: ChaseCard): string {
    const parts = [`${c.r.supplier}: ${c.r.metric}`];
    const s = scopeLabel(c.r);
    if (s) parts.push(s);
    if (c.r.period) parts.push(`period ${c.r.period}`);
    if (c.overdueDays) parts.push(`overdue by ${c.overdueDays} days`);
    else if (c.dueInDays !== null && isOpenRequest(c.r)) parts.push(`due ${daysLabel(c.dueInDays)}`);
    if (c.r.chaseCount) parts.push(`chased ${c.r.chaseCount} times`);
    if (c.chaseDue) parts.push('due a chase');
    return parts.join(', ');
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={6} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if total === 0}
  <EmptyState compact icon={Send} title="No data requests" hint="Supplier data requests appear here. Chasing logs a count; nothing is ever sent from the Hub." />
{:else}
  <div class="board" role="group" aria-label="Supplier data requests by status{overdue ? `, ${overdue} overdue` : ''}">
    {#each board as col (col.status)}
      <section class="col {col.status}" aria-label="{TITLE[col.status]}, {col.cards.length}">
        <h3 class="head">
          {TITLE[col.status]} <span class="num count">{col.cards.length}</span>
          {#if col.overdue}<Badge tone="danger" variant="solid" title="Past due and still open">{col.overdue} late</Badge>{/if}
        </h3>
        {#if col.cards.length === 0}
          <p class="none">None</p>
        {:else}
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <ul class="cards" onkeydown={listKeydown}>
            {#each col.cards as c (c.r.item.id)}
              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <li class="card" class:late={c.overdueDays !== null} class:due={c.chaseDue && c.overdueDays === null} class:busy={busy === c.r.item.id} data-row onkeydown={(e) => onCardKey(e, c)}>
                <span class="txt" tabindex="-1" data-row-focus aria-label={aria(c)}>
                  <span class="sup">{c.r.supplier}</span>
                  <span class="metric">{c.r.metric}</span>
                  <span class="meta">
                    {#if scopeLabel(c.r)}<span>{scopeLabel(c.r)}</span>{/if}
                    {#if c.r.period}<span class="num">{c.r.period}</span>{/if}
                    {#if c.r.owner}<span class="pii">{c.r.owner}</span>{/if}
                  </span>
                </span>
                <span class="foot">
                  {#if c.overdueDays !== null}
                    <Badge tone="danger" title="Due {c.r.due ? dateShort(c.r.due) : ''}">{c.overdueDays} d late</Badge>
                  {:else if c.dueInDays !== null && isOpenRequest(c.r)}
                    <Badge tone={c.dueInDays <= 7 ? 'warning' : 'neutral'} variant="outline" title="Due {c.r.due ? dateShort(c.r.due) : ''}">{daysLabel(c.dueInDays)}</Badge>
                  {/if}
                  {#if c.r.chaseCount > 0}
                    <span class="chases num" title={c.sinceChase !== null ? `Last chased ${c.sinceChase} days ago` : undefined}>×{c.r.chaseCount}{#if c.sinceChase !== null}<span class="ago"> · {c.sinceChase}d</span>{/if}</span>
                  {/if}
                  {#if !ambient}
                    <span class="acts">
                      {#if isOpenRequest(c.r)}
                        <IconButton size="sm" icon={BellRing} label="Log a chase for {c.r.supplier} (c)" disabled={busy !== null} onclick={() => act(c, 'chase')} />
                        <IconButton size="sm" icon={Inbox} label="Mark received (r)" disabled={busy !== null} onclick={() => act(c, 'received')} />
                      {:else if c.r.status === 'received'}
                        <IconButton size="sm" icon={BadgeCheck} label="Mark validated (v)" disabled={busy !== null} onclick={() => act(c, 'validated')} />
                        <IconButton size="sm" icon={Ban} label="Reject the data (x)" disabled={busy !== null} onclick={() => act(c, 'rejected')} />
                      {/if}
                    </span>
                  {/if}
                </span>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
  {#if !ambient}<p class="keys"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="c" /> log chase <Kbd keys="r" /> received <Kbd keys="v" /> validate <Kbd keys="x" /> reject · nothing is sent</p>{/if}
{/if}

<style>
  .board {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(132px, 1fr));
    gap: var(--space-2);
    align-items: start;
  }
  .col {
    min-width: 0;
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    margin: 0 0 var(--space-2);
    font-family: var(--font-label);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    color: var(--text-3);
  }
  .count {
    color: var(--text-2);
    margin-right: auto;
  }
  .none {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .cards {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .card {
    position: relative;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface);
    box-shadow: var(--elev-1);
    transition:
      border-color var(--dur-fast) var(--ease-out),
      opacity var(--dur-fast) var(--ease-out);
  }
  .card:focus-within {
    border-color: var(--border-strong);
  }
  .card.late {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .card.due::before {
    content: '';
    position: absolute;
    left: -1px;
    top: 8px;
    bottom: 8px;
    width: 3px;
    border-radius: var(--radius-full);
    background: var(--warning);
  }
  .card.busy {
    opacity: 0.6;
  }
  .txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
    outline: none;
    border-radius: var(--radius-sm);
  }
  .txt:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .sup {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
    line-height: var(--leading-snug);
  }
  .metric {
    font-size: var(--text-xs);
    color: var(--text-2);
    line-height: var(--leading-snug);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 6px;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .foot {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chases {
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    color: var(--text-2);
  }
  .ago {
    font-weight: var(--weight-regular);
    color: var(--text-3);
  }
  .acts {
    display: inline-flex;
    gap: 2px;
    margin-left: auto;
    opacity: 0.55;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .card:hover .acts,
  .card:focus-within .acts {
    opacity: 1;
  }
  .validated .card,
  .rejected .card {
    box-shadow: none;
  }
  .rejected .sup {
    text-decoration: line-through;
    text-decoration-color: var(--text-3);
  }
  .keys {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 3px;
    margin: var(--space-2) 0 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
</style>
