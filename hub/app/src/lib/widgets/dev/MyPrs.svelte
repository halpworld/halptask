<script lang="ts">
  import GitPullRequestArrow from '@lucide/svelte/icons/git-pull-request-arrow';
  import type { WidgetProps } from '../types';
  import { clock } from '../../stores/clock.svelte';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { ignoreKey, openLink, typedItems } from './actions';
  import { DEV_TYPES, myPrs, STAGES, type MyPrRow } from './logic';

  let { ambient }: WidgetProps = $props();

  const now = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const q = typedItems(DEV_TYPES.pr);
  const rows = $derived(myPrs(q.data?.items ?? [], now));
  const open = $derived(rows.filter((r) => r.stage < 3).length);

  function what(r: MyPrRow) {
    return `${r.pr.repo}#${r.pr.number}`;
  }
  function onRowKey(e: KeyboardEvent, r: MyPrRow) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 'o') {
      e.preventDefault();
      void openLink(r.pr.url, what(r));
    }
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={4} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if rows.length === 0}
  <EmptyState compact icon={GitPullRequestArrow} title="No open pull requests of yours" hint="Pull requests you open appear here once GitHub is connected." />
{:else}
  <p class="legend" aria-hidden="true">
    <span class="num">{open}</span> open
    <span class="stages">{#each STAGES as s (s)}<span>{s}</span>{/each}</span>
  </p>
  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul class="prs" onkeydown={listKeydown} aria-label="Your pull requests, the ones needing you first">
    {#each rows as r (r.pr.item.id)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="row" class:merged={r.stage === 3} data-row onkeydown={(e) => onRowKey(e, r)}>
        <button
          class="main"
          data-row-focus
          disabled={ambient}
          onclick={() => openLink(r.pr.url, what(r))}
          aria-label="{r.pr.item.title}, {what(r)}. Stage {STAGES[r.stage]}.{r.blocker ? ` ${r.blocker}.` : ''} Copy link"
        >
          <span class="t truncate"><span class="n num">#{r.pr.number}</span> <span class="pii">{r.pr.item.title}</span></span>
          <span class="sub truncate mono">{r.pr.repo}</span>
        </button>
        <ol class="pipe" aria-hidden="true">
          {#each STAGES as s, i (s)}
            <li class:reached={i <= r.stage} class:current={i === r.stage} class="{r.tone}" title={s}></li>
          {/each}
        </ol>
        <span class="blocker">
          {#if r.blocker}<Badge tone={r.tone}>{r.blocker}</Badge>{:else}<Badge tone="success" variant="outline">Merged</Badge>{/if}
        </span>
      </li>
    {/each}
  </ul>
{/if}

<style>
  .legend {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .stages {
    margin-left: auto;
    display: inline-flex;
    gap: var(--space-2);
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .prs {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto minmax(0, max-content);
    align-items: center;
    gap: var(--space-3);
    padding: 5px var(--space-2);
    margin: 0 calc(var(--space-2) * -1);
    border-radius: var(--radius-md);
    transition: background-color var(--dur-fast) var(--ease-out);
  }
  .row:hover,
  .row:focus-within {
    background: var(--surface-2);
  }
  .row.merged {
    opacity: 0.62;
  }
  .main {
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
  /* Draft → Review → Approved → Merged as four segments. */
  .pipe {
    display: flex;
    gap: 3px;
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .pipe li {
    width: 16px;
    height: 6px;
    border-radius: 3px;
    background: var(--surface-3);
  }
  .pipe li.reached {
    background: var(--accent);
    opacity: 0.45;
  }
  .pipe li.current {
    opacity: 1;
  }
  .pipe li.current.danger {
    background: var(--danger);
  }
  .pipe li.current.warning {
    background: var(--warning);
  }
  .pipe li.current.success {
    background: var(--success);
  }
  .blocker {
    min-width: 0;
    justify-self: end;
  }
</style>
