<script lang="ts">
  import TrendingDown from '@lucide/svelte/icons/trending-down';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { saveConfig, typedItems } from './actions';
  import { CSO_TYPES, readTarget, TARGET_SCOPE_LABEL, trajectoryModel, type Target } from './logic';
  import { fmtNum, fmtPct } from './format';

  let { widget, ambient }: WidgetProps = $props();
  const uid = $props.id();

  const year = $derived(hub.day?.date ? Number(hub.day.date.slice(0, 4)) : new Date(clock.now).getFullYear());
  const q = typedItems(CSO_TYPES.target);
  const targets = $derived((q.data?.items ?? []).map(readTarget).filter((t): t is Target => !!t));
  const picked = $derived(typeof widget.config.target_id === 'string' ? widget.config.target_id : undefined);
  const target = $derived(targets.find((t) => t.item.id === picked) ?? targets[0]);
  const m = $derived(target ? trajectoryModel(target, year, { width: 340, height: 170 }) : undefined);
  const unsourced = $derived(m ? m.points.filter((p) => !p.sourced) : []);

  function pick(e: Event) {
    const id = (e.currentTarget as HTMLSelectElement).value;
    void saveConfig(widget.id, { ...widget.config, target_id: id });
  }
  const VALIDATION: Record<string, string> = { validated: 'SBTi validated', committed: 'SBTi committed', none: 'Not validated' };

  const summary = $derived.by(() => {
    if (!target || !m) return '';
    const pts = m.points.map((p) => `${p.year}: ${fmtNum(p.value)}${p.sourced ? '' : ' (unsourced)'}`).join('; ');
    const gaps = m.gaps.length ? ` No figure for ${m.gaps.map((g) => g.year).join(', ')}.` : '';
    return `${target.name}. Base ${target.baseYear}: ${fmtNum(target.baseValue)} ${target.unit}. Target ${target.targetYear}: ${fmtNum(target.targetValue)} ${target.unit}. Reported: ${pts || 'none'}.${gaps}`;
  });
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if !target || !m}
  <EmptyState compact icon={TrendingDown} title="No targets" hint="Each target appears with its base year, its goal and actual figures, each with its source." />
{:else}
  <div class="wrap">
    <header class="head">
      <span class="title">
        {#if targets.length > 1 && !ambient}
          <label class="visually-hidden" for="{uid}-pick">Target to chart</label>
          <select id="{uid}-pick" value={target.item.id} onchange={pick}>
            {#each targets as t (t.item.id)}<option value={t.item.id}>{t.name}</option>{/each}
          </select>
        {:else}
          <span class="name">{target.name}</span>
        {/if}
        <span class="sub">{TARGET_SCOPE_LABEL[target.scope]} · {target.unit}{target.validation ? ` · ${VALIDATION[target.validation]}` : ''}</span>
      </span>
      {#if m.latestSourced}
        <span class="kpi">
          <span class="num val">{fmtNum(m.latestSourced.value)}</span>
          <span class="kpi-sub">
            {m.latestSourced.year}
            {#if m.changeVsBase !== undefined}<span class="num"> · {fmtPct(m.changeVsBase)} vs {target.baseYear}</span>{/if}
          </span>
        </span>
      {:else}
        <Badge tone="warning" variant="outline" title="No reported figure has a source yet">No sourced figure</Badge>
      {/if}
    </header>

    <svg class="chart" viewBox="0 0 {m.width} {m.height}" role="img" aria-labelledby="{uid}-desc" preserveAspectRatio="xMidYMid meet">
      <desc id="{uid}-desc">{summary}</desc>
      <defs>
        <pattern id="{uid}-hatch" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <line class="hatch" x1="0" y1="0" x2="0" y2="6" />
        </pattern>
      </defs>
      {#each m.ticks as t (t.value)}
        <line class="grid" x1={m.plot.left} x2={m.plot.right} y1={t.y} y2={t.y} />
        <text class="ylab num" x={m.plot.left - 5} y={t.y + 3}>{fmtNum(t.value, 0)}</text>
      {/each}
      {#each m.years.filter((y) => y.label) as y (y.year)}
        <text class="xlab num" x={y.x} y={m.plot.bottom + 13}>{y.year}</text>
      {/each}
      {#each m.gaps as g (g.year)}
        <rect class="gap" x={g.x - 5} y={m.plot.top} width="10" height={m.plot.bottom - m.plot.top} fill="url(#{uid}-hatch)" />
      {/each}
      <line class="pathway" x1={m.pathway.x1} y1={m.pathway.y1} x2={m.pathway.x2} y2={m.pathway.y2} />
      <circle class="goal" cx={m.pathway.x2} cy={m.pathway.y2} r="4" />
      {#each m.segments as d, i (i)}
        <path class="actual" {d} />
      {/each}
      {#each m.points as p (p.year)}
        <circle class="pt" class:unsourced={!p.sourced} cx={p.x} cy={p.y} r={p.sourced ? 3.2 : 3.6}>
          <title>{p.year}: {fmtNum(p.value)} {target.unit}{p.sourced ? ` (source: ${p.sourceRef})` : ' (unsourced)'}</title>
        </circle>
        {#if !p.sourced}<text class="flag" x={p.x} y={p.y - 7}>unsourced</text>{/if}
      {/each}
    </svg>

    <p class="legend">
      <span class="k actual-k" aria-hidden="true"></span> reported
      <span class="k path-k" aria-hidden="true"></span> target pathway
      {#if unsourced.length}<span class="k open-k" aria-hidden="true"></span> unsourced{/if}
      {#if m.gaps.length}<span class="k gap-k" aria-hidden="true"></span> gap: no figure ({m.gaps.map((g) => g.year).join(', ')}){/if}
    </p>
  </div>
{/if}

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    height: 100%;
    min-height: 0;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .title {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name,
  select {
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    color: var(--text);
  }
  select {
    max-width: 100%;
    padding: 1px 4px;
    margin-left: -4px;
    border: 1px solid transparent;
    border-radius: var(--radius-sm);
    background: transparent;
    font-family: inherit;
  }
  select:hover {
    border-color: var(--border);
  }
  select:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .sub {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .kpi {
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    line-height: 1.1;
  }
  .val {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    color: var(--accent-text);
    letter-spacing: var(--tracking-tight);
  }
  .kpi-sub {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .chart {
    width: 100%;
    flex: 1;
    min-height: 0;
    overflow: visible;
  }
  .grid {
    stroke: var(--border);
    stroke-width: 1;
  }
  .ylab {
    fill: var(--text-3);
    font-size: 9px;
    text-anchor: end;
  }
  .xlab {
    fill: var(--text-3);
    font-size: 9px;
    text-anchor: middle;
  }
  .hatch {
    stroke: var(--warning);
    stroke-width: 1.5;
    opacity: 0.45;
  }
  .gap {
    stroke: none;
  }
  .pathway {
    stroke: var(--text-3);
    stroke-width: 1.5;
    stroke-dasharray: 4 4;
  }
  .goal {
    fill: var(--surface);
    stroke: var(--text-2);
    stroke-width: 1.5;
  }
  .actual {
    fill: none;
    stroke: var(--accent);
    stroke-width: 2.2;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .pt {
    fill: var(--accent);
    stroke: var(--surface);
    stroke-width: 1.5;
  }
  .pt.unsourced {
    fill: var(--surface);
    stroke: var(--warning);
    stroke-width: 1.8;
    stroke-dasharray: 2 1.5;
  }
  .flag {
    fill: var(--warning);
    font-size: 8px;
    font-weight: 600;
    text-anchor: middle;
  }
  .legend {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px 4px;
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .k {
    display: inline-block;
    width: 14px;
    height: 0;
    margin-left: var(--space-2);
    vertical-align: middle;
  }
  .k:first-child {
    margin-left: 0;
  }
  .actual-k {
    border-top: 2.5px solid var(--accent);
  }
  .path-k {
    border-top: 1.5px dashed var(--text-3);
  }
  .open-k {
    width: 9px;
    height: 9px;
    border-radius: var(--radius-full);
    border: 1.5px dashed var(--warning);
  }
  .gap-k {
    width: 10px;
    height: 10px;
    background: repeating-linear-gradient(45deg, transparent 0 2px, var(--warning) 2px 3px);
  }
</style>
