// App-wide reactive state, driven entirely by the command contract: every
// change goes through `dispatch`, and every refresh is triggered by an event.

import { createClient, HubApiError, type ConsentOutcome, type HubClient } from '../api';
import type { Args, ArgsOf, CommandDescriptor, CommandName, DayState, HubEvent, Item, Layout, PendingConfirmation, ResultOf, Role } from '../api/types';
import { changedIds, moveBox, resizeBox } from '../layout/grid';
import { toasts } from '../design/toast.svelte';
import { CONSENT_SETTLED, EV, EV_PREFIX } from '../api/events';

export type ThemePref = 'system' | 'light' | 'dark';

export interface ConsentItem {
  pending: PendingConfirmation;
  /** Present when the request came from this window's own dispatch. */
  resolve?: (outcome: ConsentOutcome) => void;
  /** It replaces a request the user saw, whose scope changed (v1.4 re-ask). */
  changed?: boolean;
  /** An approval for it is in flight: events about it are ours. */
  busy?: boolean;
}

/** How many `consent.get` calls run at once when filling in shortened requests. */
export const CONSENT_GET_CONCURRENCY = 4;
/** How long a superseded waiting call waits for its re-ask before failing. */
export const SUPERSEDED_WAIT_MS = 5_000;

/** Runs `f` over `xs` with at most `limit` calls in flight; results in order. */
export async function mapLimit<T, R>(xs: readonly T[], limit: number, f: (x: T) => Promise<R>): Promise<R[]> {
  const out: R[] = new Array(xs.length);
  let next = 0;
  const worker = async () => {
    while (next < xs.length) {
      const i = next++;
      out[i] = await f(xs[i]!);
    }
  };
  await Promise.all(Array.from({ length: Math.min(limit, xs.length) }, worker));
  return out;
}

/** A local yes/no question (not agent consent: that is ConsentDialog). */
export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  resolve: (yes: boolean) => void;
}

const PREFS_KEY = 'halp.hub.prefs.v1';
const AGENT_RECENT_MS = 6000;
const TOUCH_MS = 2400;

function loadPrefs(): { theme?: ThemePref; privacy?: boolean } {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    return raw ? (JSON.parse(raw) as { theme?: ThemePref; privacy?: boolean }) : {};
  } catch {
    return {};
  }
}

function savePrefs(p: { theme: ThemePref; privacy: boolean }): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(p));
  } catch {
    /* storage unavailable: prefs are a convenience only */
  }
}

export function isAgent(actor: string): boolean {
  return actor.startsWith('agent:');
}

export function actorName(actor: string): string {
  if (actor === 'user') return 'You';
  if (actor === 'system') return 'System';
  const [kind, id = ''] = actor.split(':');
  const name = id ? id.charAt(0).toUpperCase() + id.slice(1) : actor;
  return kind === 'agent' ? name : `${name} (${kind})`;
}

class Hub {
  client: HubClient | null = null;
  status = $state<'loading' | 'ready' | 'error'>('loading');
  error = $state<string | null>(null);
  transport = $state<'tauri' | 'mock'>('mock');

  roles = $state<Role[]>([]);
  roleId = $state<string>('developer');
  layout = $state<Layout | null>(null);
  day = $state<DayState | null>(null);
  commands = $state<CommandDescriptor[]>([]);
  events = $state<HubEvent[]>([]);
  /** Bumped when items change; widgets re-query on it. */
  itemsRev = $state(0);
  /** Widget ids recently changed by someone other than you → expiry time. */
  touched = $state<Record<string, number>>({});
  agentActiveUntil = $state(0);

  ambient = $state(false);
  privacy = $state(false);
  editing = $state(false);
  theme = $state<ThemePref>('system');
  systemDark = $state(false);
  paletteOpen = $state(false);
  helpOpen = $state(false);
  settingsOpen = $state(false);
  /** Bumped on client, grant, rule, taint and integration events; the settings panel re-reads on it. */
  adminRev = $state(0);

  consent = $state<ConsentItem[]>([]);
  confirmRequest = $state<ConfirmRequest | null>(null);

  role = $derived(this.roles.find((r) => r.id === this.roleId));
  resolvedTheme = $derived<'light' | 'dark'>(this.theme === 'system' ? (this.systemDark ? 'dark' : 'light') : this.theme);
  agentEvents = $derived(this.events.filter((e) => isAgent(e.actor)));

  private refreshTimer: ReturnType<typeof setTimeout> | null = null;
  private pendingRefresh = { day: false, layout: false, markActor: '' as string };

  async init(): Promise<void> {
    const prefs = loadPrefs();
    this.theme = prefs.theme ?? 'system';
    this.privacy = prefs.privacy ?? false;
    if (typeof window !== 'undefined' && window.matchMedia) {
      const mq = window.matchMedia('(prefers-color-scheme: dark)');
      this.systemDark = mq.matches;
      mq.addEventListener('change', (e) => (this.systemDark = e.matches));
    }

    try {
      const client = await createClient();
      this.client = client;
      this.transport = client.kind;
      client.setConsentHandler((p, changed) => this.requestConsent(p, changed));
      client.onEvent((e) => this.handleEvent(e));

      const [roles, commands, day, history, pending] = await Promise.all([
        client.dispatch('role.list', {}),
        client.dispatch('commands.list', {}),
        client.dispatch('day.state', {}),
        client.dispatch('history.list', { limit: 40 }),
        client.dispatch('consent.list', {}).catch(() => ({ pending: [] as PendingConfirmation[] })),
      ]);
      this.roles = roles.roles;
      this.commands = commands.commands;
      this.applyDay(day);
      this.events = history.events;
      for (const p of await this.completePending(pending.pending)) this.enqueueConsent({ pending: p });
      this.status = 'ready';
    } catch (err) {
      this.error = err instanceof Error ? err.message : String(err);
      this.status = 'error';
    }
  }

  // ------------------------------------------------------------ Dispatching

  async dispatch<K extends CommandName>(command: K, args: ArgsOf<K>): Promise<ResultOf<K>>;
  async dispatch(command: string, args?: Args): Promise<unknown>;
  async dispatch(command: string, args: Args = {}): Promise<unknown> {
    if (!this.client) throw new Error('Hub not connected');
    return this.client.dispatch(command, args);
  }

  /** Dispatch and surface failures as a toast; returns undefined on error. */
  async run<K extends CommandName>(command: K, args: ArgsOf<K>): Promise<ResultOf<K> | undefined> {
    try {
      return await this.dispatch(command, args);
    } catch (err) {
      if (err instanceof HubApiError && err.code === 'denied') toasts.show(`Cancelled: ${command}`);
      else toasts.error(err instanceof Error ? err.message : String(err));
      return undefined;
    }
  }

  // ---------------------------------------------------------------- Events

  handleEvent(e: HubEvent): void {
    if (!this.events.some((x) => x.id === e.id)) this.events = [e, ...this.events].slice(0, 120);
    if (isAgent(e.actor)) this.agentActiveUntil = Date.now() + AGENT_RECENT_MS;

    const c = e.command;
    if (c === EV.consentRequested) {
      void this.fetchConsent(e.entity.id);
      return;
    }
    // Approved, denied, timed out or superseded: drop it from the queue.
    if (CONSENT_SETTLED.includes(c)) {
      this.dropConsent(e.entity.id, c);
      return;
    }
    if (c === EV.policyClearTaint || c.startsWith(EV_PREFIX.clients) || c.startsWith(EV_PREFIX.grants) || c.startsWith(EV_PREFIX.rules) || c.startsWith(EV_PREFIX.integrations)) {
      this.adminRev++;
    }
    if (c.startsWith(EV_PREFIX.tasks) || c.startsWith(EV_PREFIX.capture) || e.entity.kind === 'item' || c === EV.extUpsert || c === EV.integrationsDisconnect || c === EV.importTui) {
      this.itemsRev++;
      this.scheduleRefresh({ day: true });
    }
    if (c.startsWith(EV_PREFIX.focus)) this.scheduleRefresh({ day: true });
    if (c.startsWith(EV_PREFIX.layout) || e.entity.kind === 'widget' || e.entity.kind === 'layout') {
      this.scheduleRefresh({ layout: true, markActor: e.actor });
    }
    if (c === EV.roleSwitch) this.scheduleRefresh({ day: true, layout: true });
    if (c === EV.historyUndo || c === EV.historyRedo) {
      this.itemsRev++;
      this.scheduleRefresh({ day: true, layout: true, markActor: e.actor });
    }
  }

  private scheduleRefresh(what: { day?: boolean; layout?: boolean; markActor?: string }): void {
    this.pendingRefresh.day ||= !!what.day;
    this.pendingRefresh.layout ||= !!what.layout;
    if (what.markActor) this.pendingRefresh.markActor = what.markActor;
    if (this.refreshTimer) return;
    this.refreshTimer = setTimeout(() => {
      this.refreshTimer = null;
      const { day, layout, markActor } = this.pendingRefresh;
      this.pendingRefresh = { day: false, layout: false, markActor: '' };
      if (day) void this.refreshDay();
      else if (layout) void this.refreshLayout(markActor);
    }, 24);
  }

  async refreshDay(): Promise<void> {
    if (!this.client) return;
    const res = await this.client.dispatchRaw('day.state', {});
    if (res.ok) this.applyDay(res.result as DayState);
  }

  async refreshLayout(markActor = ''): Promise<void> {
    if (!this.client) return;
    const res = await this.client.dispatchRaw('layout.get', {});
    if (res.ok) this.applyLayout(res.result as Layout, markActor !== 'user' && markActor !== '');
  }

  private applyDay(day: DayState): void {
    const roleChanged = day.role !== this.roleId;
    this.day = day;
    this.roleId = day.role;
    this.applyLayout(day.layout, !roleChanged);
  }

  private applyLayout(next: Layout, markChanges = false): void {
    const prev = this.layout;
    if (markChanges && prev && prev.id === next.id) {
      const ids = changedIds(prev.widgets, next.widgets);
      if (ids.length) {
        const until = Date.now() + TOUCH_MS;
        const touched = { ...this.touched };
        for (const id of ids) touched[id] = until;
        this.touched = touched;
        setTimeout(() => this.expireTouched(), TOUCH_MS + 50);
      }
    }
    this.layout = next;
  }

  private expireTouched(): void {
    const now = Date.now();
    this.touched = Object.fromEntries(Object.entries(this.touched).filter(([, t]) => t > now));
  }

  // --------------------------------------------------------------- Consent

  private requestConsent(pending: PendingConfirmation, changed = false): Promise<ConsentOutcome> {
    return new Promise((resolve) => this.enqueueConsent({ pending, resolve, changed }));
  }

  private enqueueConsent(item: ConsentItem): void {
    const existing = this.consent.find((c) => c.pending.id === item.pending.id);
    if (existing) {
      // The event may arrive before our own dispatch returns: attach resolver.
      if (item.resolve || item.changed) {
        this.consent = this.consent.map((c) => (c.pending.id === item.pending.id ? { ...c, resolve: item.resolve ?? c.resolve, changed: c.changed || item.changed } : c));
      }
      return;
    }
    this.consent = [...this.consent, item];
    // Leave ambient so the request is never missed or rendered tiny.
    this.ambient = false;
  }

  private patchConsent(id: string, patch: Partial<ConsentItem>): void {
    this.consent = this.consent.map((c) => (c.pending.id === id ? { ...c, ...patch } : c));
  }

  /**
   * `consent.list` shortens requests past its size budget
   * (`display_truncated`); `consent.get` returns each in full when it fits.
   * Only what is still shortened after that is deny-only.
   */
  async completePending(list: PendingConfirmation[]): Promise<PendingConfirmation[]> {
    const client = this.client;
    if (!client) return list;
    return mapLimit(list, CONSENT_GET_CONCURRENCY, async (p) => {
      if (!p.display_truncated) return p;
      const res = await client.dispatchRaw('consent.get', { id: p.id });
      return res.ok ? (res.result as PendingConfirmation) : p;
    });
  }

  private async fetchConsent(id: string): Promise<void> {
    if (!this.client || this.consent.some((c) => c.pending.id === id)) return;
    const res = await this.client.dispatchRaw('consent.get', { id });
    if (!res.ok) return;
    const pending = res.result as PendingConfirmation;
    // A request of ours that was superseded elsewhere comes back under a
    // new id (same requester, command and arguments): our call follows it.
    const i = this.superseded.findIndex((w) => w.command === pending.command && w.actor === pending.actor && !!w.args_hash && w.args_hash === pending.args_hash);
    if (i >= 0) {
      const [w] = this.superseded.splice(i, 1);
      clearTimeout(w!.timer);
      this.enqueueConsent({ pending, resolve: w!.resolve, changed: true });
      return;
    }
    this.enqueueConsent({ pending });
  }

  /** Waiting calls whose request was superseded outside this window, until the re-ask arrives. */
  private superseded: { command: string; actor: string; args_hash?: string; resolve: (o: ConsentOutcome) => void; timer: ReturnType<typeof setTimeout> }[] = [];

  /** Remove a request the core has settled (approved, denied, expired or superseded). */
  private dropConsent(id: string, how: string): void {
    const item = this.consent.find((c) => c.pending.id === id);
    // Our own approval in flight: decideConsent settles it from the reply.
    if (!item || item.busy) return;
    this.consent = this.consent.filter((c) => c.pending.id !== id);
    // Decided somewhere else (the socket's deny-only channel, an expiry, or
    // another approver): settle our own waiting call instead of leaving it hanging.
    if (how === EV.consentExpired) {
      item.resolve?.({ kind: 'denied', message: 'The request expired before anyone answered it' });
      toasts.show(`A request to run ${item.pending.command} expired and was denied.`);
    } else if (how === EV.consentDeny) {
      item.resolve?.({ kind: 'denied', message: 'Denied outside this window' });
    } else if (how === EV.consentSuperseded) {
      const resolve = item.resolve;
      if (resolve) {
        const { command, actor, args_hash } = item.pending;
        const w = {
          command,
          actor,
          args_hash,
          resolve,
          // The re-ask follows at once; if it never comes, fail the call.
          timer: setTimeout(() => {
            this.superseded = this.superseded.filter((x) => x !== w);
            resolve({ kind: 'error', code: 'conflict', message: 'The request changed and was asked again' });
          }, SUPERSEDED_WAIT_MS),
        };
        this.superseded.push(w);
      }
    } else {
      item.resolve?.({ kind: 'approved', result: undefined });
    }
  }

  /** Re-read the queue from the core. */
  async refreshConsent(): Promise<void> {
    if (!this.client) return;
    const res = await this.client.dispatchRaw('consent.list', {});
    if (!res.ok) return;
    const pending = await this.completePending((res.result as { pending: PendingConfirmation[] }).pending);
    const prev = new Map(this.consent.map((c) => [c.pending.id, c]));
    // Our own waiting calls keep their resolvers; requests gone from the core
    // that we were waiting on are settled as denied.
    for (const c of this.consent) if (c.resolve && !pending.some((p) => p.id === c.pending.id)) c.resolve({ kind: 'denied', message: 'The request is no longer pending' });
    this.consent = pending.map((p) => {
      const old = prev.get(p.id);
      return { pending: p, resolve: old?.resolve, changed: old?.changed };
    });
  }

  async decideConsent(id: string, approved: boolean): Promise<void> {
    const item = this.consent.find((c) => c.pending.id === id);
    if (!item || item.busy || !this.client) return;
    // A request too large to show in full can only be denied.
    if (approved && item.pending.display_truncated) return;
    if (!approved) {
      this.consent = this.consent.filter((c) => c.pending.id !== id);
      const res = await this.client.dispatchRaw('consent.deny', { id });
      item.resolve?.({ kind: 'denied', message: 'Declined by you' });
      if (!res.ok && res.error.code !== 'not_found') toasts.error(res.error.message);
      return;
    }
    // Keep it on screen while the approval runs, so a re-ask can replace it in place.
    this.patchConsent(id, { busy: true });
    const res = await this.client.dispatchRaw('consent.approve', { id });
    if (res.ok) {
      this.consent = this.consent.filter((c) => c.pending.id !== id);
      item.resolve?.({ kind: 'approved', result: res.result });
      return;
    }
    const e = res.error;
    if (e.code === 'conflict' && e.confirmation_id && e.confirmation_id !== id) {
      // v1.4: the scope changed since it was shown. Nothing ran; the same
      // request is pending again under a new id. Show that one, here.
      const next = await this.client.dispatchRaw('consent.get', { id: e.confirmation_id });
      if (next.ok) {
        const fresh: ConsentItem = { pending: next.result as PendingConfirmation, resolve: item.resolve, changed: true };
        const rest = this.consent.filter((c) => c.pending.id !== id && c.pending.id !== fresh.pending.id);
        const at = Math.max(0, this.consent.findIndex((c) => c.pending.id === id));
        rest.splice(Math.min(at, rest.length), 0, fresh);
        this.consent = rest;
        return;
      }
    }
    this.consent = this.consent.filter((c) => c.pending.id !== id);
    item.resolve?.({ kind: 'error', code: e.code, message: e.message });
    if (e.code === 'conflict') {
      toasts.error('Not approved: the request changed since you saw it. Check it again.');
      await this.refreshConsent();
    } else if (!item.resolve) toasts.error(e.message);
  }

  // --------------------------------------------------------------- Actions

  /** Ask the user a yes/no question in the app's own dialog. */
  confirm(title: string, message: string, confirmLabel = 'Continue'): Promise<boolean> {
    this.confirmRequest?.resolve(false);
    return new Promise((resolve) => {
      this.confirmRequest = {
        title,
        message,
        confirmLabel,
        resolve: (yes) => {
          this.confirmRequest = null;
          resolve(yes);
        },
      };
    });
  }

  /**
   * Complete (or reopen) an item. A bullet or a task with subtasks is a group:
   * the core refuses it without cascade=true, so ask first and then cascade.
   * Returns false when nothing was done.
   */
  async complete(item: Item, done = true): Promise<boolean> {
    let group = item.kind === 'bullet';
    if (!group) {
      const kids = await this.run('tasks.list', { parent_id: item.id, limit: 1 });
      if (!kids) return false;
      group = kids.items.length > 0;
    }
    if (group) {
      const verb = done ? 'Complete' : 'Reopen';
      const ok = await this.confirm(`${verb} the whole group?`, `"${item.title}" has nested items. This will ${done ? 'complete' : 'reopen'} it and every task under it.`, `${verb} all`);
      if (!ok) return false;
    }
    return (await this.run('tasks.complete', { id: item.id, done, ...(group ? { cascade: true } : {}) })) !== undefined;
  }

  async switchRole(roleId: string): Promise<void> {
    if (roleId === this.roleId) return;
    const res = await this.run('role.switch', { id: roleId });
    if (res) {
      this.roleId = res.role.id;
      this.layout = res.layout;
      void this.refreshDay();
    }
  }

  async moveWidget(id: string, x: number, y: number): Promise<void> {
    const l = this.layout;
    if (!l) return;
    this.layout = { ...l, widgets: moveBox(l.widgets, id, x, y, l.columns) };
    const res = await this.run('layout.move_widget', { widget_id: id, x, y });
    if (res) this.layout = res;
    else void this.refreshLayout();
  }

  async resizeWidget(id: string, w: number, h: number): Promise<void> {
    const l = this.layout;
    if (!l) return;
    this.layout = { ...l, widgets: resizeBox(l.widgets, id, w, h, l.columns) };
    const res = await this.run('layout.update_widget', { widget_id: id, patch: { w, h } });
    if (res) this.layout = res;
    else void this.refreshLayout();
  }

  async removeWidget(id: string): Promise<void> {
    const res = await this.run('layout.remove_widget', { widget_id: id });
    if (res) this.layout = res;
  }

  async undo(eventId?: string): Promise<void> {
    const res = await this.run('history.undo', eventId ? { event_id: eventId } : {});
    if (res?.event) toasts.show(`Undid: ${res.event.summary}`, { tone: 'success', action: { label: 'Redo', run: () => this.redo() } });
    else if (res) toasts.show('Nothing to undo');
  }

  async redo(): Promise<void> {
    const res = await this.run('history.redo', {});
    if (res?.event) toasts.show(`Redid: ${res.event.summary}`);
  }

  setTheme(t: ThemePref): void {
    this.theme = t;
    savePrefs({ theme: this.theme, privacy: this.privacy });
  }

  cycleTheme(): void {
    this.setTheme(this.resolvedTheme === 'dark' ? 'light' : 'dark');
  }

  togglePrivacy(): void {
    this.privacy = !this.privacy;
    savePrefs({ theme: this.theme, privacy: this.privacy });
  }

  toggleAmbient(): void {
    this.ambient = !this.ambient;
    if (this.ambient) {
      this.editing = false;
      this.paletteOpen = false;
    }
  }

  toggleEditing(): void {
    this.editing = !this.editing;
    if (this.editing) this.ambient = false;
  }
}

export const hub = new Hub();
