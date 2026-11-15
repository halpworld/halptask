<script lang="ts">
  import Bot from '@lucide/svelte/icons/bot';
  import User from '@lucide/svelte/icons/user';
  import Plug from '@lucide/svelte/icons/plug';
  import Cog from '@lucide/svelte/icons/cog';
  import { isAgent } from '../../stores/hub.svelte';

  let { actor, size = 22 }: { actor: string; size?: number } = $props();
  const kind = $derived(isAgent(actor) ? 'agent' : actor === 'user' ? 'user' : actor.startsWith('integration:') || actor.startsWith('module:') ? 'integration' : 'system');
  const Icon = $derived(kind === 'agent' ? Bot : kind === 'user' ? User : kind === 'integration' ? Plug : Cog);
</script>

<span class="av {kind}" style:width="{size}px" style:height="{size}px" aria-hidden="true"><Icon size={Math.round(size * 0.58)} /></span>

<style>
  .av {
    flex: none;
    display: inline-grid;
    place-items: center;
    border-radius: 50%;
    background: var(--surface-3);
    color: var(--text-2);
  }
  .agent {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
</style>
