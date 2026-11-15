<script lang="ts">
  import ShieldCheck from '@lucide/svelte/icons/shield-check';
  import Lock from '@lucide/svelte/icons/lock';
  import Copy from '@lucide/svelte/icons/copy';
  import FileWarning from '@lucide/svelte/icons/file-warning';
  import Check from '@lucide/svelte/icons/check';
  import X from '@lucide/svelte/icons/x';
  import RotateCcw from '@lucide/svelte/icons/rotate-ccw';
  import type { WidgetProps } from '../types';
  import Badge from '../../design/Badge.svelte';
  import EmptyState from '../../design/EmptyState.svelte';
  import IconButton from '../../design/IconButton.svelte';
  import Kbd from '../../design/Kbd.svelte';
  import Skeleton from '../../design/Skeleton.svelte';
  import { listKeydown } from '../parts/listnav';
  import { copyText, ignoreKey, typedItems } from './actions';
  import { chainState, CSO_TYPES, evidenceGaps, evidenceRows, isLocked, readDataRequest, readEvidence, shortHash, type ChainState, type DataRequest, type Evidence } from './logic';
  import { fmtBytes, humanize } from './format';

  let { ambient }: WidgetProps = $props();

  const evQ = typedItems(CSO_TYPES.evidence);
  const reqQ = typedItems(CSO_TYPES.dataRequest);
  const evidence = $derived((evQ.data?.items ?? []).map(readEvidence).filter((e): e is Evidence => !!e));
  const requests = $derived((reqQ.data?.items ?? []).map(readDataRequest).filter((r): r is DataRequest => !!r));
  const rows = $derived(evidenceRows(evidence));
  const gaps = $derived(evidenceGaps(requests, evidence));
  const approved = $derived(evidence.filter((e) => chainState(e) === 'approved').length);

  const STATE: Record<ChainState, { label: string; tone: 'success' | 'warning' | 'danger' | 'info' | 'neutral' }> = {
    approved: { label: 'Approved', tone: 'success' },
    pending: { label: 'In review', tone: 'info' },
    none: { label: 'No review yet', tone: 'neutral' },
    changes: { label: 'Changes requested', tone: 'warning' },
    rejected: { label: 'Rejected', tone: 'danger' },
    inconsistent: { label: 'Approval missing', tone: 'danger' },
  };
  const DECISION_ICON = { approved: Check, rejected: X, changes_requested: RotateCcw } as const;
  const stamp = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });

  function copyHash(e: Evidence) {
    if (e.sha256) void copyText(e.sha256, `SHA-256 of ${e.fileName}`);
  }
  function onRowKey(ev: KeyboardEvent, e: Evidence) {
    if (ambient || ignoreKey(ev)) return;
    if (ev.key === 'y' && e.sha256) {
      ev.preventDefault();
      copyHash(e);
    }
  }
  function aria(e: Evidence): string {
    const s = chainState(e);
    const chain = e.approvals.map((a) => `${a.reviewer} ${humanize(a.decision)}`).join(', then ');
    return `${e.fileName}${e.dataPoint ? `, for ${e.dataPoint}` : ''}, ${STATE[s].label}${isLocked(e) ? ', locked' : ''}${chain ? `, ${chain}` : ''}, hash ${shortHash(e.sha256)}`;
  }
</script>

{#if (evQ.loading && !evQ.data) || (reqQ.loading && !reqQ.data)}
  <Skeleton lines={6} />
{:else if (evQ.error && !evQ.data) || (reqQ.error && !reqQ.data)}
  <EmptyState compact title="Couldn’t load this card" hint={evQ.error ?? reqQ.error ?? undefined} guide={false} />
{:else if evidence.length === 0 && gaps.length === 0}
  <EmptyState compact icon={ShieldCheck} title="No evidence yet" hint="Evidence files appear here with their hash and approval chain." />
{:else}
  <p class="sum">
    <span class="num big">{approved}</span><span class="of">/ {evidence.length} approved</span>
    {#if gaps.length}<Badge tone="warning" icon={FileWarning} title="Data points with a value but no approved evidence">{gaps.length} gap{gaps.length === 1 ? '' : 's'}</Badge>{/if}
  </p>

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <ul class="rows" aria-label="Evidence, items needing attention first" onkeydown={listKeydown}>
    {#each rows as e (e.item.id)}
      {@const s = chainState(e)}
      <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
      <li class="row {s}" data-row onkeydown={(ev) => onRowKey(ev, e)}>
        <span class="top">
          <span class="file" tabindex="-1" data-row-focus aria-label={aria(e)}>
            {#if isLocked(e)}<Lock size={12} aria-hidden="true" />{/if}
            <span class="name truncate">{e.fileName}</span>
          </span>
          <Badge tone={STATE[s].tone} variant={s === 'approved' ? 'soft' : 'outline'} title={s === 'inconsistent' ? 'Marked approved, but no reviewer approval or hash is recorded.' : undefined}>{STATE[s].label}</Badge>
        </span>
        <span class="meta">
          {#if e.dataPoint}<span class="dp truncate">{e.dataPoint}</span>{/if}
          <span class="hash mono" title={e.sha256 ?? 'No valid SHA-256 recorded'}>{shortHash(e.sha256)}</span>
          {#if !ambient && e.sha256}<IconButton size="sm" icon={Copy} label="Copy the full SHA-256 of {e.fileName} (y)" onclick={() => copyHash(e)} />{/if}
          {#if e.sizeBytes !== undefined}<span class="num">{fmtBytes(e.sizeBytes)}</span>{/if}
          {#if e.sourceSystem}<span>{e.sourceSystem}</span>{/if}
        </span>
        {#if e.approvals.length}
          <ol class="chain" aria-label="Approval chain">
            {#each e.approvals as a, i (i)}
              {@const Icon = DECISION_ICON[a.decision]}
              <li class="step {a.decision}" title="{a.reviewer}: {humanize(a.decision)}, {stamp.format(a.at)}{a.note ? ` – ${a.note}` : ''}">
                <span class="dot" aria-hidden="true"><Icon size={10} strokeWidth={3} /></span>
                <span class="who pii">{a.reviewer}</span>
              </li>
            {/each}
          </ol>
        {/if}
      </li>
    {/each}
  </ul>

  {#if gaps.length}
    <section class="gaps" aria-label="Evidence gaps">
      <h3 class="head">Gaps: data without approved evidence</h3>
      <ul>
        {#each gaps as g (g.r.item.id)}
          <li class={g.kind}>
            <span class="gap-dot" aria-hidden="true"></span>
            <span class="truncate"><strong>{g.r.supplier}</strong> · {g.r.metric}{g.r.period ? ` · ${g.r.period}` : ''}</span>
            <span class="kind">{g.kind === 'missing' ? 'no evidence' : 'not approved'}</span>
          </li>
        {/each}
      </ul>
    </section>
  {/if}
  {#if !ambient}<p class="keys"><Kbd keys="j" /><Kbd keys="k" /> move <Kbd keys="y" /> copy hash · approved evidence is locked</p>{/if}
{/if}

<style>
  .sum {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    margin: 0 0 var(--space-2);
  }
  .big {
    font-size: var(--text-2xl);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-tight);
    color: var(--accent-text);
    line-height: 1;
  }
  .of {
    font-size: var(--text-xs);
    color: var(--text-2);
    margin-right: auto;
  }
  .rows {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
  }
  .row {
    display: flex;
    flex-direction: column;
    gap: 3px;
    padding: var(--space-2) 0;
    border-bottom: 1px solid var(--border);
  }
  .row:last-child {
    border-bottom: 0;
  }
  .row:focus-within {
    background: var(--surface-2);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .file {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    min-width: 0;
    flex: 1;
    color: var(--text);
    outline: none;
    border-radius: var(--radius-sm);
  }
  .file:focus-visible {
    box-shadow: var(--focus-ring);
  }
  .approved .file {
    color: var(--success);
  }
  .name {
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
    color: var(--text);
  }
  .meta {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0 var(--space-2);
    font-size: var(--text-2xs);
    color: var(--text-3);
    min-width: 0;
  }
  .dp {
    max-width: 100%;
    color: var(--text-2);
  }
  .hash {
    padding: 1px 5px;
    border-radius: var(--radius-sm);
    background: var(--surface-inset);
    color: var(--text-2);
    font-size: var(--text-2xs);
  }
  .chain {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 2px;
    margin: 2px 0 0;
    padding: 0;
    list-style: none;
  }
  .step {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--text-2xs);
    color: var(--text-2);
  }
  .step + .step::before {
    content: '';
    display: inline-block;
    width: 12px;
    height: 1px;
    margin-right: 2px;
    background: var(--border-strong);
  }
  .dot {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 16px;
    height: 16px;
    border-radius: var(--radius-full);
    color: var(--surface);
    background: var(--text-3);
  }
  .step.approved .dot {
    background: var(--success);
  }
  .step.rejected .dot {
    background: var(--danger);
  }
  .step.changes_requested .dot {
    background: var(--warning);
  }
  .gaps {
    margin-top: var(--space-2);
    padding: var(--space-2);
    border: 1px dashed var(--warning);
    border-radius: var(--radius-md);
    background: var(--warning-soft);
  }
  .head {
    margin: 0 0 var(--space-1);
    font-family: var(--font-label);
    font-size: var(--text-2xs);
    font-weight: var(--weight-semibold);
    letter-spacing: var(--tracking-caps);
    text-transform: uppercase;
    color: var(--text-2);
  }
  .gaps ul {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .gaps li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--text-xs);
    color: var(--text);
    min-width: 0;
  }
  .gaps li .truncate {
    flex: 1;
  }
  .gap-dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: var(--radius-full);
    border: 1.5px dashed var(--warning);
  }
  .unapproved .gap-dot {
    border-style: solid;
  }
  .kind {
    font-size: var(--text-2xs);
    color: var(--text-2);
    white-space: nowrap;
  }
  .keys {
    display: flex;
    align-items: center;
    gap: 3px;
    margin: var(--space-2) 0 0;
    font-size: var(--text-2xs);
    color: var(--text-3);
  }
</style>
