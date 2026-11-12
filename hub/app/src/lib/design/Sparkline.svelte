<script lang="ts">
  interface Props {
    values: number[];
    /** Optional second series drawn dashed (e.g. a target path). */
    target?: number[];
    label: string;
    height?: number;
    fill?: boolean;
    tone?: 'accent' | 'success' | 'warning' | 'danger' | 'muted';
    /** Force a y-domain (otherwise min..max of both series). */
    min?: number;
    max?: number;
  }

  let { values, target, label, height = 40, fill = true, tone = 'accent', min, max }: Props = $props();
  const uid = $props.id();
  const W = 100;

  const domain = $derived.by(() => {
    const all = [...values, ...(target ?? [])].filter(Number.isFinite);
    const lo = min ?? (all.length ? Math.min(...all) : 0);
    const hi = max ?? (all.length ? Math.max(...all) : 1);
    return { lo, hi: hi === lo ? lo + 1 : hi };
  });

  function pts(series: number[]): [number, number][] {
    const n = series.length;
    const { lo, hi } = domain;
    const pad = 3;
    return series.map((v, i) => [n <= 1 ? W / 2 : (i / (n - 1)) * W, pad + (1 - (v - lo) / (hi - lo)) * (height - pad * 2)]);
  }

  function path(p: [number, number][]): string {
    return p.map(([x, y], i) => `${i ? 'L' : 'M'}${x.toFixed(2)} ${y.toFixed(2)}`).join(' ');
  }

  const main = $derived(pts(values));
  const line = $derived(path(main));
  const area = $derived(main.length ? `${line} L${W} ${height} L0 ${height} Z` : '');
  const tline = $derived(target ? path(pts(target)) : '');
  const last = $derived(main.at(-1));
</script>

<div class="spark {tone}" role="img" aria-label={label} style:height="{height}px">
  <svg viewBox="0 0 {W} {height}" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <linearGradient id="g-{uid}" x1="0" x2="0" y1="0" y2="1">
        <stop offset="0%" stop-color="currentColor" stop-opacity="0.22" />
        <stop offset="100%" stop-color="currentColor" stop-opacity="0" />
      </linearGradient>
    </defs>
    {#if fill && area}<path d={area} fill="url(#g-{uid})" stroke="none" />{/if}
    {#if tline}<path class="target" d={tline} />{/if}
    {#if line}<path class="line" d={line} />{/if}
  </svg>
  {#if last}<span class="dot" style:left="{last[0]}%" style:top="{(last[1] / height) * 100}%"></span>{/if}
</div>

<style>
  .spark {
    position: relative;
    width: 100%;
    color: var(--accent);
  }
  svg {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    overflow: visible;
  }
  .success {
    color: var(--success);
  }
  .warning {
    color: var(--warning);
  }
  .danger {
    color: var(--danger);
  }
  .muted {
    color: var(--text-3);
  }
  .line {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.75;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .target {
    fill: none;
    stroke: var(--text-3);
    stroke-width: 1.25;
    stroke-dasharray: 4 4;
    vector-effect: non-scaling-stroke;
  }
  .dot {
    position: absolute;
    width: 7px;
    height: 7px;
    margin: -3.5px 0 0 -3.5px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 0 2px var(--surface), 0 0 0 5px color-mix(in srgb, currentColor 18%, transparent);
  }
</style>
