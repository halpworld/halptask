// What the consent dialog shows, as plain data so it can be tested without a
// DOM. Everything in a PendingConfirmation except `tier` and the timestamps
// can come from an agent (args) or from the user's data (titles), so it is
// only ever rendered as text, never as HTML, and never shortened: the user
// must see exactly what they approve.

import type { CommandDescriptor, ConsentTarget, PendingConfirmation } from '../api/types';
import { escapeHidden } from '../util/format-chars';

export interface ConsentView {
  /** "You", "Claude", "System", ... */
  actorName: string;
  /** The raw actor id, e.g. `agent:claude`, shown next to the name. */
  actorId: string;
  actorKind: 'user' | 'agent' | 'other';
  /** "wants to run a high-risk command" etc. */
  ask: string;
  tier: PendingConfirmation['tier'];
  command: string;
  description: string | null;
  summary: string;
  targets: ConsentTarget[];
  effects: string[];
  /** The full arguments, pretty-printed JSON. */
  argsText: string;
  argsHash: string | null;
  /** Set when the agent read external content before asking. */
  taintWarning: string | null;
  /** Whole seconds until the core auto-denies, or null when unknown. */
  expiresInSec: number | null;
  /** The core could not show the args in full: the user can only deny. */
  truncated: boolean;
  canApprove: boolean;
  /** Heading over the args. Claims "exactly as they will run" only when true. */
  argsLabel: string;
  /** A neutral line when the request is about a private item, else null. */
  privateNote: string | null;
}

export const PRIVATE_NOTICE = 'This request is about a private item. Agents see it only as "a private item"; you see the full arguments.';
export const NO_ARGS_NOTICE = 'The arguments of this request are not available here, so it can only be denied.';

/** The core marks private requests in the payload and in the summary. */
export function isPrivateRequest(p: Pick<PendingConfirmation, 'private' | 'summary'>): boolean {
  return p.private === true || / on a private item$/.test(p.summary ?? '');
}

export const TRUNCATED_NOTICE = 'This request is too large to display in full, so it can only be denied.';

export function actorDisplayName(actor: string): string {
  if (actor === 'user') return 'You';
  if (actor === 'system') return 'System';
  const i = actor.indexOf(':');
  const kind = i < 0 ? actor : actor.slice(0, i);
  const id = i < 0 ? '' : actor.slice(i + 1);
  const name = id ? id.charAt(0).toUpperCase() + id.slice(1) : actor;
  return kind === 'agent' ? name : `${name} (${kind})`;
}

export function taintWarning(p: Pick<PendingConfirmation, 'tainted' | 'taint_source'>): string | null {
  if (!p.tainted) return null;
  const source = p.taint_source && p.taint_source.trim() ? p.taint_source : 'an unknown source';
  return `This request was made after the agent read external content from ${source}. Check it carefully.`;
}

/**
 * Stable, complete, human-readable arguments. Never truncated. Hidden
 * characters (C0/C1 controls, bidi and other format characters, joiners,
 * variation selectors) are shown as `\uXXXX` escapes, so nothing in the
 * args can reorder or hide what the user reads. Inside JSON strings the
 * escape means the same character, so the text is still exact.
 */
export function formatArgs(args: unknown): string {
  let text: string;
  try {
    text = JSON.stringify(args ?? {}, null, 2) ?? '{}';
  } catch {
    text = String(args);
  }
  return escapeHidden(text);
}

export function consentView(p: PendingConfirmation, descriptor?: CommandDescriptor, now: number = Date.now()): ConsentView {
  const actorKind = p.actor === 'user' ? 'user' : p.actor.startsWith('agent:') ? 'agent' : 'other';
  const risky = p.tier === 'T3' ? 'a high-risk command' : 'a command that needs your approval';
  const ask = actorKind === 'user' ? `You asked to run ${risky}` : `wants to run ${risky}`;
  const exp = p.expires_at ? Date.parse(p.expires_at) : NaN;
  return {
    actorName: actorDisplayName(p.actor),
    actorId: p.actor,
    actorKind,
    ask,
    tier: p.tier,
    command: p.command,
    description: descriptor?.description ?? null,
    summary: p.summary ?? '',
    targets: p.details?.targets ?? [],
    effects: p.details?.effects ?? [],
    argsText: p.args === undefined ? '(not shown)' : formatArgs(p.args),
    argsHash: p.args_hash ?? null,
    taintWarning: taintWarning(p),
    expiresInSec: Number.isFinite(exp) ? Math.max(0, Math.ceil((exp - now) / 1000)) : null,
    truncated: p.display_truncated === true || p.args === undefined,
    // Never approve what cannot be seen in full.
    canApprove: p.display_truncated !== true && p.args !== undefined,
    argsLabel: p.args === undefined ? 'Arguments not available' : p.display_truncated === true ? 'Arguments, shortened for display' : 'Arguments, exactly as they will run',
    privateNote: isPrivateRequest(p) ? PRIVATE_NOTICE : null,
  };
}
