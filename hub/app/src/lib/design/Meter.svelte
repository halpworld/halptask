<script lang="ts">
  interface Props {
    value: number;
    max?: number;
    label: string;
    /** Fractions of max at which tone shifts to warning / danger. */
    warnAt?: number;
    dangerAt?: number;
    variant?: 'bar' | 'gauge';
    /** Text shown in the gauge centre; defaults to a percentage. */
    display?: string;
    size?: number;
    /** When true, low values are bad (e.g. data coverage). */
    invert?: boolean;
  }

  let { value, max = 1, label, warnAt = 0.7, dangerAt = 0.9, variant = 'bar', display, size = 120, invert = false }: Props = $props();

  const frac = $derived(Math.max(0, Math.min(1, max > 0 ? value / max : 0)));
  const tone = $derived.by(() => {
    const f = invert ? 1 - frac : frac;
    return f >= dangerAt ? 'danger' : f >= warnAt ? 'warning' : 'ok';
  });
  const text = $derived(display ?? `${Math.round(frac * 100)}%`);

  // Gauge: 220° arc.
  const SWEEP = 220;
  const r = $derived(size / 2 - 8);
  const arcLen = $derived((SWEEP / 360) * 2 * Math.PI * r);
  const circ = $derived(2 * Math.PI * r);
</script>

{#if variant === 'bar'}
  <div class="bar {tone}" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={max} aria-valuenow={value} aria-valuetext={text}>
    <div class="fill" style:transform="scaleX({frac})"></div>
  </div>
{:else}
  <div class="gauge {tone}" style:width="{size}px" style:height="{size * 0.78}px" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={max} aria-valuenow={value} aria-valuetext={text}>
    <svg viewBox="0 0 {size} {size}" width={size} height={size} aria-hidden="true">
      <g transform="rotate({90 + (360 - SWEEP) / 2} {size / 2} {size / 2})">
        <circle class="track" cx={size / 2} cy={size / 2} {r} stroke-dasharray="{arcLen} {circ}" />
        <circle class="value" cx={size / 2} cy={size / 2} {r} stroke-dasharray="{arcLen * frac} {circ}" />
      </g>
    </svg>
    <span class="g-text num">{text}</span>
  </div>
{/if}

<style>
  .bar,
  .gauge {
    --c: var(--accent);
  }
  .warning {
    --c: var(--warning);
  }
  .danger {
    --c: var(--danger);
  }
  .bar {
    position: relative;
    height: 6px;
    border-radius: var(--radius-full);
    background: var(--surface-3);
    overflow: hidden;
  }
  .fill {
    position: absolute;
    inset: 0;
    background: var(--c);
    border-radius: inherit;
    transform-origin: left center;
    transition:
      transform var(--dur-layout) var(--ease-out),
      background-color var(--dur-slow) var(--ease-in-out);
  }
  .gauge {
    position: relative;
    display: inline-block;
    overflow: hidden;
  }
  .gauge svg {
    position: absolute;
    top: 0;
    left: 0;
  }
  circle {
    fill: none;
    stroke-width: 9;
    stroke-linecap: round;
  }
  .track {
    stroke: var(--surface-3);
  }
  .value {
    stroke: var(--c);
    transition:
      stroke-dasharray var(--dur-layout) var(--ease-out),
      stroke var(--dur-slow) var(--ease-in-out);
  }
  .g-text {
    position: absolute;
    left: 0;
    right: 0;
    top: 50%;
    transform: translateY(-50%);
    text-align: center;
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
  }
</style>
