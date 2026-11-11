<script lang="ts">
  import type { Snippet } from 'svelte';
  import X from '@lucide/svelte/icons/x';

  interface Props {
    open: boolean;
    title: string;
    description?: string;
    /** Called for Esc, backdrop click and the close button. */
    onclose: () => void;
    children?: Snippet;
    footer?: Snippet;
    size?: 'sm' | 'md' | 'lg';
    /** Hide the close button and ignore backdrop clicks (consent dialog). */
    modal?: boolean;
    class?: string;
  }

  let { open, title, description, onclose, children, footer, size = 'md', modal = false, class: cls = '' }: Props = $props();

  let el: HTMLDialogElement | undefined = $state();
  const id = $props.id();

  // Native <dialog> + showModal(): focus trap, inert background, Esc and
  // top-layer stacking for free.
  $effect(() => {
    if (!el) return;
    if (open && !el.open) el.showModal();
    else if (!open && el.open) el.close();
  });

  function onCancel(e: Event) {
    e.preventDefault();
    onclose();
  }

  function onBackdrop(e: MouseEvent) {
    if (!modal && e.target === el) onclose();
  }
</script>

<dialog
  bind:this={el}
  class="dialog {size} {cls}"
  aria-labelledby="dlg-title-{id}"
  aria-describedby={description ? `dlg-desc-${id}` : undefined}
  oncancel={onCancel}
  onclick={onBackdrop}
>
  {#if open}
    <div class="inner">
      <header>
        <div class="titles">
          <h2 id="dlg-title-{id}">{title}</h2>
          {#if description}<p id="dlg-desc-{id}" class="desc">{description}</p>{/if}
        </div>
        {#if !modal}
          <button class="close" aria-label="Close" onclick={onclose}><X size={16} aria-hidden="true" /></button>
        {/if}
      </header>
      {#if children}<div class="body">{@render children()}</div>{/if}
      {#if footer}<footer>{@render footer()}</footer>{/if}
    </div>
  {/if}
</dialog>

<style>
  .dialog {
    padding: 0;
    border: 1px solid var(--border);
    border-radius: var(--radius-xl);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--elev-4);
    width: min(92vw, 520px);
    max-height: min(86vh, 760px);
    overflow: hidden;
  }
  .sm {
    width: min(92vw, 400px);
  }
  .lg {
    width: min(94vw, 720px);
  }
  .dialog[open] {
    animation: dlg-in var(--dur-base) var(--ease-out);
  }
  .dialog::backdrop {
    background: var(--scrim);
    backdrop-filter: blur(3px);
    animation: fade-in var(--dur-base) var(--ease-out);
  }
  @keyframes dlg-in {
    from {
      opacity: 0;
      transform: translateY(8px) scale(0.98);
    }
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
  .inner {
    display: flex;
    flex-direction: column;
    max-height: inherit;
  }
  header {
    display: flex;
    align-items: flex-start;
    gap: var(--space-3);
    padding: var(--space-5) var(--space-5) var(--space-3);
  }
  .titles {
    flex: 1;
    min-width: 0;
  }
  h2 {
    font-size: var(--text-lg);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
  }
  .desc {
    margin-top: var(--space-1);
    color: var(--text-2);
    font-size: var(--text-sm);
  }
  .close {
    border: 0;
    background: transparent;
    color: var(--text-3);
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border-radius: var(--radius-sm);
    margin: -4px -6px 0 0;
  }
  .close:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .body {
    padding: 0 var(--space-5) var(--space-4);
    overflow: auto;
  }
  footer {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    padding: var(--space-3) var(--space-5) var(--space-5);
  }
</style>
