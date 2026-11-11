<script lang="ts">
  import Plus from '@lucide/svelte/icons/plus';
  import Button from '../design/Button.svelte';
  import Badge from '../design/Badge.svelte';
  import { packs } from '../stores/packs.svelte';
  import { hub } from '../stores/hub.svelte';
  import { toasts } from '../design/toast.svelte';
  import type { PackKey } from '../api/types';
  import { checkPubkey, fingerprintGroups, invalidItemsNotice, safe } from './packs';

  let pubkey = $state('');
  let label = $state('');
  let formError = $state<string | null>(null);

  let invalid = $state<ReturnType<typeof invalidItemsNotice>>(null);

  // Revoke here (not through the store) so the result's invalid_items can be shown.
  async function revoke(k: PackKey) {
    const ok = await hub.confirm(
      `Revoke the key of ${safe(k.label)}?`,
      `${k.packs} installed ${k.packs === 1 ? 'pack is' : 'packs are'} signed with it. Revoking disables ${k.packs === 1 ? 'it' : 'them'}, retires ${k.packs === 1 ? 'its' : 'their'} types, and stops new bundles with this key from verifying. To use them again you trust the key anew.`,
      'Revoke key',
    );
    if (!ok) return;
    packs.busy = `revoke:${k.key_id}`;
    try {
      const r = await hub.dispatch('packs.keys_revoke', { key_id: k.key_id });
      invalid = invalidItemsNotice(r.invalid_items ?? []);
      toasts.show(r.disabled_packs.length ? `Revoked; disabled ${r.disabled_packs.length} pack${r.disabled_packs.length === 1 ? '' : 's'}` : 'Revoked', { tone: 'success' });
      await packs.load();
    } catch (err) {
      toasts.error(err instanceof Error ? err.message : String(err));
    } finally {
      packs.busy = null;
    }
  }

  const fmt = (ts: string | null) => (ts ? new Date(ts).toLocaleString() : null);

  async function add(e: SubmitEvent) {
    e.preventDefault();
    const bad = checkPubkey(pubkey) ?? (label.trim() ? null : 'Give the key a label, such as the publisher name.');
    if (bad) {
      formError = bad;
      return;
    }
    formError = await packs.addKey(pubkey, label);
    if (!formError) {
      pubkey = '';
      label = '';
    }
  }
</script>

<section class="sp-section" aria-labelledby="keys-h">
  <h3 id="keys-h">Publisher keys</h3>
  <p class="sp-lead">
    A pack signed by a key you trust installs as <em>community</em> and may use privileged annotations, modules and custom widgets, after you review them. Add a key
    only from a source you trust, and compare its fingerprint with the one the publisher shows on their own site. HalpTask keys are built in and cannot be removed.
  </p>
  {#if packs.keysError}<p class="sp-error" role="alert">{packs.keysError}</p>{/if}

  {#if invalid}
    <div class="sp-card notice" role="alert" data-testid="invalid-items">
      <p>{invalid.text}</p>
      <p class="sp-mono sp-meta">{invalid.shown.join(', ')}{#if invalid.more > 0} and {invalid.more} more{/if}</p>
    </div>
  {/if}

  <ul class="sp-list" aria-label="Publisher keys">
    {#each packs.keys as k (k.key_id)}
      {@const g = fingerprintGroups(k.fingerprint)}
      <li class="sp-card" class:revoked={k.revoked} data-key={k.key_id}>
        <div class="sp-row between">
          <div class="sp-row">
            <span class="sp-title">{safe(k.label)}</span>
            <span class="sp-mono sp-meta">{k.key_id}</span>
            {#if k.first_party}<Badge tone="success">HalpTask</Badge>{:else if k.revoked}<Badge tone="danger">Revoked</Badge>{:else}<Badge tone="info">Trusted</Badge>{/if}
          </div>
          {#if !k.first_party && !k.revoked}
            <Button size="sm" variant="danger" disabled={packs.busy === `revoke:${k.key_id}`} onclick={() => revoke(k)}>Revoke…</Button>
          {/if}
        </div>
        {#if g}<p class="fp sp-mono" aria-label="Fingerprint">{#each g as part, i (i)}<span>{part}</span>{/each}</p>{/if}
        <p class="sp-meta">
          {k.packs} installed {k.packs === 1 ? 'pack' : 'packs'} signed with it{#if fmt(k.added_at)} · added {fmt(k.added_at)}{/if}
        </p>
      </li>
    {/each}
  </ul>
  {#if !packs.keys.length && !packs.keysError}<p class="sp-meta">{packs.loading ? 'Loading…' : 'No keys.'}</p>{/if}

  <form class="sp-card" onsubmit={add} aria-labelledby="key-add-h">
    <h4 id="key-add-h" class="sp-title">Trust a publisher key</h4>
    <label class="sp-field">
      <span>Public key (base64, 32 bytes)</span>
      <textarea bind:value={pubkey} spellcheck="false" autocomplete="off" required aria-invalid={formError ? 'true' : undefined} placeholder="MCowBQYDK2VwAyEA…"></textarea>
    </label>
    <label class="sp-field">
      <span>Label</span>
      <input bind:value={label} autocomplete="off" maxlength="80" required placeholder="Acme Platform Team" />
    </label>
    {#if formError}<p class="sp-error" role="alert">{formError}</p>{/if}
    <div class="sp-row"><Button type="submit" variant="primary" icon={Plus} disabled={packs.busy === 'key-add'}>Trust key</Button></div>
  </form>
</section>

<style>
  .notice {
    border-color: var(--warning);
    background: var(--warning-soft);
    overflow-wrap: anywhere;
  }
  .revoked {
    opacity: 0.75;
  }
  .fp {
    display: flex;
    flex-wrap: wrap;
    gap: var(--space-1) var(--space-3);
    margin: 0;
    overflow-wrap: anywhere;
  }
</style>
