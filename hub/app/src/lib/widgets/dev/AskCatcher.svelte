<script lang="ts">
  import MessageCircleQuestion from '@lucide/svelte/icons/message-circle-question';
  import ListPlus from '@lucide/svelte/icons/list-plus';
  import Check from '@lucide/svelte/icons/check';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { query } from '../../stores/query.svelte';
  import { clock } from '../../stores/clock.svelte';
  import { toasts } from '../../design/toast.svelte';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Kbd from '../../design/Kbd.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { ignoreKey, openLink } from './actions';
  import { ageLabel, askGroups, DEV_TYPES, replyBy, SOURCE_LABEL, type Ask } from './logic';

  let { ambient }: WidgetProps = $props();

  const now = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const q = query(
    () => hub.dispatch('tasks.list', { type_id: DEV_TYPES.ask, status: ['todo', 'in_progress'], role: hub.roleId, limit: 500 }),
    () => [hub.itemsRev, hub.roleId],
  );
  const groups = $derived(askGroups(q.data?.items ?? []));
  const total = $derived(groups.reduce((n, g) => n + g.asks.length, 0));
  const TONE = { late: 'danger', soon: 'warning', later: 'neutral' } as const;

  let busy = $state<string | null>(null);

  /** Turn the ask into a plain task due today, then close the ask. */
  async function toTask(a: Ask) {
    if (busy) return;
    busy = a.item.id;
    const where = a.channel ? `${SOURCE_LABEL[a.source]} ${a.channel}` : SOURCE_LABEL[a.source];
    const created = await hub.run('tasks.create', {
      title: `Reply to ${a.from}: ${a.item.title}`.slice(0, 500),
      notes: `Asked by ${a.from} via ${where}.`,
      tags: ['ask'],
      due: hub.day?.date ?? undefined,
      roles: [hub.roleId],
    });
    if (created) {
      await hub.run('tasks.complete', { id: a.item.id, done: true });
      toasts.show('Ask turned into a task due today.', { tone: 'success' });
    }
    busy = null;
  }
  async function markDone(a: Ask) {
    if (busy) return;
    busy = a.item.id;
    await hub.complete(a.item);
    busy = null;
  }
  function onRowKey(e: KeyboardEvent, a: Ask) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 't') {
      e.preventDefault();
      void toTask(a);
    } else if (e.key === 'x') {
      e.preventDefault();
      void markDone(a);
    } else if (e.key === 'o') {
      e.preventDefault();
      void openLink(a.permalink, `the ask from ${a.from}`);
    }
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if total === 0}
  <EmptyState compact icon={MessageCircleQuestion} title="No one is waiting on you" hint="Questions people are waiting on you to answer land here." />
{:else}
  {#if !ambient}
    <p class="keys" aria-hidden="true"><Kbd keys="t" /> make task <Kbd keys="x" /> done <Kbd keys="o" /> copy link</p>
  {/if}
  <div class="asks" onkeydown={listKeydown} role="presentation">
    {#each groups as g (g.source)}
      <section aria-label="{g.label}, {g.asks.length}">
        <h3 class="label">{g.label}<span class="count num">{g.asks.length}</span></h3>
        <ul>
          {#each g.asks as a (a.item.id)}
            {@const rb = replyBy(a.replyBy, now)}
            <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
            <li class="row" class:busy={busy === a.item.id} data-row onkeydown={(e) => onRowKey(e, a)}>
              <button
                class="main"
                data-row-focus
                disabled={ambient}
                aria-label="{a.from}{a.channel ? ` in ${a.channel}` : ''}: {a.item.title}. {ageLabel(now - a.receivedAt)} ago.{rb ? ` ${rb.text}.` : ''} Press t to make a task, x when done."
                onclick={() => openLink(a.permalink, `the ask from ${a.from}`)}
              >
                <span class="who truncate"><span class="from pii">{a.from}</span>{#if a.channel}<span class="ch pii"> · {a.channel}</span>{/if}<span class="ago num"> · {ageLabel(now - a.receivedAt)}</span></span>
                <span class="t pii">{a.item.title}</span>
              </button>
              {#if rb}<Badge tone={TONE[rb.tone]} title={rb.text}>{rb.tone === 'late' ? 'late' : rb.text.replace('reply in ', '')}</Badge>{/if}
              {#if !ambient}
                <span class="acts">
                  <button class="act" tabindex="-1" title="Make a task (t)" aria-label="Make a task from: {a.item.title}" onclick={() => toTask(a)}><ListPlus size={14} aria-hidden="true" /></button>
                  <button class="act" tabindex="-1" title="Done (x)" aria-label="Mark done: {a.item.title}" onclick={() => markDone(a)}><Check size={14} aria-hidden="true" /></button>
                </span>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}

<style>
  .keys {
    display: flex;
    align-items: center;
    gap: 3px;
    margin: 0 0 var(--space-2);
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .asks {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  h3 {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 2px;
  }
  .count {
    margin-left: auto;
    font-weight: var(--weight-medium);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border-radius: var(--radius-md);
    transition:
      background-color var(--dur-fast) var(--ease-out),
      opacity var(--dur-fast) var(--ease-out);
  }
  .row:hover,
  .row:focus-within {
    background: var(--surface-2);
  }
  .row.busy {
    opacity: 0.5;
  }
  .main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    padding: 0;
    border: 0;
    background: none;
    color: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: var(--radius-sm);
  }
  .main:disabled {
    cursor: default;
  }
  .who {
    max-width: 100%;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .from {
    color: var(--text-2);
    font-weight: var(--weight-semibold);
  }
  .t {
    font-size: var(--text-sm);
    line-height: var(--leading-snug);
    display: -webkit-box;
    -webkit-line-clamp: 2;
    line-clamp: 2;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .acts {
    display: flex;
    gap: 2px;
    opacity: 0;
    transition: opacity var(--dur-fast) var(--ease-out);
  }
  .row:hover .acts,
  .row:focus-within .acts {
    opacity: 1;
  }
  .act {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    padding: 0;
    border: 0;
    border-radius: var(--radius-sm);
    background: none;
    color: var(--text-3);
    cursor: pointer;
  }
  .act:hover {
    background: var(--surface-3);
    color: var(--text);
  }
</style>
