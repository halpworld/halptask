<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    text: string;
    placement?: 'top' | 'bottom';
    children: Snippet;
  }
  let { text, placement = 'top', children }: Props = $props();
  const id = $props.id();
</script>

<!-- Wrapper shows the tip on hover and when anything inside has keyboard focus.
     The trigger should reference it with aria-describedby="tip-{id}" when it
     is not already labelled with the same text. -->
<span class="tip-wrap" data-tip-id="tip-{id}">
  {@render children()}
  <span class="tip {placement}" role="tooltip" id="tip-{id}">{text}</span>
</span>

<style>
  .tip-wrap {
    position: relative;
    display: inline-flex;
  }
  .tip {
    position: absolute;
    left: 50%;
    z-index: var(--z-toast);
    padding: 5px 8px;
    border-radius: var(--radius-sm);
    background: var(--text);
    color: var(--bg);
    font-size: var(--text-xs);
    font-weight: var(--weight-medium);
    line-height: 1.3;
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transform: translate(-50%, 4px);
    transition:
      opacity var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-out);
    box-shadow: var(--elev-2);
  }
  .top {
    bottom: calc(100% + 6px);
  }
  .bottom {
    top: calc(100% + 6px);
    transform: translate(-50%, -4px);
  }
  .tip-wrap:hover .tip,
  .tip-wrap:focus-within .tip {
    opacity: 1;
    transform: translate(-50%, 0);
    transition-delay: 350ms;
  }
  .tip-wrap:focus-within .tip {
    transition-delay: 0ms;
  }
</style>
