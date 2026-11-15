<script lang="ts">
  import type { Role } from '../api/types';
  import { roleMeta } from '../design/roles';

  interface Props {
    roles: Role[];
    current: string;
    onswitch: (id: string) => void;
  }
  let { roles, current, onswitch }: Props = $props();

  function onKey(e: KeyboardEvent) {
    const i = roles.findIndex((r) => r.id === current);
    let next = -1;
    if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (i + 1) % roles.length;
    else if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (i - 1 + roles.length) % roles.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = roles.length - 1;
    const r = roles[next];
    if (!r) return;
    e.preventDefault();
    onswitch(r.id);
    queueMicrotask(() => (e.currentTarget as HTMLElement | null)?.querySelector<HTMLElement>(`[data-role-id="${r.id}"]`)?.focus());
  }
</script>

<div class="switcher" role="radiogroup" aria-label="Role" tabindex="-1" onkeydown={onKey}>
  {#each roles as r, i (r.id)}
    {@const meta = roleMeta(r.id)}
    {@const on = r.id === current}
    <button
      type="button"
      role="radio"
      aria-checked={on}
      tabindex={on ? 0 : -1}
      class:on
      data-role-id={r.id}
      title="{r.title} ({i + 1})"
      onclick={() => onswitch(r.id)}
    >
      <meta.icon size={15} aria-hidden="true" />
      <span class="name">{r.title}</span>
      <span class="short">{meta.short}</span>
    </button>
  {/each}
</div>

<style>
  .switcher {
    display: inline-flex;
    padding: 3px;
    gap: 2px;
    border-radius: var(--radius-md);
    background: var(--surface-3);
    border: 1px solid var(--border);
  }
  button {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 var(--space-3);
    border: 0;
    border-radius: 7px;
    background: transparent;
    color: var(--text-2);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out);
  }
  button:hover {
    color: var(--text);
  }
  button.on {
    background: var(--surface);
    color: var(--accent-text);
    box-shadow: var(--elev-1);
  }
  .short {
    display: none;
  }
  @media (max-width: 980px) {
    .name {
      display: none;
    }
    .short {
      display: inline;
    }
  }
</style>
