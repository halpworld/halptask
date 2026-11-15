<script lang="ts">
  import Tv from '@lucide/svelte/icons/tv';
  import EyeOff from '@lucide/svelte/icons/eye-off';
  import SunMoon from '@lucide/svelte/icons/sun-moon';
  import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
  import Keyboard from '@lucide/svelte/icons/keyboard';
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import Redo2 from '@lucide/svelte/icons/redo-2';
  import PenLine from '@lucide/svelte/icons/pen-line';
  import SquareTerminal from '@lucide/svelte/icons/square-terminal';
  import Plus from '@lucide/svelte/icons/plus';
  import Bot from '@lucide/svelte/icons/bot';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import Settings from '@lucide/svelte/icons/settings';
  import { hub } from '../stores/hub.svelte';
  import { roleMeta } from '../design/roles';
  import { registry } from '../widgets/registry';
  import CommandPalette, { type PaletteItem } from '../design/CommandPalette.svelte';
  import ToastHost from '../design/ToastHost.svelte';
  import Skeleton from '../design/Skeleton.svelte';
  import EmptyState from '../design/EmptyState.svelte';
  import { toasts } from '../design/toast.svelte';
  import Grid from '../layout/Grid.svelte';
  import TopBar from './TopBar.svelte';
  import DayHeader from './DayHeader.svelte';
  import AmbientHeader from './AmbientHeader.svelte';
  import AgentStrip from './AgentStrip.svelte';
  import ConsentDialog from './ConsentDialog.svelte';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import ShortcutsHelp from './ShortcutsHelp.svelte';
  import SettingsPanel from '../settings/SettingsPanel.svelte';
  import DevModeStrip from '../settings/DevModeStrip.svelte';
  import { createKeyHandler } from './keys';
  import { agentComplete, agentRearrange, agentRequestDelete, agentRequestDeleteThenChange, demoAvailable } from './demo';

  function focusCapture() {
    const el = document.querySelector<HTMLInputElement>('[data-capture]');
    if (el) el.focus();
    else toasts.show('Add a Capture widget to this layout to capture from here.');
  }

  function widgetCells(): HTMLElement[] {
    return [...document.querySelectorAll<HTMLElement>('[data-widget]')].sort((a, b) => {
      const ra = a.getBoundingClientRect();
      const rb = b.getBoundingClientRect();
      return Math.round(ra.top - rb.top) || ra.left - rb.left;
    });
  }

  function nextWidget(delta: 1 | -1) {
    const cells = widgetCells();
    if (!cells.length) return;
    const i = cells.findIndex((c) => c.contains(document.activeElement));
    const next = cells[i < 0 ? (delta > 0 ? 0 : cells.length - 1) : (i + delta + cells.length) % cells.length];
    focusCell(next);
  }

  function focusCell(cell: HTMLElement | undefined) {
    if (!cell) return;
    // Prefer the first row or control inside; fall back to the frame.
    const inner = hub.editing ? null : cell.querySelector<HTMLElement>('[data-row-focus], [data-capture], button, input');
    (inner ?? cell).focus();
    cell.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }

  function goto(target: 'now' | 'day' | 'agenda') {
    const kind = target === 'now' ? 'now' : target === 'agenda' ? 'agenda' : 'task-list';
    const cell = document.querySelector<HTMLElement>(`[data-kind="${kind}"]`);
    if (cell) focusCell(cell);
    else toasts.show(`No ${target === 'day' ? 'task list' : target} widget on this layout.`);
  }

  const onKey = createKeyHandler({
    palette: () => (hub.paletteOpen = !hub.paletteOpen),
    help: () => (hub.helpOpen = true),
    settings: () => (hub.settingsOpen = true),
    undo: () => hub.undo(),
    redo: () => hub.redo(),
    capture: focusCapture,
    ambient: () => hub.toggleAmbient(),
    privacy: () => hub.togglePrivacy(),
    theme: () => hub.cycleTheme(),
    role: (i) => {
      const r = hub.roles[i];
      if (r) void hub.switchRole(r.id);
    },
    nextWidget,
    goto,
    edit: () => hub.toggleEditing(),
    escape: () => {
      if (hub.ambient) {
        hub.toggleAmbient();
        return true;
      }
      if (hub.editing) {
        hub.toggleEditing();
        return true;
      }
      return false;
    },
    blocked: () => hub.paletteOpen || hub.helpOpen || hub.settingsOpen || hub.consent.length > 0,
  });

  const paletteItems = $derived.by<PaletteItem[]>(() => {
    const out: PaletteItem[] = [];
    hub.roles.forEach((r, i) => {
      const m = roleMeta(r.id);
      out.push({ id: `role:${r.id}`, group: 'Roles', title: `Switch to ${r.title}`, subtitle: m.tagline, icon: m.icon, shortcut: String(i + 1), run: () => hub.switchRole(r.id) });
    });
    out.push(
      { id: 'view:ambient', group: 'View', title: hub.ambient ? 'Leave ambient mode' : 'Ambient mode', subtitle: 'Big type for a wall screen', icon: Tv, shortcut: 'a', run: () => hub.toggleAmbient() },
      { id: 'view:privacy', group: 'View', title: hub.privacy ? 'Turn off privacy blur' : 'Privacy blur', subtitle: 'Blur names and titles for screen sharing', icon: EyeOff, shortcut: 'b', run: () => hub.togglePrivacy() },
      { id: 'view:theme', group: 'View', title: hub.resolvedTheme === 'dark' ? 'Light theme' : 'Dark theme', icon: SunMoon, shortcut: 't', run: () => hub.cycleTheme() },
      { id: 'view:edit', group: 'View', title: hub.editing ? 'Done editing layout' : 'Edit layout', subtitle: 'Drag, resize and remove widgets', icon: LayoutDashboard, shortcut: 'e', run: () => hub.toggleEditing() },
      { id: 'go:capture', group: 'Go to', title: 'Quick capture', icon: PenLine, shortcut: 'n', run: focusCapture },
      { id: 'go:now', group: 'Go to', title: 'Now', keywords: 'focus', shortcut: 'g n', run: () => goto('now') },
      { id: 'go:day', group: 'Go to', title: 'Today’s tasks', shortcut: 'g d', run: () => goto('day') },
      { id: 'go:agenda', group: 'Go to', title: 'Agenda', shortcut: 'g a', run: () => goto('agenda') },
      { id: 'hist:undo', group: 'History', title: 'Undo last change', icon: Undo2, shortcut: 'mod+z', run: () => hub.undo() },
      { id: 'hist:redo', group: 'History', title: 'Redo', icon: Redo2, shortcut: 'mod+shift+z', run: () => hub.redo() },
      { id: 'help', group: 'Help', title: 'Keyboard shortcuts', icon: Keyboard, shortcut: '?', run: () => void (hub.helpOpen = true) },
      { id: 'settings', group: 'Settings', title: 'Agents and connections', subtitle: 'Pair agents, grants, standing rules, audit, data, integrations', keywords: 'settings mcp pair token grants rules audit purge import integrations github', icon: Settings, shortcut: 'mod+,', run: () => void (hub.settingsOpen = true) },
    );
    for (const entry of registry.values()) {
      out.push({
        id: `add:${entry.kind}`,
        group: 'Add widget',
        title: `Add ${entry.title}`,
        subtitle: entry.placeholder ? `${entry.kind} · sample data` : entry.kind,
        icon: Plus,
        run: async () => {
          const l = await hub.run('layout.add_widget', { kind: entry.kind, title: entry.title, w: Math.max(entry.minW, 4), h: Math.max(entry.minH, 3) });
          if (l) hub.layout = l;
        },
      });
    }
    if (demoAvailable()) {
      const role = hub.roleId;
      out.push(
        { id: 'demo:move', group: 'Demo (mock core)', title: 'Agent rearranges the layout', subtitle: 'as agent:claude', icon: Bot, run: async () => void (await agentRearrange()) },
        { id: 'demo:complete', group: 'Demo (mock core)', title: 'Agent completes a task', subtitle: 'as agent:claude', icon: Bot, run: async () => void (await agentComplete(role)) },
        { id: 'demo:delete', group: 'Demo (mock core)', title: 'Agent asks to delete a task', subtitle: 'T3: needs your approval', icon: Bot, tier: 'T3', run: async () => void (await agentRequestDelete(role)) },
        { id: 'demo:reask', group: 'Demo (mock core)', title: 'Agent asks to delete, then changes the task', subtitle: 'Approve it to see "changed since you saw it"', icon: Bot, tier: 'T3', run: async () => void (await agentRequestDeleteThenChange(role)) },
      );
    }
    for (const c of hub.commands) {
      out.push({ id: `cmd:${c.name}`, group: 'Commands', title: c.name, subtitle: c.description, icon: SquareTerminal, tier: c.tier, command: { name: c.name, schema: c.input_schema } });
    }
    return out;
  });

  async function runCommand(name: string, args: Record<string, unknown>) {
    try {
      await hub.dispatch(name, args);
      toasts.show(`Ran ${name}`, { tone: 'success' });
    } catch (err) {
      toasts.error(err instanceof Error ? err.message : String(err));
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="shell" class:ambient={hub.ambient}>
  {#if !hub.ambient}
    <TopBar />
  {/if}

  <main class="main" aria-busy={hub.status === 'loading'}>
    {#if hub.status === 'loading'}
      <div class="loading">
        <Skeleton lines={1} height="32px" />
        <div class="sk-grid">
          {#each Array.from({ length: 6 }, (_, i) => i) as i (i)}<div class="sk-card"><Skeleton lines={4} /></div>{/each}
        </div>
      </div>
    {:else if hub.status === 'error'}
      <EmptyState icon={TriangleAlert} title="Couldn’t reach halp-core" hint={hub.error ?? 'Unknown error'} />
    {:else}
      {#if hub.ambient}<AmbientHeader />{:else}<DayHeader />{/if}
      {#if hub.editing}
        <p class="edit-hint" role="status">
          Editing layout. Drag a widget by its handle, drag the corner to resize, or focus a widget and use arrow keys (Shift to resize). Press E or Esc when done.
        </p>
      {/if}
      <div class="grid-wrap">
        {#if hub.layout}
          <Grid
            layout={hub.layout}
            editing={hub.editing}
            ambient={hub.ambient}
            touched={hub.touched}
            onmove={(id, x, y) => hub.moveWidget(id, x, y)}
            onresize={(id, w, h) => hub.resizeWidget(id, w, h)}
            onremove={(id) => hub.removeWidget(id)}
          />
        {/if}
      </div>
    {/if}
  </main>

  {#if !hub.ambient}
    <AgentStrip />
  {/if}
</div>

<CommandPalette open={hub.paletteOpen} items={paletteItems} onclose={() => (hub.paletteOpen = false)} oncommand={runCommand} />
<ConfirmDialog />
<ConsentDialog />
<ShortcutsHelp />
<SettingsPanel />
<DevModeStrip />
<ToastHost />

<style>
  .shell {
    display: grid;
    grid-template-rows: auto 1fr auto;
    height: 100vh;
    height: 100dvh;
    background:
      radial-gradient(1200px 500px at 8% -10%, color-mix(in srgb, var(--accent) 7%, transparent), transparent 60%),
      var(--bg);
  }
  .shell.ambient {
    grid-template-rows: 1fr;
  }
  .main {
    display: flex;
    flex-direction: column;
    min-height: 0;
    padding: 0 var(--space-5);
  }
  .ambient .main {
    padding: 0 var(--space-8) var(--space-4);
  }
  .grid-wrap {
    flex: 1;
    min-height: 0;
  }
  .edit-hint {
    margin: 0 0 var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--text-sm);
  }
  .loading {
    padding: var(--space-6) 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
  }
  .sk-grid {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 14px;
  }
  .sk-card {
    padding: var(--space-4);
    border-radius: var(--radius-lg);
    background: var(--surface);
    border: 1px solid var(--border);
    height: 180px;
  }
  @media (max-width: 760px) {
    .main {
      padding: 0 var(--space-3);
    }
    .sk-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
