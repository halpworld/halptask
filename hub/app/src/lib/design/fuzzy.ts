/**
 * Small subsequence matcher for the command palette. Returns a score
 * (higher is better) or -1 when `query` is not a subsequence of `text`.
 * Rewards matches at word starts and consecutive runs.
 */
export function fuzzyScore(query: string, text: string): number {
  const q = query.trim().toLowerCase();
  if (!q) return 0;
  const t = text.toLowerCase();
  if (t.includes(q)) return 1000 - t.indexOf(q) * 2 - (t.length - q.length) * 0.1;

  let score = 0;
  let ti = 0;
  let run = 0;
  for (const ch of q) {
    if (ch === ' ') continue;
    const found = t.indexOf(ch, ti);
    if (found < 0) return -1;
    const prev = found > 0 ? t[found - 1] : ' ';
    const wordStart = prev === ' ' || prev === '.' || prev === '-' || prev === '_' || prev === '/';
    run = found === ti ? run + 1 : 0;
    score += 10 + (wordStart ? 12 : 0) + run * 6 - Math.min(found - ti, 10);
    ti = found + 1;
  }
  return score;
}

export function fuzzyFilter<T>(items: readonly T[], query: string, text: (item: T) => string): T[] {
  if (!query.trim()) return [...items];
  return items
    .map((item) => ({ item, s: fuzzyScore(query, text(item)) }))
    .filter((x) => x.s >= 0)
    .sort((a, b) => b.s - a.s)
    .map((x) => x.item);
}
