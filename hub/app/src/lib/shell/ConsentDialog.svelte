<script lang="ts">
  import ShieldAlert from '@lucide/svelte/icons/shield-alert';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import RefreshCw from '@lucide/svelte/icons/refresh-cw';
  import { hub } from '../stores/hub.svelte';
  import { clock } from '../stores/clock.svelte';
  import Dialog from '../design/Dialog.svelte';
  import Button from '../design/Button.svelte';
  import Badge from '../design/Badge.svelte';
  import ActorAvatar from '../widgets/parts/ActorAvatar.svelte';
  import { consentView, TRUNCATED_NOTICE, NO_ARGS_NOTICE } from './consent';

  // Requests are shown one at a time, oldest first. Deny is the default
  // focus, and Approve unlocks after a short delay so a stray Enter (or an
  // agent racing keystrokes) can't approve by accident. Every value below
  // is rendered as text (Svelte escapes it); args are shown in full. Privacy
  // mode never blurs this dialog (base.css): the user must see what they
  // approve. A request the core could not show in full can only be denied.
  // When an approval comes back as "changed since you saw it" (v1.4), the
  // new request replaces the old one here with a banner, and Approve re-arms.
  const ARM_MS = 600;

  const current = $derived(hub.consent[0] ?? null);
  const queued = $derived(Math.max(0, hub.consent.length - 1));
  const descriptor = $derived(current ? hub.commands.find((c) => c.name === current.pending.command) : undefined);
  const v = $derived(current ? consentView(current.pending, descriptor, clock.now) : null);

  let armed = $state(false);

  $effect(() => {
    const id = current?.pending.id;
    armed = false;
    if (!id) return;
    setTimeout(() => document.querySelector<HTMLElement>('[data-consent-deny]')?.focus(), 0);
    const t = setTimeout(() => (armed = true), ARM_MS);
    return () => clearTimeout(t);
  });

  const busy = $derived(current?.busy === true);

  function decide(approved: boolean) {
    if (!current || busy) return;
    if (approved && (!armed || !v?.canApprove)) return;
    void hub.decideConsent(current.pending.id, approved);
  }

  const mmss = (s: number) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
</script>

<Dialog open={!!current} title="Approval needed" modal size="md" onclose={() => decide(false)} class="consent">
  {#if current && v}
    {#if current.changed}
      <div class="changed" role="alert">
        <RefreshCw size={16} aria-hidden="true" />
        <p>This request changed since you saw it — review again. Nothing ran; what it would touch is shown below as it is now.</p>
      </div>
    {/if}
    {#if v.taintWarning}
      <div class="taint" role="alert">
        <TriangleAlert size={16} aria-hidden="true" />
        <p>{v.taintWarning}</p>
      </div>
    {/if}

    <div class="who" class:agent={v.actorKind === 'agent'}>
      <ActorAvatar actor={v.actorId} size={38} />
      <div>
        <p class="actor">
          {#if v.actorKind === 'user'}You{:else}{v.actorName}{/if}
          <span class="actor-id mono">{v.actorId}</span>
        </p>
        <p class="wants">{v.ask}</p>
      </div>
      <Badge tone="danger" variant="solid" icon={ShieldAlert}>{v.tier}</Badge>
    </div>

    <p class="summary">{v.summary}</p>

    {#if v.targets.length || v.effects.length}
      <div class="impact">
        {#if v.targets.length}
          <ul class="targets">
            {#each v.targets as t (t.kind + t.id)}
              <li>
                <span class="tkind">{t.kind}</span>
                <span class="ttitle">{t.title}</span>
                {#if t.descendants > 0}<span class="tdesc">+{t.descendants} nested</span>{/if}
              </li>
            {/each}
          </ul>
        {/if}
        {#if v.effects.length}
          <ul class="effects">
            {#each v.effects as e, i (i)}<li>{e}</li>{/each}
          </ul>
        {/if}
      </div>
    {/if}

    <dl class="details">
      <dt>Command</dt>
      <dd class="mono">{v.command}</dd>
      {#if v.description}
        <dt>What it does</dt>
        <dd>{v.description}</dd>
      {/if}
    </dl>

    {#if v.privateNote}<p class="private-note">{v.privateNote}</p>{/if}
    {#if v.truncated}
      <div class="taint" role="alert">
        <TriangleAlert size={16} aria-hidden="true" />
        <p>{v.argsText === '(not shown)' ? NO_ARGS_NOTICE : TRUNCATED_NOTICE}</p>
      </div>
    {/if}
    <p class="args-label" id="consent-args-label">{v.argsLabel}</p>
    <!-- Focusable so keyboard users can scroll long arguments (WCAG 2.1.1). -->
    <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
    <pre class="args mono" tabindex="0" aria-labelledby="consent-args-label">{v.argsText}</pre>
    <p class="foot">
      {#if v.expiresInSec !== null}<span>Auto-denied in <span class="num">{mmss(v.expiresInSec)}</span></span>{/if}
      {#if v.argsHash}<span class="hash mono" title="sha256 of the arguments">{v.argsHash.slice(0, 16)}</span>{/if}
    </p>
    {#if queued}<p class="queued">{queued} more waiting after this one.</p>{/if}
  {/if}

  {#snippet footer()}
    <Button variant="secondary" disabled={busy} onclick={() => decide(false)} data-consent-deny>Deny</Button>
    <Button variant="danger" disabled={busy || !armed || !v?.canApprove} onclick={() => decide(true)} aria-describedby="consent-note">{busy ? 'Approving…' : 'Approve'}</Button>
  {/snippet}
</Dialog>

<p id="consent-note" class="visually-hidden">Approving runs the command as the requesting actor. It is recorded in history.</p>

<style>
  .who {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    margin-bottom: var(--space-4);
  }
  .who > div {
    flex: 1;
  }
  .actor {
    font-weight: var(--weight-semibold);
  }
  .wants {
    color: var(--text-3);
    font-size: var(--text-sm);
    font-weight: var(--weight-regular);
  }
  .summary {
    font-size: var(--text-lg);
    font-weight: var(--weight-medium);
    margin-bottom: var(--space-4);
    overflow-wrap: anywhere;
  }
  .details {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: var(--space-2) var(--space-4);
    margin: 0;
    padding: var(--space-3) var(--space-4);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    border: 1px solid var(--border);
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
  .taint {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
    padding: var(--space-3);
    margin-bottom: var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--danger);
    background: var(--danger-soft);
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }
  .taint :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--danger);
  }
  .changed {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
    padding: var(--space-3);
    margin-bottom: var(--space-4);
    border-radius: var(--radius-md);
    border: 1px solid var(--warning);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--text-sm);
    font-weight: var(--weight-medium);
  }
  .changed :global(svg) {
    flex: none;
    margin-top: 2px;
    color: var(--warning);
  }
  .who.agent .actor {
    font-size: var(--text-lg);
  }
  .actor-id {
    margin-left: var(--space-2);
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--surface-inset);
    border: 1px solid var(--border);
    font-size: var(--text-xs);
    font-weight: var(--weight-regular);
    color: var(--text-2);
  }
  .impact {
    margin-bottom: var(--space-4);
    font-size: var(--text-sm);
  }
  .targets,
  .effects {
    margin: 0;
    padding: 0;
    list-style: none;
    display: grid;
    gap: var(--space-1);
  }
  .targets li {
    display: flex;
    gap: var(--space-2);
    align-items: baseline;
    min-width: 0;
  }
  .tkind,
  .tdesc {
    color: var(--text-3);
    flex: none;
  }
  .ttitle {
    overflow-wrap: anywhere;
    font-weight: var(--weight-medium);
  }
  .effects {
    margin-top: var(--space-2);
    color: var(--text-2);
  }
  .effects li::before {
    content: '\2022  ';
    color: var(--text-3);
  }
  .private-note {
    font-size: var(--text-sm);
    color: var(--text-2);
    background: var(--neutral-soft);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
  }
  .args-label {
    margin: var(--space-4) 0 var(--space-1);
    color: var(--text-3);
    font-size: var(--text-xs);
  }
  .args {
    margin: 0;
    max-height: 220px;
    overflow: auto;
    padding: var(--space-3);
    border-radius: var(--radius-md);
    background: var(--surface-inset);
    border: 1px solid var(--border);
    font-size: var(--text-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    color: var(--text);
  }
  .foot {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    margin-top: var(--space-2);
    color: var(--text-3);
    font-size: var(--text-xs);
  }
  .hash {
    margin-left: auto;
  }
  .queued {
    margin-top: var(--space-3);
    color: var(--text-3);
    font-size: var(--text-sm);
  }
</style>
