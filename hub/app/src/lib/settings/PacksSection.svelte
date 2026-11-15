<script lang="ts">
  import FileUp from '@lucide/svelte/icons/file-up';
  import Undo2 from '@lucide/svelte/icons/undo-2';
  import LayoutDashboard from '@lucide/svelte/icons/layout-dashboard';
  import Trash2 from '@lucide/svelte/icons/trash-2';
  import TriangleAlert from '@lucide/svelte/icons/triangle-alert';
  import Button from '../design/Button.svelte';
  import Badge from '../design/Badge.svelte';
  import InstallReview from './InstallReview.svelte';
  import { packs } from '../stores/packs.svelte';
  import { capabilitySummary, checkDevPath, formatFingerprint, safe, trustInfo } from './packs';
  import type { PackView } from '../api/types';

  let fileInput: HTMLInputElement | undefined = $state();
  let devInput = $state('');
  let devError = $state<string | null>(null);

  async function picked(e: Event & { currentTarget: HTMLInputElement }) {
    const input = e.currentTarget;
    const file = input.files?.[0];
    if (!file) return;
    await packs.choose(file);
    // The same file can then be chosen again (a fresh preview).
    input.value = '';
  }

  function cancelReview() {
    packs.clearPreview();
    fileInput?.focus();
  }

  async function toggle(e: Event & { currentTarget: HTMLInputElement }, p: PackView) {
    const el = e.currentTarget;
    await packs.setEnabled(p, el.checked);
    // A declined consent leaves the pack as it was: show that.
    el.checked = packs.packs.find((x) => x.id === p.id)?.enabled ?? p.enabled;
  }

  async function devOn(e: SubmitEvent) {
    e.preventDefault();
    devError = checkDevPath(devInput);
    if (devError) return;
    await packs.setDev(devInput.trim());
  }

  const chain = (p: PackView) => p.extends.map(safe).join(' → ');
</script>

<section class="sp-section" aria-labelledby="packs-h">
  <h3 id="packs-h">Packs</h3>
  <p class="sp-lead">
    A pack adds a role, task types, widgets and prompts. Everything a pack says about itself is untrusted text. You review exactly what it asks for before it is
    installed, and only you can install, update or enable one: agents cannot.
  </p>
  {#if packs.error}<p class="sp-error" role="alert">{packs.error}</p>{/if}

  <ul class="sp-list" aria-label="Installed packs">
    {#each packs.packs as p (p.id)}
      {@const t = trustInfo(p.trust)}
      {@const fp = formatFingerprint(p.key_fp)}
      <li class="sp-card pack" class:warn={t.warn} data-pack={p.id}>
        <div class="sp-row between">
          <div class="sp-row">
            <span class="sp-title">{safe(p.title)}</span>
            <span class="sp-mono sp-meta">{safe(p.id)} · {safe(p.version)}</span>
            <Badge tone={t.tone} variant={t.warn ? 'solid' : 'soft'} icon={t.warn ? TriangleAlert : undefined} title={t.blurb}>{t.label}</Badge>
            {#if p.status === 'blocked'}<Badge tone="danger">Key revoked</Badge>{:else if p.status === 'error'}<Badge tone="danger">Error</Badge>{/if}
          </div>
          <label class="sp-check">
            <input type="checkbox" role="switch" checked={p.enabled} disabled={packs.busy === `enable:${p.id}` || p.status === 'blocked'} onchange={(e) => toggle(e, p)} />
            {p.enabled ? 'Enabled' : 'Disabled'}<span class="sr"> {safe(p.title)}</span>
          </label>
        </div>
        {#if p.description}<p class="desc">{safe(p.description)}</p>{/if}
        <p class="sp-meta">
          Role <span class="sp-mono">{safe(p.role_id)}</span>{#if p.publisher} · by {safe(p.publisher)}{/if}{#if p.extends.length}
            · extends <span class="sp-mono">{chain(p)}</span>{/if}
        </p>
        <p class="sp-meta">{capabilitySummary(p.capabilities)}</p>
        {#if fp}<p class="sp-meta">Key <span class="sp-mono">{fp}</span></p>{/if}
        {#if t.warn}<p class="note">{t.blurb}</p>{/if}
        <div class="sp-row">
          {#if p.previous_version}
            <Button size="sm" icon={Undo2} disabled={packs.busy === `rollback:${p.id}`} onclick={() => packs.rollback(p)}>Roll back to {safe(p.previous_version)}</Button>
          {/if}
          <Button size="sm" icon={LayoutDashboard} disabled={packs.busy === `layout:${p.id}`} onclick={() => packs.resetLayout(p)}>Reset layout</Button>
          {#if p.trust !== 'builtin' && p.trust !== 'dev'}
            <Button size="sm" variant="danger" icon={Trash2} disabled={packs.busy === `uninstall:${p.id}`} onclick={() => packs.uninstall(p)}>Uninstall…</Button>
          {/if}
        </div>
      </li>
    {/each}
  </ul>
  {#if !packs.packs.length && !packs.error}<p class="sp-meta">{packs.loading ? 'Loading…' : 'No packs installed.'}</p>{/if}

  <div class="sp-card" role="group" aria-labelledby="inst-h">
    <h4 id="inst-h" class="sp-title">Install or update a pack</h4>
    <p class="sp-meta">Choose a <span class="sp-mono">.halppack</span> file. Nothing is installed yet: you get a review first.</p>
    <div class="sp-row">
      <label class="sp-field">
        <span>Pack file</span>
        <input bind:this={fileInput} type="file" accept=".halppack,application/json" onchange={picked} disabled={packs.busy === 'preview'} />
      </label>
      {#if packs.busy === 'preview'}<span class="sp-meta" role="status"><FileUp size={14} aria-hidden="true" /> Reading and verifying…</span>{/if}
    </div>
    {#if packs.previewError && !packs.preview}
      <p class="sp-error" role="alert" data-testid="preview-error">
        {packs.previewFile ? `${safe(packs.previewFile)}: ` : ''}{packs.previewError}
      </p>
    {/if}
  </div>
  <InstallReview oncancel={cancelReview} />

  <form class="sp-card" class:warn={!!packs.devPath} onsubmit={devOn} aria-labelledby="dev-h">
    <h4 id="dev-h" class="sp-title">Dev mode</h4>
    <p class="sp-meta">
      Load packs straight from a folder while you write them. They run unsigned, are never stored, and are not offered to agents as prompts. It turns off when the app
      restarts.
    </p>
    {#if packs.devPath}
      <p class="note" role="status">Dev mode is on for <span class="sp-mono">{safe(packs.devPath)}</span>.</p>
      <div class="sp-row"><Button variant="danger" disabled={packs.busy === 'dev'} onclick={() => packs.setDev(null)}>Turn off dev mode</Button></div>
    {:else}
      <div class="sp-row">
        <label class="sp-field">
          <span>Folder path</span>
          <input bind:value={devInput} class="sp-mono" placeholder="/home/me/packs" spellcheck="false" autocomplete="off" aria-invalid={devError ? 'true' : undefined} />
        </label>
        <Button type="submit" variant="secondary" disabled={packs.busy === 'dev'}>Turn on dev mode</Button>
      </div>
      {#if devError}<p class="sp-error" role="alert">{devError}</p>{/if}
    {/if}
  </form>
</section>

<style>
  .pack.warn {
    border-color: var(--warning);
  }
  .desc {
    font-size: var(--text-sm);
    color: var(--text-2);
    overflow-wrap: anywhere;
  }
  .note {
    border: 1px solid var(--warning);
    background: var(--warning-soft);
    border-radius: var(--radius-md);
    padding: var(--space-2) var(--space-3);
    font-size: var(--text-xs);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
  .pack :global(.sp-title),
  .pack :global(.sp-meta) {
    overflow-wrap: anywhere;
  }
</style>
