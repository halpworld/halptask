import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { actorDisplayName, consentView, formatArgs, taintWarning, TRUNCATED_NOTICE } from '../src/lib/shell/consent';
import type { PendingConfirmation } from '../src/lib/api/types';

const base: PendingConfirmation = {
  id: 'a'.repeat(64),
  command: 'tasks.delete',
  tier: 'T3',
  actor: 'agent:claude',
  args: { id: 'dev-1' },
  summary: 'Delete "Q3 plan" and 4 nested items',
  created_at: '2026-11-10T09:00:00.000Z',
};

describe('consent dialog view', () => {
  it('shows the actor, tier, summary, targets and effects', () => {
    const v = consentView(
      { ...base, details: { targets: [{ kind: 'item', id: 'dev-1', title: 'Q3 plan', descendants: 4 }], effects: ['Removes 5 items'] } },
      { name: 'tasks.delete', description: 'Delete an item and everything under it.', tier: 'T3', input_schema: {}, output_schema: {} },
    );
    expect(v).toMatchObject({ actorName: 'Claude', actorId: 'agent:claude', actorKind: 'agent', tier: 'T3', summary: base.summary, description: 'Delete an item and everything under it.' });
    expect(v.ask).toBe('wants to run a high-risk command');
    expect(v.targets).toHaveLength(1);
    expect(v.effects).toEqual(['Removes 5 items']);
    expect(v.taintWarning).toBeNull();
  });

  it('names the user as the requester for their own T3 call', () => {
    const v = consentView({ ...base, actor: 'user' });
    expect(v.actorKind).toBe('user');
    expect(v.ask).toMatch(/^You asked/);
  });

  it('never truncates arguments and keeps markup as text', () => {
    const long = 'x'.repeat(5000);
    const evil = '<img src=x onerror=alert(1)>';
    const text = consentView({ ...base, args: { id: long, title: evil, nested: { a: [1, 2] } } }).argsText;
    expect(text).toContain(long);
    expect(text).toContain(evil); // escaped by Svelte at render, not altered here
    expect(JSON.parse(text)).toEqual({ id: long, title: evil, nested: { a: [1, 2] } });
  });

  it('warns when the agent read external content', () => {
    expect(taintWarning({ tainted: true, taint_source: 'https://evil.example/x' })).toBe(
      'This request was made after the agent read external content from https://evil.example/x. Check it carefully.',
    );
    expect(taintWarning({ tainted: true, taint_source: null })).toContain('an unknown source');
    expect(taintWarning({ tainted: false, taint_source: 'x' })).toBeNull();
  });

  it('counts down to expiry', () => {
    const v = consentView({ ...base, expires_at: '2026-11-10T09:05:00.000Z' }, undefined, Date.parse('2026-11-10T09:03:30.000Z'));
    expect(v.expiresInSec).toBe(90);
    expect(consentView(base).expiresInSec).toBeNull();
  });

  it('formats odd values safely', () => {
    expect(formatArgs(undefined)).toBe('{}');
    const cyclic: Record<string, unknown> = {};
    cyclic.self = cyclic;
    expect(typeof formatArgs(cyclic)).toBe('string');
    expect(actorDisplayName('mcp:cursor')).toBe('Cursor (mcp)');
    expect(actorDisplayName('system')).toBe('System');
  });
});

// Review S2: JSON.stringify leaves bidi overrides, zero-width and C1
// characters raw, so an agent's args could reorder what the user reads.
describe('consent dialog: hidden characters in the args', () => {
  it('shows every hidden character as an escape and keeps the JSON exact', () => {
    const hidden = ['\u202E', '\u2066', '\u200B', '\u200D', '\u0085', '\u009B', '\u2028', '\uFEFF', '\u{E0041}', '\u{E0100}', '\u0007'];
    const args = { title: `pay ${hidden.join('')} bob`, ['k\u202Eey']: 1 };
    const text = consentView({ ...base, args }).argsText;
    for (const c of hidden) expect(text.includes(c), `raw ${JSON.stringify(c)}`).toBe(false);
    expect(text).toContain('\\u202E');
    expect(text).toContain('\\uDB40\\uDD00');
    expect(text).toContain('\\u0085');
    // Still valid JSON meaning exactly the same args.
    expect(JSON.parse(text)).toEqual(args);
    // Ordinary text, newlines between keys and non-Latin scripts stay as they are.
    const plain = formatArgs({ a: 'héllo 世界 مرحبا', b: 'line\nnext' });
    expect(plain).toBe(JSON.stringify({ a: 'héllo 世界 مرحبا', b: 'line\nnext' }, null, 2));
  });
});

describe('consent dialog: requests too large to show', () => {
  it('claims "exactly as they will run" only when nothing was cut', () => {
    const full = consentView({ ...base, display_truncated: false });
    expect(full).toMatchObject({ truncated: false, canApprove: true, argsLabel: 'Arguments, exactly as they will run' });
    expect(consentView(base).canApprove).toBe(true);
    const cut = consentView({ ...base, display_truncated: true });
    expect(cut).toMatchObject({ truncated: true, canApprove: false });
    expect(cut.argsLabel).not.toMatch(/exactly/);
    expect(TRUNCATED_NOTICE).toMatch(/too large to display in full/);
  });
});

describe('consent dialog: privacy mode', () => {
  const dialog = readFileSync(new URL('../src/lib/shell/ConsentDialog.svelte', import.meta.url), 'utf8');
  const css = readFileSync(new URL('../src/lib/design/base.css', import.meta.url), 'utf8');

  it('marks nothing in the dialog for blurring', () => {
    expect(dialog).not.toMatch(/\bpii\b(?!-)/);
    expect(dialog).toMatch(/<Dialog [^\n]*class="consent"/);
  });

  it('exempts the whole dialog, nested parts included, from the blur', () => {
    const rule = /:root\.privacy dialog\.consent \.pii,\s*:root\.privacy dialog\.consent \.pii-strong \{\s*filter: none;/;
    expect(css).toMatch(rule);
    // The exemption comes after the blur rules, so it wins at equal or higher specificity.
    expect(css.indexOf(':root.privacy dialog.consent')).toBeGreaterThan(css.indexOf(':root.privacy .pii-strong'));
  });

  it('disables Approve when the core could not show everything', () => {
    expect(dialog).toMatch(/disabled=\{busy \|\| !armed \|\| !v\?\.canApprove\}/);
    expect(dialog).toMatch(/approved && \(!armed \|\| !v\?\.canApprove\)/);
  });
});

describe('round 2: private items, missing args, pinned args', () => {
  it('shows a neutral private-item line and the full args the user is given', () => {
    const v = consentView({ ...base, summary: 'Asked: tasks.update on a private item', args: { id: 'n1', patch: { title: 'x' } } });
    expect(v.privateNote).toMatch(/private item/);
    expect(v.canApprove).toBe(true);
    expect(consentView({ ...base, private: true }).privateNote).not.toBeNull();
    expect(consentView(base).privateNote).toBeNull();
  });

  it('does not crash on a payload without args, and offers deny only', () => {
    const { args: _, ...slim } = base;
    const v = consentView({ ...slim, private: true } as PendingConfirmation);
    expect(v.argsText).toBe('(not shown)');
    expect(v.canApprove).toBe(false);
    expect(v.truncated).toBe(true);
  });

  it('shows pinned args (Inbox parent, resolved dues) as they are', () => {
    const v = consentView({ ...base, command: 'capture.quick', args: { text: 'Pay rent due:fri', parent_id: 'inbox-1', due: '2026-11-13' } });
    expect(v.argsText).toContain('"parent_id": "inbox-1"');
    expect(v.argsText).toContain('"due": "2026-11-13"');
  });
});

describe('KeyGate', () => {
  const gate = readFileSync(new URL('../src/lib/shell/KeyGate.svelte', import.meta.url), 'utf8');
  it('treats a second ready as a status update, never a new unlock', () => {
    expect(gate).toMatch(/if \(readyFired && s\.state !== 'ready'\) return;/);
    expect(gate).toMatch(/s\.state === 'ready' && !readyFired/);
  });
});
