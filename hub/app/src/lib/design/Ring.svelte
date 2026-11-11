<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    /** 0..1 */
    value: number;
    size?: number;
    stroke?: number;
    tone?: 'accent' | 'success' | 'warning' | 'danger' | 'muted';
    label: string;
    children?: Snippet;
  }

  let { value, size = 44, stroke = 4, tone = 'accent', label, children }: Props = $props();
  const r = $derived((size - stroke) / 2);
  const c = $derived(2 * Math.PI * r);
  const clamped = $derived(Math.max(0, Math.min(1, Number.isFinite(value) ? value : 0)));
</script>

<div class="ring {tone}" style:width="{size}px" style:height="{size}px" role="img" aria-label="{label}: {Math.round(clamped * 100)}%">
  <svg viewBox="0 0 {size} {size}" width={size} height={size} aria-hidden="true">
    <circle class="track" cx={size / 2} cy={size / 2} {r} stroke-width={stroke} />
    <circle
      class="value"
      cx={size / 2}
      cy={size / 2}
      {r}
      stroke-width={stroke}
      stroke-dasharray={c}
      stroke-dashoffset={c * (1 - clamped)}
      transform="rotate(-90 {size / 2} {size / 2})"
    />
  </svg>
  {#if children}<div class="center">{@render children()}</div>{/if}
</div>

<style>
  .ring {
    position: relative;
    display: inline-grid;
    place-items: center;
    flex: none;
    --c: var(--accent);
  }
  .success {
    --c: var(--success);
  }
  .warning {
    --c: var(--warning);
  }
  .danger {
    --c: var(--danger);
  }
  .muted {
    --c: var(--text-3);
  }
  svg {
    position: absolute;
    inset: 0;
  }
  circle {
    fill: none;
  }
  .track {
    stroke: var(--surface-3);
  }
  .value {
    stroke: var(--c);
    stroke-linecap: round;
    transition:
      stroke-dashoffset var(--dur-layout) var(--ease-out),
      stroke var(--dur-slow) var(--ease-in-out);
  }
  .center {
    position: relative;
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    font-variant-numeric: tabular-nums;
    color: var(--text-2);
  }
</style>
