<script lang="ts">
  import type { IconComponent } from './icon';
  import type { HTMLButtonAttributes } from 'svelte/elements';

  interface Props extends HTMLButtonAttributes {
    /** Accessible name; also used as the tooltip. Required. */
    label: string;
    icon: IconComponent;
    size?: 'sm' | 'md' | 'lg';
    pressed?: boolean;
    shortcut?: string;
  }

  let { label, icon: Icon, size = 'md', pressed, shortcut, type = 'button', class: cls = '', ...rest }: Props = $props();
  const px = $derived(size === 'sm' ? 14 : size === 'lg' ? 20 : 16);
</script>

<button
  {type}
  class="icon-btn {size} {cls}"
  aria-label={label}
  aria-pressed={pressed}
  title={shortcut ? `${label} (${shortcut})` : label}
  {...rest}
>
  <Icon size={px} aria-hidden="true" />
</button>

<style>
  .icon-btn {
    display: inline-grid;
    place-items: center;
    border: 1px solid transparent;
    border-radius: var(--radius-md);
    background: transparent;
    color: var(--text-2);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out);
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--surface-3);
    color: var(--text);
  }
  .icon-btn:active:not(:disabled) {
    transform: scale(0.94);
  }
  .icon-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }
  .icon-btn[aria-pressed='true'] {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .icon-btn:focus-visible {
    border-radius: var(--radius-md);
  }
  .sm {
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
  }
  .md {
    width: 32px;
    height: 32px;
  }
  .lg {
    width: 40px;
    height: 40px;
  }
</style>
