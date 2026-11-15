<script lang="ts">
  import History from '@lucide/svelte/icons/history';
  import Play from '@lucide/svelte/icons/play';
  import GitBranch from '@lucide/svelte/icons/git-branch';
  import FileCode from '@lucide/svelte/icons/file-code';
  import Terminal from '@lucide/svelte/icons/terminal';
  import type { WidgetProps } from '../types';
  import { hub } from '../../stores/hub.svelte';
  import { query } from '../../stores/query.svelte';
  import { clock } from '../../stores/clock.svelte';
  import Badge from '../../design/Badge.svelte';
  import Button from '../../design/Button.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { typedItems } from './actions';
  import { ageLabel, DEV_TYPES, latestContexts, type Context } from './logic';

  let { ambient }: WidgetProps = $props();

  const now = $derived(Math.floor(clock.now / 60_000) * 60_000);
  const q = typedItems(DEV_TYPES.context);
  const contexts = $derived(latestContexts(q.data?.items ?? []));
  // Open tasks, to name the task each snapshot resumes (and to know it still exists).
  const open = query(
    () => hub.dispatch('tasks.list', { status: ['todo', 'in_progress'], role: hub.roleId, limit: 1000 }),
    () => [hub.itemsRev, hub.roleId],
  );
  const titles = $derived(new Map((open.data?.items ?? []).map((i) => [i.id, i.title])));
  const focusId = $derived(hub.day?.focus?.id ?? null);
  const [first, ...rest] = $derived(contexts);

  function resume(c: Context) {
    if (c.taskId) void hub.run('focus.set', { id: c.taskId });
  }
  const FILES = 4;
</script>

{#snippet action(c: Context)}
  {#if !c.taskId}
    <span class="hint">No linked task</span>
  {:else if c.taskId === focusId}
    <Badge tone="accent">In focus</Badge>
  {:else if !titles.has(c.taskId)}
    <span class="hint">Task closed</span>
  {:else if !ambient}
    <Button size="sm" variant="secondary" icon={Play} data-row-focus onclick={() => resume(c)} aria-label="Resume {titles.get(c.taskId)}: set it as focus">Resume</Button>
  {/if}
{/snippet}

{#if q.loading && !q.data}
  <Skeleton lines={5} />
{:else if (q.error && !q.data)}
  <EmptyState compact title="Couldn’t load this card" hint={q.error} guide={false} />
{:else if !first}
  <EmptyState compact icon={History} title="Nothing to pick up" hint="Snapshots of where you left off (branch, changed files, last commit) appear here." />
{:else}
  <div class="ctx" onkeydown={listKeydown} role="presentation">
    <article class="hero" data-row aria-label="Latest: {first.repo} on {first.branch}">
      <header>
        <span class="repo mono truncate pii">{first.repo}</span>
        <span class="ago num">{ageLabel(now - first.capturedAt)} ago</span>
      </header>
      <p class="branch"><GitBranch size={13} aria-hidden="true" /><span class="mono truncate pii">{first.branch}</span></p>
      {#if first.taskId && titles.has(first.taskId)}<p class="task pii">{titles.get(first.taskId)}</p>{/if}
      {#if first.note}<p class="note pii">{first.note}</p>{/if}
      {#if first.files.length}
        <ul class="files" aria-label="Open files">
          {#each first.files.slice(0, FILES) as f (f)}
            <li class="truncate"><FileCode size={12} aria-hidden="true" /><span class="mono pii" title={f}>{f}</span></li>
          {/each}
          {#if first.files.length > FILES}<li class="more">+{first.files.length - FILES} more</li>{/if}
        </ul>
      {/if}
      <footer>
        {#if first.lastCommand}<code class="cmd truncate pii" title={first.lastCommand}><Terminal size={12} aria-hidden="true" />{first.lastCommand}</code>{:else}<span></span>{/if}
        {@render action(first)}
      </footer>
    </article>
    {#if rest.length}
      <ul class="others" aria-label="Other repos">
        {#each rest as c (c.item.id)}
          <li data-row>
            <span class="o-main">
              <span class="mono truncate pii">{c.repo}</span>
              <span class="o-sub truncate"><span class="mono pii">{c.branch}</span> · {ageLabel(now - c.capturedAt)} ago</span>
            </span>
            {@render action(c)}
          </li>
        {/each}
      </ul>
    {/if}
  </div>
{/if}

<style>
  .ctx {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .hero {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: var(--space-3);
    border-radius: var(--radius-lg);
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
  }
  .repo {
    font-weight: var(--weight-semibold);
    font-size: var(--text-sm);
  }
  .ago {
    margin-left: auto;
    flex: none;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  p {
    margin: 0;
  }
  .branch {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: var(--text-xs);
    color: var(--accent-text);
  }
  .task {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }
  .note {
    font-size: var(--text-xs);
    color: var(--text-2);
    line-height: var(--leading-snug);
  }
  .files {
    margin: 2px 0 0;
    padding: 0;
    list-style: none;
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .files li {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .more {
    color: var(--text-3);
    padding-left: 17px;
  }
  footer {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-2);
    margin-top: 4px;
  }
  .cmd {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
    padding: 2px 6px;
    border-radius: var(--radius-sm);
    background: var(--surface-inset);
    font-family: var(--font-mono);
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .hint {
    flex: none;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
  .others {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .others li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px 0;
    border-top: 1px solid var(--border);
  }
  .o-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-size: var(--text-xs);
  }
  .o-sub {
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
</style>
