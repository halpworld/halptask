<script lang="ts">
  // The quick-capture window (global shortcut / tray). Talks to the same
  // core through the same `dispatch` door; the main window sees the new
  // item through the `halp://event` it causes.
  import { formatCharProblem } from '../util/format-chars';
  import { onMount } from 'svelte';
  import Hash from '@lucide/svelte/icons/hash';
  import CalendarDays from '@lucide/svelte/icons/calendar-days';
  import { createClient, type HubClient } from '../api';
  import { hideCapture, onCaptureShown } from '../api/shell';
  import { previewCapture, type ParsedCapture } from '../capture/parse';
  import { relativeDue } from '../util/dates';

  let client: HubClient | null = null;
  let input = $state<HTMLInputElement | null>(null);
  let text = $state('');
  let busy = $state(false);
  let note = $state<{ tone: 'ok' | 'err'; text: string } | null>(null);
  let today = $state(localToday());
  let remote = $state<{ text: string; parsed: ParsedCapture | null } | null>(null);
  let seq = 0;

  function localToday(): string {
    const d = new Date();
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  }

  const local = $derived(text.trim() ? previewCapture(text, today) : null);
  const preview = $derived(remote && remote.text === text ? remote.parsed : local);

  $effect(() => {
    const t = text;
    if (!t.trim() || !client) {
      remote = null;
      return;
    }
    const mine = ++seq;
    const timer = setTimeout(async () => {
      let parsed: ParsedCapture | null = null;
      try {
        parsed = (await client!.dispatch('capture.parse', { text: t })) as ParsedCapture;
      } catch {
        parsed = null;
      }
      if (mine === seq) remote = { text: t, parsed };
    }, 150);
    return () => clearTimeout(timer);
  });

  function reset() {
    text = '';
    note = null;
    today = localToday();
    input?.focus();
  }

  onMount(() => {
    const dark = window.matchMedia?.('(prefers-color-scheme: dark)').matches;
    document.documentElement.dataset.theme = dark ? 'dark' : 'light';
    let off: (() => void) | null = null;
    let gone = false;
    void createClient().then((c) => (client = c));
    void onCaptureShown(reset).then((fn) => (gone ? fn() : (off = fn)));
    input?.focus();
    return () => {
      gone = true;
      off?.();
    };
  });

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!client || !preview?.title || busy) return;
    const bad = formatCharProblem(text);
    if (bad) {
      note = { tone: 'err', text: bad };
      return;
    }
    busy = true;
    try {
      const res = (await client.dispatch('capture.quick', { text })) as { item: { title: string } };
      text = '';
      note = { tone: 'ok', text: `Captured “${res.item.title}”` };
      setTimeout(() => void hideCapture(), 700);
    } catch (err) {
      note = { tone: 'err', text: err instanceof Error ? err.message : String(err) };
    } finally {
      busy = false;
    }
  }

  function keydown(e: KeyboardEvent) {
    if (e.key === 'Escape') {
      e.preventDefault();
      void hideCapture();
    }
  }
</script>

<svelte:window onkeydown={keydown} />

<form class="capture" onsubmit={submit}>
  <label class="visually-hidden" for="quick-capture">Quick capture</label>
  <input
    id="quick-capture"
    bind:this={input}
    bind:value={text}
    placeholder="Capture… e.g. Fix login #bug due:fri"
    autocomplete="off"
    spellcheck="false"
    maxlength="500"
    disabled={busy}
  />
  <div class="row" aria-live="polite">
    {#if note}
      <span class="note {note.tone}">{note.text}</span>
    {:else if preview && (preview.tags.length || preview.due)}
      {#if preview.due}<span class="chip due"><CalendarDays size={11} aria-hidden="true" />{relativeDue(preview.due, today)}</span>{/if}
      {#each preview.tags as t (t)}<span class="chip"><Hash size={11} aria-hidden="true" />{t}</span>{/each}
    {:else}
      <span class="hint">Enter to save to your Inbox · Esc to close</span>
    {/if}
  </div>
</form>

<style>
  .capture {
    display: flex;
    flex-direction: column;
    justify-content: center;
    gap: var(--space-3);
    height: 100vh;
    padding: var(--space-5);
    background: var(--surface);
    color: var(--text);
  }
  input {
    width: 100%;
    height: 52px;
    padding: 0 var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--text-lg);
  }
  input:focus {
    border-color: var(--accent);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  .row {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    min-height: 22px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 22px;
    padding: 0 8px;
    border-radius: var(--radius-full);
    background: var(--surface-3);
    color: var(--text-2);
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
  }
  .chip.due {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .hint {
    font-size: var(--text-xs);
    color: var(--text-3);
  }
  .note.ok {
    color: var(--accent-text);
    font-size: var(--text-sm);
  }
  .note.err {
    color: var(--danger);
    font-size: var(--text-sm);
  }
</style>
