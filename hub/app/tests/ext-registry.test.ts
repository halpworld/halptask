import { describe, expect, it } from 'vitest';
import { EXT_ENTRY, FALLBACK, GENERIC_KINDS, isKnownKind, resolveWidget } from '../src/lib/widgets/registry';

describe('Tier C kinds in the registry', () => {
  it('resolves x.<pack_id>.<name> to the sandboxed frame entry', () => {
    for (const k of ['x.acme.sre.oncall', 'x.dev.acme.sre.oncall', 'x.a.b.w']) {
      expect(resolveWidget(k), k).toBe(EXT_ENTRY);
      expect(isKnownKind(k)).toBe(true);
    }
  });

  it('gives malformed ext kinds the fallback', () => {
    for (const k of ['x.acme.w', 'x.Acme.sre.w', 'x.acme.sre.', 'x.acme.sre.w/x', 'x.', 'xacme.sre.w', 'constructor', '__proto__', 'toString']) {
      expect(resolveWidget(k), k).toBe(FALLBACK);
      expect(isKnownKind(k), k).toBe(false);
    }
    expect(resolveWidget(undefined)).toBe(FALLBACK);
    expect(resolveWidget({ kind: 'x.acme.sre.w' })).toBe(FALLBACK);
  });

  it('leaves built-in kinds as they were', () => {
    for (const k of GENERIC_KINDS) {
      const e = resolveWidget(k);
      expect(e.kind).toBe(k);
      expect(e).not.toBe(EXT_ENTRY);
    }
  });
});
