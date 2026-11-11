<script lang="ts">
  import Check from '@lucide/svelte/icons/check';
  import Crosshair from '@lucide/svelte/icons/crosshair';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import GitPullRequest from '@lucide/svelte/icons/git-pull-request';
  import type { Item } from '../../api/types';
  import { hub } from '../../stores/hub.svelte';
  import { diffDays, relativeDue } from '../../util/dates';
  import Badge from '../../design/Badge.svelte';

  interface Props {
    item: Item;
    /** Number of subtasks done / total, when this row has children in view. */
    sub?: { done: number; total: number };
    indent?: boolean;
    showDue?: boolean;
    dense?: boolean;
    interactive?: boolean;
  }

  let { item, sub, indent = false, showDue = true, dense = false, interactive = true }: Props = $props();

  const today = $derived(hub.day?.date ?? '');
  const done = $derived(item.status === 'done');
  const overdue = $derived(!done && !!item.due && !!today && item.due < today);
  const visibleTags = $derived(item.tags.slice(0, 2));
  const isFocus = $derived(hub.day?.focus?.id === item.id);
  const typeBadge = $derived.by(() => {
    if (item.type_id === 'dev.pull_request' && typeof item.fields.number === 'number') return `#${item.fields.number}`;
    return null;
  });
  const dueTone = $derived(overdue ? 'danger' : item.due === today ? 'accent' : 'neutral');

  function toggle() {
    void hub.complete(item, !done);
  }
  function focus() {
    void hub.run('focus.set', { id: item.id });
  }
  function remove() {
    void hub.run('tasks.delete', { id: item.id });
  }
  function onKey(e: KeyboardEvent) {
    if (!interactive || e.metaKey || e.ctrlKey || e.altKey) return;
    if (e.key === 'x') {
      e.preventDefault();
      toggle();
    } else if (e.key === 'f') {
      e.preventDefault();
      focus();
    } else if (e.key === 'Delete' || e.key === 'Backspace') {
      e.preventDefault();
      remove();
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<li class="row" class:done class:indent class:dense class:focused={isFocus} data-row onkeydown={onKey}>
  {#if interactive}
    <button
      class="check"
      class:on={done}
      data-row-focus
      aria-label="{done ? 'Reopen' : 'Complete'}: {item.title}"
      aria-pressed={done}
      onclick={toggle}
    >
      <Check size={12} strokeWidth={3} aria-hidden="true" />
    </button>
  {:else}
    <span class="dot" class:on={done} aria-hidden="true"></span>
  {/if}
  <span class="title-wrap">
    <span class="title pii">{item.title}</span>
    {#if item.status === 'in_progress'}<span class="live" title="In progress"><span class="visually-hidden">In progress</span></span>{/if}
  </span>
  <span class="meta">
    {#if typeBadge}<Badge tone="info" icon={item.type_id === 'dev.pull_request' ? GitPullRequest : undefined}>{typeBadge}</Badge>{/if}
    {#if sub && sub.total > 0}<span class="sub num" title="Subtasks done">{sub.done}/{sub.total}</span>{/if}
    {#each visibleTags as t (t)}<span class="tag">#{t}</span>{/each}
    {#if showDue && item.due && today}
      <Badge tone={dueTone} title={item.due}>{overdue ? `${-diffDays(item.due, today)}d late` : relativeDue(item.due, today)}</Badge>
    {/if}
  </span>
  {#if interactive}
    <span class="actions">
      <button class="act" aria-label="Focus on: {item.title}" title="Focus (f)" onclick={focus}><Crosshair size={14} aria-hidden="true" /></button>
      <button class="act danger" aria-label="Delete: {item.title}" title="Delete (needs approval)" onclick={remove}><Trash2 size={14} aria-hidden="true" /></button>
    </span>
  {/if}
</li>

<style>
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: calc(36px * var(--type-scale));
    padding: 4px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border-radius: var(--radius-md);
    list-style: none;
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .row.dense {
    min-height: calc(30px * var(--type-scale));
  }
  .row:hover,
  .row:focus-within {
    background: var(--surface-2);
  }
  .row.indent {
    padding-left: calc(var(--space-2) + 26px);
  }
  .row.focused::before {
    content: '';
    position: absolute;
    left: -2px;
    top: 8px;
    bottom: 8px;
    width: 3px;
    border-radius: 3px;
    background: var(--accent);
  }
  .check {
    flex: none;
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border-radius: 6px;
    border: 1.5px solid var(--border-strong);
    background: var(--surface);
    color: transparent;
    transition:
      background-color var(--dur-fast) var(--ease-out),
      border-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      transform var(--dur-fast) var(--ease-spring);
  }
  .check:hover {
    border-color: var(--accent);
    color: color-mix(in srgb, var(--accent) 55%, transparent);
  }
  .check.on {
    background: var(--accent);
    border-color: var(--accent);
    color: var(--text-on-accent);
    transform: scale(1.04);
  }
  .check:focus-visible {
    border-radius: 6px;
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    border: 1.5px solid var(--border-strong);
  }
  .dot.on {
    background: var(--accent);
    border-color: var(--accent);
  }
  .title-wrap {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
    transition: color var(--dur-base) var(--ease-out);
  }
  .done .title {
    color: var(--text-3);
    text-decoration: line-through;
    text-decoration-color: color-mix(in srgb, var(--text-3) 60%, transparent);
  }
  .live {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  .meta {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    flex: none;
  }
  .tag {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .sub {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .actions {
    display: flex;
    gap: 2px;
    flex: none;
    width: 0;
    overflow: hidden;
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .row:hover .actions,
  .row:focus-within .actions {
    width: auto;
    opacity: 1;
  }
  .act {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    border: 0;
    border-radius: var(--radius-sm);
    background: transparent;
    color: var(--text-3);
  }
  .act:hover {
    background: var(--surface-3);
    color: var(--text);
  }
  .act.danger:hover {
    color: var(--danger);
    background: var(--danger-soft);
  }
</style>
