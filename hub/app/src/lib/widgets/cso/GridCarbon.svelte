<script lang="ts">
  import Zap from '@lucide/svelte/icons/zap';
  import ZapOff from '@lucide/svelte/icons/zap-off';
  import Clock from '@lucide/svelte/icons/clock';
  import type { WidgetProps } from '../types';
  import { clock } from '../../stores/clock.svelte';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import Sparkline from '../../design/Sparkline.svelte';
  import { typedItems } from './actions';
  import { CSO_TYPES, DEFAULT_BANDS, DEFAULT_STALE_MINUTES, energyViews, intIn, numIn, type Band, type EnergyView } from './logic';
  import { ageLabel, clockLabel, fmtNum } from './format';

  let { widget, ambient }: WidgetProps = $props();

  const minute = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const opts = $derived({
    zone: typeof widget.config.zone === 'string' && widget.config.zone ? widget.config.zone : undefined,
    staleMinutes: intIn(widget.config.stale_minutes, 5, 1440, DEFAULT_STALE_MINUTES),
    low: numIn(widget.config.low, 0, 3000, DEFAULT_BANDS.low),
    high: numIn(widget.config.high, 0, 3000, DEFAULT_BANDS.high),
  });
  const q = typedItems(CSO_TYPES.energy);
  const views = $derived(energyViews(q.data?.items ?? [], minute, opts));
  const main = $derived(views[0]);
  const rest = $derived(views.slice(1, 4));

  const BAND: Record<Band, { label: string; tone: 'success' | 'warning' | 'danger' }> = {
    low: { label: 'Clean', tone: 'success' },
    mid: { label: 'Moderate', tone: 'warning' },
    high: { label: 'Carbon heavy', tone: 'danger' },
  };
  const SOURCE = { electricity_maps: 'Electricity Maps', mqtt: 'Site meter (MQTT)', manual: 'Manual entry' } as const;
  const place = (v: EnergyView) => v.r.zoneLabel ?? v.r.site ?? v.r.zone;
  const sparkTone = (v: EnergyView) => (v.freshness === 'stale' ? 'muted' : v.band === 'high' ? 'danger' : v.band === 'mid' ? 'warning' : 'success');
</script>

{#if q.loading && !q.data}
  <Skeleton lines={4} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if !main}
  <EmptyState compact icon={ZapOff} title="No grid data" hint="Nothing is shown until a real reading arrives; no figure is ever made up." />
{:else}
  <div class="wrap" class:stale={main.freshness === 'stale'}>
    <div class="now" role="group" aria-label="Grid carbon intensity, {place(main)}">
      <span class="place">{place(main)}</span>
      {#if main.r.gco2 !== undefined}
        <span class="reading">
          <span class="num big">{fmtNum(main.r.gco2, 0)}</span>
          <span class="unit">gCO₂/kWh</span>
          {#if main.r.estimated}<Badge tone="neutral" title="The provider estimated this value; it is not a measured reading.">Estimated</Badge>{/if}
        </span>
      {:else}
        <span class="reading"><span class="nodata">No intensity in the last reading</span></span>
      {/if}
      <span class="tags">
        {#if main.freshness === 'stale'}
          <Badge tone="warning" variant="solid" icon={Clock} title="Last reading at {clockLabel(main.r.at)}. Older than {opts.staleMinutes} minutes.">Stale · {ageLabel(main.ageMinutes)}</Badge>
        {:else}
          {#if main.band}<Badge tone={BAND[main.band].tone} title="Below {opts.low} is clean, above {opts.high} is carbon heavy">{BAND[main.band].label}</Badge>{/if}
          <span class="age"><span class="live-dot" aria-hidden="true"></span>{ageLabel(main.ageMinutes)}</span>
        {/if}
      </span>
    </div>

    {#if main.intensity.length >= 2}
      <div class="spark">
        <Sparkline values={main.intensity} label="Carbon intensity, last 24 hours, {main.intensity.length} readings, from {fmtNum(main.low ?? 0, 0)} to {fmtNum(main.high ?? 0, 0)} gCO2 per kWh" height={ambient ? 56 : 44} tone={sparkTone(main)} min={0} />
        <span class="range num">24 h · low {fmtNum(main.low ?? 0, 0)} · high {fmtNum(main.high ?? 0, 0)}</span>
      </div>
    {:else}
      <p class="thin">Not enough readings for a 24-hour trend yet.</p>
    {/if}

    <p class="facts">
      {#if main.r.powerKw !== undefined}<span><Zap size={11} aria-hidden="true" /> <span class="num">{fmtNum(main.r.powerKw)}</span> kW on site</span>{/if}
      {#if main.r.renewablePct !== undefined}<span><span class="num">{fmtNum(main.r.renewablePct, 0)}</span>% renewable</span>{/if}
      <span class="src">{SOURCE[main.r.source]}</span>
    </p>

    {#if rest.length}
      <ul class="others" aria-label="Other zones">
        {#each rest as v (v.r.item.id)}
          <li class:stale={v.freshness === 'stale'}>
            <span class="truncate">{place(v)}</span>
            <span class="num">{v.r.gco2 !== undefined ? fmtNum(v.r.gco2, 0) : '–'}</span>
            <span class="age">{v.freshness === 'stale' ? `stale · ${ageLabel(v.ageMinutes)}` : ageLabel(v.ageMinutes)}</span>
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    height: 100%;
    min-height: 0;
  }
  .now {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .place {
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    color: var(--text-2);
  }
  .reading {
    display: flex;
    align-items: baseline;
    gap: var(--space-1);
  }
  .big {
    font-size: var(--text-4xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    line-height: 1;
    color: var(--text);
  }
  .unit {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .nodata {
    font-size: var(--text-sm);
    color: var(--text-3);
    font-style: italic;
  }
  .stale .big {
    color: var(--text-3);
    text-decoration: line-through;
    text-decoration-thickness: 2px;
    text-decoration-color: var(--warning);
  }
  .tags {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-top: 2px;
  }
  .age {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .live-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  @media (prefers-reduced-motion: no-preference) {
    .live-dot {
      animation: pulse 2.4s var(--ease-out) infinite;
    }
  }
  @keyframes pulse {
    50% {
      box-shadow: 0 0 0 5px transparent;
    }
  }
  .spark {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .stale .spark {
    opacity: 0.55;
  }
  .range {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .thin {
    margin: 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .facts {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px var(--space-3);
    margin: 0;
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .facts > span {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .src {
    margin-left: auto;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .others {
    margin: 0;
    padding: var(--space-1) 0 0;
    list-style: none;
    border-top: 1px solid var(--border);
  }
  .others li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto auto;
    gap: var(--space-2);
    align-items: baseline;
    font-size: var(--text-xs);
    color: var(--text-2);
    padding: 2px 0;
  }
  .others li.stale .num {
    color: var(--text-3);
  }
</style>
