// The core rejects bidi and other invisible format controls in titles,
// tags, notes, labels and widget titles with `invalid_args` (CONTRACT core
// notes, round 2 §5). ZWJ (U+200D) and ZWNJ (U+200C) stay allowed. This is
// a local early warning only: the core's answer is the truth, and its
// message is shown inline where the text was typed.

// Also the line and paragraph separators (U+2028, U+2029) and the tag and
// variation-selector-supplement planes (U+E0000-E007F, U+E0100-E01EF).
const FORMAT_CHARS = /[\u00AD\u061C\u180E\u200B\u200E\u200F\u2028-\u202E\u2060-\u2064\u2066-\u206F\uFEFF\uFFF9-\uFFFB]|\uDB40[\uDC00-\uDC7F\uDD00-\uDDEF]/;

/**
 * Every character that does not show as itself: C0 and C1 controls, the
 * format characters above, joiners and variation selectors. Global, for
 * escaping text that must be shown exactly (the consent dialog's args).
 */
export const HIDDEN_CHARS = /[\u0000-\u0008\u000B-\u001F\u007F-\u009F\u00AD\u061C\u180E\u200B-\u200F\u2028-\u202E\u2060-\u2064\u2066-\u206F\uFE00-\uFE0F\uFEFF\uFFF9-\uFFFB]|\uDB40[\uDC00-\uDC7F\uDD00-\uDDEF]/g;

/** `text` with every hidden character written as a `\uXXXX` escape. */
export function escapeHidden(text: string): string {
  return text.replace(HIDDEN_CHARS, (m) =>
    Array.from({ length: m.length }, (_, i) => '\\u' + m.charCodeAt(i).toString(16).toUpperCase().padStart(4, '0')).join(''),
  );
}

const hex = (cp: number) => cp.toString(16).toUpperCase().padStart(4, '0');

/**
 * The first misused BMP variation selector (U+FE00-FE0F), mirroring the
 * core's `misused_variation_selector` for single-line text: U+FE00-FE0D are
 * refused outright; U+FE0E / U+FE0F are allowed once, directly after a
 * visible character (an emoji or symbol), never at the start, after white
 * space or after another selector.
 */
export function misusedVariationSelector(text: string): number | null {
  let prev: string | null = null;
  for (const ch of text) {
    const cp = ch.codePointAt(0)!;
    if (cp >= 0xfe00 && cp <= 0xfe0d) return cp;
    if (cp === 0xfe0e || cp === 0xfe0f) {
      const p = prev === null ? null : prev.codePointAt(0)!;
      if (p === null || (p >= 0xfe00 && p <= 0xfe0f) || /\s/u.test(prev!) || p < 0x20 || (p >= 0x7f && p <= 0x9f)) return cp;
    }
    prev = ch;
  }
  return null;
}

/** A short problem text when `text` holds an invisible format character, else null. */
export function formatCharProblem(text: string): string | null {
  const m = FORMAT_CHARS.exec(text);
  if (m) {
    const cp = m[0].codePointAt(0)!;
    return `Remove the invisible character U+${hex(cp)} (bidi or zero-width controls are not allowed).`;
  }
  const vs = misusedVariationSelector(text);
  if (vs !== null) {
    return `Remove the variation selector U+${hex(vs)} (one U+FE0E or U+FE0F directly after a symbol is allowed).`;
  }
  return null;
}
