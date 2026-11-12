<script lang="ts">
  // A Tier B `computed` widget (docs/hub/P5-EXTENSIONS.md §5): runs the
  // pack transform named in its config through `ext_run_transform` and shows
  // the checked output as a metric, list, series or table. Text only: the
  // output is a pack's data, so nothing from it becomes HTML, a URL or a
  // style. The chrome names the publisher and trust like Tier C frames do.
  // Re-runs on item and day changes, at most once per 2 s per widget; on a
  // failed refresh it keeps the last good output.
  import { onDestroy } from 'svelte';
  import Sigma from '@lucide/svelte/icons/sigma';
  import type { WidgetProps } from '../types';
  import EmptyState from '../../design/EmptyState.svelte';
  import Sparkline from '../../design/Sparkline.svelte';
  import { hub } from '../../stores/hub.svelte';
  import { Debouncer, isWidgetId, parseComputedConfig, parseOutput, runTransform, trustLabel, unavailableTransformText, type ParsedView, type TransformResult } from './computed';

  let { widget }: WidgetProps = $props();

  const cfg = $derived(parseComputedConfig(widget.config));
  const okCfg = $derived('error' in cfg ? null : cfg);
  /** Re-run key: the transform and the widget it runs for. */
  const key = $derived(okCfg && isWidgetId(widget.id) ? `${okCfg.packId}/${okCfg.name}#${widget.id}` : '');

  type Problem = { state: 'unavailable' | 'locked' | 'browser' | 'failed'; reason?: string };
  let last = $state<{ result: TransformResult; parsed: ParsedView | { error: string } } | null>(null);
  let problem = $state<Problem | null>(null);
  let loading = $state(true);

  let debouncer: Debouncer | null = null;
  let gen = 0;

  async function refresh(c: NonNullable<typeof okCfg>, id: string, myGen: number) {
    const a = await runTransform(c.packId, c.name, id);
    if (myGen !== gen) return;
    loading = false;
    switch (a.state) {
      case 'ok':
        last = { result: a.result, parsed: parseOutput(c.view, a.result.output) };
        problem = null;
        if (a.result.log) console.debug(`halp: ${c.packId}/${c.name} log:\n${a.result.log}`);
        break;
      case 'busy':
        // Keep what is shown; the next change tries again.
        break;
      case 'failed':
        problem = { state: 'failed', reason: a.reason };
        break;
      default:
        // Unavailable, locked or browser: the last output no longer applies.
        last = null;
        problem = { state: a.state };
    }
  }

  // A new debouncer per transform/widget; the first run is immediate.
  $effect(() => {
    const k = key;
    const c = okCfg;
    const id = widget.id;
    debouncer?.close();
    debouncer = null;
    last = null;
    problem = null;
    loading = true;
    const myGen = ++gen;
    if (!k || !c) return;
    const d = new Debouncer(() => refresh(c, id, myGen));
    debouncer = d;
    d.request();
    return () => d.close();
  });

  // Data changes: items (tasks, captures, synced items, undo) and the day
  // state (focus, role switch, counts).
  let primed = false;
  $effect(() => {
    void hub.itemsRev;
    void hub.day;
    if (primed) debouncer?.request();
    primed = true;
  });

  const view = $derived(okCfg?.view ?? 'metric');
  const parsed = $derived(last?.parsed ?? null);
  const trust = $derived(last?.result.trust ?? 'unknown');
  const publisher = $derived(last ? last.result.publisher || last.result.title || okCfg?.packId || '' : (okCfg?.packId ?? ''));
  const unavailable = $derived(problem && !last ? unavailableTransformText(problem.state, problem.reason) : null);

  onDestroy(() => {
    gen++;
    debouncer?.close();
  });
</script>

{#if 'error' in cfg}
  <EmptyState compact icon={Sigma} title="Computed widget" hint={cfg.error} />
{:else if unavailable}
  <EmptyState compact icon={Sigma} title={unavailable.title} hint={unavailable.hint} />
{:else if loading && !last}
  <div class="loading" aria-busy="true"></div>
{:else if last && parsed}
  <div class="computed" class:dev={trust === 'dev'} class:blurred={hub.privacy}>
    <div class="chrome" title="Computed by a module from {publisher}. It runs in a sandbox and can only read general data.">
      <span class="pub">{publisher}</span>
      {#if problem}<span class="stale" title="The last refresh failed; this is the previous result.">stale</span>{/if}
      <span class="trust trust-{trust}">{trustLabel(trust)}</span>
    </div>
    <div class="body">
      {#if 'error' in parsed}
        <p class="misfit">{parsed.error}</p>
      {:else if parsed.view === 'metric'}
        <div class="metric">
          <span class="value num">{parsed.value}{#if parsed.unit}<span class="unit">{parsed.unit}</span>{/if}</span>
          {#if parsed.delta}<span class="delta delta-{parsed.trend}">{parsed.delta}</span>{/if}
          {#if parsed.label}<span class="lbl">{parsed.label}</span>{/if}
          {#if parsed.hint}<span class="hint">{parsed.hint}</span>{/if}
        </div>
      {:else if parsed.view === 'list'}
        {#if parsed.items.length === 0}
          <p class="misfit">Nothing to show.</p>
        {:else}
          <ul class="list">
            {#each parsed.items as it, i (i)}
              <li>
                <span class="li-label">{it.label}</span>
                {#if it.value}<span class="li-value num">{it.value}</span>{/if}
                {#if it.hint}<span class="li-hint">{it.hint}</span>{/if}
              </li>
            {/each}
          </ul>
          {#if parsed.more}<p class="more">and {parsed.more} more</p>{/if}
        {/if}
      {:else if parsed.view === 'series'}
        <div class="series">
          {#if parsed.label}<span class="lbl">{parsed.label}{#if parsed.unit}<span class="unit"> ({parsed.unit})</span>{/if}</span>{/if}
          <Sparkline values={parsed.points.map((p) => p.y)} label={parsed.label ?? 'Series'} height={56} min={parsed.min} max={parsed.max} />
          <div class="axis">
            <span>{parsed.firstX}</span>
            <span class="num">{parsed.lastY}</span>
            <span>{parsed.lastX}</span>
          </div>
        </div>
      {:else if parsed.view === 'table'}
        <div class="table-wrap">
          <table>
            {#if parsed.columns.some((c) => c !== '')}
              <thead><tr>{#each parsed.columns as c, i (i)}<th scope="col">{c}</th>{/each}</tr></thead>
            {/if}
            <tbody>
              {#each parsed.rows as r, i (i)}
                <tr>{#each r as cell, j (j)}<td>{cell}</td>{/each}</tr>
              {/each}
            </tbody>
          </table>
          {#if parsed.more}<p class="more">and {parsed.more} more rows</p>{/if}
        </div>
      {/if}
    </div>
  </div>
{:else}
  <EmptyState compact icon={Sigma} title="Computed widget" hint="No output yet ({view})." />
{/if}

<style>
  .loading {
    height: 100%;
    min-height: 48px;
  }
  .computed {
    height: 100%;
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-height: 0;
  }
  .chrome {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--text-2xs);
    color: var(--text-3);
    overflow: hidden;
  }
  .pub {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .trust,
  .stale {
    flex: none;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    border: 1px solid var(--border);
    border-radius: 4px;
    padding: 0 4px;
  }
  .trust-dev {
    color: #b45309;
    border-color: #d97706;
  }
  .dev .body {
    border-left: 3px solid #d97706;
    padding-left: 6px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    overflow-wrap: anywhere;
  }
  .blurred .body {
    filter: blur(8px);
    pointer-events: none;
    user-select: none;
  }
  .misfit,
  .more,
  .hint,
  .li-hint {
    color: var(--text-3);
    font-size: var(--text-xs);
    margin: 0;
  }
  .metric {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .value {
    font-size: var(--text-3xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    line-height: 1.05;
  }
  .unit {
    font-size: var(--text-sm);
    color: var(--text-3);
    margin-left: 2px;
  }
  .delta {
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .lbl {
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .list li {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: 0 8px;
    font-size: var(--text-sm);
  }
  .li-hint {
    grid-column: 1 / -1;
  }
  .series {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .axis {
    display: flex;
    justify-content: space-between;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .table-wrap {
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: var(--text-xs);
  }
  th,
  td {
    text-align: left;
    padding: 2px 6px;
    border-bottom: 1px solid var(--border);
  }
  th {
    color: var(--text-3);
    font-weight: var(--weight-medium, 500);
  }
</style>
