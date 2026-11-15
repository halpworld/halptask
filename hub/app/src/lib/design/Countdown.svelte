<script lang="ts">
  import { clock } from '../stores/clock.svelte';

  interface Props {
    /** ISO date-time or Date. A bare YYYY-MM-DD means end of that local day. */
    target: string | Date;
    size?: 'sm' | 'md' | 'lg' | 'xl';
    /** Show seconds (off in ambient mode for calmer motion). */
    seconds?: boolean;
    label?: string;
  }

  let { target, size = 'md', seconds = true, label }: Props = $props();

  const targetMs = $derived.by(() => {
    if (target instanceof Date) return target.getTime();
    if (/^\d{4}-\d{2}-\d{2}$/.test(target)) return new Date(`${target}T23:59:59`).getTime();
    return new Date(target).getTime();
  });

  const parts = $derived.by(() => {
    const diff = targetMs - clock.now;
    const past = diff < 0;
    let s = Math.floor(Math.abs(diff) / 1000);
    const d = Math.floor(s / 86400);
    s -= d * 86400;
    const h = Math.floor(s / 3600);
    s -= h * 3600;
    const m = Math.floor(s / 60);
    s -= m * 60;
    return { past, d, h, m, s, total: Math.abs(diff) };
  });

  const pad = (n: number) => String(n).padStart(2, '0');
  const spoken = $derived(
    `${parts.past ? 'Overdue by ' : ''}${parts.d ? `${parts.d} days ` : ''}${parts.h} hours ${parts.m} minutes${label ? ` until ${label}` : ''}`,
  );
  const urgency = $derived(parts.past ? 'past' : parts.total < 6 * 3600_000 ? 'soon' : parts.total < 3 * 86400_000 ? 'near' : 'far');
</script>

<span class="countdown {size} {urgency}" role="timer" aria-live="off" aria-label={spoken}>
  {#if parts.past}<span class="sign" aria-hidden="true">+</span>{/if}
  {#if parts.d > 0}
    <span class="seg"><span class="n num">{parts.d}</span><span class="u">d</span></span>
  {/if}
  <span class="seg"><span class="n num">{parts.d > 0 ? parts.h : pad(parts.h)}</span><span class="u">h</span></span>
  <span class="seg"><span class="n num">{pad(parts.m)}</span><span class="u">m</span></span>
  {#if seconds && parts.d === 0}
    <span class="seg sec"><span class="n num">{pad(parts.s)}</span><span class="u">s</span></span>
  {/if}
</span>

<style>
  .countdown {
    display: inline-flex;
    align-items: baseline;
    gap: 0.32em;
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--text);
    line-height: 1;
  }
  .seg {
    display: inline-flex;
    align-items: baseline;
    gap: 0.06em;
  }
  .u {
    font-size: 0.5em;
    font-weight: var(--weight-medium);
    color: var(--text-3);
    letter-spacing: 0;
  }
  .sec .n {
    color: var(--text-2);
  }
  .sign {
    color: var(--danger);
  }
  .sm {
    font-size: var(--text-md);
  }
  .md {
    font-size: var(--text-2xl);
  }
  .lg {
    font-size: var(--text-4xl);
  }
  .xl {
    font-size: var(--text-5xl);
  }
  .soon .n {
    color: var(--accent-text);
  }
  .past .n {
    color: var(--danger);
  }
</style>
