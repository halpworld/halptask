import { describe, expect, it } from 'vitest';
import { formatCharProblem } from '../src/lib/util/format-chars';

describe('format characters (core rejects them with invalid_args)', () => {
  it('flags bidi and zero-width controls, allows ZWJ and ZWNJ', () => {
    for (const c of ['​', '‎', '‮', '⁦', '﻿', '­', '\u{E0041}']) expect(formatCharProblem(`a${c}b`), c).toMatch(/U\+/);
    expect(formatCharProblem('a‮b')).toContain('U+202E');
    expect(formatCharProblem('family 👨‍👩‍👧')).toBeNull();
    expect(formatCharProblem('می‌خواهم')).toBeNull();
    expect(formatCharProblem('Fix login #bug due:fri')).toBeNull();
  });

  it('flags the line separators and the variation selector supplement (review A2)', () => {
    expect(formatCharProblem('a\u2028b')).toContain('U+2028');
    expect(formatCharProblem('a\u2029b')).toContain('U+2029');
    expect(formatCharProblem('a\u{E0100}b')).toContain('U+E0100');
    expect(formatCharProblem('a\u{E01EF}b')).toContain('U+E01EF');
  });

  it('allows one emoji/text selector after a symbol, flags runs and stray selectors (core rule)', () => {
    for (const ok of ['Ship it \u2764\uFE0F', 'Keycap 1\uFE0F\u20E3', 'Text \u2709\uFE0E', '\u{1F441}\uFE0F\u200D\u{1F5E8}\uFE0F']) {
      expect(formatCharProblem(ok), ok).toBeNull();
    }
    expect(formatCharProblem('a\uFE0F\uFE0F')).toContain('U+FE0F');
    expect(formatCharProblem('\u2764\uFE0E\uFE0F')).toContain('U+FE0F');
    expect(formatCharProblem('\uFE0Fstart')).toContain('U+FE0F');
    expect(formatCharProblem('a \uFE0Fb')).toContain('U+FE0F');
    expect(formatCharProblem('a\uFE00b')).toContain('U+FE00');
    expect(formatCharProblem('a\uFE0Db')).toContain('U+FE0D');
  });
});
