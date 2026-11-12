<script lang="ts">
  import FileText from '@lucide/svelte/icons/file-text';
  import Copy from '@lucide/svelte/icons/copy';
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { query } from '../../stores/query.svelte';
  import { toasts } from '../../design/toast.svelte';
  import { toISODate } from '../../util/dates';
  import { withoutPrivate } from '../../privacy/private';
  import Button from '../../design/Button.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { peopleById, typedItems } from './actions';
  import { EM_TYPES, weeklySections, weeklyText } from './logic';
  import { addDays, endOfWeek } from '../../util/dates';

  let { widget, ambient }: WidgetProps = $props();

  const today = $derived(hub.day?.date ?? toISODate(new Date(Math.floor(clock.now / 3_600_000) * 3_600_000)));
  const tasksQ = query(
    () => hub.dispatch('tasks.list', { kind: 'task', role: hub.roleId, limit: 500 }),
    () => [hub.itemsRev, hub.roleId],
  );
  const commitQ = typedItems(EM_TYPES.commitment);
  const signalQ = typedItems(EM_TYPES.signal);
  const peopleQ = typedItems(EM_TYPES.person);

  const sections = $derived(
    weeklySections({
      today,
      tasks: withoutPrivate(tasksQ.data?.items ?? []),
      commitments: commitQ.data?.items ?? [],
      signals: signalQ.data?.items ?? [],
      people: peopleById(peopleQ.data?.items),
    }),
  );
  const generated = $derived(weeklyText(sections, addDays(endOfWeek(today), -6)));
  const empty = $derived(sections.shipped.length + sections.inProgress.length + sections.risks.length + sections.asks.length === 0);

  // Your edits stay until you regenerate; new data does not overwrite them.
  let draft = $state('');
  let edited = $state(false);
  $effect(() => {
    if (!edited) draft = generated;
  });

  function regenerate() {
    edited = false;
    draft = generated;
  }
  async function copy() {
    try {
      await navigator.clipboard.writeText(draft);
      toasts.show('Update copied. Paste it wherever you share it.', { tone: 'success', timeout: 3000 });
    } catch {
      toasts.show('Could not copy. Select the text and copy it by hand.', { timeout: 6000 });
    }
  }
</script>

{#if tasksQ.loading && !tasksQ.data}
  <Skeleton lines={6} />
{:else if (tasksQ.error && !tasksQ.data) || (commitQ.error && !commitQ.data) || (signalQ.error && !signalQ.data) || (peopleQ.error && !peopleQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={tasksQ.error ?? commitQ.error ?? signalQ.error ?? peopleQ.error ?? undefined} guide={false} />
{:else if empty}
  <EmptyState compact icon={FileText} title="Nothing to report yet" hint="The draft builds itself from the tasks you finish, your commitments and team signals." />
{:else}
  <div class="wrap">
    <label class="visually-hidden" for="{widget.id}-draft">Weekly update draft</label>
    <textarea id="{widget.id}-draft" class="pii" bind:value={draft} oninput={() => (edited = true)} readonly={ambient} spellcheck="true"></textarea>
    {#if !ambient}
      <div class="acts">
        <span class="note">{edited ? 'Edited' : 'Draft'} · never sent from here</span>
        <Button size="sm" variant="ghost" icon={RotateCcw} disabled={!edited} onclick={regenerate}>Regenerate</Button>
        <Button size="sm" variant="primary" icon={Copy} onclick={copy}>Copy</Button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .wrap {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    height: 100%;
    min-height: 0;
  }
  textarea {
    flex: 1;
    min-height: 8em;
    resize: none;
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    line-height: var(--leading-normal);
    color: var(--text);
    background: var(--surface-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    padding: var(--space-2);
  }
  textarea:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
  }
  .acts {
    display: flex;
    align-items: center;
    gap: var(--space-1);
  }
  .note {
    margin-right: auto;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
</style>
