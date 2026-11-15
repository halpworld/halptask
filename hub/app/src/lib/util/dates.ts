// Local-date helpers. Dates in the contract are plain `YYYY-MM-DD` strings.

export function toISODate(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, '0');
  const day = String(d.getDate()).padStart(2, '0');
  return `${y}-${m}-${day}`;
}

export function parseISODate(s: string): Date {
  const [y, m, d] = s.split('-').map(Number);
  return new Date(y ?? 1970, (m ?? 1) - 1, d ?? 1);
}

export function addDays(date: string, n: number): string {
  const d = parseISODate(date);
  d.setDate(d.getDate() + n);
  return toISODate(d);
}

export function diffDays(a: string, b: string): number {
  const ms = parseISODate(a).getTime() - parseISODate(b).getTime();
  return Math.round(ms / 86_400_000);
}

export function today(now: Date = new Date()): string {
  return toISODate(now);
}

/** Monday-based end of week (Sunday). */
export function endOfWeek(date: string): string {
  const d = parseISODate(date);
  const dow = (d.getDay() + 6) % 7; // 0 = Monday
  return addDays(date, 6 - dow);
}

const fmtCache = new Map<string, Intl.DateTimeFormat>();
function fmt(opts: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = JSON.stringify(opts);
  let f = fmtCache.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, opts);
    fmtCache.set(key, f);
  }
  return f;
}

export function formatLongDate(date: string): string {
  return fmt({ weekday: 'long', month: 'long', day: 'numeric' }).format(parseISODate(date));
}

export function formatTime(d: Date): string {
  return fmt({ hour: '2-digit', minute: '2-digit' }).format(d);
}

/** "today", "tomorrow", "2d overdue", "Fri", "Nov 24". */
export function relativeDue(due: string, ref: string): string {
  const n = diffDays(due, ref);
  if (n === 0) return 'today';
  if (n === 1) return 'tomorrow';
  if (n === -1) return 'yesterday';
  if (n < 0) return `${-n}d overdue`;
  if (n < 7) return fmt({ weekday: 'short' }).format(parseISODate(due));
  return fmt({ month: 'short', day: 'numeric' }).format(parseISODate(due));
}

export function relativeTime(iso: string, now: number = Date.now()): string {
  const s = Math.max(0, Math.round((now - new Date(iso).getTime()) / 1000));
  if (s < 10) return 'just now';
  if (s < 60) return `${s}s ago`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 24) return `${h}h ago`;
  return `${Math.round(h / 24)}d ago`;
}
