// Pure logic behind the developer widgets (src/lib/widgets/dev/logic.ts).

import { describe, expect, it } from 'vitest';
import type { Item, JsonValue } from '../src/lib/api/types';
import {
  askGroups,
  blockerOf,
  ciRadar,
  clockLabel,
  dayPlan,
  latestContexts,
  meetingsOf,
  meetingWindow,
  myPrs,
  planTasks,
  readPr,
  readSession,
  replyBy,
  reviewQueue,
  safeUrl,
  sessionProgress,
  sizeLabel,
  staleness,
} from '../src/lib/widgets/dev/logic';

const H = 3_600_000;
const M = 60_000;
const NOW = new Date(2026, 10, 10, 10, 0).getTime(); // local 10:00
const iso = (t: number) => new Date(t).toISOString();
let seq = 0;

function item(p: Partial<Item> & { fields?: Record<string, JsonValue> }): Item {
  const id = p.id ?? `i${++seq}`;
  return {
    id,
    parent_id: null,
    order: seq,
    kind: 'task',
    status: 'todo',
    title: id,
    notes: '',
    tags: [],
    inherited_tags: [],
    due: null,
    type_id: null,
    fields: {},
    roles: [],
    origin: 'user',
    created_at: iso(NOW - 100 * H),
    updated_at: iso(NOW - 100 * H),
    version: 1,
    ...p,
  };
}
const pr = (fields: Record<string, JsonValue>, p: Partial<Item> = {}) => item({ type_id: 'dev.pull_request', fields: { repo: 'a/b', number: 1, ...fields }, ...p });
const ci = (fields: Record<string, JsonValue>) => item({ kind: 'bullet', type_id: 'dev.ci_run', fields: { repo: 'a/b', branch: 'main', workflow: 'ci', status: 'passing', ...fields } });
const ask = (fields: Record<string, JsonValue>, p: Partial<Item> = {}) => item({ type_id: 'dev.ask', fields: { source: 'slack', from: 'Mira', received_at: iso(NOW - H), ...fields }, ...p });
const meeting = (startMin: number, endMin: number, fields: Record<string, JsonValue> = {}) =>
  item({ kind: 'bullet', type_id: 'dev.meeting', fields: { starts_at: iso(NOW + startMin * M), ends_at: iso(NOW + endMin * M), ...fields } });

describe('safeUrl', () => {
  it('allows https only, without credentials', () => {
    expect(safeUrl('https://github.com/a/b/pull/1')).toBe('https://github.com/a/b/pull/1');
    for (const bad of ['http://x.test', 'javascript:alert(1)', 'data:text/html,hi', 'https://u:p@x.test/', 'file:///etc/passwd', 'not a url', 42, null, `https://x.test/${'a'.repeat(2100)}`]) {
      expect(safeUrl(bad), String(bad)).toBeNull();
    }
  });
});

describe('review queue', () => {
  it('scores staleness against the threshold', () => {
    expect(staleness(2 * H)).toBe(0);
    expect(staleness(8 * H)).toBe(1);
    expect(staleness(30 * H)).toBe(2);
    expect(staleness(80 * H)).toBe(3);
    expect(staleness(5 * H, 4)).toBe(2);
  });

  it('labels size by lines changed', () => {
    expect(sizeLabel(5, 2)).toBe('XS');
    expect(sizeLabel(60, 20)).toBe('S');
    expect(sizeLabel(300, 50)).toBe('M');
    expect(sizeLabel(700, 200)).toBe('L');
    expect(sizeLabel(1800, 400)).toBe('XL');
    expect(sizeLabel(undefined, undefined)).toBeNull();
  });

  it('keeps open reviews asked of me, oldest request first', () => {
    const items = [
      pr({ number: 1, requested_from_me: true, requested_at: iso(NOW - 3 * H) }),
      pr({ number: 2, requested_from_me: true, requested_at: iso(NOW - 50 * H) }),
      pr({ number: 3, requested_from_me: true, updated_at: iso(NOW - 10 * H) }),
      pr({ number: 4, requested_from_me: false, requested_at: iso(NOW - 90 * H) }),
      pr({ number: 5, requested_from_me: true, state: 'merged' }),
      pr({ number: 6, requested_from_me: true, state: 'draft' }),
      pr({ number: 7, requested_from_me: true, authored_by_me: true }),
      pr({ number: 8, requested_from_me: true }, { status: 'done' }),
      item({ type_id: 'dev.pull_request', fields: { repo: 'a/b' } }), // malformed: no number
    ];
    const q = reviewQueue(items, NOW);
    expect(q.map((r) => r.pr.number)).toEqual([2, 3, 1]);
    expect(q.map((r) => r.stale)).toEqual([2, 1, 0]);
  });

  it('is empty for no data', () => {
    expect(reviewQueue([], NOW)).toEqual([]);
  });
});

describe('my pull requests', () => {
  const p = (f: Record<string, JsonValue>) => readPr(pr({ authored_by_me: true, ...f }))!;

  it('names what blocks each PR', () => {
    expect(blockerOf(p({ state: 'review_requested', ci: 'failing' }))).toEqual({ text: 'CI failing', tone: 'danger' });
    expect(blockerOf(p({ state: 'approved', mergeable: 'conflicting' })).text).toBe('Merge conflicts');
    expect(blockerOf(p({ state: 'changes_requested' })).text).toBe('Changes requested');
    expect(blockerOf(p({ state: 'draft' })).text).toBe('Still a draft');
    expect(blockerOf(p({ state: 'approved', ci: 'passing', mergeable: 'clean' }))).toEqual({ text: 'Ready to merge', tone: 'success' });
    expect(blockerOf(p({ state: 'review_requested', reviewers: ['mira', 'oskar', 'jonas'] })).text).toBe('Waiting on mira, oskar +1');
    expect(blockerOf(p({ state: 'open' })).text).toBe('No reviewer yet');
    expect(blockerOf(p({ state: 'merged' })).text).toBeNull();
  });

  it('puts the PRs needing me first and drops old merges', () => {
    const rows = myPrs(
      [
        pr({ number: 1, authored_by_me: true, state: 'review_requested', reviewers: ['mira'] }),
        pr({ number: 2, authored_by_me: true, state: 'open', ci: 'failing' }),
        pr({ number: 3, authored_by_me: true, state: 'merged', updated_at: iso(NOW - 2 * H) }),
        pr({ number: 4, authored_by_me: true, state: 'merged', updated_at: iso(NOW - 30 * H) }),
        pr({ number: 5, authored_by_me: true, state: 'approved', ci: 'passing' }),
        pr({ number: 6, state: 'open' }), // not mine
      ],
      NOW,
    );
    expect(rows.map((r) => r.pr.number)).toEqual([2, 5, 1, 3]);
    expect(rows.map((r) => r.stage)).toEqual([1, 2, 1, 3]);
    expect(myPrs([], NOW)).toEqual([]);
  });
});

describe('CI radar', () => {
  it('keeps the latest run per workflow and groups by repo, failing first', () => {
    const repos = ciRadar([
      ci({ repo: 'a/b', branch: 'main', status: 'failing', finished_at: iso(NOW - 3 * H) }),
      ci({ repo: 'a/b', branch: 'main', status: 'passing', finished_at: iso(NOW - H) }), // newer: wins
      ci({ repo: 'a/b', branch: 'feat', status: 'running', started_at: iso(NOW - 5 * M) }),
      ci({ repo: 'c/d', branch: 'fix', status: 'failing', finished_at: iso(NOW - 10 * M) }),
      ci({ repo: 'c/d', branch: 'main', workflow: 'release', status: 'passing', finished_at: iso(NOW - 9 * H) }),
      item({ type_id: 'dev.ci_run', fields: { repo: 'x/y', branch: 'main', workflow: 'ci', status: 'exploded' } }),
    ]);
    expect(repos.map((r) => r.repo)).toEqual(['c/d', 'a/b']);
    expect(repos[0]!.branches.map((b) => [b.branch, b.status])).toEqual([
      ['main', 'passing'],
      ['fix', 'failing'],
    ]);
    expect(repos[1]!.branches.map((b) => [b.branch, b.status])).toEqual([
      ['main', 'passing'],
      ['feat', 'running'],
    ]);
    expect(repos[0]!.failing).toBe(1);
    expect(ciRadar([])).toEqual([]);
  });
});

describe('asks', () => {
  it('groups open asks by source, oldest first', () => {
    const groups = askGroups([
      ask({ source: 'email', from: 'Priya', received_at: iso(NOW - 20 * H) }),
      ask({ source: 'slack', from: 'Jonas', received_at: iso(NOW - 1 * H) }),
      ask({ source: 'slack', from: 'Mira', received_at: iso(NOW - 3 * H) }),
      ask({ source: 'manual', from: 'Standup' }),
      ask({ source: 'slack', from: 'Done' }, { status: 'done' }),
      ask({ source: 'pigeon', from: 'Nope' }),
    ]);
    expect(groups.map((g) => [g.label, g.asks.map((a) => a.from)])).toEqual([
      ['Slack', ['Mira', 'Jonas']],
      ['Email', ['Priya']],
      ['Noted', ['Standup']],
    ]);
    expect(askGroups([])).toEqual([]);
  });

  it('counts down to reply-by', () => {
    expect(replyBy(undefined, NOW)).toBeNull();
    expect(replyBy(NOW + 70 * M, NOW)).toMatchObject({ tone: 'soon' });
    expect(replyBy(NOW + 6 * H, NOW)).toMatchObject({ tone: 'later' });
    expect(replyBy(NOW - 60 * M, NOW)).toMatchObject({ tone: 'late', text: expect.stringContaining('past') });
  });
});

describe('context restore', () => {
  it('keeps the latest snapshot per repo, newest first, and only an explicit task link', () => {
    const ctx = (repo: string, mins: number, extra: Record<string, JsonValue> = {}, p: Partial<Item> = {}) =>
      item({ kind: 'bullet', type_id: 'dev.context', fields: { repo, branch: 'main', captured_at: iso(NOW - mins * M), ...extra }, ...p });
    const out = latestContexts([ctx('a/b', 240, { task_id: 'old' }), ctx('a/b', 25, { task_id: 'dev-1' }), ctx('c/d', 60, {}, { parent_id: 'some-bullet' }), ctx('e/f', 10, { captured_at: 'never' })]);
    expect(out.map((c) => [c.repo, c.taskId])).toEqual([
      ['a/b', 'dev-1'],
      ['c/d', null],
    ]);
    expect(latestContexts([])).toEqual([]);
  });
});

describe('focus session', () => {
  it('reads only well-formed sessions', () => {
    expect(readSession({ item_id: 'dev-1', started_at: iso(NOW), minutes: 50 })).toEqual({ item_id: 'dev-1', started_at: iso(NOW), minutes: 50 });
    expect(readSession({ started_at: iso(NOW), minutes: 50 })?.item_id).toBeNull();
    for (const bad of [null, undefined, 'x', [], { started_at: 'soon', minutes: 50 }, { started_at: iso(NOW), minutes: 0 }, { started_at: iso(NOW), minutes: 9999 }] as JsonValue[]) {
      expect(readSession(bad)).toBeNull();
    }
  });

  it('tracks progress and formats the clock', () => {
    const s = { item_id: null, started_at: iso(NOW - 18 * M), minutes: 50 };
    expect(sessionProgress(s, NOW)).toMatchObject({ remainingMs: 32 * M, done: false });
    expect(sessionProgress(s, NOW + H).done).toBe(true);
    expect(clockLabel(32 * M)).toBe('32:00');
    expect(clockLabel(65 * M + 5000)).toBe('1:05:05');
  });

  it('finds the current and next busy meeting', () => {
    const ms = meetingsOf([meeting(-30, 15), meeting(30, 60, { busy: false }), meeting(90, 120)]);
    const w = meetingWindow(ms, NOW);
    expect(w.current?.start).toBe(NOW - 30 * M);
    expect(w.next?.start).toBe(NOW + 90 * M);
    expect(meetingWindow([], NOW)).toEqual({ current: null, next: null });
  });
});

describe('day plan', () => {
  const date = '2026-11-10';

  it('fills the free slots between meetings with tasks, focus first', () => {
    const tasks = [
      item({ id: 'a', type_id: 'base.task', fields: { estimate_minutes: 45 } }),
      item({ id: 'focus', type_id: 'base.task', fields: { estimate_minutes: 30 } }),
      item({ id: 'c' }), // default 30
      item({ id: 'big', type_id: 'base.task', fields: { estimate_minutes: 400 } }),
    ];
    // Day 09:00-13:00, now 10:00. Meetings 10:30-11:00 and 11:30-12:00 (+ an overlapping 11:45-12:15).
    const plan = dayPlan({ date, now: NOW, meetings: meetingsOf([meeting(30, 60), meeting(90, 120), meeting(105, 135), meeting(150, 160, { busy: false })]), tasks, focusId: 'focus', dayStart: '09:00', dayEnd: '13:00' });
    const shape = plan.blocks.map((b) => `${b.kind}:${b.kind === 'meeting' ? '' : (b.item?.id ?? '')}@${new Date(b.start).getHours()}:${String(new Date(b.start).getMinutes()).padStart(2, '0')}`);
    expect(shape).toEqual([
      'free:@9:00', // before now: past
      'task:focus@10:00',
      'meeting:@10:30',
      'task:a@11:00', // 45 min would overrun 11:30: it is cut to the 30 min room
      'meeting:@11:30',
      'meeting:@11:45',
      'task:c@12:15',
      'free:@12:45', // 15 min left: too short to start the 400-minute task
    ]);
    expect(plan.blocks[0]!.past).toBe(true);
    expect(plan.blocks.find((b) => b.item?.id === 'focus')?.focus).toBe(true);
    expect(plan.unplaced.map((t) => t.id)).toEqual(['big']);
  });

  it('leaves short gaps free and reports what does not fit', () => {
    const tasks = [item({ id: 'x' }), item({ id: 'y' })];
    // Day ends 10:40: only a 10-minute gap before a 10:10-10:30 meeting and 10 minutes after.
    const plan = dayPlan({ date, now: NOW, meetings: meetingsOf([meeting(10, 30)]), tasks, dayStart: '10:00', dayEnd: '10:40' });
    expect(plan.blocks.filter((b) => b.kind === 'task')).toEqual([]);
    expect(plan.unplaced.map((t) => t.id)).toEqual(['x', 'y']);
    expect(plan.freeMs).toBe(20 * M);
  });

  it('is an all-free day with no data', () => {
    const plan = dayPlan({ date, now: NOW - 2 * H, meetings: [], tasks: [] });
    expect(plan.blocks.map((b) => b.kind)).toEqual(['free']);
    expect(plan.unplaced).toEqual([]);
  });

  it('plans overdue, due-today and in-progress tasks only', () => {
    const items = [
      item({ id: 'later', due: '2026-11-12' }),
      item({ id: 'today', due: date }),
      item({ id: 'overdue', due: '2026-11-08' }),
      item({ id: 'wip', status: 'in_progress' }),
      item({ id: 'idle' }),
      item({ id: 'done', due: date, status: 'done' }),
      item({ id: 'note', kind: 'bullet', due: date }),
      item({ id: 'mtg', due: date, type_id: 'dev.meeting' }),
    ];
    expect(planTasks(items, date).map((i) => i.id)).toEqual(['overdue', 'today', 'wip']);
  });
});

describe('archived items (removed on the synced side)', () => {
  it('are hidden from PRs, CI, asks, meetings and the day plan', () => {
    const archived: Partial<Item> = { tags: ['archived'], status: 'done' };
    expect(readPr(pr({}, archived))).toBeNull();
    expect(readPr(pr({}, { inherited_tags: ['archived'] }))).toBeNull();
    expect(readPr(pr({}))).not.toBeNull();
    expect(myPrs([pr({ authored_by_me: true }), pr({ authored_by_me: true }, { tags: ['archived'] })], NOW)).toHaveLength(1);
    const radar = ciRadar([ci({}), item({ kind: 'bullet', type_id: 'dev.ci_run', tags: ['archived'], fields: { repo: 'a/b', branch: 'x', workflow: 'ci', status: 'failing' } })]);
    expect(radar.flatMap((r) => r.branches)).toHaveLength(1);
    expect(radar.every((r) => r.failing === 0)).toBe(true);
    expect(askGroups([ask({}, { tags: ['archived'] })]).flatMap((g) => g.asks)).toEqual([]);
    expect(meetingsOf([item({ kind: 'bullet', type_id: 'dev.meeting', tags: ['archived'], fields: { starts_at: iso(NOW + 10 * M), ends_at: iso(NOW + 40 * M) } })])).toEqual([]);
    const today = new Date(NOW).toISOString().slice(0, 10);
    expect(planTasks([item({ status: 'in_progress', title: 'live' }), item({ status: 'in_progress', title: 'gone', tags: ['archived'] })], today).map((i) => i.title)).toEqual(['live']);
  });
});
