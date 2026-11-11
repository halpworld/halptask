<script lang="ts">
  // Settings: a full-height drawer on a native <dialog> (showModal gives the
  // focus trap, inert background and Esc). Reachable from the top bar, the
  // command palette and mod+,.
  import X from '@lucide/svelte/icons/x';
  import Bot from '@lucide/svelte/icons/bot';
  import Shield from '@lucide/svelte/icons/shield';
  import ScrollText from '@lucide/svelte/icons/scroll-text';
  import Database from '@lucide/svelte/icons/database';
  import Plug from '@lucide/svelte/icons/plug';
  import Terminal from '@lucide/svelte/icons/terminal';
  import Package from '@lucide/svelte/icons/package';
  import KeyRound from '@lucide/svelte/icons/key-round';
  import { hub } from '../stores/hub.svelte';
  import { settings, type SettingsSection } from '../stores/settings.svelte';
  import AgentsSection from './AgentsSection.svelte';
  import RulesSection from './RulesSection.svelte';
  import AuditSection from './AuditSection.svelte';
  import DataSection from './DataSection.svelte';
  import ConnectionsSection from './ConnectionsSection.svelte';
  import ShellSection from './ShellSection.svelte';
  import PacksSection from './PacksSection.svelte';
  import KeysSection from './KeysSection.svelte';
  import './settings.css';

  const SECTIONS: { id: SettingsSection; label: string; icon: typeof Bot }[] = [
    { id: 'agents', label: 'Agents', icon: Bot },
    { id: 'rules', label: 'Standing rules', icon: Shield },
    { id: 'audit', label: 'Audit', icon: ScrollText },
    { id: 'data', label: 'Data', icon: Database },
    { id: 'connections', label: 'Connections', icon: Plug },
    { id: 'packs', label: 'Packs', icon: Package },
    { id: 'keys', label: 'Publisher keys', icon: KeyRound },
    { id: 'shell', label: 'Shell and MCP', icon: Terminal },
  ];

  let el: HTMLDialogElement | undefined = $state();
  const open = $derived(hub.settingsOpen);

  $effect(() => {
    if (!el) return;
    if (open && !el.open) el.showModal();
    else if (!open && el.open) el.close();
  });

  // Load the section on open and on switch, and again when an admin event
  // (clients, grants, rules, integrations, clear_taint) arrives.
  $effect(() => {
    if (!open) return;
    void hub.adminRev;
    void settings.reload(settings.section);
  });

  function close() {
    settings.reset();
    hub.settingsOpen = false;
  }

  function onCancel(e: Event) {
    e.preventDefault();
    // A nested dialog (pairing, consent) handles its own Esc.
    if (settings.pairing) return;
    close();
  }

  function onNavKey(e: KeyboardEvent, i: number) {
    const n = SECTIONS.length;
    let j = -1;
    if (e.key === 'ArrowDown') j = (i + 1) % n;
    else if (e.key === 'ArrowUp') j = (i - 1 + n) % n;
    else if (e.key === 'Home') j = 0;
    else if (e.key === 'End') j = n - 1;
    const next = SECTIONS[j];
    if (!next) return;
    e.preventDefault();
    settings.section = next.id;
    document.getElementById(`settings-tab-${next.id}`)?.focus();
  }
</script>

<dialog bind:this={el} class="drawer settings-panel" aria-labelledby="settings-title" oncancel={onCancel}>
  {#if open}
    <div class="frame">
      <header>
        <h2 id="settings-title">Settings</h2>
        <button class="close" aria-label="Close settings" onclick={close}><X size={16} aria-hidden="true" /></button>
      </header>
      <div class="cols">
        <div class="nav" role="tablist" aria-orientation="vertical" aria-label="Settings sections">
          {#each SECTIONS as s, i (s.id)}
            {@const Icon = s.icon}
            <button
              id="settings-tab-{s.id}"
              role="tab"
              aria-selected={settings.section === s.id}
              aria-controls="settings-pane"
              tabindex={settings.section === s.id ? 0 : -1}
              class:active={settings.section === s.id}
              onclick={() => (settings.section = s.id)}
              onkeydown={(e) => onNavKey(e, i)}
            >
              <Icon size={15} aria-hidden="true" />
              <span>{s.label}</span>
            </button>
          {/each}
        </div>
        <div id="settings-pane" class="pane" role="tabpanel" aria-labelledby="settings-tab-{settings.section}" tabindex="-1">
          {#if settings.section === 'agents'}<AgentsSection />
          {:else if settings.section === 'rules'}<RulesSection />
          {:else if settings.section === 'audit'}<AuditSection />
          {:else if settings.section === 'data'}<DataSection />
          {:else if settings.section === 'connections'}<ConnectionsSection />
          {:else if settings.section === 'packs'}<PacksSection />
          {:else if settings.section === 'keys'}<KeysSection />
          {:else}<ShellSection />{/if}
        </div>
      </div>
    </div>
  {/if}
</dialog>

<style>
  .drawer {
    margin: 0 0 0 auto;
    padding: 0;
    height: 100vh;
    max-height: 100vh;
    width: min(100vw, 980px);
    max-width: 100vw;
    border: 0;
    border-left: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--elev-4);
    z-index: var(--z-dialog);
  }
  .drawer[open] {
    animation: slide-in var(--dur-base) var(--ease-out);
  }
  .drawer::backdrop {
    background: var(--scrim);
  }
  @keyframes slide-in {
    from {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  .frame {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-4) var(--space-5);
    border-bottom: 1px solid var(--border);
  }
  h2 {
    font-size: var(--text-xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
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
  }
  .close:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .close:focus-visible,
  .nav button:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .cols {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 200px 1fr;
  }
  .nav {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    padding: var(--space-3);
    border-right: 1px solid var(--border);
    background: var(--bg-tint);
  }
  .nav button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    border: 0;
    background: transparent;
    color: var(--text-2);
    text-align: left;
    font: inherit;
    font-size: var(--text-sm);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
  }
  .nav button:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .nav button.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    font-weight: var(--weight-medium);
  }
  .pane {
    overflow-y: auto;
    padding: var(--space-5);
  }
  .pane:focus {
    outline: none;
  }
  @media (max-width: 640px) {
    .cols {
      grid-template-columns: 1fr;
      grid-template-rows: auto 1fr;
    }
    .nav {
      flex-direction: row;
      overflow-x: auto;
      border-right: 0;
      border-bottom: 1px solid var(--border);
    }
  }
</style>
