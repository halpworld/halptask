<script lang="ts">
  // The card a Tier C widget shows when it cannot run here. On Windows,
  // Tier C is off in this prototype (P5-EXTENSIONS.md §6.4): no iframe is
  // ever created there.
  import Puzzle from '@lucide/svelte/icons/puzzle';
  import EmptyState from '../../design/EmptyState.svelte';
  import { unavailableText } from './text';

  let { reason, kind }: { reason: string; kind: string } = $props();
  const t = $derived(unavailableText(reason));
</script>

<EmptyState compact icon={Puzzle} title={t.title} hint={t.hint} />
<span class="kind">{kind}</span>

<style>
  .kind {
    display: block;
    text-align: center;
    font-size: var(--text-2xs);
    color: var(--text-3);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
