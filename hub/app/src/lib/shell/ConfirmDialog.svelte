<script lang="ts">
  import { hub } from '../stores/hub.svelte';
  import Dialog from '../design/Dialog.svelte';
  import Button from '../design/Button.svelte';

  // Local yes/no questions (e.g. "complete the whole group?"). Agent consent
  // has its own dialog; this one never approves anything on the core's behalf.
  const req = $derived(hub.confirmRequest);
</script>

<Dialog open={!!req} title={req?.title ?? ''} size="sm" onclose={() => req?.resolve(false)}>
  {#if req}<p class="msg pii">{req.message}</p>{/if}
  {#snippet footer()}
    <Button variant="secondary" onclick={() => req?.resolve(false)}>Cancel</Button>
    <Button variant="primary" onclick={() => req?.resolve(true)}>{req?.confirmLabel ?? 'Continue'}</Button>
  {/snippet}
</Dialog>

<style>
  .msg {
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
</style>
