import { describe, expect, it } from 'vitest';
import { DEV_KINDS, EM_KINDS, FALLBACK, GENERIC_KINDS, CSO_KINDS, registry, resolveWidget } from '../src/lib/widgets/registry';
import { seedLayouts } from '../src/lib/api/mock/seed';

describe('widget registry', () => {
  it('has every generic kind, none marked placeholder', () => {
    for (const k of GENERIC_KINDS) {
      const e = resolveWidget(k);
      expect(e.kind).toBe(k);
      expect(e.placeholder).toBeUndefined();
    }
  });

  it('has no placeholder kinds left', () => {
    for (const e of registry.values()) expect(e.placeholder, e.kind).toBeUndefined();
  });

  it('registers the developer, EM and CSO kinds as live widgets with a config schema', () => {
    for (const k of [...DEV_KINDS, ...EM_KINDS, ...CSO_KINDS]) {
      const e = resolveWidget(k);
      expect(e.kind).toBe(k);
      expect(e.placeholder).toBeUndefined();
      expect(e.config).toMatchObject({ type: 'object', additionalProperties: false });
    }
  });

  it('falls back safely for unknown or hostile kinds', () => {
    for (const k of ['nope', '__proto__', 'constructor', 'toString', '', '<img src=x onerror=alert(1)>', 42, null, undefined]) {
      expect(resolveWidget(k)).toBe(FALLBACK);
    }
  });

  it('hides interactive-only widgets in ambient mode', () => {
    expect(resolveWidget('quick-capture').ambient).toBe('hide');
    expect(resolveWidget('countdown').ambient).toBe('show');
  });

  it('can render every kind used by the seed layouts', () => {
    for (const l of seedLayouts('2026-11-10')) {
      for (const w of l.widgets) expect(registry.has(w.kind), `${l.id}/${w.kind}`).toBe(true);
    }
  });

  it('minimum sizes are sane', () => {
    for (const e of registry.values()) {
      expect(e.minW).toBeGreaterThanOrEqual(2);
      expect(e.minH).toBeGreaterThanOrEqual(2);
    }
  });
});
