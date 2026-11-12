<script lang="ts">
  interface Props {
    /** e.g. "mod+k", "g d", "?" — "mod" renders as ⌘ on Mac, Ctrl elsewhere. */
    keys: string;
  }
  let { keys }: Props = $props();

  const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const pretty = (k: string): string => {
    const map: Record<string, string> = { mod: isMac ? '⌘' : 'Ctrl', shift: '⇧', alt: isMac ? '⌥' : 'Alt', enter: '↵', esc: 'Esc', up: '↑', down: '↓', left: '←', right: '→' };
    return map[k.toLowerCase()] ?? (k.length === 1 ? k.toUpperCase() : k);
  };
  const chords = $derived(keys.split(' ').map((c) => c.split('+').map(pretty)));
</script>

<span class="kbd-group">
  {#each chords as chord, i (i)}
    {#if i > 0}<span class="then">then</span>{/if}
    {#each chord as k, j (j)}<kbd>{k}</kbd>{/each}
  {/each}
</span>

<style>
  .kbd-group {
    display: inline-flex;
    align-items: center;
    gap: 3px;
  }
  .then {
    font-size: var(--text-2xs);
    color: var(--text-3);
    padding: 0 2px;
  }
  kbd {
    display: inline-grid;
    place-items: center;
    min-width: 20px;
    height: 20px;
    padding: 0 5px;
    font-family: var(--font-sans);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    line-height: 1;
    color: var(--text-2);
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-bottom-width: 2px;
    border-radius: var(--radius-xs);
  }
</style>
