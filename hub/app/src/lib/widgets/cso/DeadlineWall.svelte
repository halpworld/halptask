<script lang="ts">
  import Landmark from '@lucide/svelte/icons/landmark';
  import Link from '@lucide/svelte/icons/link';
  import CircleHelp from '@lucide/svelte/icons/circle-help';
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
  import { ignoreKey, openLink, typedItems } from './actions';
  import { CSO_TYPES, deadlineRows, FRAMEWORK_LABEL, intIn, type Certainty, type DeadlineRow } from './logic';
  import { dateLong, daysLabel, humanize } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const today = $derived(hub.day?.date ?? toISODate(new Date(Math.floor(clock.now / 3_600_000) * 3_600_000)));
  const horizon = $derived(intIn(widget.config.horizon_days, 7, 1825, 800));
  const showFiled = $derived(widget.config.show_filed === true);
  const q = typedItems(CSO_TYPES.disclosure);
  const rows = $derived(deadlineRows(q.data?.items ?? [], today, { horizonDays: horizon, showFiled }));

  const CERTAINTY_TONE: Record<Certainty, 'success' | 'warning' | 'danger' | 'info'> = { confirmed: 'success', proposed: 'info', litigated: 'danger', delayed: 'warning' };
  const CERTAINTY_HINT: Record<Certainty, string> = {
    confirmed: 'Confirmed in law or by the regulator.',
    proposed: 'Proposed only. The date may change.',
    litigated: 'Under litigation. The date may not hold.',
    delayed: 'Delayed or paused. The date is not certain.',
  };

  function bigDays(r: DeadlineRow): string {
    if (r.state === 'filed') return '✓';
    if (r.daysLeft === 0) return 'Today';
    return String(Math.abs(r.daysLeft));
  }
  function aria(r: DeadlineRow): string {
    const when = r.state === 'filed' ? 'filed' : daysLabel(r.daysLeft);
    return `${r.d.name}, ${FRAMEWORK_LABEL[r.d.framework]}, due ${dateLong(r.d.due)}, ${when}, ${r.d.certainty}${r.certain ? '' : ', not certain'}, status ${humanize(r.d.status)}`;
  }
  function onRowKey(e: KeyboardEvent, r: DeadlineRow) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 'o' || e.key === 'Enter') {
      e.preventDefault();
      void openLink(r.d.sourceUrl, `the source for ${r.d.name}`);
    }
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if rows.length === 0}
  <EmptyState compact icon={Landmark} title="No disclosure deadlines" hint="Only confirmed dates are shown as certain." />
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul class="wall" aria-label="Disclosure deadlines, soonest first" onkeydown={listKeydown}>
    {#each rows as r (r.d.item.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="row {r.state}" class:uncertain={!r.certain} data-row onkeydown={(e) => onRowKey(e, r)}>
        <span class="days" aria-hidden="true">
          <span class="n num">{bigDays(r)}</span>
          {#if r.state !== 'filed' && r.daysLeft !== 0}<span class="u">{r.daysLeft < 0 ? 'days late' : 'days'}</span>{/if}
        </span>
        <span class="main" tabindex="-1" data-row-focus aria-label={aria(r)}>
          <span class="name">{r.d.name}</span>
          <span class="meta">
            <span class="fw">{FRAMEWORK_LABEL[r.d.framework]}</span>
            {#if r.d.jurisdiction}<span>· {r.d.jurisdiction}</span>{/if}
            <span class="num">· {dateLong(r.d.due)}</span>
            {#if r.d.owner}<span class="pii">· {r.d.owner}</span>{/if}
          </span>
          {#if !r.certain && r.d.certaintyNote}<span class="note">{r.d.certaintyNote}</span>{/if}
        </span>
        <span class="tags">
          <Badge tone={CERTAINTY_TONE[r.d.certainty]} variant={r.certain ? 'soft' : 'outline'} icon={r.certain ? undefined : CircleHelp} title={CERTAINTY_HINT[r.d.certainty]}>{r.d.certainty}</Badge>
          <span class="status">{humanize(r.d.status)}</span>
        </span>
        {#if !ambient && r.d.sourceUrl}
          <IconButton size="sm" icon={Link} label="Copy the source link for {r.d.name} (o)" onclick={() => openLink(r.d.sourceUrl, `the source for ${r.d.name}`)} />
        {/if}
      </li>
    {/each}
  </ul>
  <p class="legend">
    <span class="key solid" aria-hidden="true"></span> confirmed
    <span class="key hatched" aria-hidden="true"></span> not certain: proposed, delayed or litigated
    {#if !ambient}<span class="kb"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="o" /> copy source</span>{/if}
  </p>
{/if}

<style>
  .wall {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  .row {
    --edge: var(--border-strong);
    position: relative;
    display: grid;
    grid-template-columns: 4.6em minmax(0, 1fr) auto auto;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-2);
    border: 1px solid var(--border);
    border-left: 3px solid var(--edge);
    border-radius: var(--radius-md);
    background: var(--surface);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .row:hover,
  .row:focus-within {
    background: var(--surface-2);
  }
  .row.soon,
  .row.today {
    --edge: var(--accent);
  }
  .row.overdue {
    --edge: var(--danger);
  }
  .row.filed {
    --edge: var(--success);
    opacity: 0.75;
  }
  /* Never shown as certain: dashed outline and a hatched day count. */
  .row.uncertain {
    border-style: dashed;
    border-left-style: dashed;
  }
  .days {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    min-height: 2.8em;
    border-radius: var(--radius-sm);
    background: var(--surface-inset);
    line-height: 1;
  }
  .uncertain .days {
    background: repeating-linear-gradient(-45deg, var(--surface-inset) 0 5px, var(--neutral-soft) 5px 10px);
    outline: 1px dashed var(--border-strong);
    outline-offset: -1px;
  }
  .n {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--text);
  }
  .today .n,
  .soon .n {
    color: var(--accent-text);
  }
  .today .n {
    font-size: var(--text-md);
  }
  .overdue .n {
    color: var(--danger);
  }
  .uncertain .n {
    color: var(--text-2);
  }
  .u {
    margin-top: 2px;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    outline: none;
    border-radius: var(--radius-sm);
  }
  .main:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .name {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
    line-height: var(--leading-snug);
  }
  .meta {
    display: flex;
    flex-wrap: wrap;
    gap: 0 4px;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .fw {
    font-weight: var(--weight-semibold);
    color: var(--text-2);
  }
  .note {
    font-size: var(--text-2xs);
    color: var(--text-2);
    font-style: italic;
  }
  .tags {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 2px;
  }
  .status {
    font-size: var(--text-2xs);
    color: var(--text-3);
    text-transform: capitalize;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 4px;
    margin: var(--space-2) 0 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .key {
    display: inline-block;
    width: 14px;
    height: 10px;
    border-radius: 2px;
    margin-left: var(--space-2);
  }
  .key:first-child {
    margin-left: 0;
  }
  .key.solid {
    background: var(--surface-inset);
    border: 1px solid var(--border-strong);
  }
  .key.hatched {
    background: repeating-linear-gradient(-45deg, var(--surface-inset) 0 3px, var(--neutral-soft) 3px 6px);
    border: 1px dashed var(--border-strong);
  }
  .kb {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    margin-left: auto;
  }
</style>
