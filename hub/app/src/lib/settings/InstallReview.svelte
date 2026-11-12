<script lang="ts">
  // The review of a previewed pack before install or update (§3.4). It reads
  // the preview from the packs store. Install stays disabled until the
  // preview is current (not expired, same preview_id, same bytes) and the
  // user ticked the confirmation. All pack text is untrusted: it is rendered
  // as text and passed through `safe` (escapeHidden) first.
  import ShieldAlert from '@lucide/svelte/icons/shield-alert';
  import ShieldCheck from '@lucide/svelte/icons/shield-check';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import Button from '../design/Button.svelte';
  import Badge from '../design/Badge.svelte';
  import { packs } from '../stores/packs.svelte';
  import { clock } from '../stores/clock.svelte';
  import { blockingWarnings, classEscalations, clockText, diffAddsCapability, diffRows, diffSummary, fingerprintGroups, migrationSummary, overrideViews, plainWarnings, safe, secondsLeft, sizeText, trustInfo } from './packs';

  let { oncancel }: { oncancel: () => void } = $props();

  const p = $derived(packs.preview);
  const trust = $derived(p ? trustInfo(p.trust, p.signature) : null);
  const groups = $derived(fingerprintGroups(p?.key_fp));
  const rows = $derived(diffRows(p?.permission_diff));
  const blocking = $derived(p ? blockingWarnings(p) : []);
  const plain = $derived(p ? plainWarnings(p) : []);
  const escalations = $derived(classEscalations(p?.permission_diff));
  const overrides = $derived(p ? overrideViews(p) : []);
  const left = $derived(p ? secondsLeft(p, clock.now) : 0);
  const gate = $derived(packs.gate);
  let heading: HTMLHeadingElement | undefined = $state();

  // Move focus to the review when a new one arrives.
  $effect(() => {
    if (p && heading) heading.focus();
  });
</script>

{#if p && trust}
  <section class="review" aria-labelledby="rv-h" data-testid="install-review">
    <header class="top">
      <div class="who">
        <h4 id="rv-h" tabindex="-1" bind:this={heading}>{p.installed_version ? 'Update' : 'Install'} {safe(p.title)}</h4>
        <p class="sp-meta">
          <span class="sp-mono">{safe(p.id)}</span> · version <span class="sp-mono">{safe(p.version)}</span>{#if p.installed_version}
            (installed: <span class="sp-mono">{safe(p.installed_version)}</span>){/if} · by {safe(p.publisher)}
        </p>
      </div>
      <Badge tone={trust.tone} variant={trust.warn ? 'solid' : 'soft'} icon={trust.warn ? TriangleAlert : ShieldCheck}>{trust.label}</Badge>
    </header>
    {#if p.description}<p class="desc">{safe(p.description)}</p>{/if}

    <p class="trust {trust.tone}" class:danger={p.signature === 'invalid'} role={trust.warn ? 'alert' : undefined} data-testid="trust-note">
      {#if trust.warn}<ShieldAlert size={15} aria-hidden="true" />{/if}
      <span><strong>{trust.label}.</strong> {trust.blurb}</span>
    </p>

    <div class="block" data-testid="fingerprint">
      <h5 id="rv-fp">Signing key</h5>
      {#if groups}
        <p class="fp sp-mono" aria-labelledby="rv-fp">
          {#each groups as g, i (i)}<span class="grp">{g}</span>{/each}
        </p>
        <p class="sp-meta">Compare all four groups with the fingerprint the publisher gave you somewhere else, such as their site. The short key id is the first group.</p>
      {:else}
        <p class="sp-meta">{p.signature === 'none' ? 'This bundle is not signed, so nobody vouches for who made it.' : 'No usable key fingerprint came with this review.'}</p>
      {/if}
    </div>

    {#if blocking.length}
      <div class="alert" role="alert" data-testid="blocking-errors">
        <h5>Cannot be installed</h5>
        <ul class="plain">
          {#each blocking as w, i (i)}<li>{safe(w)}</li>{/each}
        </ul>
      </div>
    {/if}

    {#if escalations.length}
      <div class="alert" role="alert" data-testid="class-escalations">
        <h5>Data class weakened</h5>
        <ul class="plain">
          {#each escalations as e, i (i)}<li>{e.text}</li>{/each}
        </ul>
      </div>
    {/if}

    {#if plain.length}
      <div class="block">
        <h5>Warnings</h5>
        <ul class="plain warn" aria-label="Warnings">
          {#each plain as w, i (i)}<li>{safe(w)}</li>{/each}
        </ul>
      </div>
    {/if}

    <div class="block" data-testid="permission-diff">
      <h5>{p.installed_version ? 'Permission changes' : 'Permissions'}</h5>
      <p class="sp-meta">{diffSummary(p.permission_diff)}{#if diffAddsCapability(p.permission_diff)} This update adds capabilities, so you decide again.{/if}</p>
      {#if rows.length}
        <ul class="diff" aria-label="Permission changes">
          {#each rows as r, i (i)}
            <li class={r.sign === '+' ? 'add' : 'del'}>
              <span class="sign" aria-hidden="true">{r.sign}</span>
              <span class="sr">{r.sign === '+' ? 'Added' : 'Removed'}:</span>
              <span class="kind">{r.label}</span>
              <span class="sp-mono txt">{r.text}</span>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    {#if overrides.length}
      <div class="block" data-testid="overrides">
        <h5>Changes to existing types — AI agents will read these descriptions</h5>
        <ul class="plain ov">
          {#each overrides as o, i (i)}
            <li>
              <span class="sp-mono">{o.typeId}</span>
              {#if o.title !== null}<div class="sp-meta">New title: {o.title}</div>{/if}
              {#if o.icon !== null}<div class="sp-meta">New icon: {o.icon}</div>{/if}
              {#if o.addedProperties.length}<div class="sp-meta">Added properties: <span class="sp-mono">{o.addedProperties.join(', ')}</span></div>{/if}
              {#each o.addedEnums as en, j (j)}
                <div class="sp-meta">Added values for <span class="sp-mono">{en.property}</span>: <span class="sp-mono">{en.values.join(', ')}</span></div>
              {/each}
              {#each o.descriptions as d, j (j)}
                <div class="desc-row"><span class="sp-mono sp-meta">{d.property}</span><span class="dtext">{d.text}</span></div>
              {/each}
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    <div class="grid">
      <div class="block">
        <h5>Role</h5>
        <p>{safe(p.role.title)} <span class="sp-mono sp-meta">{safe(p.role.id)}</span></p>
        {#if p.chain.length}
          <p class="sp-meta">Extends: <span class="sp-mono">{p.chain.map(safe).join(' → ')}</span></p>
        {:else}<p class="sp-meta">Extends nothing.</p>{/if}
      </div>
      <div class="block">
        <h5>Task types ({p.types.length})</h5>
        {#if p.types.length}
          <ul class="plain">
            {#each p.types as t, i (i)}<li>{safe(t.title)} <span class="sp-mono sp-meta">{safe(t.id)}</span></li>{/each}
          </ul>
        {:else}<p class="sp-meta">None.</p>{/if}
      </div>
      <div class="block">
        <h5>Privileged annotations ({p.annotations.length})</h5>
        {#if p.annotations.length}
          <ul class="plain">
            {#each p.annotations as a, i (i)}<li class="sp-mono">{safe(a.type_id)}: {safe(a.annotation)}</li>{/each}
          </ul>
        {:else}<p class="sp-meta">None.</p>{/if}
      </div>
      <div class="block">
        <h5>Data classes ({p.data_classes.length})</h5>
        {#if p.data_classes.length}
          <ul class="plain">
            {#each p.data_classes as d, i (i)}<li class="sp-mono">{safe(d)}</li>{/each}
          </ul>
        {:else}<p class="sp-meta">None.</p>{/if}
      </div>
    </div>

    <div class="block">
      <h5>Modules ({p.modules.length})</h5>
      {#if p.modules.length}
        <ul class="plain">
          {#each p.modules as m, i (i)}
            <li>{safe(m.title)} <span class="sp-mono sp-meta">{safe(m.id)}</span>
              <span class="sp-meta">reads: {m.inputs.length ? m.inputs.map(safe).join(', ') : 'nothing'}</span></li>
          {/each}
        </ul>
      {:else}<p class="sp-meta">None. Modules run sandboxed code on items you point them at.</p>{/if}
    </div>

    <div class="block">
      <h5>Custom widgets ({p.ui_widgets.length})</h5>
      {#if p.ui_widgets.length}
        <ul class="plain">
          {#each p.ui_widgets as w, i (i)}
            <li>{safe(w.title)} <span class="sp-mono sp-meta">{safe(w.kind)}</span>
              <span class="sp-meta">calls: {w.calls.length ? w.calls.map(safe).join(', ') : 'none'}</span></li>
          {/each}
        </ul>
      {:else}<p class="sp-meta">None.</p>{/if}
    </div>

    <div class="block">
      <h5>Prompts ({p.prompts.length})</h5>
      {#if p.prompts.length}
        <ul class="plain">
          {#each p.prompts as x, i (i)}<li><span class="sp-mono">{safe(x.name)}</span>{#if x.description} <span class="sp-meta">{safe(x.description)}</span>{/if}</li>{/each}
        </ul>
        <p class="sp-meta">Agents can list these once installed.</p>
      {:else}<p class="sp-meta">None.</p>{/if}
    </div>

    <div class="block" data-testid="migrations">
      <h5>Migration</h5>
      <p class:bad={p.migrations.items_failed > 0}>{migrationSummary(p.migrations)}</p>
      {#if p.migrations.failed_ids.length}
        <p class="sp-meta">Items that would fail: <span class="sp-mono">{p.migrations.failed_ids.map(safe).join(', ')}</span></p>
      {/if}
    </div>

    <details class="block">
      <summary>Files ({p.files.length})</summary>
      {#if p.files.length}
        <ul class="plain files">
          {#each p.files as f, i (i)}<li><span class="sp-mono">{safe(f.path)}</span> <span class="sp-meta">{sizeText(f.size)}</span></li>{/each}
        </ul>
      {:else}<p class="sp-meta">No files besides the manifest.</p>{/if}
    </details>

    <footer class="actions">
      <p class="timer sp-meta" role="status" data-testid="timer">
        {#if left > 0}Review valid for {clockText(left)}.{:else}This review expired.{/if}
      </p>
      <label class="sp-check">
        <input type="checkbox" bind:checked={packs.confirmed} disabled={p.signature === 'invalid'} />
        I have read this review and want to {p.installed_version ? 'update' : 'install'} {safe(p.title)} {safe(p.version)}.
      </label>
      {#if gate.offersTrustKey}
        <label class="sp-check">
          <input type="checkbox" bind:checked={packs.trustKey} />
          Trust this publisher key ({safe(p.publisher)}) for future updates. Only if the fingerprint above matches.
        </label>
      {/if}
      <div class="sp-row">
        <Button variant="primary" disabled={!gate.ok || packs.busy === 'install'} aria-describedby={gate.reason ? 'rv-why' : undefined} onclick={() => packs.install()}>
          {p.installed_version ? 'Update' : 'Install'}
        </Button>
        <Button variant="ghost" onclick={oncancel}>Cancel</Button>
        {#if gate.reason}<span id="rv-why" class="sp-meta" data-testid="gate-reason">{gate.reason}</span>{/if}
      </div>
      {#if packs.previewError}<p class="sp-error" role="alert">{packs.previewError}</p>{/if}
    </footer>
  </section>
{/if}

<style>
  .review {
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-lg);
    background: var(--surface);
    padding: var(--space-4);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
  }
  .top {
    display: flex;
    justify-content: space-between;
    align-items: flex-start;
    gap: var(--space-3);
  }
  h4 {
    font-size: var(--text-md);
    font-weight: var(--weight-semibold);
    overflow-wrap: anywhere;
  }
  h4:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-sm);
  }
  h5 {
    font-size: var(--text-xs);
    font-weight: var(--weight-semibold);
    color: var(--text-2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: var(--space-1);
  }
  .desc {
    color: var(--text-2);
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }
  .trust {
    display: flex;
    gap: var(--space-2);
    align-items: flex-start;
    font-size: var(--text-sm);
    padding: var(--space-3);
    border-radius: var(--radius-md);
    border: 1px solid var(--border);
    background: var(--surface-2);
  }
  .trust.warning {
    border-color: var(--warning);
    background: var(--warning-soft);
  }
  .trust.danger {
    border-color: var(--danger);
    background: var(--danger-soft);
  }
  .block {
    font-size: var(--text-sm);
    overflow-wrap: anywhere;
  }
  .fp {
    display: grid;
    grid-template-columns: repeat(4, max-content);
    gap: var(--space-2) var(--space-3);
    padding: var(--space-3);
    background: var(--surface-inset);
    border: 1px solid var(--border);
    border-radius: var(--radius-md);
    margin: 0 0 var(--space-1);
    overflow-x: auto;
    user-select: all;
  }
  .grp {
    letter-spacing: 0.03em;
  }
  @media (max-width: 760px) {
    .fp {
      grid-template-columns: repeat(2, max-content);
    }
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
    gap: var(--space-3);
  }
  ul.plain {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
  }
  ul.warn li {
    color: var(--text);
    border-left: 3px solid var(--warning);
    padding-left: var(--space-2);
  }
  ul.files {
    max-height: 12rem;
    overflow-y: auto;
    margin-top: var(--space-2);
  }
  summary {
    cursor: pointer;
    font-weight: var(--weight-medium);
  }
  summary:focus-visible {
    outline: none;
    box-shadow: var(--focus-ring);
    border-radius: var(--radius-sm);
  }
  .diff {
    list-style: none;
    margin: var(--space-2) 0 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .diff li {
    display: grid;
    grid-template-columns: 1.2rem 11rem 1fr;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2);
    border-radius: var(--radius-xs);
    align-items: baseline;
  }
  .diff .add {
    background: var(--success-soft);
  }
  .diff .del {
    background: var(--danger-soft);
    text-decoration: line-through;
    text-decoration-color: color-mix(in srgb, var(--danger) 50%, transparent);
  }
  .diff .sign {
    font-family: var(--font-mono);
    font-weight: var(--weight-semibold);
  }
  .diff .kind {
    color: var(--text-2);
    font-size: var(--text-xs);
  }
  .diff .txt {
    overflow-wrap: anywhere;
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .bad {
    color: var(--danger);
  }
  .alert {
    border: 1px solid var(--danger);
    background: var(--danger-soft);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    font-size: var(--text-sm);
    font-weight: var(--weight-semibold);
    overflow-wrap: anywhere;
  }
  ul.ov > li {
    border-left: 3px solid var(--warning);
    padding-left: var(--space-2);
  }
  .desc-row {
    display: flex;
    flex-direction: column;
    margin-top: var(--space-1);
  }
  .dtext {
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    border-top: 1px solid var(--border);
    padding-top: var(--space-3);
  }
</style>
