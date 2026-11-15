<script lang="ts">
  import Radar from '@lucide/svelte/icons/radar';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import type { WidgetProps } from '../types';
  import { clock } from '../../stores/clock.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { ignoreKey, openLink, typedItems } from './actions';
  import { ageLabel, ciRadar, DEV_TYPES, type CiRun, type CiStatus } from './logic';

  let { ambient }: WidgetProps = $props();

  const now = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const q = typedItems(DEV_TYPES.ci);
  const repos = $derived(ciRadar(q.data?.items ?? []));
  const failing = $derived(repos.reduce((n, r) => n + r.failing, 0));

  const LABEL: Record<CiStatus, string> = { failing: 'failing', running: 'running', queued: 'queued', cancelled: 'cancelled', passing: 'passing' };

  function when(r: CiRun) {
    if (r.status === 'running' && r.startedAt) return `for ${ageLabel(now - r.startedAt)}`;
    return r.at ? `${ageLabel(now - r.at)} ago` : '';
  }
  function onKey(e: KeyboardEvent, r: CiRun) {
    if (ambient || ignoreKey(e)) return;
    if (e.key === 'o') {
      e.preventDefault();
      void openLink(r.url, `${r.workflow} on ${r.branch}`);
    }
  }
</script>

{#if q.loading && !q.data}
  <Skeleton lines={4} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if repos.length === 0}
  <EmptyState compact icon={Radar} title="No CI runs yet" hint="The latest GitHub Actions run per branch appears here once GitHub is connected." />
{:else}
  <p class="summary" role="status">
    {#if failing}<span class="bad"><span class="num">{failing}</span> branch{failing === 1 ? '' : 'es'} failing</span>{:else}<span class="good">All green</span>{/if}
    <span class="muted">· {repos.length} repo{repos.length === 1 ? '' : 's'}</span>
  </p>
  <div class="radar" onkeydown={listKeydown} role="presentation">
    {#each repos as repo (repo.repo)}
      <section class="repo" aria-label="{repo.repo}{repo.failing ? `, ${repo.failing} failing` : ''}">
        <h3 class="name mono truncate pii">{repo.repo}</h3>
        <ul>
          {#each repo.branches as b (b.branch)}
            <li class="branch">
              <span class="bname truncate"><GitBranch size={12} aria-hidden="true" /><span class="mono pii">{b.branch}</span></span>
              <span class="runs">
                {#each b.runs as r (r.item.id)}
                  <button
                      class="chip {r.status}"
                      data-row
                      onkeydown={(e) => onKey(e, r)}
                      disabled={ambient}
                      title="{r.workflow}: {LABEL[r.status]} {when(r)}"
                      aria-label="{r.workflow} on {b.branch}: {LABEL[r.status]}, {when(r)}. Copy link"
                      onclick={() => openLink(r.url, `${r.workflow} on ${b.branch}`)}
                    >
                      <span class="dot" aria-hidden="true"></span>
                      <span class="wf pii">{r.workflow}</span>
                      <span class="when num">{when(r)}</span>
                    </button>
                {/each}
              </span>
            </li>
          {/each}
        </ul>
      </section>
    {/each}
  </div>
{/if}

<style>
  .summary {
    margin: 0 0 var(--space-2);
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .bad {
    color: var(--danger);
    font-weight: var(--weight-semibold);
  }
  .good {
    color: var(--success);
    font-weight: var(--weight-semibold);
  }
  .muted {
    color: var(--text-3);
  }
  .radar {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: var(--space-3) var(--space-4);
  }
  .repo {
    min-width: 0;
  }
  .name {
    margin: 0 0 4px;
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text-2);
  }
  ul {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .branch {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 3px 0;
    border-top: 1px solid var(--border);
  }
  .bname {
    display: flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-xs);
    color: var(--text-2);
  }
  .runs {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
  }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 8px 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-full);
    background: var(--surface);
    color: var(--text-2);
    font-size: var(--text-2xs);
    cursor: pointer;
  }
  .chip:hover {
    background: var(--surface-2);
  }
  .chip:disabled {
    cursor: default;
  }
  .when {
    color: var(--text-3);
  }
  .dot {
    position: relative;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--text-3);
  }
  .passing .dot {
    background: var(--success);
  }
  .running .dot,
  .queued .dot {
    background: var(--warning);
  }
  .failing {
    border-color: color-mix(in oklab, var(--danger) 40%, var(--border));
    color: var(--danger);
    background: var(--danger-soft);
  }
  .failing .dot {
    background: var(--danger);
  }
  /* A new failure pulses once, gently, then simply stays red. */
  .failing .dot::after {
    content: '';
    position: absolute;
    inset: -3px;
    border-radius: 50%;
    border: 2px solid var(--danger);
    opacity: 0;
    animation: pulse-once 1.6s var(--ease-out) 1;
  }
  @keyframes pulse-once {
    0% {
      opacity: 0.7;
      transform: scale(0.6);
    }
    100% {
      opacity: 0;
      transform: scale(2.2);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .failing .dot::after {
      animation: none;
    }
  }
</style>
