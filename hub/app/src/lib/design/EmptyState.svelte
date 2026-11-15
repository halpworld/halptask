<script lang="ts">
  import { getContext, type Snippet } from 'svelte';
  import type { IconComponent } from './icon';
  import FillGuide from '../widgets/FillGuide.svelte';
  import { FILL_KIND } from '../widgets/FillContext.svelte';

  interface Props {
    title: string;
    hint?: string;
    icon?: IconComponent;
    children?: Snippet;
    compact?: boolean;
    /** Show the card's "how to fill" guide (off for error states). */
    guide?: boolean;
  }
  let { title, hint, icon: Icon, children, compact = false, guide = true }: Props = $props();
  // Set by the dashboard grid for the card this empty state is in.
  const fillKind = getContext<(() => string) | undefined>(FILL_KIND);
</script>

<div class="empty" class:compact>
  {#if Icon}<span class="ic"><Icon size={compact ? 18 : 22} aria-hidden="true" /></span>{/if}
  <p class="t">{title}</p>
  {#if hint}<p class="h">{hint}</p>{/if}
  {#if children}<div class="a">{@render children()}</div>{/if}
  {#if guide && fillKind?.()}<div class="a"><FillGuide kind={fillKind()} /></div>{/if}
</div>

<style>
  .empty {
    /* min-height, not height: with the fill guide the content can be taller
       than a small card, and a fixed-height centred column would overflow
       upwards, out of scroll reach (review F2). */
    min-height: max(96px, 100%);
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    text-align: center;
    gap: var(--space-1);
    padding: var(--space-4);
    color: var(--text-3);
  }
  .ic {
    display: grid;
    place-items: center;
    width: 44px;
    height: 44px;
    border-radius: var(--radius-full);
    background: var(--accent-soft);
    color: var(--accent-text);
    margin-bottom: var(--space-2);
  }
  .compact .ic {
    width: 34px;
    height: 34px;
  }
  .t {
    color: var(--text-2);
    font-weight: var(--weight-medium);
  }
  .h {
    font-size: var(--text-sm);
    max-width: 36ch;
  }
  .a {
    margin-top: var(--space-3);
  }
</style>
