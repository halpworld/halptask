// Packs as plain data, so it can be tested without a DOM: the command names,
// preview currency, fingerprint formatting, trust display and the permission diff.
// Every string from a pack is untrusted; `safe` escapes hidden characters.

import { escapeHidden } from '../util/format-chars';
import type { PackCapabilities, PackPreview, PackTrust, PermissionChange, PermissionDiff, PermissionKind } from '../api/types';

/**
 * The packs.* command names, in one place. hub/docs/fixtures/commands.json
 * lists them all now (P5-B), so tests/contract.test.ts can check them like
 * any other command name.
 */
export const PACKS_CMD = {
  list: 'packs.list',
  preview: 'packs.preview',
  install: 'packs.install',
  uninstall: 'packs.uninstall',
  enable: 'packs.enable',
  disable: 'packs.disable',
  rollback: 'packs.rollback',
  resetLayout: 'packs.reset_layout',
  devSet: 'packs.dev_set',
  keysList: 'packs.keys_list',
  keysAdd: 'packs.keys_add',
  keysRevoke: 'packs.keys_revoke',
} as const;

/** The bundle limit (§3.2). Larger files are not read at all. */
export const BUNDLE_MAX_BYTES = 8 * 1024 * 1024;

/** An untrusted pack string, shown as text with hidden characters escaped. */
export const safe = (s: string | null | undefined): string => escapeHidden(s ?? '');

// ----------------------------------------------------------- Fingerprints

const HEX64 = /^[0-9a-f]{64}$/i;

/** The fingerprint as 4 groups of 16 hex characters, or null when it is not 64 hex. */
export function fingerprintGroups(fp: string | null | undefined): [string, string, string, string] | null {
  if (!fp || !HEX64.test(fp)) return null;
  const f = fp.toLowerCase();
  return [f.slice(0, 16), f.slice(16, 32), f.slice(32, 48), f.slice(48, 64)];
}

/** `fingerprintGroups` joined with spaces (for copying and for tests). */
export function formatFingerprint(fp: string | null | undefined): string | null {
  return fingerprintGroups(fp)?.join(' ') ?? null;
}

/** key_id: the first 16 hex characters of the fingerprint (§4.1). */
export const keyIdOf = (fp: string): string => fp.slice(0, 16).toLowerCase();

/** A public key as pasted: standard base64 of exactly 32 bytes. */
export function checkPubkey(input: string): string | null {
  const t = input.trim();
  if (!t) return 'Paste the public key.';
  if (!/^[A-Za-z0-9+/]+={0,2}$/.test(t)) return 'That is not base64.';
  let bytes: number;
  try {
    bytes = atob(t).length;
  } catch {
    return 'That is not valid base64.';
  }
  return bytes === 32 ? null : `An Ed25519 public key is 32 bytes; this one is ${bytes}.`;
}

// ------------------------------------------------------------------ Trust

export type TrustTone = 'success' | 'info' | 'accent' | 'warning' | 'danger' | 'neutral';

export interface TrustInfo {
  label: string;
  tone: TrustTone;
  /** Unsigned and dev get a visible warning style. */
  warn: boolean;
  /** One sentence on what the level means. */
  blurb: string;
}

const TRUST: Record<PackTrust, TrustInfo> = {
  builtin: { label: 'Built in', tone: 'neutral', warn: false, blurb: 'Ships with HalpTask.' },
  first_party: { label: 'HalpTask', tone: 'success', warn: false, blurb: 'Signed by a HalpTask key compiled into this app.' },
  community: { label: 'Community', tone: 'info', warn: false, blurb: 'Signed by a publisher key you trust.' },
  unsigned: {
    label: 'Unsigned',
    tone: 'warning',
    warn: true,
    blurb: 'Not signed by a key you trust. It cannot use privileged annotations, modules or custom UI, and nobody vouches for it.',
  },
  dev: { label: 'DEV', tone: 'warning', warn: true, blurb: 'Loaded from your dev folder. Not signed, never stored, and not offered to agents as prompts.' },
};

/**
 * How to show a trust level. A bad signature is its own danger state, never
 * unsigned; an unknown trust value (a newer core) is shown as unverified
 * rather than guessed.
 */
export function trustInfo(trust: PackTrust | string, signature?: PackPreview['signature']): TrustInfo {
  if (signature === 'invalid') {
    return { label: 'Bad signature', tone: 'danger', warn: true, blurb: 'The signature does not match the contents. Do not install this.' };
  }
  return TRUST[trust as PackTrust] ?? { label: 'Unverified', tone: 'warning', warn: true, blurb: 'The trust level is not one this app knows.' };
}

// --------------------------------------------------------------- Capabilities

/** "2 modules, 1 widget, 3 prompts" or "no extra capabilities". */
export function capabilitySummary(c: PackCapabilities): string {
  const parts: string[] = [];
  const add = (n: number, one: string, many: string) => n > 0 && parts.push(`${n} ${n === 1 ? one : many}`);
  add(c.modules, 'module', 'modules');
  add(c.ui_widgets, 'custom widget', 'custom widgets');
  add(c.prompts, 'prompt', 'prompts');
  add(c.annotations, 'privileged annotation', 'privileged annotations');
  return parts.length ? parts.join(', ') : 'No extra capabilities';
}

// ---------------------------------------------------------------- Preview

export interface PreviewState {
  preview: PackPreview | null;
  /** The preview_id the user is looking at (what Install would send). */
  shownId: string | null;
  /** SHA-256 (hex) of the bytes this window read from the file, when known. */
  localSha: string | null;
  now: number;
}

/** Why Install is not available yet, or null when the preview is current. */
export function previewProblem(s: PreviewState): string | null {
  const p = s.preview;
  if (!p) return 'Choose a pack file first.';
  if (s.shownId !== p.preview_id) return 'This review is out of date. Choose the file again.';
  const exp = Date.parse(p.expires_at);
  if (!Number.isFinite(exp) || exp <= s.now) return 'This review expired. Choose the file again to get a fresh one.';
  if (s.localSha && s.localSha.toLowerCase() !== p.bundle_sha256.toLowerCase()) return 'The core reviewed different bytes than the file you chose. Choose it again.';
  return null;
}

/** True when the preview is current: not expired, matching preview_id (and bytes). */
export const previewCurrent = (s: PreviewState): boolean => previewProblem(s) === null;

/** Whole seconds left on a preview, never negative; 0 when unparseable. */
export function secondsLeft(p: Pick<PackPreview, 'expires_at'>, now: number): number {
  const exp = Date.parse(p.expires_at);
  return Number.isFinite(exp) ? Math.max(0, Math.floor((exp - now) / 1000)) : 0;
}

/** "9:41" for 581 seconds. */
export const clockText = (secs: number): string => `${Math.floor(secs / 60)}:${String(secs % 60).padStart(2, '0')}`;

export interface InstallGate {
  /** Install may be pressed. */
  ok: boolean;
  /** The reason it may not, for the screen reader and the label. */
  reason: string | null;
  /** The "trust this publisher key" checkbox applies (valid signature by an unknown key). */
  offersTrustKey: boolean;
}

/** Everything that gates the Install button, in one place. */
export function installGate(s: PreviewState & { confirmed: boolean }): InstallGate {
  const p = s.preview;
  const offersTrustKey = !!p && p.signature === 'unknown_key' && !!fingerprintGroups(p.key_fp);
  const stale = previewProblem(s);
  if (stale) return { ok: false, reason: stale, offersTrustKey };
  if (!p) return { ok: false, reason: 'Choose a pack file first.', offersTrustKey };
  if (p.signature === 'invalid') return { ok: false, reason: 'The signature is bad, so this pack cannot be installed.', offersTrustKey };
  if (blockingWarnings(p).length) return { ok: false, reason: 'The core says this pack cannot be installed as it is.', offersTrustKey };
  if (p.migrations.items_failed > 0) return { ok: false, reason: `${p.migrations.items_failed} items would fail to migrate, so the core will refuse this update.`, offersTrustKey };
  if (!s.confirmed) return { ok: false, reason: 'Tick the box to confirm you have read this review.', offersTrustKey };
  return { ok: true, reason: null, offersTrustKey };
}

/** `trust_key` for packs.install: only when offered and ticked. */
export const trustKeyArg = (offered: boolean, ticked: boolean): boolean | undefined => (offered && ticked ? true : undefined);

// -------------------------------------------------------- Permission diff

export const KIND_LABEL: Record<PermissionKind, string> = {
  module: 'Module',
  ui_widget_call: 'Widget call',
  annotation: 'Privileged annotation',
  data_class: 'Data class',
  prompt: 'Prompt',
};

const KIND_ORDER: PermissionKind[] = ['module', 'ui_widget_call', 'annotation', 'data_class', 'prompt'];

export interface DiffRow {
  sign: '+' | '-';
  kind: PermissionKind;
  label: string;
  /** Escaped text, safe to render as is. */
  text: string;
}

const row = (sign: '+' | '-') => (c: PermissionChange): DiffRow => ({
  sign,
  kind: c.kind,
  label: KIND_LABEL[c.kind] ?? c.kind,
  text: c.detail ? `${safe(c.id)} (${safe(c.detail)})` : safe(c.id),
});

const byKind = (a: DiffRow, b: DiffRow) => KIND_ORDER.indexOf(a.kind) - KIND_ORDER.indexOf(b.kind) || a.text.localeCompare(b.text);

/** Added rows first, then removed, each grouped by kind and sorted. */
export function diffRows(diff: PermissionDiff | null | undefined): DiffRow[] {
  if (!diff) return [];
  return [...diff.added.map(row('+')).sort(byKind), ...diff.removed.map(row('-')).sort(byKind)];
}

/** An update that adds any capability needs consent again (§3.4). */
export const diffAddsCapability = (diff: PermissionDiff | null | undefined): boolean => !!diff && diff.added.length > 0;

/** "2 added, 1 removed" or "No change in permissions". */
export function diffSummary(diff: PermissionDiff | null | undefined): string {
  if (!diff) return 'First install: everything below is new.';
  const a = diff.added.length;
  const r = diff.removed.length;
  if (!a && !r) return 'No change in permissions.';
  return [a ? `${a} added` : '', r ? `${r} removed` : ''].filter(Boolean).join(', ');
}

/** "3 of 40 items will be migrated" style line for the review. */
export function migrationSummary(m: PackPreview['migrations']): string {
  if (m.items_checked === 0) return 'No existing items are affected.';
  const base = `${m.items_migrated} of ${m.items_checked} items will be migrated`;
  return m.items_failed ? `${base}; ${m.items_failed} would fail` : `${base}; none would fail`;
}

/** Human size: 1.2 KiB. */
export function sizeText(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KiB`;
  return `${(n / 1024 / 1024).toFixed(1)} MiB`;
}

/** Hex SHA-256 of some bytes through WebCrypto, or null where it is missing. */
export async function sha256Hex(bytes: Uint8Array): Promise<string | null> {
  const subtle = globalThis.crypto?.subtle;
  if (!subtle) return null;
  const d = new Uint8Array(await subtle.digest('SHA-256', bytes as BufferSource));
  return Array.from(d, (b) => b.toString(16).padStart(2, '0')).join('');
}

/** A dev folder path: non-empty, no control characters, at most 1024 chars. */
export function checkDevPath(input: string): string | null {
  const t = input.trim();
  if (!t) return 'Enter the folder path.';
  if (t.length > 1024) return 'That path is too long.';
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001F\u007F]/.test(t)) return 'The path has a control character.';
  return null;
}

// ------------------------------------------------- Overrides and escalations

const BLOCKING_PREFIX = 'It cannot be installed as it is:';

/** Preview warnings the core marks as blocking: shown as errors, Install stays off. */
export const blockingWarnings = (p: Pick<PackPreview, 'warnings'>): string[] => p.warnings.filter((w) => w.startsWith(BLOCKING_PREFIX));

/** The non-blocking warnings. */
export const plainWarnings = (p: Pick<PackPreview, 'warnings'>): string[] => p.warnings.filter((w) => !w.startsWith(BLOCKING_PREFIX));

/** A data class change on a type that stays: items become visible under another class. */
export interface ClassEscalation {
  typeId: string;
  from: string;
  to: string;
  /** Escaped sentence, safe to render. */
  text: string;
}

export function classEscalations(diff: PermissionDiff | null | undefined): ClassEscalation[] {
  if (!diff) return [];
  return diff.added
    .filter((c) => c.kind === 'data_class' && !!c.detail && c.detail.includes('->'))
    .map((c) => {
      const [from, to] = (c.detail as string).split('->').map((x) => x.trim());
      return {
        typeId: c.id,
        from: safe(from),
        to: safe(to),
        text: `Items of type ${safe(c.id)} would become visible under a weaker data class (${safe(from)} \u2192 ${safe(to)}).`,
      };
    });
}

export interface OverrideView {
  typeId: string;
  title: string | null;
  icon: string | null;
  addedProperties: string[];
  addedEnums: { property: string; values: string[] }[];
  /** Every description, in full, as escaped text. */
  descriptions: { property: string; text: string }[];
}

const enumText = (v: unknown): string => (typeof v === 'string' ? v : JSON.stringify(v));

/** Everything an override changes, escaped, nothing truncated. */
export function overrideViews(p: Pick<PackPreview, 'overrides'>): OverrideView[] {
  return (p.overrides ?? []).map((o) => ({
    typeId: safe(o.type_id),
    title: o.title == null ? null : safe(o.title),
    icon: o.icon == null ? null : safe(o.icon),
    addedProperties: (o.added_properties ?? []).map(safe),
    addedEnums: Object.entries(o.added_enum_values ?? {}).map(([k, v]) => ({ property: safe(k), values: v.map((x) => safe(enumText(x))) })),
    descriptions: Object.entries(o.property_descriptions ?? {}).map(([k, v]) => ({ property: safe(k), text: safe(v) })),
  }));
}

// ------------------------------------------------------------ Key revoke

export const INVALID_ITEMS_CAP = 20;

/** The notice after a key revoke left items that no longer match their type. */
export function invalidItemsNotice(ids: string[]): { shown: string[]; more: number; text: string } | null {
  if (!ids.length) return null;
  const shown = ids.slice(0, INVALID_ITEMS_CAP).map(safe);
  const more = ids.length - shown.length;
  const n = ids.length;
  return {
    shown,
    more,
    text: `${n} ${n === 1 ? 'item no longer matches' : 'items no longer match'} its type since the key's packs were disabled. ${n === 1 ? 'It keeps' : 'They keep'} its data; open and edit ${n === 1 ? 'it' : 'them'} to bring ${n === 1 ? 'it' : 'them'} back in line.`,
  };
}

// ------------------------------------------------------------- Ext widget

/** The toast for `ui/open-item`. Never the item's title: a widget could use it to read a private item. */
export const extOpenItemMessage = (publisher: string): string => `${safe(publisher)} widget asked to open an item`;
