<script lang="ts" module>
  import type { IconComponent } from './icon';
  import type { JsonSchema, Tier } from '../api/types';

  export interface PaletteItem {
    id: string;
    group: string;
    title: string;
    subtitle?: string;
    icon?: IconComponent;
    shortcut?: string;
    tier?: Tier;
    /** Extra words to match on. */
    keywords?: string;
    /** Run immediately. */
    run?: () => void | Promise<void>;
    /** Or: a contract command; if it has required args the palette asks for JSON. */
    command?: { name: string; schema: JsonSchema };
  }
</script>

<script lang="ts">
  import Search from '@lucide/svelte/icons/search';
  import CornerDownLeft from '@lucide/svelte/icons/corner-down-left';
  import ArrowLeft from '@lucide/svelte/icons/arrow-left';
  import { fuzzyFilter } from './fuzzy';
  import Kbd from './Kbd.svelte';
  import Badge from './Badge.svelte';

  interface Props {
    open: boolean;
    items: PaletteItem[];
    onclose: () => void;
    /** Called for `command` items, with parsed args. */
    oncommand: (name: string, args: Record<string, unknown>) => void | Promise<void>;
  }

  let { open, items, onclose, oncommand }: Props = $props();

  let el: HTMLDialogElement | undefined = $state();
  let input: HTMLInputElement | undefined = $state();
  let argsEl: HTMLTextAreaElement | undefined = $state();
  let q = $state('');
  let active = $state(0);
  let argsFor = $state<PaletteItem | null>(null);
  let argsText = $state('');
  let argsError = $state<string | null>(null);
  const id = $props.id();

  const results = $derived(fuzzyFilter(items, q, (i) => `${i.title} ${i.subtitle ?? ''} ${i.group} ${i.keywords ?? ''}`).slice(0, 60));
  const grouped = $derived.by(() => {
    const groups: { name: string; items: { item: PaletteItem; index: number }[] }[] = [];
    results.forEach((item, index) => {
      let g = groups.find((x) => x.name === item.group);
      if (!g) groups.push((g = { name: item.group, items: [] }));
      g.items.push({ item, index });
    });
    return groups;
  });

  $effect(() => {
    if (!el) return;
    if (open && !el.open) {
      q = '';
      active = 0;
      argsFor = null;
      el.showModal();
      queueMicrotask(() => input?.focus());
    } else if (!open && el.open) el.close();
  });

  $effect(() => {
    // Keep the highlighted row valid as results change.
    if (active >= results.length) active = Math.max(0, results.length - 1);
  });

  function requiredArgs(schema: JsonSchema): string[] {
    return schema.required ?? [];
  }

  function skeleton(schema: JsonSchema): string {
    const out: Record<string, unknown> = {};
    for (const k of requiredArgs(schema)) {
      const t = schema.properties?.[k]?.type;
      out[k] = t === 'number' || t === 'integer' ? 0 : t === 'boolean' ? false : t === 'object' ? {} : t === 'array' ? [] : '';
    }
    return JSON.stringify(out, null, 2);
  }

  async function choose(item: PaletteItem | undefined) {
    if (!item) return;
    if (item.run) {
      onclose();
      await item.run();
      return;
    }
    if (item.command) {
      if (requiredArgs(item.command.schema).length === 0) {
        onclose();
        await oncommand(item.command.name, {});
        return;
      }
      argsFor = item;
      argsText = skeleton(item.command.schema);
      argsError = null;
      queueMicrotask(() => argsEl?.focus());
    }
  }

  async function submitArgs() {
    if (!argsFor?.command) return;
    let parsed: unknown;
    try {
      parsed = JSON.parse(argsText);
    } catch (e) {
      argsError = `Not valid JSON: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) {
      argsError = 'Arguments must be a JSON object.';
      return;
    }
    const missing = requiredArgs(argsFor.command.schema).filter((k) => !(k in parsed));
    if (missing.length) {
      argsError = `Missing: ${missing.join(', ')}`;
      return;
    }
    const name = argsFor.command.name;
    onclose();
    await oncommand(name, parsed as Record<string, unknown>);
  }

  function onKey(e: KeyboardEvent) {
    if (argsFor) {
      if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        void submitArgs();
      }
      return;
    }
    if (e.key === 'ArrowDown' || (e.ctrlKey && e.key === 'n')) {
      e.preventDefault();
      active = Math.min(results.length - 1, active + 1);
      scrollActive();
    } else if (e.key === 'ArrowUp' || (e.ctrlKey && e.key === 'p')) {
      e.preventDefault();
      active = Math.max(0, active - 1);
      scrollActive();
    } else if (e.key === 'Enter') {
      e.preventDefault();
      void choose(results[active]);
    }
  }

  function scrollActive() {
    queueMicrotask(() => el?.querySelector(`[data-pi="${active}"]`)?.scrollIntoView({ block: 'nearest' }));
  }

  function onCancel(e: Event) {
    e.preventDefault();
    if (argsFor) {
      argsFor = null;
      queueMicrotask(() => input?.focus());
    } else onclose();
  }
</script>

<dialog bind:this={el} class="palette" aria-label="Command palette" oncancel={onCancel} onclick={(e) => e.target === el && onclose()}>
  {#if open}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="inner" onkeydown={onKey}>
      {#if !argsFor}
        <div class="search">
          <Search size={17} aria-hidden="true" />
          <input
            bind:this={input}
            bind:value={q}
            oninput={() => (active = 0)}
            placeholder="Type a command or search…"
            role="combobox"
            aria-expanded="true"
            aria-controls="pal-list-{id}"
            aria-activedescendant={results.length ? `pal-${id}-${active}` : undefined}
            aria-autocomplete="list"
            spellcheck="false"
            autocomplete="off"
          />
          <Kbd keys="esc" />
        </div>
        <div class="list" id="pal-list-{id}" role="listbox" aria-label="Results">
          {#each grouped as g (g.name)}
            <div class="group" role="presentation">
              <div class="group-name label" role="presentation">{g.name}</div>
              {#each g.items as { item, index } (item.id)}
                <div
                  id="pal-{id}-{index}"
                  data-pi={index}
                  class="row"
                  class:active={index === active}
                  role="option"
                  tabindex="-1"
                  aria-selected={index === active}
                  onclick={() => choose(item)}
                  onkeydown={() => {}}
                  onpointermove={() => (active = index)}
                >
                  {#if item.icon}<span class="ic"><item.icon size={15} aria-hidden="true" /></span>{/if}
                  <span class="txt">
                    <span class="title truncate">{item.title}</span>
                    {#if item.subtitle}<span class="sub truncate">{item.subtitle}</span>{/if}
                  </span>
                  {#if item.tier && item.tier !== 'T0'}
                    <Badge tone={item.tier === 'T3' ? 'danger' : item.tier === 'T2' ? 'warning' : 'neutral'} variant="outline" title="Permission tier">{item.tier}</Badge>
                  {/if}
                  {#if item.shortcut}<Kbd keys={item.shortcut} />{/if}
                </div>
              {/each}
            </div>
          {:else}
            <p class="none">No matches for “{q}”.</p>
          {/each}
        </div>
        <footer class="foot">
          <span><Kbd keys="up" /><Kbd keys="down" /> navigate</span>
          <span><Kbd keys="enter" /> run</span>
        </footer>
      {:else}
        <div class="args">
          <div class="args-head">
            <button type="button" class="back" aria-label="Back to results" onclick={() => (argsFor = null)}><ArrowLeft size={16} aria-hidden="true" /></button>
            <div>
              <p class="mono args-name">{argsFor.command?.name}</p>
              {#if argsFor.subtitle}<p class="sub">{argsFor.subtitle}</p>{/if}
            </div>
          </div>
          <label class="label" for="pal-args-{id}">Arguments (JSON)</label>
          <textarea
            id="pal-args-{id}"
            bind:this={argsEl}
            bind:value={argsText}
            class="mono"
            rows="7"
            spellcheck="false"
            aria-invalid={argsError ? 'true' : undefined}
            aria-describedby={argsError ? `pal-err-${id}` : undefined}
          ></textarea>
          {#if argsError}<p class="err" id="pal-err-{id}" role="alert">{argsError}</p>{/if}
          <div class="args-foot">
            <span class="sub">T3 commands still ask for your approval.</span>
            <button type="button" class="run" onclick={submitArgs}>Run <Kbd keys="mod+enter" /></button>
          </div>
        </div>
      {/if}
    </div>
  {/if}
</dialog>

<style>
  .palette {
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-xl);
    background: var(--surface);
    color: var(--text);
    box-shadow: var(--elev-4);
    width: min(640px, calc(100vw - 32px));
    margin: 12vh auto auto;
    max-height: 70vh;
    overflow: hidden;
  }
  .palette::backdrop {
    background: var(--scrim);
    backdrop-filter: blur(3px);
  }
  .palette[open] {
    animation: pop var(--dur-base) var(--ease-out);
  }
  @keyframes pop {
    from {
      opacity: 0;
      transform: translateY(-6px) scale(0.985);
    }
  }
  .inner {
    display: flex;
    flex-direction: column;
    max-height: 70vh;
  }
  .search {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-3) var(--space-4);
    border-bottom: 1px solid var(--border);
    color: var(--text-3);
  }
  .search input {
    flex: 1;
    border: 0;
    background: transparent;
    font-size: var(--text-lg);
    padding: var(--space-2) 0;
    color: var(--text);
  }
  .search input::placeholder {
    color: var(--text-3);
  }
  .search input:focus-visible {
    box-shadow: none;
  }
  .list {
    overflow-y: auto;
    padding: var(--space-2);
    min-height: 0;
  }
  .group + .group {
    margin-top: var(--space-2);
  }
  .group-name {
    padding: var(--space-2) var(--space-3) var(--space-1);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    cursor: pointer;
    min-height: 40px;
  }
  .row.active {
    background: var(--accent-soft);
    box-shadow: inset 2px 0 0 var(--accent);
  }
  .ic {
    color: var(--text-3);
    display: grid;
  }
  .row.active .ic {
    color: var(--accent-text);
  }
  .txt {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .title {
    font-weight: var(--weight-medium);
  }
  .sub {
    color: var(--text-3);
    font-size: var(--text-sm);
  }
  .txt .sub {
    flex: 1;
  }
  .none {
    padding: var(--space-6);
    text-align: center;
    color: var(--text-3);
  }
  .foot {
    display: flex;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--border);
    color: var(--text-3);
    font-size: var(--text-xs);
    background: var(--surface-2);
  }
  .foot span {
    display: inline-flex;
    gap: 4px;
    align-items: center;
  }
  .args {
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .args-head {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    margin-bottom: var(--space-2);
  }
  .args-name {
    font-weight: var(--weight-semibold);
  }
  .back {
    display: grid;
    place-items: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  textarea {
    width: 100%;
    resize: vertical;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    padding: var(--space-3);
    line-height: 1.45;
  }
  textarea:focus-visible {
    border-color: var(--accent);
  }
  .err {
    color: var(--danger);
    font-size: var(--text-sm);
  }
  .args-foot {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-3);
    margin-top: var(--space-2);
  }
  .run {
    display: inline-flex;
    align-items: center;
    gap: var(--space-2);
    height: 34px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-md);
    border: 0;
    background: var(--accent);
    color: var(--text-on-accent);
    font-weight: var(--weight-semibold);
  }
</style>
