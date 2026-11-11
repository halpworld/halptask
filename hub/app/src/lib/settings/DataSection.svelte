<script lang="ts">
  import Upload from '@lucide/svelte/icons/upload';
  import Button from '../design/Button.svelte';
  import { settings } from '../stores/settings.svelte';
  import { IMPORT_MAX_BYTES, looksEncrypted, purgeArgs } from './admin';
  import type { ImportReport, PurgeResult } from '../api/types';

  let days = $state('');
  let remember = $state(false);
  let deletedItems = $state(false);
  let purgeError = $state<string | null>(null);
  let purged = $state<PurgeResult | null>(null);

  let fileInput: HTMLInputElement | undefined = $state();
  let file = $state<File | null>(null);
  let passphrase = $state('');
  let imported = $state<ImportReport | null>(null);
  const encrypted = $derived(!!file && looksEncrypted(file.name));

  async function purge(e: SubmitEvent) {
    e.preventDefault();
    const r = purgeArgs(days, remember, deletedItems);
    if (!r.ok) {
      purgeError = r.error;
      return;
    }
    purgeError = null;
    purged = await settings.purge(r.args);
  }

  function pick(e: Event) {
    file = (e.currentTarget as HTMLInputElement).files?.[0] ?? null;
    imported = null;
  }

  async function runImport(e: SubmitEvent) {
    e.preventDefault();
    if (!file) return;
    const f = file;
    // Clear our copy of the passphrase before the call.
    const p = passphrase;
    passphrase = '';
    imported = await settings.importFile(f, encrypted ? p : '');
    if (imported) {
      file = null;
      if (fileInput) fileInput.value = '';
    }
  }
</script>

<section class="sp-section" aria-labelledby="data-h">
  <h3 id="data-h">Data</h3>

  <form class="sp-card" onsubmit={purge} aria-labelledby="purge-h">
    <h4 id="purge-h" class="sp-title">Purge history</h4>
    <p class="sp-warn" role="note">
      This cannot be undone. It removes the text of old history entries (titles, notes and arguments) so they can no longer be read or undone. What
      happened and when stays.
    </p>
    <div class="sp-row">
      <label class="sp-field"><span>Older than (days)</span><input bind:value={days} inputmode="numeric" placeholder="90" /></label>
      <label class="sp-check"><input type="checkbox" bind:checked={remember} disabled={!days.trim()} /> Keep doing this (retention)</label>
      <label class="sp-check"><input type="checkbox" bind:checked={deletedItems} /> Also redact deleted items</label>
    </div>
    {#if purgeError}<p class="sp-error" role="alert">{purgeError}</p>{/if}
    <div class="sp-row"><Button type="submit" variant="danger" disabled={settings.busy === 'purge'}>Purge…</Button></div>
    <p class="sp-meta">You will be asked to confirm in the consent dialog.</p>
    {#if purged}<p class="sp-meta" role="status">Redacted {purged.redacted} entries{#if purged.retention_days} · retention {purged.retention_days} days{/if}.</p>{/if}
  </form>

  <form class="sp-card" onsubmit={runImport} aria-labelledby="import-h">
    <h4 id="import-h" class="sp-title">Import from the HalpTask TUI</h4>
    <p class="sp-lead">Pick a TUI export (Markdown, or an encrypted .halp file). Items land under a new top-level bullet. At most {IMPORT_MAX_BYTES / 1024} KiB.</p>
    <label class="sp-field"><span>File</span><input bind:this={fileInput} type="file" accept=".md,.txt,.halp,.json" onchange={pick} /></label>
    {#if encrypted}
      <label class="sp-field"><span>Passphrase for {file?.name}</span><input type="password" bind:value={passphrase} autocomplete="off" /></label>
    {/if}
    <div class="sp-row"><Button type="submit" icon={Upload} disabled={!file || settings.busy === 'import' || (encrypted && !passphrase)}>Import…</Button></div>
    <p class="sp-meta">You will be asked to confirm in the consent dialog.</p>
    {#if imported}
      <p class="sp-meta" role="status">Imported {imported.items} items ({imported.format}){#if imported.skipped_deleted} · skipped {imported.skipped_deleted} deleted{/if}.</p>
      {#each imported.warnings as w, i (i)}<p class="sp-meta">{w}</p>{/each}
    {/if}
  </form>
</section>
