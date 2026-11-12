// State and actions behind the Settings panel (agents, standing rules,
// audit, data, connections). Every call goes through `hub.dispatch`, so T3
// commands (history.purge, import.tui, integrations.disconnect) reach the
// consent dialog like any other. Two kinds of secret pass through here and
// neither is kept: the pairing token lives in `pairing` only while its
// dialog is open (closePairing drops it), and integration tokens go from
// the input straight to the shell (setIntegrationToken) and are never
// assigned to any field. Neither is ever logged.

import { hub } from './hub.svelte';
import { HubApiError } from '../api/client';
import type { AuditEntry, ClientView, ImportReport, IntegrationEntry, PairResult, PurgeResult, Rule } from '../api/types';
import { getSecretHint, setIntegrationToken, TOKEN_ERROR_TEXT, tokenErrorCode } from '../api/shell';
import { bytesToBase64, clientActivity, DATA_CLASSES, grantableClasses, type GrantableClass, filterAudit, IMPORT_MAX_BYTES, utcOffsetMinutes, type ClientActivity, type GithubConnectArgs, type CalendarConnectArgs, type GrantsDraft, grantsSetArgs, type RuleArgs } from '../settings/admin';
import { toasts } from '../design/toast.svelte';
import { packs } from './packs.svelte';

export const AUDIT_PAGE = 50;

const message = (err: unknown) => (err instanceof Error ? err.message : String(err));

export type SettingsSection = 'agents' | 'rules' | 'audit' | 'data' | 'connections' | 'packs' | 'keys' | 'shell';

class SettingsStore {
  section = $state<SettingsSection>('agents');

  clients = $state<ClientView[]>([]);
  activity = $state<Record<string, ClientActivity>>({});
  /** Built-in classes plus those loaded types declare (x-halp-data-class). */
  dataClasses = $state<GrantableClass[]>(grantableClasses(DATA_CLASSES, []));
  agentsError = $state<string | null>(null);
  agentsLoading = $state(false);

  /** Only while the pairing dialog is open. */
  pairing = $state<PairResult | null>(null);

  rules = $state<Rule[]>([]);
  rulesError = $state<string | null>(null);

  audit = $state<AuditEntry[]>([]);
  auditActor = $state('');
  auditDecision = $state('');
  auditMore = $state(false);
  auditError = $state<string | null>(null);
  auditLoading = $state(false);

  integrations = $state<IntegrationEntry[]>([]);
  integrationsRunning = $state(false);
  /** What may be shown of each stored secret (never the secret), by id. */
  secretHints = $state<Record<string, string | null>>({});
  integrationsError = $state<string | null>(null);

  busy = $state<string | null>(null);

  /** The audit page, filtered by decision here (audit.list has no decision filter). */
  readonly shownAudit = $derived(filterAudit(this.audit, this.auditDecision));

  // ------------------------------------------------------------- Agents

  async loadAgents(): Promise<void> {
    this.agentsLoading = true;
    try {
      const [list, grants, types] = await Promise.all([
        hub.dispatch('clients.list', {}),
        hub.dispatch('grants.list', {}),
        hub.dispatch('types.list', {}).catch(() => ({ types: [] })),
      ]);
      this.clients = list.clients;
      // grants.list names only the built-ins; grants.set also takes the
      // classes loaded types declare, so offer those too.
      this.dataClasses = grantableClasses(grants.data_classes.length ? grants.data_classes : DATA_CLASSES, types.types);
      this.agentsError = null;
      // Last seen and taint come with clients.list (v1.8, policy memory).
      const out: Record<string, ClientActivity> = {};
      for (const c of this.clients) out[c.id] = clientActivity(c);
      this.activity = out;
    } catch (err) {
      this.agentsError = message(err);
    } finally {
      this.agentsLoading = false;
    }
  }

  async pair(name: string, id?: string): Promise<boolean> {
    this.busy = 'pair';
    try {
      this.pairing = await hub.dispatch('clients.pair', id ? { name, id } : { name });
      await this.loadAgents();
      return true;
    } catch (err) {
      toasts.error(message(err));
      return false;
    } finally {
      this.busy = null;
    }
  }

  /** Forget the token: it is shown once, and never again. */
  closePairing(): void {
    this.pairing = null;
  }

  async revoke(c: ClientView): Promise<void> {
    const ok = await hub.confirm(`Revoke ${c.name}?`, `Its token stops working at once and its standing rules are deleted. To use it again you pair it anew.`, 'Revoke');
    if (!ok) return;
    this.busy = `revoke:${c.id}`;
    try {
      await hub.dispatch('clients.revoke', { id: c.id });
      toasts.show(`Revoked ${c.name}`, { tone: 'success' });
      await Promise.all([this.loadAgents(), this.loadRules()]);
    } catch (err) {
      toasts.error(message(err));
    } finally {
      this.busy = null;
    }
  }

  async saveGrants(c: ClientView, draft: GrantsDraft): Promise<boolean> {
    const args = grantsSetArgs(c.id, c.grants, draft);
    if (!args) return true;
    this.busy = `grants:${c.id}`;
    try {
      const view = await hub.dispatch('grants.set', args);
      this.clients = this.clients.map((x) => (x.id === view.id ? view : x));
      toasts.show(`Saved what ${c.name} may do`, { tone: 'success' });
      return true;
    } catch (err) {
      toasts.error(message(err));
      return false;
    } finally {
      this.busy = null;
    }
  }

  async clearTaint(c: ClientView): Promise<void> {
    this.busy = `taint:${c.id}`;
    try {
      const r = await hub.dispatch('policy.clear_taint', { client_id: c.id });
      toasts.show(r.was_tainted_by ? `${c.name} is trusted again (it had read ${r.was_tainted_by})` : `${c.name} was not tainted`, { tone: 'success' });
      this.activity = { ...this.activity, [c.id]: { lastSeen: this.activity[c.id]?.lastSeen ?? null, tainted: false, taintedUntil: null } };
    } catch (err) {
      toasts.error(message(err));
    } finally {
      this.busy = null;
    }
  }

  // -------------------------------------------------------------- Rules

  async loadRules(): Promise<void> {
    try {
      this.rules = (await hub.dispatch('rules.list', {})).rules;
      this.rulesError = null;
    } catch (err) {
      this.rulesError = message(err);
    }
  }

  async createRule(args: RuleArgs): Promise<boolean> {
    this.busy = 'rule';
    try {
      const rule = await hub.dispatch('rules.create', args);
      this.rules = [...this.rules.filter((r) => r.id !== rule.id), rule];
      toasts.show(`Standing rule added: ${rule.client_id} may run ${rule.command}`, { tone: 'success' });
      return true;
    } catch (err) {
      toasts.error(message(err));
      return false;
    } finally {
      this.busy = null;
    }
  }

  async deleteRule(r: Rule): Promise<void> {
    this.busy = `rule:${r.id}`;
    try {
      this.rules = (await hub.dispatch('rules.delete', { id: r.id })).rules;
    } catch (err) {
      toasts.error(message(err));
    } finally {
      this.busy = null;
    }
  }

  // -------------------------------------------------------------- Audit

  async loadAudit(more = false): Promise<void> {
    this.auditLoading = true;
    try {
      const before = more ? this.audit.at(-1)?.id : undefined;
      const actor = this.auditActor.trim();
      const { records } = await hub.dispatch('audit.list', { limit: AUDIT_PAGE, ...(before ? { before } : {}), ...(actor ? { actor_prefix: actor } : {}) });
      this.audit = more ? [...this.audit, ...records] : records;
      this.auditMore = records.length === AUDIT_PAGE;
      this.auditError = null;
    } catch (err) {
      this.auditError = message(err);
    } finally {
      this.auditLoading = false;
    }
  }

  // --------------------------------------------------------------- Data

  async purge(args: { older_than_days?: number; deleted_items?: boolean; remember?: boolean }): Promise<PurgeResult | null> {
    this.busy = 'purge';
    try {
      // T3: the consent dialog asks first.
      return await hub.dispatch('history.purge', args);
    } catch (err) {
      if (!(err instanceof HubApiError && err.code === 'denied')) toasts.error(message(err));
      return null;
    } finally {
      this.busy = null;
    }
  }

  /**
   * Read a TUI export and hand it to the core. The bytes are only
   * base64-encoded and sent; their content is never logged or kept.
   */
  async importFile(file: File, passphrase: string): Promise<ImportReport | null> {
    if (file.size > IMPORT_MAX_BYTES) {
      toasts.error(`That file is ${Math.ceil(file.size / 1024)} KiB; the import takes at most ${IMPORT_MAX_BYTES / 1024} KiB.`);
      return null;
    }
    this.busy = 'import';
    try {
      const content_base64 = bytesToBase64(new Uint8Array(await file.arrayBuffer()));
      return await hub.dispatch('import.tui', {
        content_base64,
        file_name: file.name,
        utc_offset_minutes: utcOffsetMinutes(),
        ...(passphrase ? { passphrase } : {}),
      });
    } catch (err) {
      if (!(err instanceof HubApiError && err.code === 'denied')) toasts.error(message(err));
      return null;
    } finally {
      this.busy = null;
    }
  }

  // -------------------------------------------------------- Connections

  async loadIntegrations(): Promise<void> {
    try {
      const r = await hub.dispatch('integrations.list', {});
      this.integrations = r.integrations;
      this.integrationsRunning = r.running;
      this.integrationsError = null;
      const hints: Record<string, string | null> = {};
      await Promise.all(r.integrations.filter((e) => e.status.has_secret).map(async (e) => (hints[e.config.id] = await getSecretHint(e.config.id))));
      this.secretHints = hints;
    } catch (err) {
      this.integrationsError = message(err);
    }
  }

  async connectGithub(args: GithubConnectArgs | CalendarConnectArgs): Promise<boolean> {
    this.busy = 'connect';
    try {
      await hub.dispatch('integrations.connect', args);
      await this.loadIntegrations();
      return true;
    } catch (err) {
      if (!(err instanceof HubApiError && err.code === 'denied')) toasts.error(message(err));
      return false;
    } finally {
      this.busy = null;
    }
  }

  /**
   * Hand a token to the shell (OS keychain). The caller clears its input
   * before awaiting this; nothing here keeps or logs the value. Returns an
   * error text, or null when it was stored.
   */
  async setToken(id: string, token: string): Promise<string | null> {
    this.busy = `token:${id}`;
    try {
      await setIntegrationToken(id, token);
      toasts.show('Token saved to the system keychain', { tone: 'success' });
      await this.loadIntegrations();
      return null;
    } catch (err) {
      return TOKEN_ERROR_TEXT[tokenErrorCode(err)];
    } finally {
      this.busy = null;
    }
  }

  async syncNow(id: string): Promise<void> {
    this.busy = `sync:${id}`;
    try {
      const r = await hub.dispatch('integrations.sync_now', { id });
      toasts.show(r.queued.length ? 'Sync started' : 'Nothing to sync: it is disabled or waiting for a token');
      await this.loadIntegrations();
    } catch (err) {
      toasts.error(message(err));
    } finally {
      this.busy = null;
    }
  }

  async disconnect(id: string, removeItems: boolean): Promise<void> {
    this.busy = `disconnect:${id}`;
    try {
      // T3: the consent dialog asks first.
      const r = await hub.dispatch('integrations.disconnect', { id, remove_items: removeItems });
      toasts.show(r.removed_items ? `Disconnected; removed ${r.removed_items} synced items` : 'Disconnected', { tone: 'success' });
      await this.loadIntegrations();
    } catch (err) {
      if (!(err instanceof HubApiError && err.code === 'denied')) toasts.error(message(err));
    } finally {
      this.busy = null;
    }
  }

  /** Re-read what a section shows (on open, and when admin events arrive). */
  async reload(section: SettingsSection = this.section): Promise<void> {
    if (section === 'agents') await Promise.all([this.loadAgents(), this.loadRules()]);
    else if (section === 'rules') await Promise.all([this.loadRules(), this.clients.length ? Promise.resolve() : this.loadAgents()]);
    else if (section === 'audit') await this.loadAudit();
    else if (section === 'connections') await this.loadIntegrations();
    else if (section === 'packs' || section === 'keys') await packs.load();
  }

  /** Drop everything when the panel closes, the pairing token first. */
  reset(): void {
    this.pairing = null;
    this.busy = null;
  }
}

export const settings = new SettingsStore();
