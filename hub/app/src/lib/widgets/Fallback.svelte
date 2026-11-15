<script lang="ts">
  import Puzzle from '@lucide/svelte/icons/puzzle';
  import type { WidgetProps } from './types';
  import EmptyState from '../design/EmptyState.svelte';

  let { widget }: WidgetProps = $props();
  // The kind string comes from layout data (possibly an agent or a module):
  // render it as text only.
</script>

<EmptyState compact icon={Puzzle} title="Unknown widget" hint="No renderer for “{widget.kind}”. It may come from a module that isn’t installed." />
