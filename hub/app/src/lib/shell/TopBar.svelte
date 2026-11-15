<script lang="ts">
  import Command from '@lucide/svelte/icons/command';
  import EyeOff from '@lucide/svelte/icons/eye-off';
  import Eye from '@lucide/svelte/icons/eye';
  import Tv from '@lucide/svelte/icons/tv';
  import Sun from '@lucide/svelte/icons/sun';
  import Moon from '@lucide/svelte/icons/moon';
  import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
  import Check from '@lucide/svelte/icons/check';
  import Keyboard from '@lucide/svelte/icons/keyboard';
  import Settings from '@lucide/svelte/icons/settings';
  import { hub } from '../stores/hub.svelte';
  import { roleMeta } from '../design/roles';
  import IconButton from '../design/IconButton.svelte';
  import Kbd from '../design/Kbd.svelte';
  import RoleSwitcher from './RoleSwitcher.svelte';

  const meta = $derived(roleMeta(hub.roleId));
</script>

<header class="topbar">
  <div class="brand">
    <span class="mark" aria-hidden="true">
      <svg viewBox="0 0 24 24" width="22" height="22"><rect class="m-bg" x="2" y="2" width="20" height="20" rx="6" /><path class="m-fg" d="M8 7v10M16 7v10M8 12h8" stroke-width="2.4" stroke-linecap="round" fill="none" /></svg>
    </span>
    <div class="brand-txt">
      <span class="product">HalpTask Hub</span>
      <span class="tagline">{meta.tagline}</span>
    </div>
  </div>

  <RoleSwitcher roles={hub.roles} current={hub.roleId} onswitch={(id) => hub.switchRole(id)} />

  <span class="spacer"></span>

  <button type="button" class="palette-btn" onclick={() => (hub.paletteOpen = true)} aria-label="Open command palette">
    <Command size={14} aria-hidden="true" />
    <span class="ph">Search commands…</span>
    <Kbd keys="mod+k" />
  </button>

  <div class="tools">
    <IconButton
      icon={hub.editing ? Check : LayoutDashboard}
      label={hub.editing ? 'Done editing layout' : 'Edit layout'}
      shortcut="E"
      pressed={hub.editing}
      onclick={() => hub.toggleEditing()}
    />
    <IconButton icon={hub.privacy ? EyeOff : Eye} label="Privacy blur" shortcut="B" pressed={hub.privacy} onclick={() => hub.togglePrivacy()} />
    <IconButton icon={Tv} label="Ambient mode" shortcut="A" pressed={hub.ambient} onclick={() => hub.toggleAmbient()} />
    <IconButton
      icon={hub.resolvedTheme === 'dark' ? Sun : Moon}
      label={hub.resolvedTheme === 'dark' ? 'Light theme' : 'Dark theme'}
      shortcut="T"
      onclick={() => hub.cycleTheme()}
    />
    <IconButton icon={Keyboard} label="Keyboard shortcuts" shortcut="?" onclick={() => (hub.helpOpen = true)} />
    <IconButton icon={Settings} label="Settings: agents and connections" shortcut="mod+," pressed={hub.settingsOpen} onclick={() => (hub.settingsOpen = true)} />
  </div>

  <span class="conn {hub.transport}" title={hub.transport === 'mock' ? 'Running in a browser against the in-memory mock core' : 'Connected to halp-core'}>
    <span class="dot" aria-hidden="true"></span>{hub.transport === 'mock' ? 'Mock core' : 'Core'}
  </span>
</header>

<style>
  .topbar {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    height: 56px;
    padding: 0 var(--space-5);
    border-bottom: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface) 82%, transparent);
    backdrop-filter: saturate(1.4) blur(12px);
    position: relative;
    z-index: var(--z-chrome);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-width: 0;
  }
  .mark {
    display: grid;
  }
  .m-bg {
    fill: var(--accent);
  }
  .m-fg {
    stroke: var(--text-on-accent);
  }
  .brand-txt {
    display: flex;
    flex-direction: column;
    line-height: 1.15;
  }
  .product {
    font-weight: var(--weight-bold);
    letter-spacing: var(--tracking-tight);
  }
  .tagline {
    font-size: var(--text-xs);
    color: var(--text-3);
    font-family: var(--font-label, var(--font-sans));
  }
  .spacer {
    flex: 1;
  }
  .palette-btn {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    height: 34px;
    min-width: 240px;
    padding: 0 6px 0 var(--space-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    background: var(--surface-2);
    color: var(--text-3);
    font-size: var(--text-sm);
  }
  .palette-btn:hover {
    border-color: var(--border-strong);
    color: var(--text-2);
  }
  .ph {
    flex: 1;
    text-align: left;
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .conn {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: var(--text-xs);
    color: var(--text-3);
    white-space: nowrap;
  }
  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--success);
  }
  .conn.mock .dot {
    background: var(--warning);
  }
  @media (max-width: 1100px) {
    .palette-btn {
      min-width: 0;
    }
    .ph,
    .tagline {
      display: none;
    }
  }
  @media (max-width: 760px) {
    .conn,
    .brand-txt {
      display: none;
    }
    .topbar {
      gap: var(--space-2);
      padding: 0 var(--space-3);
    }
  }
</style>
