<script lang="ts">
  // The fixed, non-dismissable strip shown in the app chrome while pack dev
  // mode is on (§3.8). It sits outside every widget, above dialogs, and has no
  // close button: only turning dev mode off (Settings > Packs) removes it.
  import { packs } from '../stores/packs.svelte';
  import { safe } from './packs';

  const path = $derived(packs.devPath);

  $effect(() => {
    document.documentElement.toggleAttribute('data-devmode', !!path);
    return () => document.documentElement.removeAttribute('data-devmode');
  });
</script>

{#if path}
  <div class="dev-strip" role="status" data-testid="dev-strip">DEV MODE: <span class="p">{safe(path)}</span></div>
{/if}

<style>
  .dev-strip {
    position: fixed;
    inset: 0 0 auto 0;
    height: 22px;
    z-index: calc(var(--z-consent) + 10);
    display: flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-2);
    padding: 0 var(--space-3);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    letter-spacing: 0.04em;
    color: var(--text);
    background: repeating-linear-gradient(135deg, var(--warning-soft) 0 10px, color-mix(in srgb, var(--warning) 45%, var(--surface)) 10px 20px);
    border-bottom: 2px solid var(--warning);
    white-space: nowrap;
    overflow: hidden;
  }
  .p {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Make room: the shell is 100vh tall, so shift and shrink it. */
  :global(html[data-devmode] .shell) {
    margin-top: 22px;
    height: calc(100dvh - 22px);
  }
</style>
