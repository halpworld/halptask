<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { IconComponent } from './icon';

  interface Props {
    tone?: 'neutral' | 'accent' | 'success' | 'warning' | 'danger' | 'info';
    variant?: 'soft' | 'outline' | 'solid';
    icon?: IconComponent;
    title?: string;
    children: Snippet;
  }

  let { tone = 'neutral', variant = 'soft', icon: Icon, title, children }: Props = $props();
</script>

<span class="badge {tone} {variant}" {title}>
  {#if Icon}<Icon size={11} strokeWidth={2.4} aria-hidden="true" />{/if}
  {@render children()}
</span>

<style>
  .badge {
    --c: var(--text-2);
    --bg-c: var(--neutral-soft);
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 20px;
    padding: 0 7px;
    border-radius: var(--radius-full);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    line-height: 1;
    white-space: nowrap;
    font-variant-numeric: tabular-nums;
    color: var(--c);
    background: var(--bg-c);
    border: 1px solid transparent;
    flex: none;
  }
  .accent {
    --c: var(--accent-text);
    --bg-c: var(--accent-soft);
  }
  .success {
    --c: var(--success);
    --bg-c: var(--success-soft);
  }
  .warning {
    --c: var(--warning);
    --bg-c: var(--warning-soft);
  }
  .danger {
    --c: var(--danger);
    --bg-c: var(--danger-soft);
  }
  .info {
    --c: var(--info);
    --bg-c: var(--info-soft);
  }
  .outline {
    background: transparent;
    border-color: color-mix(in srgb, var(--c) 40%, transparent);
  }
  .solid {
    background: var(--c);
    color: var(--surface);
  }
</style>
