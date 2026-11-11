<script lang="ts">
  import { onMount } from 'svelte';
  import { hub } from './lib/stores/hub.svelte';
  import { clock } from './lib/stores/clock.svelte';
  import HubShell from './lib/shell/HubShell.svelte';
  import KeyGate from './lib/shell/KeyGate.svelte';
  import CaptureWindow from './lib/shell/CaptureWindow.svelte';
  import { windowLabel } from './lib/api/shell';

  // The same bundle serves the main window and the quick-capture window.
  let label = $state<string | null>(null);
  onMount(() => {
    void windowLabel().then((l) => (label = l));
  });

  function start() {
    clock.start();
    void hub.init();
  }

  // Theme, role accent, ambient and privacy live on <html> so tokens cascade
  // everywhere, including dialogs in the top layer.
  $effect(() => {
    if (label !== 'main') return;
    const root = document.documentElement;
    root.dataset.theme = hub.resolvedTheme;
    root.dataset.role = hub.roleId;
    root.classList.toggle('ambient', hub.ambient);
    root.classList.toggle('privacy', hub.privacy);
    const meta = document.querySelector('meta[name="theme-color"]');
    if (meta) meta.setAttribute('content', hub.resolvedTheme === 'dark' ? '#0d0f12' : '#f3f2ee');
  });
</script>

{#if label === 'capture'}
  <CaptureWindow />
{:else if label === 'main'}
  <KeyGate onready={start}>
    <HubShell />
  </KeyGate>
{/if}
