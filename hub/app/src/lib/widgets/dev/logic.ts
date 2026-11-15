// Pure logic for the developer pack's widgets (hub/packs/developer).
//
// Integrations sync external objects (GitHub PRs, CI runs, Slack asks, local
// git context, calendar meetings) into the store as typed items. The widgets
// only read them back through tasks.list. Field values come from outside the
// app, so every reader here re-checks types and never trusts a shape: a
// malformed item is skipped rather than crashing a widget.

import type { Item, JsonValue } from '../../api/types';
import { hideArchived, isArchived } from '../parts/archived';

export const DEV_TYPES = {
  pr: 'dev.pull_request',
  ci: 'dev.ci_run',
  ask: 'dev.ask',
  context: 'dev.context',
  meeting: 'dev.meeting',
} as const;

// ------------------------------------------------------------ Field readers

type Fields = Record<string, JsonValue>;

const str = (f: Fields, k: string): string | undefined => {
  const v = f[k];
  return typeof v === 'string' && v.length > 0 ? v : undefined;
};
const int = (f: Fields, k: string): number | undefined => {
  const v = f[k];
  return typeof v === 'number' && Number.isFinite(v) ? Math.max(0, Math.floor(v)) : undefined;
};
const bool = (f: Fields, k: string): boolean => f[k] === true;
const strs = (f: Fields, k: string, max = 100): string[] => {
  const v = f[k];
  return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string' && x.length > 0).slice(0, max) : [];
};
const ints = (f: Fields, k: string, max = 20): number[] => {
  const v = f[k];
  return Array.isArray(v) ? v.filter((x): x is number => typeof x === 'number' && Number.isInteger(x) && x > 0).slice(0, max) : [];
};
function oneOf<T extends string>(f: Fields, k: string, allowed: readonly T[]): T | undefined {
  const v = f[k];
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : undefined;
}
/** Milliseconds since epoch, or undefined for a missing or unparseable time. */
export function timeOf(iso: string | undefined): number | undefined {
  if (!iso) return undefined;
  const t = Date.parse(iso);
  return Number.isFinite(t) ? t : undefined;
}

/** Only https URLs are ever offered to open or copy. */
export function safeUrl(raw: unknown): string | null {
  if (typeof raw !== 'string' || raw.length > 2048) return null;
  try {
    const u = new URL(raw);
    return u.protocol === 'https:' && !u.username && !u.password ? u.href : null;
  } catch {
    return null;
  }
}

// ------------------------------------------------------------ Pull requests

export const PR_STATES = ['draft', 'open', 'review_requested', 'changes_requested', 'approved', 'merged', 'closed'] as const;
export type PrState = (typeof PR_STATES)[number];
export type CiState = 'unknown' | 'pending' | 'passing' | 'failing';

export interface Pr {
  item: Item;
  repo: string;
  number: number;
  url: string | null;
  state: PrState;
  ci: CiState;
  author?: string;
  reviewers: string[];
  requestedFromMe: boolean;
  authoredByMe: boolean;
  /** When the review was asked of me, else last update, else when synced. */
  since: number;
  updatedAt?: number;
  additions?: number;
  deletions?: number;
  branch?: string;
  mergeable: 'clean' | 'conflicting' | 'blocked' | 'unknown';
}

export function readPr(item: Item): Pr | null {
  if (isArchived(item)) return null;
  if (item.type_id !== DEV_TYPES.pr) return null;
  const f = item.fields;
  const repo = str(f, 'repo');
  const number = int(f, 'number');
  if (!repo || !number) return null;
  const updatedAt = timeOf(str(f, 'updated_at'));
  const since = timeOf(str(f, 'requested_at')) ?? updatedAt ?? timeOf(item.created_at) ?? 0;
  return {
    item,
    repo,
    number,
    url: safeUrl(f.url),
    state: oneOf(f, 'state', PR_STATES) ?? 'open',
    ci: oneOf(f, 'ci', ['unknown', 'pending', 'passing', 'failing'] as const) ?? 'unknown',
    author: str(f, 'author'),
    reviewers: strs(f, 'reviewers', 30),
    requestedFromMe: bool(f, 'requested_from_me'),
    authoredByMe: bool(f, 'authored_by_me'),
    since,
    updatedAt,
    additions: int(f, 'additions'),
    deletions: int(f, 'deletions'),
    branch: str(f, 'branch'),
    mergeable: oneOf(f, 'mergeable', ['clean', 'conflicting', 'blocked', 'unknown'] as const) ?? 'unknown',
  };
}

const CLOSED: readonly PrState[] = ['merged', 'closed'];

/** 0 fresh, 1 settling, 2 stale, 3 very stale. `staleHours` is the "stale" mark. */
export function staleness(ageMs: number, staleHours = 24): 0 | 1 | 2 | 3 {
  const h = ageMs / 3_600_000;
  const s = Math.max(1, staleHours);
  if (h < s / 4) return 0;
  if (h < s) return 1;
  if (h < s * 3) return 2;
  return 3;
}

/** "12m", "5h", "3d". */
export function ageLabel(ms: number): string {
  const m = Math.max(0, Math.floor(ms / 60_000));
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

export type SizeLabel = 'XS' | 'S' | 'M' | 'L' | 'XL';
export function sizeLabel(additions?: number, deletions?: number): SizeLabel | null {
  if (additions === undefined && deletions === undefined) return null;
  const n = (additions ?? 0) + (deletions ?? 0);
  if (n <= 10) return 'XS';
  if (n <= 100) return 'S';
  if (n <= 400) return 'M';
  if (n <= 1000) return 'L';
  return 'XL';
}

/** Compact "+1.2k". */
export function compactCount(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1).replace(/\.0$/, '')}k`;
  return `${Math.round(n / 1000)}k`;
}

export interface QueueRow {
  pr: Pr;
  ageMs: number;
  stale: 0 | 1 | 2 | 3;
  size: SizeLabel | null;
}

/** Reviews asked of me that are still open, oldest request first. */
export function reviewQueue(items: Item[], now: number, staleHours = 24): QueueRow[] {
  return items
    .filter((i) => i.status !== 'done')
    .map(readPr)
    .filter((p): p is Pr => !!p && p.requestedFromMe && !p.authoredByMe && !CLOSED.includes(p.state) && p.state !== 'draft')
    .map((pr) => {
      const ageMs = Math.max(0, now - pr.since);
      return { pr, ageMs, stale: staleness(ageMs, staleHours), size: sizeLabel(pr.additions, pr.deletions) };
    })
    .sort((a, b) => b.ageMs - a.ageMs || a.pr.repo.localeCompare(b.pr.repo) || a.pr.number - b.pr.number);
}

/** The four stages a PR of mine walks through. */
export const STAGES = ['Draft', 'Review', 'Approved', 'Merged'] as const;

export interface MyPrRow {
  pr: Pr;
  /** Index into STAGES. */
  stage: 0 | 1 | 2 | 3;
  /** What stands between this PR and the next stage, or null when nothing does. */
  blocker: string | null;
  tone: 'neutral' | 'accent' | 'success' | 'warning' | 'danger';
}

export function stageOf(state: PrState): 0 | 1 | 2 | 3 {
  switch (state) {
    case 'draft':
      return 0;
    case 'approved':
      return 2;
    case 'merged':
      return 3;
    default:
      return 1;
  }
}

export function blockerOf(pr: Pr): { text: string | null; tone: MyPrRow['tone'] } {
  if (pr.state === 'merged') return { text: null, tone: 'success' };
  if (pr.ci === 'failing') return { text: 'CI failing', tone: 'danger' };
  if (pr.mergeable === 'conflicting') return { text: 'Merge conflicts', tone: 'warning' };
  if (pr.state === 'changes_requested') return { text: 'Changes requested', tone: 'warning' };
  if (pr.state === 'draft') return { text: 'Still a draft', tone: 'neutral' };
  if (pr.ci === 'pending') return { text: 'Waiting on CI', tone: 'neutral' };
  if (pr.state === 'approved') {
    if (pr.mergeable === 'blocked') return { text: 'Blocked by branch rules', tone: 'warning' };
    return { text: 'Ready to merge', tone: 'success' };
  }
  if (pr.reviewers.length) {
    const who = pr.reviewers.slice(0, 2).join(', ') + (pr.reviewers.length > 2 ? ` +${pr.reviewers.length - 2}` : '');
    return { text: `Waiting on ${who}`, tone: 'accent' };
  }
  return { text: 'No reviewer yet', tone: 'warning' };
}

/**
 * My open PRs, the ones needing me first (failing, conflicts, changes asked,
 * ready to merge), then the ones waiting on others. Merged PRs stay for a
 * day so a finished thing gets its moment.
 */
export function myPrs(items: Item[], now: number, keepMergedMs = 24 * 3_600_000): MyPrRow[] {
  const rank: Record<MyPrRow['tone'], number> = { danger: 0, warning: 1, success: 2, accent: 3, neutral: 4 };
  return items
    .map(readPr)
    .filter((p): p is Pr => !!p && p.authoredByMe && p.state !== 'closed')
    .filter((p) => p.state !== 'merged' || now - (p.updatedAt ?? p.since) < keepMergedMs)
    .map((pr) => {
      const b = blockerOf(pr);
      return { pr, stage: stageOf(pr.state), blocker: b.text, tone: b.tone };
    })
    .sort((a, b) => Number(a.stage === 3) - Number(b.stage === 3) || rank[a.tone] - rank[b.tone] || a.pr.since - b.pr.since);
}

// ------------------------------------------------------------------- CI runs

export type CiStatus = 'queued' | 'running' | 'passing' | 'failing' | 'cancelled';
export interface CiRun {
  item: Item;
  repo: string;
  branch: string;
  workflow: string;
  status: CiStatus;
  url: string | null;
  at: number;
  startedAt?: number;
  finishedAt?: number;
  prNumber?: number;
}

export function readCi(item: Item): CiRun | null {
  if (isArchived(item)) return null;
  if (item.type_id !== DEV_TYPES.ci) return null;
  const f = item.fields;
  const repo = str(f, 'repo');
  const branch = str(f, 'branch');
  const workflow = str(f, 'workflow');
  const status = oneOf(f, 'status', ['queued', 'running', 'passing', 'failing', 'cancelled'] as const);
  if (!repo || !branch || !workflow || !status) return null;
  const startedAt = timeOf(str(f, 'started_at'));
  const finishedAt = timeOf(str(f, 'finished_at'));
  return { item, repo, branch, workflow, status, url: safeUrl(f.url), startedAt, finishedAt, at: finishedAt ?? startedAt ?? timeOf(item.updated_at) ?? 0, prNumber: int(f, 'pr_number') };
}

export interface CiBranch {
  branch: string;
  /** The latest run per workflow on this branch. */
  runs: CiRun[];
  /** Worst status across those runs. */
  status: CiStatus;
}
export interface CiRepo {
  repo: string;
  branches: CiBranch[];
  failing: number;
}

const SEVERITY: Record<CiStatus, number> = { failing: 0, running: 1, queued: 2, cancelled: 3, passing: 4 };

/** Latest run per repo/branch/workflow, grouped by repo. Failing repos first, then by name. */
export function ciRadar(items: Item[]): CiRepo[] {
  const latest = new Map<string, CiRun>();
  for (const it of items) {
    const r = readCi(it);
    if (!r) continue;
    const key = `${r.repo}\u0000${r.branch}\u0000${r.workflow}`;
    const prev = latest.get(key);
    if (!prev || r.at > prev.at) latest.set(key, r);
  }
  const repos = new Map<string, Map<string, CiRun[]>>();
  for (const r of latest.values()) {
    const b = repos.get(r.repo) ?? new Map<string, CiRun[]>();
    b.set(r.branch, [...(b.get(r.branch) ?? []), r]);
    repos.set(r.repo, b);
  }
  const out: CiRepo[] = [];
  for (const [repo, bmap] of repos) {
    const branches: CiBranch[] = [...bmap].map(([branch, runs]) => {
      runs.sort((a, b) => SEVERITY[a.status] - SEVERITY[b.status] || a.workflow.localeCompare(b.workflow));
      return { branch, runs, status: runs[0]!.status };
    });
    const isMain = (b: string) => b === 'main' || b === 'master';
    branches.sort((a, b) => Number(isMain(b.branch)) - Number(isMain(a.branch)) || SEVERITY[a.status] - SEVERITY[b.status] || a.branch.localeCompare(b.branch));
    out.push({ repo, branches, failing: branches.filter((b) => b.status === 'failing').length });
  }
  return out.sort((a, b) => Number(b.failing > 0) - Number(a.failing > 0) || a.repo.localeCompare(b.repo));
}

// -------------------------------------------------------------------- Asks

export const ASK_SOURCES = ['slack', 'email', 'github', 'manual'] as const;
export type AskSource = (typeof ASK_SOURCES)[number];
export const SOURCE_LABEL: Record<AskSource, string> = { slack: 'Slack', email: 'Email', github: 'GitHub', manual: 'Noted' };

export interface Ask {
  item: Item;
  source: AskSource;
  from: string;
  channel?: string;
  permalink: string | null;
  receivedAt: number;
  replyBy?: number;
}

export function readAsk(item: Item): Ask | null {
  if (isArchived(item)) return null;
  if (item.type_id !== DEV_TYPES.ask) return null;
  const f = item.fields;
  const source = oneOf(f, 'source', ASK_SOURCES);
  const from = str(f, 'from');
  const receivedAt = timeOf(str(f, 'received_at'));
  if (!source || !from || receivedAt === undefined) return null;
  return { item, source, from, channel: str(f, 'channel'), permalink: safeUrl(f.permalink), receivedAt, replyBy: timeOf(str(f, 'reply_by')) };
}

export interface AskGroup {
  source: AskSource;
  label: string;
  asks: Ask[];
}

/** Open asks grouped by source (fixed order), oldest first within a group. */
export function askGroups(items: Item[]): AskGroup[] {
  const asks = items
    .filter((i) => i.status !== 'done')
    .map(readAsk)
    .filter((a): a is Ask => !!a);
  return ASK_SOURCES.map((source) => ({
    source,
    label: SOURCE_LABEL[source],
    asks: asks.filter((a) => a.source === source).sort((a, b) => a.receivedAt - b.receivedAt || a.item.id.localeCompare(b.item.id)),
  })).filter((g) => g.asks.length > 0);
}

/** Reply-by countdown: text and a calm tone (late, soon = under 2 h, later). */
export function replyBy(deadline: number | undefined, now: number): { text: string; tone: 'late' | 'soon' | 'later' } | null {
  if (deadline === undefined) return null;
  const diff = deadline - now;
  if (diff <= 0) return { text: `${ageLabel(-diff)} past reply-by`, tone: 'late' };
  return { text: `reply in ${ageLabel(diff)}`, tone: diff < 2 * 3_600_000 ? 'soon' : 'later' };
}

// --------------------------------------------------------- Context snapshots

export interface Context {
  item: Item;
  repo: string;
  branch: string;
  files: string[];
  note: string;
  lastCommand?: string;
  openPrs: number[];
  /** The task to resume (`task_id`), or null when the snapshot names none. */
  taskId: string | null;
  capturedAt: number;
}

export function readContext(item: Item): Context | null {
  if (isArchived(item)) return null;
  if (item.type_id !== DEV_TYPES.context) return null;
  const f = item.fields;
  const repo = str(f, 'repo');
  const branch = str(f, 'branch');
  const capturedAt = timeOf(str(f, 'captured_at'));
  if (!repo || !branch || capturedAt === undefined) return null;
  return {
    item,
    repo,
    branch,
    files: strs(f, 'files', 100),
    note: str(f, 'note') ?? item.notes ?? '',
    lastCommand: str(f, 'last_command'),
    openPrs: ints(f, 'open_prs', 20),
    taskId: str(f, 'task_id') ?? null,
    capturedAt,
  };
}

/** Latest snapshot per repo, newest repo first. */
export function latestContexts(items: Item[]): Context[] {
  const by = new Map<string, Context>();
  for (const it of items) {
    const c = readContext(it);
    if (!c) continue;
    const prev = by.get(c.repo);
    if (!prev || c.capturedAt > prev.capturedAt) by.set(c.repo, c);
  }
  return [...by.values()].sort((a, b) => b.capturedAt - a.capturedAt);
}

// ------------------------------------------------------------------ Meetings

export interface Meeting {
  item: Item;
  start: number;
  end: number;
  location?: string;
  url: string | null;
  busy: boolean;
}

export function readMeeting(item: Item): Meeting | null {
  if (isArchived(item)) return null;
  if (item.type_id !== DEV_TYPES.meeting) return null;
  const f = item.fields;
  const start = timeOf(str(f, 'starts_at'));
  const end = timeOf(str(f, 'ends_at'));
  if (start === undefined || end === undefined || end <= start) return null;
  return { item, start, end, location: str(f, 'location'), url: safeUrl(f.url), busy: f.busy !== false };
}

export function meetingsOf(items: Item[]): Meeting[] {
  return items
    .map(readMeeting)
    .filter((m): m is Meeting => !!m)
    .sort((a, b) => a.start - b.start || a.end - b.end);
}

/** The meeting in progress (if any) and the next busy one after `now`. */
export function meetingWindow(meetings: Meeting[], now: number): { current: Meeting | null; next: Meeting | null } {
  const busy = meetings.filter((m) => m.busy);
  return {
    current: busy.find((m) => m.start <= now && now < m.end) ?? null,
    next: busy.find((m) => m.start > now) ?? null,
  };
}

// ------------------------------------------------------------- Focus timer

export interface FocusSession {
  item_id: string | null;
  started_at: string;
  minutes: number;
}

/** Reads a persisted session from widget config; malformed values are ignored. */
export function readSession(raw: JsonValue | undefined): FocusSession | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null;
  const started = raw.started_at;
  const minutes = raw.minutes;
  const item = raw.item_id;
  if (typeof started !== 'string' || timeOf(started) === undefined) return null;
  if (typeof minutes !== 'number' || !Number.isFinite(minutes) || minutes < 1 || minutes > 480) return null;
  return { started_at: started, minutes: Math.round(minutes), item_id: typeof item === 'string' ? item : null };
}

export function sessionProgress(s: FocusSession, now: number): { elapsedMs: number; remainingMs: number; fraction: number; done: boolean } {
  const total = s.minutes * 60_000;
  const elapsedMs = Math.max(0, now - (timeOf(s.started_at) ?? now));
  const remainingMs = Math.max(0, total - elapsedMs);
  return { elapsedMs, remainingMs, fraction: Math.min(1, elapsedMs / total), done: remainingMs === 0 };
}

/** "25:00" or "1:05:00". */
export function clockLabel(ms: number): string {
  const s = Math.max(0, Math.ceil(ms / 1000));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const p = (n: number) => String(n).padStart(2, '0');
  return h ? `${h}:${p(m)}:${p(sec)}` : `${m}:${p(sec)}`;
}

/** "45 min", "1h 30m". */
export function minutesLabel(ms: number): string {
  const m = Math.max(0, Math.round(ms / 60_000));
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h}h ${m % 60}m` : `${h}h`;
}

// ------------------------------------------------------------------ Day plan

export interface PlanBlock {
  kind: 'meeting' | 'task' | 'free';
  start: number;
  end: number;
  item?: Item;
  /** For task blocks: this is the focused item. */
  focus?: boolean;
  /** Entirely before now. */
  past: boolean;
}

export interface DayPlanInput {
  /** The local date being planned (YYYY-MM-DD). */
  date: string;
  now: number;
  meetings: Meeting[];
  /** Open tasks to place, in priority order. */
  tasks: Item[];
  focusId?: string | null;
  /** "HH:MM" local. */
  dayStart?: string;
  dayEnd?: string;
  /** Minutes per task without an estimate. */
  defaultMinutes?: number;
  /** Gaps shorter than this stay free and unplanned. */
  minSlot?: number;
}

/** Local time on `date` from "HH:MM"; null when malformed. */
export function atTime(date: string, hhmm: string): number | null {
  const m = /^(\d{2}):(\d{2})$/.exec(hhmm);
  const d = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!m || !d) return null;
  const h = Number(m[1]);
  const min = Number(m[2]);
  if (h > 23 || min > 59) return null;
  return new Date(Number(d[1]), Number(d[2]) - 1, Number(d[3]), h, min).getTime();
}

export function estimateMinutes(item: Item, fallback: number): number {
  const e = item.type_id === 'base.task' ? item.fields.estimate_minutes : undefined;
  return typeof e === 'number' && Number.isFinite(e) && e > 0 ? Math.min(Math.round(e), 8 * 60) : fallback;
}

const STEP = 5 * 60_000;
const ceilStep = (t: number) => Math.ceil(t / STEP) * STEP;

/**
 * A time-blocked plan for one day: busy meetings stay where they are, the
 * gaps between them become free slots, and open tasks (focus first) fill the
 * future part of those gaps in order. Tasks that do not fit are returned in
 * `unplaced`. Pure: the same input always gives the same plan.
 */
export function dayPlan(input: DayPlanInput): { blocks: PlanBlock[]; unplaced: Item[]; freeMs: number } {
  const dayStart = atTime(input.date, input.dayStart ?? '09:00') ?? atTime(input.date, '09:00')!;
  let dayEnd = atTime(input.date, input.dayEnd ?? '17:30') ?? atTime(input.date, '17:30')!;
  if (dayEnd <= dayStart) dayEnd = dayStart + 8 * 3_600_000;
  const minSlot = Math.max(5, input.minSlot ?? 15) * 60_000;
  const fallback = Math.max(5, input.defaultMinutes ?? 30);
  const now = input.now;

  // Busy intervals inside the day, merged.
  const busy = input.meetings
    .filter((m) => m.busy && m.end > dayStart && m.start < dayEnd)
    .map((m) => ({ start: Math.max(m.start, dayStart), end: Math.min(m.end, dayEnd), m }))
    .sort((a, b) => a.start - b.start);
  const merged: { start: number; end: number }[] = [];
  for (const b of busy) {
    const last = merged.at(-1);
    if (last && b.start <= last.end) last.end = Math.max(last.end, b.end);
    else merged.push({ start: b.start, end: b.end });
  }
  const gaps: { start: number; end: number }[] = [];
  let cursor = dayStart;
  for (const b of merged) {
    if (b.start > cursor) gaps.push({ start: cursor, end: b.start });
    cursor = Math.max(cursor, b.end);
  }
  if (cursor < dayEnd) gaps.push({ start: cursor, end: dayEnd });

  // Tasks: focus first, the rest in the given order, each once.
  const seen = new Set<string>();
  const queue = [...input.tasks].sort((a, b) => Number(b.id === input.focusId) - Number(a.id === input.focusId)).filter((t) => (seen.has(t.id) ? false : (seen.add(t.id), true)));

  const blocks: PlanBlock[] = busy.map((b) => ({ kind: 'meeting', start: b.m.start, end: b.m.end, item: b.m.item, past: b.m.end <= now }));
  let freeMs = 0;
  for (const g of gaps) {
    // The part of a gap before now is history: show it as past free time.
    const from = Math.max(g.start, Math.min(g.end, ceilStep(now)));
    if (from > g.start) blocks.push({ kind: 'free', start: g.start, end: from, past: true });
    let t = from;
    while (queue.length && g.end - t >= minSlot) {
      const task = queue[0]!;
      const len = estimateMinutes(task, fallback) * 60_000;
      const room = g.end - t;
      if (len > room && room < minSlot * 2 && blocks.some((b) => b.kind === 'task')) break;
      const end = t + Math.min(len, room);
      blocks.push({ kind: 'task', start: t, end, item: task, focus: task.id === input.focusId, past: false });
      queue.shift();
      t = end;
    }
    if (g.end > t) {
      blocks.push({ kind: 'free', start: t, end: g.end, past: g.end <= now });
      freeMs += g.end - t;
    }
  }
  blocks.sort((a, b) => a.start - b.start || (a.kind === 'meeting' ? -1 : 1));
  return { blocks: blocks.filter((b) => b.end - b.start >= 60_000), unplaced: queue, freeMs };
}

/** Today's open tasks for the plan: overdue first, then due today, then in progress; by due then outline order. */
export function planTasks(items: Item[], today: string): Item[] {
  const rank = (i: Item) => (i.due && i.due < today ? 0 : i.due === today ? 1 : 2);
  return hideArchived(items)
    .filter((i) => i.kind === 'task' && i.status !== 'done' && (i.due ? i.due <= today : i.status === 'in_progress'))
    .filter((i) => i.type_id !== DEV_TYPES.meeting && i.type_id !== DEV_TYPES.ci && i.type_id !== DEV_TYPES.context)
    .sort((a, b) => rank(a) - rank(b) || (a.due ?? '').localeCompare(b.due ?? '') || a.order - b.order);
}
