<script lang="ts">
  import TrendingUp from '@lucide/svelte/icons/trending-up';
  import TrendingDown from '@lucide/svelte/icons/trending-down';
  import type { WidgetProps } from './types';
  import { hub } from '../stores/hub.svelte';
  import Sparkline from '../design/Sparkline.svelte';

  let { widget }: WidgetProps = $props();
  const c = $derived(widget.config);

  // `source: "counts.overdue"` binds to day.state; otherwise static config.
  const value = $derived.by(() => {
    if (typeof c.source === 'string' && c.source.startsWith('counts.') && hub.day) {
      const key = c.source.slice(7) as keyof typeof hub.day.counts;
      return String(hub.day.counts[key] ?? '–');
    }
    return c.value === undefined || c.value === null ? '–' : String(c.value);
  });
  const label = $derived(typeof c.label === 'string' ? c.label : widget.title);
  const unit = $derived(typeof c.unit === 'string' ? c.unit : '');
  const delta = $derived(typeof c.delta === 'string' ? c.delta : '');
  const tone = $derived(c.tone === 'bad' ? 'bad' : c.tone === 'good' ? 'good' : 'neutral');
  const trend = $derived(Array.isArray(c.trend) ? c.trend.filter((v): v is number => typeof v === 'number') : []);
</script>

<div class="tile">
  <div class="row">
    <span class="value num">{value}{#if unit}<span class="unit">{unit}</span>{/if}</span>
    {#if delta}
      <span class="delta {tone}">
        {#if tone === 'bad'}<TrendingDown size={13} aria-hidden="true" />{:else}<TrendingUp size={13} aria-hidden="true" />{/if}
        {delta}
      </span>
    {/if}
  </div>
  <span class="lbl">{label}</span>
  {#if trend.length > 1}
    <div class="spark"><Sparkline values={trend} label="{label} trend" height={30} tone={tone === 'bad' ? 'warning' : 'accent'} /></div>
  {/if}
</div>

<style>
  .tile {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex-wrap: wrap;
  }
  .value {
    font-size: var(--text-3xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    line-height: 1.05;
  }
  .unit {
    font-size: 0.5em;
    color: var(--text-3);
    margin-left: 3px;
    font-weight: var(--weight-medium);
  }
  .delta {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text-3);
  }
  .delta.good {
    color: var(--success);
  }
  .delta.bad {
    color: var(--warning);
  }
  .lbl {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .spark {
    margin-top: auto;
  }
</style>
