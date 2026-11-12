<script lang="ts">
  import type { Snippet } from 'svelte';
  import type { HTMLAttributes } from 'svelte/elements';
  import type { IconComponent } from './icon';

  interface Props extends HTMLAttributes<HTMLElement> {
    title?: string;
    icon?: IconComponent;
    /** Rendered at the right of the header (badges, buttons). */
    actions?: Snippet;
    /** Rendered at the left edge of the header (drag handle in edit mode). */
    lead?: Snippet;
    children?: Snippet;
    padded?: boolean;
    tone?: 'default' | 'accent' | 'inset';
    headingLevel?: 2 | 3;
  }

  let { title, icon: Icon, actions, lead, children, padded = true, tone = 'default', headingLevel = 2, class: cls = '', ...rest }: Props = $props();
</script>

<section class="panel {tone} {cls}" {...rest}>
  {#if title || actions || lead}
    <header class="panel-head">
      {#if lead}{@render lead()}{/if}
      {#if Icon}<span class="panel-icon"><Icon size={15} aria-hidden="true" /></span>{/if}
      {#if title}
        <svelte:element this={`h${headingLevel}`} class="panel-title truncate">{title}</svelte:element>
      {/if}
      <span class="spacer"></span>
      {#if actions}<div class="panel-actions">{@render actions()}</div>{/if}
    </header>
  {/if}
  <div class="panel-body" class:padded>
    {@render children?.()}
  </div>
</section>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    height: 100%;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--elev-1);
    overflow: hidden;
    transition:
      background-color var(--dur-slow) var(--ease-in-out),
      border-color var(--dur-slow) var(--ease-in-out),
      box-shadow var(--dur-base) var(--ease-out);
  }
  .panel.accent {
    background: linear-gradient(160deg, color-mix(in srgb, var(--accent-soft) 85%, var(--surface)) 0%, var(--surface) 70%);
    border-color: color-mix(in srgb, var(--accent) 22%, var(--border));
  }
  .panel.inset {
    background: var(--surface-inset);
    box-shadow: none;
  }
  .panel-head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 40px;
    padding: var(--space-2) var(--space-3) 0 var(--space-4);
    flex: none;
  }
  .panel-icon {
    display: inline-grid;
    place-items: center;
    color: var(--accent-text);
    flex: none;
  }
  .panel-title {
    font-family: var(--font-label, var(--font-sans));
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--label-tracking, var(--tracking-caps));
    text-transform: uppercase;
    color: var(--text-2);
  }
  .spacer {
    flex: 1;
  }
  .panel-actions {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex: none;
  }
  .panel-body {
    flex: 1;
    min-height: 0;
    overflow: auto;
    position: relative;
  }
  .panel-body.padded {
    padding: var(--space-2) var(--space-4) var(--space-4);
  }
</style>
