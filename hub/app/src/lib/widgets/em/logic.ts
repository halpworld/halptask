// Pure logic for the engineering manager pack's widgets (hub/packs/eng-manager).
//
// Integrations (HR roster, calendar, Slack, email, the team's delivery
// metrics) sync typed items; the widgets read them back with tasks.list and
// write only through existing commands. Values come from outside the app, so
// every reader re-checks types and skips malformed items.
//
// The EM pack never ranks individuals: people are sorted by name or by when
// their next one-on-one is, never by a number about them. Team signals are
// team aggregates only.

import type { Item, JsonValue } from '../../api/types';
import { hideArchived, isArchived } from '../parts/archived';
import { addDays, diffDays, endOfWeek, parseISODate, toISODate } from '../../util/dates';
import { timeOf } from '../dev/logic';

export const EM_TYPES = {
  person: 'em.person',
  commitment: 'em.commitment',
  oneOnOne: 'em.one_on_one',
  meeting: 'em.meeting',
  signal: 'em.team_signal',
  note: 'em.private_note',
} as const;

/** Meetings may also come from the developer pack's calendar type. */
export const MEETING_TYPES: readonly string[] = [EM_TYPES.meeting, 'dev.meeting'];

const DAY = 86_400_000;
const MIN = 60_000;

// ------------------------------------------------------------ Field readers

type Fields = Record<string, JsonValue>;

const str = (f: Fields, k: string, max = 4000): string | undefined => {
  const v = f[k];
  return typeof v === 'string' && v.trim().length > 0 ? v.slice(0, max) : undefined;
};
const num = (f: Fields, k: string): number | undefined => {
  const v = f[k];
  return typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : undefined;
};
const bool = (f: Fields, k: string): boolean => f[k] === true;
function oneOf<T extends string>(f: Fields, k: string, allowed: readonly T[]): T | undefined {
  const v = f[k];
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : undefined;
}
const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const date = (f: Fields, k: string): string | undefined => {
  const v = f[k];
  return typeof v === 'string' && DATE_RE.test(v) ? v : undefined;
};
const localDate = (ms: number): string => toISODate(new Date(ms));

// ------------------------------------------------------------ People

export interface Person {
  item: Item;
  id: string;
  name: string;
  role?: string;
  team?: string;
  startDate?: string;
  timezone?: string;
  cadenceDays: number;
  lastOneOnOne?: string;
  goals: string[];
}

export const DEFAULT_CADENCE = 14;

export function readPerson(item: Item): Person | null {
  if (isArchived(item)) return null;
  if (item.type_id !== EM_TYPES.person) return null;
  const f = item.fields;
  const name = str(f, 'name', 120);
  if (!name) return null;
  const cadence = num(f, 'cadence_days');
  const goals = Array.isArray(f.growth_goals) ? f.growth_goals.filter((g): g is string => typeof g === 'string' && g.trim().length > 0).slice(0, 10) : [];
  return {
    item,
    id: item.id,
    name,
    role: str(f, 'role', 120),
    team: str(f, 'team', 120),
    startDate: date(f, 'start_date'),
    timezone: str(f, 'timezone', 64),
    cadenceDays: cadence && cadence >= 1 ? Math.min(90, Math.floor(cadence)) : DEFAULT_CADENCE,
    lastOneOnOne: date(f, 'last_one_on_one'),
    goals,
  };
}

export const byName = (a: { name: string }, b: { name: string }) => a.name.localeCompare(b.name);

export function readPeople(items: Item[]): Person[] {
  return items.map(readPerson).filter((p): p is Person => !!p).sort(byName);
}

export type CadenceState = 'ok' | 'due' | 'overdue' | 'never';

export interface Cadence {
  /** Days since the last one-on-one, or null when there has not been one. */
  daysSince: number | null;
  cadence: number;
  state: CadenceState;
  /** 0..1 of the cadence used up, for a ring. */
  ratio: number;
  /** Days until the cadence is reached (negative when past it). */
  dueIn: number | null;
}

/** Days within the cadence that count as "due soon". */
export const DUE_SOON_DAYS = 2;

export function cadenceOf(p: Pick<Person, 'cadenceDays' | 'lastOneOnOne'>, today: string): Cadence {
  const cadence = p.cadenceDays;
  if (!p.lastOneOnOne) return { daysSince: null, cadence, state: 'never', ratio: 1, dueIn: null };
  const daysSince = Math.max(0, diffDays(today, p.lastOneOnOne));
  const dueIn = cadence - daysSince;
  const state: CadenceState = daysSince > cadence ? 'overdue' : dueIn <= DUE_SOON_DAYS ? 'due' : 'ok';
  return { daysSince, cadence, state, ratio: Math.min(1, daysSince / cadence), dueIn };
}

// ------------------------------------------------------------ Commitments

export const DIRECTIONS = ['i_owe', 'waiting_on'] as const;
export type Direction = (typeof DIRECTIONS)[number];
export const SOURCES = ['meeting', 'slack', 'email', 'manual'] as const;
export type Source = (typeof SOURCES)[number];

export interface Commitment {
  item: Item;
  direction: Direction;
  personId?: string;
  counterpart?: string;
  what: string;
  promisedOn?: string;
  due?: string;
  source: Source;
  open: boolean;
}

export function readCommitment(item: Item): Commitment | null {
  if (isArchived(item)) return null;
  if (item.type_id !== EM_TYPES.commitment) return null;
  const f = item.fields;
  const direction = oneOf(f, 'direction', DIRECTIONS);
  const what = str(f, 'what', 300) ?? (item.title.trim() || undefined);
  if (!direction || !what) return null;
  const fstatus = oneOf(f, 'status', ['open', 'done', 'dropped'] as const);
  return {
    item,
    direction,
    personId: str(f, 'person_id', 64),
    counterpart: str(f, 'counterpart', 120),
    what,
    promisedOn: date(f, 'promised_on') ?? (timeOf(item.created_at) !== undefined ? localDate(timeOf(item.created_at)!) : undefined),
    due: date(f, 'due') ?? item.due ?? undefined,
    source: oneOf(f, 'source', SOURCES) ?? 'manual',
    // Either side closing it closes it: the item (you) or the sync (status).
    open: item.status !== 'done' && fstatus !== 'done' && fstatus !== 'dropped',
  };
}

export interface CommitmentRow {
  c: Commitment;
  who: string;
  /** Days since it was promised. */
  ageDays: number | null;
  /** Days past due (positive), or null when not overdue / no due date. */
  overdueDays: number | null;
  dueInDays: number | null;
}

export function whoOf(c: Pick<Commitment, 'personId' | 'counterpart'>, people: ReadonlyMap<string, Person>): string {
  return (c.personId && people.get(c.personId)?.name) || c.counterpart || 'someone';
}

function commitmentRow(c: Commitment, people: ReadonlyMap<string, Person>, today: string): CommitmentRow {
  const ageDays = c.promisedOn ? Math.max(0, diffDays(today, c.promisedOn)) : null;
  const dueInDays = c.due ? diffDays(c.due, today) : null;
  return { c, who: whoOf(c, people), ageDays, dueInDays, overdueDays: dueInDays !== null && dueInDays < 0 ? -dueInDays : null };
}

/** Overdue first (most overdue on top), then soonest due, then oldest promise. */
function commitmentOrder(a: CommitmentRow, b: CommitmentRow): number {
  const ao = a.overdueDays ?? -1;
  const bo = b.overdueDays ?? -1;
  if (ao !== bo) return bo - ao;
  const ad = a.dueInDays ?? Number.POSITIVE_INFINITY;
  const bd = b.dueInDays ?? Number.POSITIVE_INFINITY;
  if (ad !== bd) return ad - bd;
  return (b.ageDays ?? 0) - (a.ageDays ?? 0) || a.c.what.localeCompare(b.c.what);
}

export function commitmentColumns(items: Item[], people: ReadonlyMap<string, Person>, today: string): { iOwe: CommitmentRow[]; waitingOn: CommitmentRow[] } {
  const rows = items
    .map(readCommitment)
    .filter((c): c is Commitment => !!c && c.open)
    .map((c) => commitmentRow(c, people, today))
    .sort(commitmentOrder);
  return { iOwe: rows.filter((r) => r.c.direction === 'i_owe'), waitingOn: rows.filter((r) => r.c.direction === 'waiting_on') };
}

/** The task a one-key nudge creates. Nothing is ever sent. */
export function nudgeTitle(r: Pick<CommitmentRow, 'who' | 'c'>): string {
  const what = r.c.what.replace(/\s+/g, ' ').trim();
  const t = `Nudge ${r.who} about ${what}`;
  return Array.from(t).length > 200 ? `${Array.from(t).slice(0, 199).join('')}…` : t;
}

export function agingLabel(r: CommitmentRow): string {
  if (r.overdueDays) return `${r.overdueDays}d late`;
  if (r.dueInDays === 0) return 'due today';
  if (r.dueInDays !== null && r.dueInDays > 0) return `due in ${r.dueInDays}d`;
  if (r.ageDays !== null) return r.ageDays === 0 ? 'today' : `${r.ageDays}d old`;
  return '';
}

// ------------------------------------------------------------ One-on-ones

export const ADDED_BY = ['me', 'them', 'agent'] as const;
export type AddedBy = (typeof ADDED_BY)[number];
export const ONE_STATUSES = ['scheduled', 'in_progress', 'done', 'cancelled'] as const;
export type OneStatus = (typeof ONE_STATUSES)[number];

export interface AgendaItem {
  text: string;
  added_by: AddedBy;
  carried_over: boolean;
  done: boolean;
}

export interface OneOnOne {
  item: Item;
  personId: string;
  scheduledFor?: number;
  agenda: AgendaItem[];
  status: OneStatus;
  startedAt?: number;
  endedAt?: number;
}

export const MAX_AGENDA = 50;

export function readAgenda(v: JsonValue | undefined): AgendaItem[] {
  if (!Array.isArray(v)) return [];
  const out: AgendaItem[] = [];
  for (const raw of v) {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) continue;
    const r = raw as Fields;
    const text = str(r, 'text', 300);
    if (!text) continue;
    out.push({ text, added_by: oneOf(r, 'added_by', ADDED_BY) ?? 'me', carried_over: bool(r, 'carried_over'), done: bool(r, 'done') });
    if (out.length >= MAX_AGENDA) break;
  }
  return out;
}

export function readOneOnOne(item: Item): OneOnOne | null {
  if (isArchived(item)) return null;
  if (item.type_id !== EM_TYPES.oneOnOne) return null;
  const f = item.fields;
  const personId = str(f, 'person_id', 64);
  if (!personId) return null;
  return {
    item,
    personId,
    scheduledFor: timeOf(str(f, 'scheduled_for', 40)),
    agenda: readAgenda(f.agenda),
    status: oneOf(f, 'status', ONE_STATUSES) ?? 'scheduled',
    startedAt: timeOf(str(f, 'started_at', 40)),
    endedAt: timeOf(str(f, 'ended_at', 40)),
  };
}

export function readOneOnOnes(items: Item[]): OneOnOne[] {
  return items.map(readOneOnOne).filter((o): o is OneOnOne => !!o);
}

/** The running or next upcoming one-on-one with a person. */
export function nextOneOnOne(ones: readonly OneOnOne[], personId: string, now: number): OneOnOne | null {
  const mine = ones.filter((o) => o.personId === personId);
  const running = mine.find((o) => o.status === 'in_progress');
  if (running) return running;
  // Today's meeting stays "next" until it is ended, even if it started late.
  const startOfToday = parseISODate(localDate(now)).getTime();
  const upcoming = mine
    .filter((o) => o.status === 'scheduled' && (o.scheduledFor === undefined || o.scheduledFor >= startOfToday))
    .sort((a, b) => (a.scheduledFor ?? Number.POSITIVE_INFINITY) - (b.scheduledFor ?? Number.POSITIVE_INFINITY));
  return upcoming[0] ?? null;
}

/** Unchecked items of an ended one-on-one, marked as carried over. */
export function carryOver(agenda: readonly AgendaItem[]): AgendaItem[] {
  return agenda.filter((a) => !a.done).map((a) => ({ text: a.text, added_by: a.added_by, carried_over: true, done: false }));
}

/** Carried items first, then the target's own; no duplicate texts, capped. */
export function mergeAgenda(target: readonly AgendaItem[], carried: readonly AgendaItem[]): AgendaItem[] {
  const seen = new Set<string>();
  const out: AgendaItem[] = [];
  for (const a of [...carried, ...target]) {
    const key = a.text.trim().toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(a);
    if (out.length >= MAX_AGENDA) break;
  }
  return out;
}

export function agendaJson(agenda: readonly AgendaItem[]): JsonValue {
  return agenda.map((a) => {
    const o: Record<string, JsonValue> = { text: a.text, added_by: a.added_by };
    if (a.carried_over) o.carried_over = true;
    if (a.done) o.done = true;
    return o;
  });
}

export type EndPlan =
  | { ended: Fields; next: { kind: 'update'; id: string; version: number; fields: Fields } }
  | { ended: Fields; next: { kind: 'create'; fields: Fields } }
  | { ended: Fields; next: null };

/**
 * Ending a one-on-one: mark it done, and move every unchecked agenda item to
 * the next scheduled one-on-one with the same person, or to a new one a
 * cadence from now when none is scheduled.
 */
export function planEnd(current: OneOnOne, others: readonly OneOnOne[], nowIso: string, cadenceDays: number): EndPlan {
  const now = Date.parse(nowIso);
  const ended: Fields = { ...current.item.fields, agenda: agendaJson(current.agenda), status: 'done', ended_at: nowIso };
  const carried = carryOver(current.agenda);
  if (carried.length === 0) return { ended, next: null };
  const next = others
    .filter((o) => o.item.id !== current.item.id && o.personId === current.personId && o.status === 'scheduled' && (o.scheduledFor ?? Number.POSITIVE_INFINITY) > now)
    .sort((a, b) => (a.scheduledFor ?? Number.POSITIVE_INFINITY) - (b.scheduledFor ?? Number.POSITIVE_INFINITY))[0];
  if (next) {
    return { ended, next: { kind: 'update', id: next.item.id, version: next.item.version, fields: { ...next.item.fields, agenda: agendaJson(mergeAgenda(next.agenda, carried)) } } };
  }
  const when = current.scheduledFor ?? now;
  return {
    ended,
    next: {
      kind: 'create',
      fields: { person_id: current.personId, scheduled_for: new Date(when + cadenceDays * DAY).toISOString(), status: 'scheduled', agenda: agendaJson(carried) },
    },
  };
}

export function elapsedLabel(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  const m = Math.floor(s / 60);
  const ss = String(s % 60).padStart(2, '0');
  return m >= 60 ? `${Math.floor(m / 60)}:${String(m % 60).padStart(2, '0')}:${ss}` : `${m}:${ss}`;
}

// ------------------------------------------------------------ Roster

export const ROSTER_SORTS = ['name', 'next'] as const;
export type RosterSort = (typeof ROSTER_SORTS)[number];

export interface RosterRow {
  person: Person;
  cadence: Cadence;
  next: OneOnOne | null;
  iOwe: number;
  waitingOn: number;
}

export function roster(people: readonly Person[], ones: readonly OneOnOne[], commitments: readonly Commitment[], today: string, now: number, sort: RosterSort): RosterRow[] {
  const rows = people.map((person) => {
    const mine = commitments.filter((c) => c.open && c.personId === person.id);
    return {
      person,
      cadence: cadenceOf(person, today),
      next: nextOneOnOne(ones, person.id, now),
      iOwe: mine.filter((c) => c.direction === 'i_owe').length,
      waitingOn: mine.filter((c) => c.direction === 'waiting_on').length,
    };
  });
  // Only two orders, neither of them a judgement about the person.
  if (sort === 'next') {
    const at = (r: RosterRow) => r.next?.scheduledFor ?? Number.POSITIVE_INFINITY;
    return rows.sort((a, b) => at(a) - at(b) || byName(a.person, b.person));
  }
  return rows.sort((a, b) => byName(a.person, b.person));
}

export function readSort(v: unknown): RosterSort {
  return v === 'name' ? 'name' : 'next';
}

// ------------------------------------------------------------ Meetings

export const CATEGORIES = ['1on1', 'team', 'cross-team', 'external', 'focus'] as const;
export type Category = (typeof CATEGORIES)[number];
export const CATEGORY_LABEL: Record<Category, string> = { '1on1': 'One-on-ones', team: 'Team', 'cross-team': 'Cross-team', external: 'External', focus: 'Focus' };

export interface Meeting {
  item: Item;
  title: string;
  start: number;
  end: number;
  category: Category;
  busy: boolean;
  attendees?: number;
  organizerIsMe: boolean;
  recurring: boolean;
}

function guessCategory(title: string): Category {
  if (/\b1[:-]?1\b|one[- ]on[- ]one/i.test(title)) return '1on1';
  if (/\bfocus\b|deep work/i.test(title)) return 'focus';
  return 'team';
}

export function readMeeting(item: Item): Meeting | null {
  if (isArchived(item)) return null;
  if (!item.type_id || !MEETING_TYPES.includes(item.type_id)) return null;
  const f = item.fields;
  const start = timeOf(str(f, 'starts_at', 40));
  const end = timeOf(str(f, 'ends_at', 40));
  if (start === undefined || end === undefined || end <= start || end - start > DAY) return null;
  const attendees = num(f, 'attendees_count');
  return {
    item,
    title: item.title,
    start,
    end,
    category: oneOf(f, 'category', CATEGORIES) ?? guessCategory(item.title),
    busy: f.busy !== false,
    attendees: attendees === undefined ? undefined : Math.floor(attendees),
    organizerIsMe: bool(f, 'organizer_is_me'),
    recurring: bool(f, 'recurring'),
  };
}

const HHMM_RE = /^([01]\d|2[0-3]):([0-5]\d)$/;
export function readHhmm(v: unknown, fallback: string): number {
  const m = typeof v === 'string' ? HHMM_RE.exec(v) : null;
  const src = m ?? HHMM_RE.exec(fallback)!;
  return Number(src[1]) * 60 + Number(src[2]);
}

export interface DayLoad {
  date: string;
  /** Meeting minutes inside working hours, per working hour. */
  hours: number[];
  minutes: number;
}

export interface Chain {
  date: string;
  start: number;
  end: number;
  count: number;
  titles: string[];
}

export interface MeetingLoad {
  weekStart: string;
  byCategory: Record<Category, number>;
  /** Meeting minutes this week, focus blocks and free/optional events excluded. */
  totalMinutes: number;
  budgetMinutes: number;
  /** Free working minutes left this week in blocks of at least MAKER_BLOCK_MIN. */
  makerMinutesLeft: number;
  days: DayLoad[];
  dayStartMin: number;
  dayEndMin: number;
  chains: Chain[];
}

export const MAKER_BLOCK_MIN = 60;
/** Gap that still counts as back to back. */
export const BACK_TO_BACK_GAP_MIN = 5;
export const WORKDAYS = 5;

type Span = [number, number];

function mergeSpans(spans: Span[]): Span[] {
  const s = [...spans].sort((a, b) => a[0] - b[0]);
  const out: Span[] = [];
  for (const sp of s) {
    const last = out[out.length - 1];
    if (last && sp[0] <= last[1]) last[1] = Math.max(last[1], sp[1]);
    else out.push([sp[0], sp[1]]);
  }
  return out;
}

const overlap = (a: Span, b: Span) => Math.max(0, Math.min(a[1], b[1]) - Math.max(a[0], b[0]));

/** Back-to-back runs (two or more meetings with gaps of at most BACK_TO_BACK_GAP_MIN). */
export function backToBack(meetings: readonly Meeting[], gapMin = BACK_TO_BACK_GAP_MIN): Chain[] {
  const byDay = new Map<string, Meeting[]>();
  for (const m of meetings) {
    if (!m.busy || m.category === 'focus') continue;
    const d = localDate(m.start);
    byDay.set(d, [...(byDay.get(d) ?? []), m]);
  }
  const chains: Chain[] = [];
  for (const [d, ms] of [...byDay].sort((a, b) => a[0].localeCompare(b[0]))) {
    ms.sort((a, b) => a.start - b.start || a.end - b.end);
    let run: Meeting[] = [];
    let runEnd = 0;
    const flush = () => {
      if (run.length >= 2) chains.push({ date: d, start: run[0]!.start, end: runEnd, count: run.length, titles: run.map((m) => m.title) });
      run = [];
    };
    for (const m of ms) {
      if (run.length && m.start - runEnd > gapMin * MIN) flush();
      if (!run.length) runEnd = m.end;
      run.push(m);
      runEnd = Math.max(runEnd, m.end);
    }
    flush();
  }
  return chains;
}

export function meetingLoad(items: Item[], opts: { today: string; now: number; budgetHours: number; dayStart: string; dayEnd: string }): MeetingLoad {
  const weekStart = addDays(endOfWeek(opts.today), -6);
  const weekEnd = parseISODate(addDays(weekStart, 7)).getTime();
  const weekStartMs = parseISODate(weekStart).getTime();
  const dayStartMin = readHhmm(opts.dayStart, '09:00');
  let dayEndMin = readHhmm(opts.dayEnd, '17:00');
  if (dayEndMin <= dayStartMin) dayEndMin = Math.min(24 * 60, dayStartMin + 8 * 60);

  const meetings = items.map(readMeeting).filter((m): m is Meeting => !!m && m.end > weekStartMs && m.start < weekEnd);
  const byCategory = Object.fromEntries(CATEGORIES.map((c) => [c, 0])) as Record<Category, number>;
  const loadSpans: Span[] = [];
  for (const m of meetings) {
    if (!m.busy) continue;
    const span: Span = [Math.max(m.start, weekStartMs), Math.min(m.end, weekEnd)];
    byCategory[m.category] += Math.round((span[1] - span[0]) / MIN);
    if (m.category !== 'focus') loadSpans.push(span);
  }
  const busy = mergeSpans(loadSpans);
  const totalMinutes = Math.round(busy.reduce((s, sp) => s + (sp[1] - sp[0]), 0) / MIN);

  const hourCount = Math.ceil((dayEndMin - dayStartMin) / 60);
  const days: DayLoad[] = [];
  let maker = 0;
  for (let i = 0; i < WORKDAYS; i++) {
    const date = addDays(weekStart, i);
    const base = parseISODate(date).getTime();
    const hours: number[] = [];
    for (let h = 0; h < hourCount; h++) {
      const slot: Span = [base + (dayStartMin + h * 60) * MIN, base + Math.min(dayEndMin, dayStartMin + (h + 1) * 60) * MIN];
      hours.push(Math.round(busy.reduce((s, sp) => s + overlap(sp, slot), 0) / MIN));
    }
    // Free blocks from now (or the day's start) to the day's end.
    const from = Math.max(base + dayStartMin * MIN, opts.now);
    const to = base + dayEndMin * MIN;
    if (to > from) {
      let cursor = from;
      for (const sp of busy) {
        if (sp[1] <= cursor || sp[0] >= to) continue;
        if (sp[0] - cursor >= MAKER_BLOCK_MIN * MIN) maker += sp[0] - cursor;
        cursor = Math.max(cursor, sp[1]);
      }
      if (to - cursor >= MAKER_BLOCK_MIN * MIN) maker += to - cursor;
    }
    days.push({ date, hours, minutes: hours.reduce((a, b) => a + b, 0) });
  }
  const budget = Number.isFinite(opts.budgetHours) && opts.budgetHours > 0 ? Math.min(opts.budgetHours, 80) : 16;
  return {
    weekStart,
    byCategory,
    totalMinutes,
    budgetMinutes: Math.round(budget * 60),
    makerMinutesLeft: Math.round(maker / MIN),
    days,
    dayStartMin,
    dayEndMin,
    chains: backToBack(meetings).sort((a, b) => b.count - a.count || a.start - b.start),
  };
}

export function hoursLabel(minutes: number): string {
  const h = minutes / 60;
  return h >= 10 || Number.isInteger(h) ? `${Math.round(h)} h` : `${h.toFixed(1)} h`;
}

// ------------------------------------------------------------ Team signals

export interface Signal {
  item: Item;
  team: string;
  period: string;
  mergedPrs?: number;
  cycleHours?: number;
  openIncidents?: number;
  reviewWaitHours?: number;
}

const PERIOD_RE = /^\d{4}-W\d{2}$/;

export function readSignal(item: Item): Signal | null {
  if (isArchived(item)) return null;
  if (item.type_id !== EM_TYPES.signal) return null;
  const f = item.fields;
  const team = str(f, 'team', 120);
  const period = str(f, 'period', 8);
  if (!team || !period || !PERIOD_RE.test(period)) return null;
  return { item, team, period, mergedPrs: num(f, 'merged_prs'), cycleHours: num(f, 'cycle_time_p50_hours'), openIncidents: num(f, 'open_incidents'), reviewWaitHours: num(f, 'review_wait_p50_hours') };
}

export interface PulseMetric {
  key: 'merged' | 'cycle' | 'review' | 'incidents';
  label: string;
  unit: string;
  values: number[];
  latest?: number;
  /** Lower is better for time and incident counts. */
  lowerIsBetter: boolean;
}

export interface TeamPulse {
  team: string;
  periods: string[];
  metrics: PulseMetric[];
  context: string[];
}

function metric(key: PulseMetric['key'], label: string, unit: string, lowerIsBetter: boolean, values: (number | undefined)[]): PulseMetric {
  const vs = values.filter((v): v is number => v !== undefined);
  return { key, label, unit, values: vs, latest: vs[vs.length - 1], lowerIsBetter };
}

const round1 = (v: number) => (v >= 10 ? Math.round(v) : Math.round(v * 10) / 10);

/** Plain-language lines about the trend, team level only, without judgement. */
export function pulseContext(metrics: readonly PulseMetric[], weeks: number): string[] {
  const out: string[] = [];
  for (const m of metrics) {
    if (m.values.length < 2 || m.latest === undefined) continue;
    const first = m.values[0]!;
    const last = m.latest;
    const span = `${m.values.length - 1} week${m.values.length === 2 ? '' : 's'} ago`;
    if (m.key === 'incidents') {
      if (last > 0) out.push(`${last} open incident${last === 1 ? '' : 's'} this week.`);
      continue;
    }
    const change = first === 0 ? 0 : (last - first) / first;
    if (Math.abs(change) < 0.15) out.push(`${m.label} steady at ${round1(last)}${m.unit}.`);
    else out.push(`${m.label} ${last > first ? 'up' : 'down'} to ${round1(last)}${m.unit}, from ${round1(first)}${m.unit} ${span}.`);
  }
  if (out.length === 0 && weeks > 0) out.push('Not enough weeks yet to show a trend.');
  return out;
}

export function teamPulse(items: Item[], weeks: number): TeamPulse[] {
  const n = Math.max(2, Math.min(26, Math.floor(weeks) || 6));
  const byTeam = new Map<string, Map<string, Signal>>();
  for (const s of items.map(readSignal)) {
    if (!s) continue;
    const m = byTeam.get(s.team) ?? new Map<string, Signal>();
    // The latest synced value for a period wins.
    const prev = m.get(s.period);
    if (!prev || prev.item.updated_at < s.item.updated_at) m.set(s.period, s);
    byTeam.set(s.team, m);
  }
  return [...byTeam]
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([team, m]) => {
      const periods = [...m.keys()].sort().slice(-n);
      const rows = periods.map((p) => m.get(p)!);
      const metrics = [
        metric('merged', 'Merged PRs', '', false, rows.map((r) => r.mergedPrs)),
        metric('cycle', 'Cycle time', ' h', true, rows.map((r) => r.cycleHours)),
        metric('review', 'Review wait', ' h', true, rows.map((r) => r.reviewWaitHours)),
        metric('incidents', 'Open incidents', '', true, rows.map((r) => r.openIncidents)),
      ];
      return { team, periods, metrics, context: pulseContext(metrics, periods.length) };
    });
}

// ------------------------------------------------------------ Weekly update

export interface WeeklyInput {
  today: string;
  /** Plain tasks of the role (no synced bullets, no private items). */
  tasks: Item[];
  commitments: Item[];
  signals: Item[];
  people: ReadonlyMap<string, Person>;
}

export interface WeeklySections {
  shipped: string[];
  inProgress: string[];
  risks: string[];
  asks: string[];
}

const oneLine = (s: string) => s.replace(/\s+/g, ' ').trim();

export function weeklySections(input: WeeklyInput): WeeklySections {
  const weekStart = addDays(endOfWeek(input.today), -6);
  const startMs = parseISODate(weekStart).getTime();
  const tasks = hideArchived(input.tasks).filter((t) => t.kind === 'task' && !t.type_id?.startsWith('em.private'));
  const commitments = input.commitments.map(readCommitment).filter((c): c is Commitment => !!c);

  const doneThisWeek = (i: Item) => i.status === 'done' && (timeOf(i.updated_at) ?? 0) >= startMs;
  const shipped = [
    ...tasks.filter((t) => t.type_id !== EM_TYPES.commitment && doneThisWeek(t)).map((t) => oneLine(t.title)),
    ...commitments.filter((c) => c.direction === 'i_owe' && !c.open && doneThisWeek(c.item)).map((c) => `${oneLine(c.what)} (for ${whoOf(c, input.people)})`),
  ];
  const inProgress = tasks.filter((t) => t.status === 'in_progress' && t.type_id !== EM_TYPES.commitment).map((t) => oneLine(t.title));

  const risks: string[] = [];
  for (const c of commitments) {
    if (!c.open || c.direction !== 'i_owe' || !c.due || c.due >= input.today) continue;
    risks.push(`Late: ${oneLine(c.what)} for ${whoOf(c, input.people)} (due ${c.due})`);
  }
  for (const t of tasks) {
    if (t.status !== 'done' && t.type_id !== EM_TYPES.commitment && t.due && t.due < input.today) risks.push(`Overdue: ${oneLine(t.title)} (due ${t.due})`);
  }
  for (const p of teamPulse(input.signals, 6)) {
    const inc = p.metrics.find((m) => m.key === 'incidents');
    if (inc?.latest) risks.push(`${p.team}: ${inc.latest} open incident${inc.latest === 1 ? '' : 's'}`);
    const cyc = p.metrics.find((m) => m.key === 'cycle');
    if (cyc && cyc.values.length >= 2 && cyc.latest !== undefined) {
      const prev = cyc.values[cyc.values.length - 2]!;
      if (prev > 0 && cyc.latest > prev * 1.25) risks.push(`${p.team}: cycle time up to ${round1(cyc.latest)} h from ${round1(prev)} h last week`);
    }
  }

  const asks = [
    ...commitments.filter((c) => c.open && c.direction === 'waiting_on').map((c) => `Waiting on ${whoOf(c, input.people)}: ${oneLine(c.what)}${c.due ? ` (by ${c.due})` : ''}`),
    ...tasks.filter((t) => t.status !== 'done' && t.tags.includes('ask')).map((t) => oneLine(t.title)),
  ];
  return { shipped, inProgress, risks, asks };
}

export function weeklyText(s: WeeklySections, weekStart: string): string {
  const section = (title: string, lines: string[]) => [title, ...(lines.length ? lines.map((l) => `- ${l}`) : ['- Nothing to report'])].join('\n');
  return [
    `Weekly update, week of ${weekStart}`,
    '',
    // Risks and asks first when anything is late, as the weekly-update prompt asks.
    ...(s.risks.length
      ? [section('Risks', s.risks), '', section('Asks', s.asks), '', section('Shipped', s.shipped), '', section('In progress', s.inProgress)]
      : [section('Shipped', s.shipped), '', section('In progress', s.inProgress), '', section('Risks', s.risks), '', section('Asks', s.asks)]),
  ].join('\n');
}

export function weeklyUpdate(input: WeeklyInput): string {
  return weeklyText(weeklySections(input), addDays(endOfWeek(input.today), -6));
}

export { localDate };
