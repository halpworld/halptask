<script lang="ts">
  import { fly } from 'svelte/transition';
  import X from '@lucide/svelte/icons/x';
  import Bot from '@lucide/svelte/icons/bot';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import CircleAlert from '@lucide/svelte/icons/circle-alert';
  import Info from '@lucide/svelte/icons/info';
  import { toasts } from './toast.svelte';
  import { motion } from './motion.svelte';

  const icons = { neutral: Info, success: CircleCheck, danger: CircleAlert, agent: Bot } as const;
</script>

<div class="toasts" role="region" aria-label="Notifications" aria-live="polite">
  {#each toasts.items as t (t.id)}
    {@const Icon = icons[t.tone]}
    <div class="toast {t.tone}" role={t.tone === 'danger' ? 'alert' : 'status'} transition:fly={{ y: 12, duration: motion.ms(220) }}>
      <Icon size={16} aria-hidden="true" />
      <span class="msg">{t.message}</span>
      {#if t.action}
        {@const action = t.action}
        <button
          class="action"
          onclick={async () => {
            toasts.dismiss(t.id);
            await action.run();
          }}>{action.label}</button
        >
      {/if}
      <button class="close" aria-label="Dismiss notification" onclick={() => toasts.dismiss(t.id)}><X size={14} aria-hidden="true" /></button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: var(--space-5);
    bottom: var(--space-12);
    z-index: var(--z-toast);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    align-items: flex-end;
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    max-width: 440px;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    min-height: 44px;
    background: var(--surface);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-lg);
    box-shadow: var(--elev-3);
    font-size: var(--text-sm);
  }
  .toast > :global(svg) {
    flex: none;
    color: var(--text-3);
  }
  .success > :global(svg) {
    color: var(--success);
  }
  .danger > :global(svg) {
    color: var(--danger);
  }
  .agent > :global(svg) {
    color: var(--accent-text);
  }
  .msg {
    flex: 1;
    min-width: 0;
  }
  .action {
    border: 0;
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: var(--weight-semibold);
    font-size: var(--text-sm);
    padding: 6px 10px;
    border-radius: var(--radius-sm);
  }
  .close {
    border: 0;
    background: transparent;
    color: var(--text-3);
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border-radius: var(--radius-sm);
  }
  .close:hover {
    background: var(--surface-3);
    color: var(--text);
  }
</style>
