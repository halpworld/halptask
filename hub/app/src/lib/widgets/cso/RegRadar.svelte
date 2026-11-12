<script lang="ts">
  import Radar from '@lucide/svelte/icons/radar';
  import Link from '@lucide/svelte/icons/link';
  import Eye from '@lucide/svelte/icons/eye';
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
  import { CSO_TYPES, IMPACTS, isSettled, radarRows, type Impact, type RadarRow, type RegStage } from './logic';
  import { dateShort, daysLabel, humanize } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const today = $derived(hub.day?.date ?? toISODate(new Date(Math.floor(clock.now / 3_600_000) * 3_600_000)));
  const minImpact = $derived((IMPACTS as readonly unknown[]).includes(widget.config.min_impact) ? (widget.config.min_impact as Impact) : undefined);
  const q = typedItems(CSO_TYPES.regChange);
  const rows = $derived(radarRows(q.data?.items ?? [], today, { minImpact, showWithdrawn: widget.config.show_withdrawn === true }));

  const STAGE_TONE: Record<RegStage, 'success' | 'info' | 'warning' | 'danger' | 'neutral'> = {
    in_force: 'success',
    adopted: 'success',
    proposed: 'info',
    consultation: 'info',
    delayed: 'warning',
    litigated: 'danger',
    withdrawn: 'neutral',
  };
  const IMPACT_TONE: Record<Impact, 'danger' | 'warning' | 'neutral'> = { high: 'danger', medium: 'warning', low: 'neutral' };

  function onRowKey(e: KeyboardEvent, r: RadarRow) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 'o' || e.key === 'Enter') {
      e.preventDefault();
      void openLink(r.c.sourceUrl, `the source for ${r.c.title}`);
    }
  }
  function aria(r: RadarRow): string {
    const p = [r.c.title, r.c.jurisdiction, humanize(r.c.stage)];
    if (r.c.impact) p.push(`${r.c.impact} impact`);
    if (r.daysToEffect !== null && r.c.effectiveDate) p.push(`effective ${dateShort(r.c.effectiveDate)}${isSettled(r.c.stage) ? '' : ', not certain'}`);
    if (r.stale) p.push(r.reviewAge === null ? 'never reviewed' : `last reviewed ${r.reviewAge} days ago`);
    return p.join(', ');
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if rows.length === 0}
  <EmptyState compact icon={Radar} title="Radar is clear" hint="Regulatory changes appear here, each with its official source." />
{:else}
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul class="rows" aria-label="Regulatory changes, highest impact first" onkeydown={listKeydown}>
    {#each rows as r (r.c.item.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="row" class:moving={!isSettled(r.c.stage)} data-row onkeydown={(e) => onRowKey(e, r)}>
        <span class="impact {r.c.impact ?? 'low'}" aria-hidden="true"></span>
        <span class="txt" tabindex="-1" data-row-focus aria-label={aria(r)}>
          <span class="title">{r.c.title}</span>
          <span class="meta">
            <span class="jur">{r.c.jurisdiction}</span>
            {#if r.c.instrument}<span class="truncate">· {r.c.instrument}</span>{/if}
          </span>
          {#if r.c.impactNote}<span class="note">{r.c.impactNote}</span>{/if}
        </span>
        <span class="side">
          <Badge tone={STAGE_TONE[r.c.stage]} variant={isSettled(r.c.stage) ? 'soft' : 'outline'}>{humanize(r.c.stage)}</Badge>
          {#if r.c.effectiveDate && r.daysToEffect !== null}
            <span class="eff num" class:uncertain={!isSettled(r.c.stage)} title={isSettled(r.c.stage) ? 'Effective date' : 'Effective date as proposed. Not certain.'}>{r.daysToEffect >= 0 ? daysLabel(r.daysToEffect) : dateShort(r.c.effectiveDate)}{isSettled(r.c.stage) ? '' : '?'}</span>
          {/if}
          {#if r.c.impact}<Badge tone={IMPACT_TONE[r.c.impact]} variant="outline">{r.c.impact}</Badge>{/if}
          {#if r.stale}<span class="review" title={r.reviewAge === null ? 'Never reviewed' : `Last reviewed ${r.reviewAge} days ago`}><Eye size={11} aria-hidden="true" /> review</span>{/if}
        </span>
        {#if !ambient && r.c.sourceUrl}
          <IconButton size="sm" icon={Link} label="Copy the official source link (o)" onclick={() => openLink(r.c.sourceUrl, `the source for ${r.c.title}`)} />
        {/if}
      </li>
    {/each}
  </ul>
  {#if !ambient}<p class="keys"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="o" /> copy source · dates of unsettled rules end in ?</p>{/if}
{/if}

<style>
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: 4px minmax(0, 1fr) auto auto;
    align-items: start;
    gap: var(--space-2);
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--border);
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row:focus-within {
    background: var(--surface-2);
  }
  .impact {
    align-self: stretch;
    border-radius: var(--radius-full);
    background: var(--border-strong);
  }
  .impact.high {
    background: var(--danger);
  }
  .impact.medium {
    background: var(--warning);
  }
  .moving .impact {
    background: repeating-linear-gradient(180deg, var(--border-strong) 0 4px, transparent 4px 7px);
  }
  .moving .impact.high {
    background: repeating-linear-gradient(180deg, var(--danger) 0 4px, transparent 4px 7px);
  }
  .moving .impact.medium {
    background: repeating-linear-gradient(180deg, var(--warning) 0 4px, transparent 4px 7px);
  }
  .txt {
    display: flex;
    flex-direction: column;
    gap: 1px;
    min-width: 0;
    outline: none;
    border-radius: var(--radius-sm);
  }
  .txt:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .title {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text);
    line-height: var(--leading-snug);
  }
  .meta {
    display: flex;
    gap: 4px;
    min-width: 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .jur {
    font-weight: var(--weight-semibold);
    color: var(--text-2);
    white-space: nowrap;
  }
  .note {
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .side {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 3px;
  }
  .eff {
    font-size: var(--text-2xs);
    color: var(--text-2);
    white-space: nowrap;
  }
  .eff.uncertain {
    color: var(--text-3);
    border-bottom: 1px dashed var(--border-strong);
  }
  .review {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--text-2xs);
    color: var(--warning);
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
