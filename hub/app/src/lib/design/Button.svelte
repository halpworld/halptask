<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { IconComponent } from './icon';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  interface Props extends HTMLButtonAttributes {
    variant?: 'primary' | 'secondary' | 'ghost' | 'danger';
    size?: 'sm' | 'md' | 'lg';
    icon?: IconComponent;
    children?: Snippet;
  }

  let { variant = 'secondary', size = 'md', icon: Icon, children, type = 'button', class: cls = '', ...rest }: Props = $props();
</script>

<button {type} class="btn {variant} {size} {cls}" {...rest}>
  {#if Icon}<Icon size={size === 'sm' ? 14 : size === 'lg' ? 18 : 16} aria-hidden="true" />{/if}
  {#if children}<span class="btn-label">{@render children()}</span>{/if}
</button>

<style>
  .btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    font-weight: var(--weight-medium);
    line-height: 1;
    white-space: nowrap;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  .btn:active:not(:disabled) {
    transform: translateY(1px) scale(0.99);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .btn:focus-visible {
    border-radius: var(--radius-md);
  }
  .sm {
    height: 28px;
    padding: 0 var(--space-3);
    font-size: var(--text-sm);
  }
  .md {
    height: 34px;
    padding: 0 var(--space-4);
    font-size: var(--text-md);
  }
  .lg {
    height: 42px;
    padding: 0 var(--space-5);
    font-size: var(--text-lg);
  }
  .primary {
    background: var(--accent);
    color: var(--text-on-accent);
    box-shadow: var(--elev-1);
  }
  .primary:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 88%, var(--text));
  }
  .secondary {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
    box-shadow: var(--elev-1);
  }
  .secondary:hover:not(:disabled) {
    background: var(--surface-2);
    border-color: var(--border-strong);
  }
  .ghost {
    background: transparent;
    color: var(--text-2);
  }
  .ghost:hover:not(:disabled) {
    background: var(--surface-3);
    color: var(--text);
  }
  .danger {
    background: var(--danger);
    color: var(--surface);
  }
  .danger:hover:not(:disabled) {
    background: color-mix(in srgb, var(--danger) 85%, var(--text));
  }
</style>
