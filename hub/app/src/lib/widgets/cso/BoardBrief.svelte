<script lang="ts">
  import Presentation from '@lucide/svelte/icons/presentation';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import Link2Off from '@lucide/svelte/icons/link-2-off';
  import Crosshair from '@lucide/svelte/icons/crosshair';
  import LockKeyhole from '@lucide/svelte/icons/lock-keyhole';
  import type { WidgetProps } from '../types';
  import { HubApiError } from '../../api';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { query } from '../../stores/query.svelte';
  import { toISODate } from '../../util/dates';
  import Badge from '../../design/Badge.svelte';
  import Button from '../../design/Button.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { focusItem, setBriefStatus, typedItems } from './actions';
  import { briefSections, checkBrief, citedSourceIds, lookupSources, CSO_TYPES, sourceSummary, type BriefCheck, type BriefStatus, type CheckedFigure } from './logic';
  import { dateLong, fmtNum } from './format';

  let { widget, ambient }: WidgetProps = $props();
  const uid = $props.id();

  const today = $derived(hub.day?.date ?? toISODate(new Date(Math.floor(clock.now / 3_600_000) * 3_600_000)));
  const briefQ = typedItems(CSO_TYPES.brief);
  const meetingCfg = $derived(typeof widget.config.meeting_date === 'string' && /^\d{4}-\d{2}-\d{2}$/.test(widget.config.meeting_date) ? widget.config.meeting_date : undefined);
  const view = $derived(briefSections(briefQ.data?.items ?? [], today, meetingCfg));
  // Figures may cite any item in the Hub. Each source is looked up by id, so a
  // valid one is never "missing" because of a list limit; a failed lookup is
  // "unknown", not broken.
  const cited = $derived(citedSourceIds(view.briefs));
  const srcQ = query(
    () => lookupSources(cited, (id) => hub.dispatch('tasks.get', { id }), (e) => e instanceof HubApiError && e.code === 'not_found'),
    () => [hub.itemsRev, hub.roleId, cited.join(',')],
  );
  const checks = $derived(view.briefs.map((b) => checkBrief(b, srcQ.data ?? new Map())));
  const blocking = $derived(checks.reduce((n, c) => n + c.blocking, 0));
  const unchecked = $derived(checks.reduce((n, c) => n + c.unchecked, 0));

  let open = $state<string | null>(null);
  let busy = $state(false);
  const key = (c: BriefCheck, i: number) => `${c.b.item.id}:${i}`;

  async function setStatus(c: BriefCheck, s: BriefStatus) {
    if (busy) return;
    busy = true;
    try {
      await setBriefStatus(c, s);
    } finally {
      busy = false;
    }
  }
  const STATUS_TONE: Record<BriefStatus, 'neutral' | 'success' | 'info'> = { draft: 'neutral', ready: 'success', presented: 'info' };
  function figAria(x: CheckedFigure): string {
    const v = `${x.f.label}: ${fmtNum(x.f.value, 2)} ${x.f.unit}`;
    if (x.state === 'sourced') return `${v}. Source: ${x.source?.title ?? x.f.sourceId}. Show source.`;
    if (x.state === 'unknown') return `${v}. Source ${x.f.sourceId} could not be checked. Blocks ready.`;
    if (x.state === 'broken') return `${v}. Source ${x.f.sourceId} not found. Blocks ready.`;
    return `${v}. Unsourced. Blocks ready.`;
  }
</script>

{#if (briefQ.loading && !briefQ.data) || (srcQ.loading && !srcQ.data)}
  <Skeleton lines={6} />
{:else if (briefQ.error && !briefQ.data) || (srcQ.error && !srcQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={briefQ.error ?? srcQ.error ?? undefined} guide={false} />
{:else if view.briefs.length === 0}
  <EmptyState compact icon={Presentation} title="No board brief" hint="Every figure must cite the item it came from before a section can be ready." />
{:else}
  <header class="top">
    <span class="when">{view.meeting ? `Board meeting ${dateLong(view.meeting)}` : 'Board brief'}</span>
    <Badge tone="neutral" icon={LockKeyhole} title="cso.unpublished: agents read these only with an explicit grant">Unpublished</Badge>
    {#if blocking}<Badge tone="danger" variant="solid" icon={CircleAlert} title="Figures without a valid source">{blocking} unsourced</Badge>{/if}
  </header>

  {#each checks as c (c.b.item.id)}
    <section class="sec" class:blocked={!c.canBeReady} aria-labelledby="{uid}-{c.b.item.id}">
      <div class="sec-head">
        <h3 id="{uid}-{c.b.item.id}">{c.b.section}</h3>
        <Badge tone={STATUS_TONE[c.b.status]} variant={c.b.status === 'draft' ? 'outline' : 'soft'}>{c.b.status}</Badge>
      </div>
      {#if c.b.narrative}<p class="narr">{c.b.narrative}</p>{/if}

      {#if c.figures.length}
        <ul class="figs" aria-label="Figures in {c.b.section}">
          {#each c.figures as x, i (i)}
            {@const k = key(c, i)}
            <li class="fig {x.state}">
              <button type="button" class="fig-btn" aria-expanded={open === k} aria-controls="{uid}-src-{k}" aria-label={figAria(x)} onclick={() => (open = open === k ? null : k)}>
                <span class="lbl">{x.f.label}</span>
                <span class="val"><span class="num">{fmtNum(x.f.value, 2)}</span> <span class="unit">{x.f.unit}</span></span>
                {#if x.state === 'unsourced'}
                  <span class="flag"><CircleAlert size={11} aria-hidden="true" /> unsourced</span>
                {:else if x.state === 'broken'}
                  <span class="flag"><Link2Off size={11} aria-hidden="true" /> source missing</span>
                {:else if x.state === 'unknown'}
                  <span class="flag"><CircleAlert size={11} aria-hidden="true" /> source not checked</span>
                {:else}
                  <span class="cite mono">↗ {x.f.sourceId}</span>
                {/if}
              </button>
              {#if open === k}
                <div class="src" id="{uid}-src-{k}">
                  {#if x.state === 'sourced' && x.source}
                    {@const s = sourceSummary(x.source)}
                    <p class="src-title">{x.source.title}</p>
                    <p class="src-type mono">{s.type} · {x.source.id}</p>
                    {#if s.lines.length}<ul class="src-lines">{#each s.lines as l (l)}<li>{l}</li>{/each}</ul>{/if}
                    {#if x.f.note}<p class="src-note">{x.f.note}</p>{/if}
                    {#if !ambient}<Button size="sm" variant="ghost" icon={Crosshair} onclick={() => focusItem(x.source!.id)}>Focus this source</Button>{/if}
                  {:else if x.state === 'broken'}
                    <p class="src-warn">No item with id <span class="mono">{x.f.sourceId}</span> exists. Fix the citation before this section can be ready.</p>
                  {:else if x.state === 'unknown'}
                    <p class="src-warn">The source <span class="mono">{x.f.sourceId}</span> could not be looked up just now, so it is not confirmed. It will be checked again on the next refresh.</p>
                  {:else}
                    <p class="src-warn">This figure cites no source item. Add the item it came from, or remove the figure. It is never estimated.</p>
                  {/if}
                </div>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}

      {#if !ambient}
        <div class="acts">
          {#if !c.canBeReady}<span class="why" id="{uid}-why-{c.b.item.id}">{c.blocking ? `${c.blocking} figure${c.blocking === 1 ? '' : 's'} need a source before ready` : `${c.unchecked} source${c.unchecked === 1 ? '' : 's'} could not be checked yet`}</span>{/if}
          {#if c.b.status !== 'draft'}<Button size="sm" variant="ghost" disabled={busy} onclick={() => setStatus(c, 'draft')}>Back to draft</Button>{/if}
          {#if c.b.status === 'draft'}
            <Button size="sm" variant="primary" disabled={busy || !c.canBeReady} aria-describedby={c.canBeReady ? undefined : `${uid}-why-${c.b.item.id}`} onclick={() => setStatus(c, 'ready')}>Mark ready</Button>
          {:else if c.b.status === 'ready'}
            <Button size="sm" variant="secondary" disabled={busy || !c.canBeReady} onclick={() => setStatus(c, 'presented')}>Mark presented</Button>
          {/if}
        </div>
      {/if}
    </section>
  {/each}
{/if}

<style>
  .top {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .when {
    margin-right: auto;
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text-2);
  }
  .sec {
    padding: var(--space-2) 0;
    border-top: 1px solid var(--border);
  }
  .sec-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  h3 {
    margin: 0;
    flex: 1;
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .narr {
    margin: var(--space-1) 0 0;
    font-size: var(--text-xs);
    line-height: var(--leading-normal);
    color: var(--text-2);
  }
  .figs {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(130px, 1fr));
    gap: var(--space-1);
    margin: var(--space-2) 0 0;
    padding: 0;
    list-style: none;
  }
  .fig {
    min-width: 0;
  }
  .fig-btn {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 1px;
    width: 100%;
    padding: var(--space-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    color: var(--text);
    font: inherit;
    text-align: left;
    cursor: pointer;
    transition: border-color var(--dur-fast) var(--ease-out);
  }
  .fig-btn:hover {
    border-color: var(--accent);
  }
  .fig-btn:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .fig-btn[aria-expanded='true'] {
    border-color: var(--accent);
    background: var(--accent-soft);
  }
  .unsourced .fig-btn,
  .broken .fig-btn {
    border-style: dashed;
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .lbl {
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .val {
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
  }
  .unit {
    font-size: var(--text-2xs);
    font-weight: var(--weight-regular);
    color: var(--text-3);
  }
  .cite {
    font-size: var(--text-2xs);
    color: var(--accent-text);
  }
  .flag {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    color: var(--danger);
  }
  .src {
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .src p {
    margin: 0;
  }
  .src-title {
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  .src-type {
    color: var(--text-3);
  }
  .src-lines {
    margin: var(--space-1) 0;
    padding: 0;
    list-style: none;
  }
  .src-note {
    font-style: italic;
    margin-bottom: var(--space-1) !important;
  }
  .src-warn {
    color: var(--danger);
  }
  .acts {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: var(--space-1);
    margin-top: var(--space-2);
  }
  .why {
    margin-right: auto;
    font-size: var(--text-2xs);
    color: var(--danger);
  }
</style>
