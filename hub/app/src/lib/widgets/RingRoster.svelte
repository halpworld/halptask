<script lang="ts">
  import type { WidgetProps } from './types';
  import Ring from '../design/Ring.svelte';
  import EmptyState from '../design/EmptyState.svelte';

  let { widget, ambient }: WidgetProps = $props();

  interface Entry {
    name: string;
    progress: number;
    caption: string;
  }
  const entries = $derived.by<Entry[]>(() => {
    const raw = widget.config.items;
    if (!Array.isArray(raw)) return [];
    return raw.flatMap((r) => {
      if (!r || typeof r !== 'object' || Array.isArray(r)) return [];
      const o = r as Record<string, unknown>;
      return typeof o.name === 'string' ? [{ name: o.name, progress: typeof o.progress === 'number' ? o.progress : 0, caption: typeof o.caption === 'string' ? o.caption : '' }] : [];
    });
  });
  const tone = (p: number) => (p >= 0.8 ? 'success' : p >= 0.45 ? 'accent' : 'warning');
</script>

{#if entries.length === 0}
  <EmptyState compact title="No entries" hint="This card shows entries from its settings." />
{:else}
  <ul class="roster">
    {#each entries as e (e.name)}
      <li>
        <Ring value={e.progress} size={ambient ? 46 : 36} stroke={ambient ? 5 : 4} tone={tone(e.progress)} label={e.name}>
          <span class="num pct">{Math.round(e.progress * 100)}</span>
        </Ring>
        <span class="txt">
          <span class="name pii truncate">{e.name}</span>
          {#if e.caption}<span class="cap pii truncate">{e.caption}</span>{/if}
        </span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .roster {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
    gap: var(--space-3) var(--space-4);
  }
  li {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }
  .pct {
    font-size: 10px;
  }
  .txt {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .name {
    font-weight: var(--weight-medium);
    font-size: var(--text-sm);
  }
  .cap {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
</style>
