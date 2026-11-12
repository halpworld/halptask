// State and actions behind Settings > Packs and Keys (P5 extension packs).
// Every write goes through `hub.dispatch`, so T3 commands (install, enable,
// uninstall, rollback, dev_set, keys) reach the consent dialog like any
// other. A bundle is read in this window, base64-encoded and sent: its bytes
// are never kept, only the SHA-256 that ties the preview to the file.

import { hub } from './hub.svelte';
import { HubApiError } from '../api/client';
import type { PackKey, PackPreview, PackView } from '../api/types';
import { toasts } from '../design/toast.svelte';
import { bytesToBase64 } from '../settings/admin';
import { BUNDLE_MAX_BYTES, installGate, PACKS_CMD, safe, sha256Hex, sizeText, trustKeyArg, type InstallGate } from '../settings/packs';
import { clock } from './clock.svelte';

const message = (err: unknown) => (err instanceof Error ? err.message : String(err));
const declined = (err: unknown) => err instanceof HubApiError && err.code === 'denied' && /declined/i.test(err.message);

class PacksStore {
  packs = $state<PackView[]>([]);
  keys = $state<PackKey[]>([]);
  error = $state<string | null>(null);
  keysError = $state<string | null>(null);
  loading = $state(false);
  busy = $state<string | null>(null);

  /** The folder dev mode watches, or null. Not persisted, like the core's. */
  devPath = $state<string | null>(null);

  /** The review being shown (null when none). */
  preview = $state<PackPreview | null>(null);
  previewFile = $state<string | null>(null);
  previewError = $state<string | null>(null);
  /** The id the Install button would send; it must still equal preview.preview_id. */
  shownId = $state<string | null>(null);
  localSha = $state<string | null>(null);
  confirmed = $state(false);
  trustKey = $state(false);

  readonly gate: InstallGate = $derived(
    installGate({ preview: this.preview, shownId: this.shownId, localSha: this.localSha, now: clock.now, confirmed: this.confirmed }),
  );

  async load(): Promise<void> {
    this.loading = true;
    try {
      const r = await hub.dispatch(PACKS_CMD.list, {});
      this.packs = r.packs;
      this.error = null;
    } catch (err) {
      this.error = message(err);
    }
    try {
      this.keys = (await hub.dispatch(PACKS_CMD.keysList, {})).keys;
      this.keysError = null;
    } catch (err) {
      this.keysError = message(err);
    } finally {
      this.loading = false;
    }
  }

  // ---------------------------------------------------------------- Install

  clearPreview(): void {
    this.preview = null;
    this.previewFile = null;
    this.previewError = null;
    this.shownId = null;
    this.localSha = null;
    this.confirmed = false;
    this.trustKey = false;
  }

  /** Read a `.halppack`, send it to packs.preview and keep the report. */
  async choose(file: File): Promise<boolean> {
    this.clearPreview();
    this.previewFile = file.name;
    if (file.size > BUNDLE_MAX_BYTES) {
      this.previewError = `That file is ${sizeText(file.size)}; a pack is at most ${sizeText(BUNDLE_MAX_BYTES)}.`;
      return false;
    }
    this.busy = 'preview';
    try {
      const bytes = new Uint8Array(await file.arrayBuffer());
      this.localSha = await sha256Hex(bytes);
      const report = await hub.dispatch(PACKS_CMD.preview, { bundle_base64: bytesToBase64(bytes) });
      this.preview = report;
      this.shownId = report.preview_id;
      return true;
    } catch (err) {
      this.localSha = null;
      // The core refused it (bad signature, wrong key, too big...): say so as it is.
      this.previewError = message(err);
      return false;
    } finally {
      this.busy = null;
    }
  }

  /** Install exactly the previewed bytes. The consent dialog follows (T3). */
  async install(): Promise<boolean> {
    const p = this.preview;
    if (!p || !this.gate.ok) return false;
    this.busy = 'install';
    try {
      const r = await hub.dispatch(PACKS_CMD.install, {
        preview_id: p.preview_id,
        ...(trustKeyArg(this.gate.offersTrustKey, this.trustKey) ? { trust_key: true } : {}),
      });
      toasts.show(`${p.installed_version ? 'Updated' : 'Installed'} ${safe(r.title)} ${safe(r.version)}`, { tone: 'success' });
      this.clearPreview();
      await this.load();
      return true;
    } catch (err) {
      if (!declined(err)) {
        this.previewError = message(err);
        // The preview is gone or changed: a stale one must not be installable.
        if (err instanceof HubApiError && (err.code === 'not_found' || err.code === 'conflict')) this.shownId = null;
      }
      return false;
    } finally {
      this.busy = null;
    }
  }

  // ---------------------------------------------------------------- Actions

  private async act(key: string, fn: () => Promise<string | void>): Promise<boolean> {
    this.busy = key;
    try {
      const msg = await fn();
      if (msg) toasts.show(msg, { tone: 'success' });
      await this.load();
      return true;
    } catch (err) {
      if (!declined(err)) toasts.error(message(err));
      return false;
    } finally {
      this.busy = null;
    }
  }

  setEnabled(p: PackView, on: boolean): Promise<boolean> {
    return this.act(`enable:${p.id}`, async () => {
      await hub.dispatch(on ? PACKS_CMD.enable : PACKS_CMD.disable, { id: p.id });
      return `${on ? 'Enabled' : 'Disabled'} ${safe(p.title)}`;
    });
  }

  async uninstall(p: PackView): Promise<boolean> {
    const ok = await hub.confirm(
      `Uninstall ${safe(p.title)}?`,
      `Its role and layout are removed and its files deleted. Items of its types stay, but the types are retired: you can still edit them, and private and locked items stay protected. Packs that extend it block this.`,
      'Uninstall',
    );
    if (!ok) return false;
    return this.act(`uninstall:${p.id}`, async () => {
      await hub.dispatch(PACKS_CMD.uninstall, { id: p.id });
      return `Uninstalled ${safe(p.title)}`;
    });
  }

  async rollback(p: PackView): Promise<boolean> {
    if (!p.previous_version) return false;
    const ok = await hub.confirm(
      `Roll back ${safe(p.title)}?`,
      `Go back to ${safe(p.previous_version)} and undo the item migration the last update made. It is refused if a migrated item changed since.`,
      'Roll back',
    );
    if (!ok) return false;
    return this.act(`rollback:${p.id}`, async () => {
      await hub.dispatch(PACKS_CMD.rollback, { id: p.id });
      return `Rolled ${safe(p.title)} back to ${safe(p.previous_version)}`;
    });
  }

  async resetLayout(p: PackView): Promise<boolean> {
    const ok = await hub.confirm(`Reset the ${safe(p.title)} layout?`, 'Your arrangement of this role is replaced with the pack default. Items are not touched.', 'Reset layout');
    if (!ok) return false;
    return this.act(`layout:${p.id}`, async () => {
      await hub.dispatch(PACKS_CMD.resetLayout, { id: p.id });
      void hub.refreshLayout();
      return `Reset layout of ${safe(p.title)}`;
    });
  }

  // -------------------------------------------------------------- Dev mode

  /** Turn dev mode on for `path`, or off with null. Needs consent (T3). */
  async setDev(path: string | null): Promise<boolean> {
    this.busy = 'dev';
    try {
      const r = await hub.dispatch(PACKS_CMD.devSet, { path });
      this.devPath = r.path;
      toasts.show(r.path ? 'Dev mode on' : 'Dev mode off', { tone: 'success' });
      await this.load();
      return true;
    } catch (err) {
      if (!declined(err)) toasts.error(message(err));
      return false;
    } finally {
      this.busy = null;
    }
  }

  // ------------------------------------------------------------------- Keys

  async addKey(pubkey: string, label: string): Promise<string | null> {
    this.busy = 'key-add';
    try {
      await hub.dispatch(PACKS_CMD.keysAdd, { pubkey_b64: pubkey.trim(), label: label.trim() });
      toasts.show(`Trusted ${safe(label.trim())}`, { tone: 'success' });
      await this.load();
      return null;
    } catch (err) {
      return declined(err) ? 'Not added: you declined.' : message(err);
    } finally {
      this.busy = null;
    }
  }

  async revokeKey(k: PackKey): Promise<void> {
    const ok = await hub.confirm(
      `Revoke the key of ${safe(k.label)}?`,
      `${k.packs} installed ${k.packs === 1 ? 'pack is' : 'packs are'} signed with it. Revoking disables ${k.packs === 1 ? 'it' : 'them'}, retires ${k.packs === 1 ? 'its' : 'their'} types, and stops new bundles with this key from verifying. To use them again you trust the key anew.`,
      'Revoke key',
    );
    if (!ok) return;
    await this.act(`revoke:${k.key_id}`, async () => {
      const r = await hub.dispatch(PACKS_CMD.keysRevoke, { key_id: k.key_id });
      return r.disabled_packs.length ? `Revoked; disabled ${r.disabled_packs.length} pack${r.disabled_packs.length === 1 ? '' : 's'}` : 'Revoked';
    });
  }
}

export const packs = new PacksStore();
