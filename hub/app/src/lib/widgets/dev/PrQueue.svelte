<script lang="ts">
  import GitPullRequest from '@lucide/svelte/icons/git-pull-request';
  import CircleCheck from '@lucide/svelte/icons/circle-check';
  import CircleX from '@lucide/svelte/icons/circle-x';
  import CircleDashed from '@lucide/svelte/icons/circle-dashed';
  import LoaderCircle from '@lucide/svelte/icons/loader-circle';
  import Link from '@lucide/svelte/icons/link';
  import Check from '@lucide/svelte/icons/check';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { clock } from '../../stores/clock.svelte';
  import Badge from '../../design/Badge.svelte';
  import Button from '../../design/Button.svelte';
  import Dialog from '../../design/Dialog.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Kbd from '../../design/Kbd.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { ignoreKey, openLink, typedItems } from './actions';
  import { ageLabel, compactCount, DEV_TYPES, reviewQueue, type CiState, type QueueRow } from './logic';

  let { widget, ambient }: WidgetProps = $props();

  const staleHours = $derived.by(() => {
    const v = widget.config.stale_hours;
    return typeof v === 'number' && v >= 1 && v <= 720 ? v : 24;
  });
  // Ages only need minute precision; re-derive once a minute, not every tick.
  const now = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const q = typedItems(DEV_TYPES.pr);
  const rows = $derived(reviewQueue(q.data?.items ?? [], now, staleHours));

  let openId = $state<string | null>(null);
  const open = $derived(rows.find((r) => r.pr.item.id === openId) ?? null);

  const ciInfo: Record<CiState, { label: string; tone: 'success' | 'danger' | 'neutral' | 'warning' }> = {
    passing: { label: 'CI passing', tone: 'success' },
    failing: { label: 'CI failing', tone: 'danger' },
    pending: { label: 'CI running', tone: 'warning' },
    unknown: { label: 'CI unknown', tone: 'neutral' },
  };
  const staleText = ['fresh', 'waiting', 'stale', 'very stale'];

  function what(r: QueueRow) {
    return `${r.pr.repo}#${r.pr.number}`;
  }
  function markReviewed(r: QueueRow) {
    openId = null;
    void hub.complete(r.pr.item);
  }
  function onRowKey(e: KeyboardEvent, r: QueueRow) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 'o') {
      e.preventDefault();
      void openLink(r.pr.url, what(r));
    } else if (e.key === 'x') {
      e.preventDefault();
      markReviewed(r);
    }
  }
</script>

{#snippet ciIcon(ci: CiState)}
  {#if ci === 'passing'}<CircleCheck size={14} aria-hidden="true" />{:else if ci === 'failing'}<CircleX size={14} aria-hidden="true" />{:else if ci === 'pending'}<LoaderCircle size={14} aria-hidden="true" />{:else}<CircleDashed size={14} aria-hidden="true" />{/if}
{/snippet}

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if rows.length === 0}
  <EmptyState compact icon={GitPullRequest} title="No reviews waiting on you" hint="Review requests appear here once GitHub is connected." />
{:else}
  <p class="summary">
    <span class="num">{rows.length}</span> waiting · oldest <span class="num">{ageLabel(rows[0]!.ageMs)}</span>
    {#if !ambient}<span class="keys"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="Enter" /> details <Kbd keys="o" /> copy link</span>{/if}
  </p>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul class="queue" onkeydown={listKeydown} aria-label="Pull requests waiting for your review, oldest first">
    {#each rows as r (r.pr.item.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="row s{r.stale}" data-row onkeydown={(e) => onRowKey(e, r)}>
        <span class="ci {ciInfo[r.pr.ci].tone}" title={ciInfo[r.pr.ci].label}>
          {@render ciIcon(r.pr.ci)}<span class="visually-hidden">{ciInfo[r.pr.ci].label}</span>
        </span>
        <button class="main" data-row-focus disabled={ambient} onclick={() => (openId = r.pr.item.id)} aria-label="{r.pr.item.title}, {what(r)}, waiting {ageLabel(r.ageMs)}, {staleText[r.stale]}. Open details">
          <span class="t truncate"><span class="n num">#{r.pr.number}</span> <span class="pii">{r.pr.item.title}</span></span>
          <span class="sub truncate"><span class="mono">{r.pr.repo}</span>{#if r.pr.author}<span class="pii"> · {r.pr.author}</span>{/if}</span>
        </button>
        {#if r.size}<Badge variant="outline" title="{compactCount(r.pr.additions ?? 0)} added, {compactCount(r.pr.deletions ?? 0)} removed">{r.size}</Badge>{/if}
        <Badge tone={r.stale >= 3 ? 'danger' : r.stale === 2 ? 'warning' : 'neutral'} title="Waiting {ageLabel(r.ageMs)} for your review">{ageLabel(r.ageMs)}</Badge>
      </li>
    {/each}
  </ul>
{/if}

<Dialog open={!!open} title={open ? `${open.pr.repo} #${open.pr.number}` : ''} description={open?.pr.item.title} onclose={() => (openId = null)}>
  {#if open}
    <dl class="details">
      <dt>Author</dt>
      <dd class="pii">{open.pr.author ?? 'Unknown'}</dd>
      {#if open.pr.branch}<dt>Branch</dt><dd class="mono pii">{open.pr.branch}</dd>{/if}
      <dt>Waiting</dt>
      <dd>{ageLabel(open.ageMs)} <span class="muted">({staleText[open.stale]})</span></dd>
      <dt>Checks</dt>
      <dd><span class="ci inline {ciInfo[open.pr.ci].tone}">{@render ciIcon(open.pr.ci)}</span> {ciInfo[open.pr.ci].label}</dd>
      {#if open.size}<dt>Size</dt><dd><span class="add num">+{compactCount(open.pr.additions ?? 0)}</span> <span class="del num">−{compactCount(open.pr.deletions ?? 0)}</span> · {open.size}</dd>{/if}
      {#if open.pr.mergeable !== 'unknown'}<dt>Merge</dt><dd>{open.pr.mergeable === 'clean' ? 'No conflicts' : open.pr.mergeable === 'conflicting' ? 'Has conflicts' : 'Blocked by branch rules'}</dd>{/if}
      {#if open.pr.reviewers.length}<dt>Reviewers</dt><dd class="pii">{open.pr.reviewers.join(', ')}</dd>{/if}
    </dl>
  {/if}
  {#snippet footer()}
    {#if open}
      <Button variant="ghost" icon={Link} disabled={!open.pr.url} onclick={() => open && openLink(open.pr.url, what(open))}>Copy link</Button>
      <Button variant="primary" icon={Check} onclick={() => open && markReviewed(open)}>Mark reviewed</Button>
    {/if}
  {/snippet}
</Dialog>

<style>
  .summary {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .keys {
    margin-left: auto;
    display: inline-flex;
    align-items: center;
    gap: 3px;
    color: var(--text-3);
    font-size: var(--text-2xs);
  }
  .queue {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    position: relative;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 5px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border-radius: var(--radius-md);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  /* Staleness tint: a quiet bar that warms up as a review waits. */
  .row::before {
    content: '';
    position: absolute;
    left: 0;
    top: 7px;
    bottom: 7px;
    width: 3px;
    border-radius: 3px;
    background: transparent;
  }
  .row.s1::before {
    background: var(--border-strong);
  }
  .row.s2 {
    background: color-mix(in oklab, var(--warning-soft) 45%, transparent);
  }
  .row.s2::before {
    background: var(--warning);
  }
  .row.s3 {
    background: color-mix(in oklab, var(--danger-soft) 45%, transparent);
  }
  .row.s3::before {
    background: var(--danger);
  }
  .row:hover,
  .row:focus-within {
    background: var(--surface-2);
  }
  .ci {
    display: grid;
    flex: none;
    color: var(--text-3);
  }
  .ci.inline {
    display: inline-grid;
    vertical-align: -2px;
  }
  .ci.success {
    color: var(--success);
  }
  .ci.danger {
    color: var(--danger);
  }
  .ci.warning {
    color: var(--warning);
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
  .t {
    max-width: 100%;
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }
  .n {
    color: var(--accent-text);
  }
  .sub {
    max-width: 100%;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .details {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-2) var(--space-4);
    margin: 0;
    font-size: var(--text-sm);
  }
  dt {
    color: var(--text-3);
  }
  dd {
    margin: 0;
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .muted {
    color: var(--text-3);
  }
  .add {
    color: var(--success);
  }
  .del {
    color: var(--danger);
  }
</style>
