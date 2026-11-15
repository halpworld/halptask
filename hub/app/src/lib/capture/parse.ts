// Port of the core's capture grammar, kept exact:
//   hub/crates/halp-domain/src/dates.rs    -> parseDue
//   hub/crates/halp-domain/src/capture.rs  -> parseCaptureText
//   hub/crates/halp-core/src/commands/capture.rs (resolve) -> resolveCapture
//
// The live preview asks the core (`capture.parse`); this port exists for the
// mock transport and as an instant first paint while that call is in flight.
// When the core's grammar changes, change this file and tests/parse.test.ts
// together. Do not add grammar the core does not have.

export type CaptureStatus = 'todo' | 'in_progress' | 'done';
export type CaptureKind = 'task' | 'bullet';

/** The result of `capture.parse` (and `capture.quick`'s `parsed`). */
export interface ParsedCapture {
  title: string;
  tags: string[];
  due: string | null;
  kind: CaptureKind;
  status: CaptureStatus;
}

export class CaptureError extends Error {
  readonly code = 'invalid_args' as const;
}

const MAX_DAYS = 36_500;
const MAX_WEEKS = 5_200;
const MAX_MONTHS = 1_200;
const MAX_CAPTURE_CHARS = 4_000;
const MAX_TITLE_CHARS = 2_000;
const MAX_TAG_CHARS = 64;
const MAX_TAGS_PER_ITEM = 32;

// ---- dates.rs -------------------------------------------------------------

interface Ymd {
  y: number;
  m: number; // 1-12
  d: number;
}

const chars = (s: string) => Array.from(s).length;

function fromYmd(y: number, m: number, d: number): Ymd | null {
  if (m < 1 || m > 12 || d < 1 || d > daysIn(y, m)) return null;
  return { y, m, d };
}
function daysIn(y: number, m: number): number {
  const dt = new Date(0);
  dt.setUTCFullYear(y, m, 0);
  return dt.getUTCDate();
}
function parseIso(s: string): Ymd {
  const [y, m, d] = s.split('-').map(Number) as [number, number, number];
  return { y, m, d };
}
function iso({ y, m, d }: Ymd): string {
  return `${String(y).padStart(4, '0')}-${String(m).padStart(2, '0')}-${String(d).padStart(2, '0')}`;
}
function addDays(t: Ymd, n: number): Ymd {
  const dt = new Date(0);
  dt.setUTCFullYear(t.y, t.m - 1, t.d + n);
  return { y: dt.getUTCFullYear(), m: dt.getUTCMonth() + 1, d: dt.getUTCDate() };
}
/** chrono's checked_add_months: clamps the day to the end of the month. */
function addMonths(t: Ymd, n: number): Ymd {
  const total = t.y * 12 + (t.m - 1) + n;
  const y = Math.floor(total / 12);
  const m = (total - y * 12) + 1;
  return { y, m, d: Math.min(t.d, daysIn(y, m)) };
}
function weekdayIndex(t: Ymd): number {
  const dt = new Date(0);
  dt.setUTCFullYear(t.y, t.m - 1, t.d);
  return dt.getUTCDay(); // 0 = Sunday, like num_days_from_sunday
}

const WEEKDAYS: Record<string, number> = {
  sunday: 0, sun: 0, monday: 1, mon: 1, tuesday: 2, tue: 2, wednesday: 3, wed: 3,
  thursday: 4, thu: 4, friday: 5, fri: 5, saturday: 6, sat: 6,
};
const MONTHS = ['january', 'february', 'march', 'april', 'may', 'june', 'july', 'august', 'september', 'october', 'november', 'december'];

const QUOTES = /^["'`]+|["'`]+$/g;

/** dates::normalize_due_input */
export function normalizeDueInput(raw: string): string {
  let s = raw.trim();
  if (s.startsWith('@due(')) {
    s = s.slice(5);
    if (raw.startsWith('@due(') && s.endsWith(')')) s = s.slice(0, -1);
  }
  if (s.startsWith('due:')) s = s.slice(4);
  if (s.startsWith('due=')) s = s.slice(4);
  return s.replace(QUOTES, '').trim();
}

function monthName(name: string): number | null {
  const lower = name.replace(/[A-Z]/g, (c) => c.toLowerCase());
  const i = MONTHS.findIndex((m) => m === lower || (lower.length === 3 && m.startsWith(lower)));
  return i < 0 ? null : i + 1;
}
function num(s: string, min: number, max: number): number | null {
  if (s.length < min || s.length > max || !/^[0-9]+$/.test(s)) return null;
  return Number(s);
}
/** Rust's i64::from_str after stripping one '+': optional sign, digits. */
function offset(lower: string, suffix: string): number | null {
  if (!lower.endsWith(suffix)) return null;
  let body = lower.slice(0, -1);
  if (body.startsWith('+')) body = body.slice(1);
  if (!/^[+-]?[0-9]+$/.test(body)) return null;
  const n = Number(body);
  if (!Number.isSafeInteger(n) || n === 0) return null; // overflow -> parse error in Rust
  return n;
}

function parseCalendar(val: string, today: Ymd): Ymd | null {
  for (const sep of ['-', '/']) {
    const parts = val.split(sep);
    if (parts.length === 3) {
      const y = num(parts[0]!, 4, 4);
      const m = num(parts[1]!, 1, 2);
      const d = num(parts[2]!, 1, 2);
      if (y === null || m === null || d === null) return null;
      return fromYmd(y, m, d);
    }
    if (parts.length === 2) {
      const m = num(parts[0]!, 1, 2);
      const d = num(parts[1]!, 1, 2);
      if (m === null || d === null) return null;
      return fromYmd(today.y, m, d);
    }
  }
  const parts = val.split(' ');
  if (parts.length === 2) {
    const m1 = monthName(parts[0]!);
    const d1 = num(parts[1]!, 1, 2);
    if (m1 !== null && d1 !== null) return fromYmd(today.y, m1, d1);
    const d2 = num(parts[0]!, 1, 2);
    const m2 = monthName(parts[1]!);
    if (d2 !== null && m2 !== null) return fromYmd(today.y, m2, d2);
  }
  return null;
}

/** dates::parse_due. Returns `YYYY-MM-DD`, or null where the core errors. */
export function parseDue(input: string, todayIso: string): string | null {
  const val = normalizeDueInput(input);
  if (val.length === 0 || new TextEncoder().encode(val).length > 64) return null;
  const lower = val.toLowerCase();
  const today = parseIso(todayIso);

  switch (lower) {
    case 'today':
    case 'tdy':
      return iso(today);
    case 'tomorrow':
    case 'tmrw':
      return iso(addDays(today, 1));
    case 'yesterday':
    case 'ydy':
      return iso(addDays(today, -1));
  }

  const target = WEEKDAYS[lower];
  if (target !== undefined) {
    let days = (target - weekdayIndex(today) + 7) % 7;
    if (days === 0) days = 7;
    return iso(addDays(today, days));
  }

  let n = offset(lower, 'd');
  if (n !== null) return Math.abs(n) > MAX_DAYS ? null : iso(addDays(today, n));
  n = offset(lower, 'w');
  if (n !== null) return Math.abs(n) > MAX_WEEKS ? null : iso(addDays(today, n * 7));
  n = offset(lower, 'm');
  if (n !== null) return Math.abs(n) > MAX_MONTHS ? null : iso(addMonths(today, n));

  const cal = parseCalendar(val, today);
  return cal ? iso(cal) : null;
}

// ---- limits.rs ------------------------------------------------------------

/** limits::tag. Returns the normalised tag or null when the core refuses it. */
export function normalizeTag(raw: string): string | null {
  let t = raw.trim();
  if (t.startsWith('#')) t = t.slice(1);
  t = t.toLowerCase();
  if (!/^[a-zA-Z]/.test(t)) return null;
  if (chars(t) > MAX_TAG_CHARS) return null;
  if (!/^[\p{L}\p{N}_\-/]*$/u.test(t)) return null;
  return t;
}

/** limits::title. */
function checkTitle(raw: string): string | null {
  const t = raw.trim();
  if (!t || chars(t) > MAX_TITLE_CHARS || /\p{Cc}/u.test(t)) return null;
  return t;
}

// ---- capture.rs -----------------------------------------------------------

export interface Capture {
  title: string;
  tags: string[];
  rawDue: string | null;
  status: CaptureStatus;
  bulletPrefix: boolean;
}

const STATUS_PREFIXES: [string[], CaptureStatus][] = [
  [['- [ ] ', '* [ ] '], 'todo'],
  [['- [~] ', '* [~] '], 'in_progress'],
  [['- [x] ', '- [X] ', '* [x] ', '* [X] '], 'done'],
];

function stripStatusPrefix(text: string): [string, CaptureStatus, boolean] {
  for (const [prefixes, st] of STATUS_PREFIXES) {
    for (const p of prefixes) if (text.startsWith(p)) return [text.slice(p.length).trim(), st, false];
  }
  for (const p of ['- ', '* ', '• ']) if (text.startsWith(p)) return [text.slice(p.length).trim(), 'todo', true];
  return [text, 'todo', false];
}

/** capture::parse_capture_text */
export function parseCaptureText(raw: string): Capture {
  const [text, status, bulletPrefix] = stripStatusPrefix(raw.trim());
  const titleWords: string[] = [];
  const tags: string[] = [];
  let rawDue: string | null = null;

  for (const w of text.split(/\s+/).filter(Boolean)) {
    const lower = w.toLowerCase();
    if (lower.startsWith('@due(') && w.endsWith(')') && w.length > 6) {
      rawDue = w.slice(5, -1);
      continue;
    }
    if ((lower.startsWith('due:') || lower.startsWith('due=')) && w.length > 4) {
      const val = w.slice(4).replace(/^["']+|["']+$/g, '');
      if (val) {
        rawDue = val;
        continue;
      }
    }
    if (w.startsWith('#') && /^[a-zA-Z]/.test(w.slice(1))) {
      const t = normalizeTag(w.slice(1).replace(/[,.;:!?)]+$/, ''));
      if (t !== null) {
        if (!tags.includes(t)) tags.push(t);
        continue;
      }
    }
    titleWords.push(w);
  }
  return { title: titleWords.join(' '), tags, rawDue, status, bulletPrefix };
}

/**
 * capture.rs `resolve`: what capture.parse returns and capture.quick creates.
 * Throws CaptureError where the core answers invalid_args.
 */
export function resolveCapture(text: string, todayIso: string, extraTags: string[] = [], kind: CaptureKind = 'task'): ParsedCapture {
  if (chars(text) > MAX_CAPTURE_CHARS) throw new CaptureError(`capture text exceeds ${MAX_CAPTURE_CHARS} characters`);
  const cap = parseCaptureText(text);
  let title = cap.title;
  let due: string | null = null;
  if (cap.rawDue !== null) {
    due = parseDue(cap.rawDue, todayIso);
    if (due === null) title = `${title} due:${cap.rawDue}`.trim();
  }
  const checked = checkTitle(title);
  if (checked === null) throw new CaptureError('nothing to capture');
  const tags: string[] = [];
  for (const raw of [...cap.tags, ...extraTags]) {
    const t = normalizeTag(raw);
    if (t === null) throw new CaptureError(`tag ${JSON.stringify(raw)} is not valid`);
    if (!tags.includes(t)) tags.push(t);
  }
  if (tags.length > MAX_TAGS_PER_ITEM) throw new CaptureError(`at most ${MAX_TAGS_PER_ITEM} tags per item`);
  return { title: checked, tags, due, kind, status: kind === 'task' ? cap.status : 'todo' };
}

/** Preview helper: like resolveCapture but returns null instead of throwing. */
export function previewCapture(text: string, todayIso: string): ParsedCapture | null {
  try {
    return resolveCapture(text, todayIso);
  } catch {
    return null;
  }
}
