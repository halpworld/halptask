<script lang="ts">
  import CornerDownLeft from '@lucide/svelte/icons/corner-down-left';
  import Hash from '@lucide/svelte/icons/hash';
  import CalendarDays from '@lucide/svelte/icons/calendar-days';
  import type { WidgetProps } from './types';
  import { hub } from '../stores/hub.svelte';
  import { previewCapture, type ParsedCapture } from '../capture/parse';
  import { relativeDue } from '../util/dates';
  import { toasts } from '../design/toast.svelte';
  import { HubApiError } from '../api/client';
  import { formatCharProblem } from '../util/format-chars';

  let { widget }: WidgetProps = $props();

  let text = $state('');
  let busy = $state(false);
  /** The core's invalid_args message, shown under the field until the text changes. */
  let serverError = $state<{ text: string; message: string } | null>(null);
  const inlineError = $derived(formatCharProblem(text) ?? (serverError && serverError.text === text ? serverError.message : null));
  const today = $derived(hub.day?.date ?? '');
  // The core's own parse (capture.parse) is the truth; the local port paints
  // instantly while that debounced call is in flight. Stale answers are dropped.
  const PARSE_DEBOUNCE_MS = 150;
  const local = $derived(text.trim() && today ? previewCapture(text, today) : null);
  let remote = $state<{ text: string; parsed: ParsedCapture | null } | null>(null);
  let parseSeq = 0;
  $effect(() => {
    const t = text;
    if (!t.trim()) {
      remote = null;
      return;
    }
    const seq = ++parseSeq;
    const timer = setTimeout(async () => {
      let parsed: ParsedCapture | null = null;
      try {
        parsed = await hub.dispatch('capture.parse', { text: t });
      } catch {
        parsed = null; // e.g. "nothing to capture"
      }
      if (seq === parseSeq) remote = { text: t, parsed };
    }, PARSE_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });
  const preview = $derived(remote && remote.text === text ? remote.parsed : local);
  const inputId = $props.id();

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!preview?.title || busy || inlineError) return;
    busy = true;
    const sent = text;
    let res;
    try {
      res = await hub.dispatch('capture.quick', { text: sent });
    } catch (err) {
      if (err instanceof HubApiError && err.code === 'invalid_args') serverError = { text: sent, message: err.message };
      else if (err instanceof HubApiError && err.code === 'denied') toasts.show('Cancelled: capture.quick');
      else toasts.error(err instanceof Error ? err.message : String(err));
      return;
    } finally {
      busy = false;
    }
    serverError = null;
    text = '';
    const created = res.item;
    const due = created.due && today ? ` · due ${relativeDue(created.due, today)}` : '';
    toasts.show(`Captured “${created.title}”${due}`, {
      tone: 'success',
      action: {
        label: 'Undo',
        run: () => {
          const ev = hub.events.find((x) => x.command === 'capture.quick' && x.entity.id === created.id);
          return hub.undo(ev?.id);
        },
      },
    });
  }
</script>

<form class="capture" onsubmit={submit}>
  <label class="visually-hidden" for="cap-{inputId}">Capture a task in {widget.title}</label>
  <div class="field">
    <input
      id="cap-{inputId}"
      data-capture
      bind:value={text}
      placeholder="Capture… e.g. Fix login #bug due:fri"
      autocomplete="off"
      spellcheck="false"
      maxlength="500"
      aria-invalid={inlineError ? 'true' : undefined}
      aria-describedby={inlineError ? `cap-err-${inputId}` : undefined}
    />
    <button class="go" type="submit" disabled={!preview?.title || busy} aria-label="Add task"><CornerDownLeft size={14} aria-hidden="true" /></button>
  </div>
  {#if inlineError}<p class="err" id="cap-err-{inputId}" role="alert">{inlineError}</p>{/if}
  <div class="chips" aria-live="polite">
    {#if preview && (preview.tags.length || preview.due)}
      {#if preview.due}<span class="chip due"><CalendarDays size={11} aria-hidden="true" />{relativeDue(preview.due, today)}</span>{/if}
      {#each preview.tags as t (t)}<span class="chip"><Hash size={11} aria-hidden="true" />{t}</span>{/each}
    {:else}
      <span class="hint"><code>#tag</code> <code>due:fri</code> <code>due:+3d</code></span>
    {/if}
  </div>
</form>

<style>
  .err {
    margin: 0;
    color: var(--danger);
    font-size: var(--text-xs);
  }
  .capture {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }
  .field {
    position: relative;
    display: flex;
    align-items: center;
  }
  input {
    width: 100%;
    height: calc(38px * var(--type-scale));
    padding: 0 40px 0 var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--text-md);
    transition:
      border-color var(--dur-fast) var(--ease-out),
      box-shadow var(--dur-fast) var(--ease-out),
      background-color var(--dur-fast) var(--ease-out);
  }
  input::placeholder {
    color: var(--text-3);
  }
  input:focus {
    border-color: var(--accent);
    background: var(--surface);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  .go {
    position: absolute;
    right: 5px;
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border: 0;
    border-radius: var(--radius-sm);
    background: var(--accent);
    color: var(--text-on-accent);
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .go:disabled {
    opacity: 0;
    pointer-events: none;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1);
    min-height: 20px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    height: 20px;
    padding: 0 7px;
    border-radius: var(--radius-full);
    background: var(--surface-3);
    color: var(--text-2);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
  }
  .chip.due {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .hint {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  code {
    font-family: var(--font-mono);
    font-size: 0.95em;
    padding: 1px 4px;
    border-radius: var(--radius-xs);
    background: var(--surface-3);
  }
</style>
