// Engineering manager widget logic: cadence, carry-over, meeting load,
// weekly update and the private-note rules. Pure functions, no DOM.

import { describe, expect, it } from 'vitest';
import type { HubEvent, Item } from '../src/lib/api/types';
import {
  backToBack,
  cadenceOf,
  carryOver,
  commitmentColumns,
  meetingLoad,
  mergeAgenda,
  nudgeTitle,
  planEnd,
  readMeeting,
  readOneOnOne,
  readPeople,
  readSort,
  roster,
  ROSTER_SORTS,
  weeklySections,
  weeklyUpdate,
  type Meeting,
} from '../src/lib/widgets/em/logic';
import { PRIVATE_SUMMARY, eventSummary, redactEvents, searchItems, withoutPrivate } from '../src/lib/privacy/private';
import { peopleById } from '../src/lib/widgets/em/actions';

const TODAY = '2026-11-10'; // a Tuesday
const NOW = new Date('2026-11-10T09:30:00').getTime();
const local = (date: string, hhmm: string) => new Date(`${date}T${hhmm}:00`).toISOString();

let seq = 0;
function item(p: Partial<Item>): Item {
  return {
    id: p.id ?? `i${seq++}`,
    parent_id: null,
    order: 0,
    kind: 'task',
    status: 'todo',
    title: 'x',
    notes: '',
    tags: [],
    inherited_tags: [],
    due: null,
    type_id: null,
    fields: {},
    roles: ['eng-manager'],
    origin: 'user',
    created_at: local(TODAY, '08:00'),
    updated_at: local(TODAY, '08:00'),
    version: 1,
    ...p,
  };
}
const person = (id: string, name: string, f: Item['fields'] = {}) => item({ id, title: name, kind: 'bullet', type_id: 'em.person', fields: { name, ...f } });
const meeting = (title: string, date: string, start: string, end: string, f: Item['fields'] = {}) =>
  item({ title, kind: 'bullet', type_id: 'em.meeting', fields: { starts_at: local(date, start), ends_at: local(date, end), busy: true, category: 'team', ...f } });

describe('one-on-one cadence', () => {
  it('is ok, due soon, overdue or never', () => {
    expect(cadenceOf({ cadenceDays: 7, lastOneOnOne: '2026-11-08' }, TODAY)).toMatchObject({ daysSince: 2, state: 'ok', dueIn: 5 });
    expect(cadenceOf({ cadenceDays: 7, lastOneOnOne: '2026-11-05' }, TODAY)).toMatchObject({ daysSince: 5, state: 'due', dueIn: 2 });
    expect(cadenceOf({ cadenceDays: 7, lastOneOnOne: '2026-11-03' }, TODAY)).toMatchObject({ daysSince: 7, state: 'due', dueIn: 0, ratio: 1 });
    expect(cadenceOf({ cadenceDays: 7, lastOneOnOne: '2026-11-02' }, TODAY)).toMatchObject({ daysSince: 8, state: 'overdue', dueIn: -1, ratio: 1 });
    expect(cadenceOf({ cadenceDays: 14, lastOneOnOne: undefined }, TODAY)).toMatchObject({ daysSince: null, state: 'never' });
  });

  it('clamps or defaults the cadence and ignores bad dates', () => {
    const [p, q] = readPeople([person('p1', 'Ana', { cadence_days: 500, last_one_on_one: 'soon' }), person('p2', 'Bo', { cadence_days: 'weekly' })]);
    expect(p!.cadenceDays).toBe(90);
    expect(p!.lastOneOnOne).toBeUndefined();
    expect(q!.cadenceDays).toBe(14);
  });
});

describe('commitments', () => {
  const people = peopleById([person('p1', 'Priya Nair')]);
  const commit = (id: string, f: Item['fields'], extra: Partial<Item> = {}) => item({ id, title: String(f.what), type_id: 'em.commitment', fields: f, ...extra });

  it('splits by direction, most overdue first, and skips closed ones', () => {
    const cols = commitmentColumns(
      [
        commit('a', { direction: 'i_owe', what: 'Later', due: '2026-11-20' }),
        commit('b', { direction: 'i_owe', what: 'Very late', due: '2026-11-01' }),
        commit('c', { direction: 'i_owe', what: 'Late', due: '2026-11-08' }),
        commit('d', { direction: 'waiting_on', what: 'Estimate', person_id: 'p1', due: '2026-11-09' }),
        commit('e', { direction: 'waiting_on', what: 'Dropped', status: 'dropped' }),
        commit('f', { direction: 'i_owe', what: 'Done', due: '2026-11-01' }, { status: 'done' }),
      ],
      people,
      TODAY,
    );
    expect(cols.iOwe.map((r) => r.c.item.id)).toEqual(['b', 'c', 'a']);
    expect(cols.iOwe[0]!.overdueDays).toBe(9);
    expect(cols.waitingOn.map((r) => r.who)).toEqual(['Priya Nair']);
    expect(nudgeTitle(cols.waitingOn[0]!)).toBe('Nudge Priya Nair about Estimate');
  });
});

describe('one-on-one carry-over', () => {
  const one = (id: string, scheduled: string, agenda: unknown[], status = 'scheduled') =>
    readOneOnOne(item({ id, kind: 'bullet', type_id: 'em.one_on_one', fields: { person_id: 'p1', scheduled_for: scheduled, status, agenda: agenda as never } }))!;

  it('carries unchecked items only, marked as carried', () => {
    const cur = one('o1', local(TODAY, '10:00'), [{ text: 'A', done: true }, { text: 'B', added_by: 'them' }]);
    expect(carryOver(cur.agenda)).toEqual([{ text: 'B', added_by: 'them', carried_over: true, done: false }]);
  });

  it('moves them onto the next scheduled 1:1, first and without duplicates', () => {
    const cur = one('o1', local(TODAY, '10:00'), [{ text: 'B' }, { text: 'Shared' }], 'in_progress');
    const next = one('o2', local('2026-11-17', '10:00'), [{ text: 'shared ' }, { text: 'C' }]);
    const other = readOneOnOne(item({ id: 'o3', type_id: 'em.one_on_one', fields: { person_id: 'p2', scheduled_for: local('2026-11-11', '10:00') } }))!;
    const plan = planEnd(cur, [cur, next, other], local(TODAY, '10:35'), 7);
    expect(plan.ended).toMatchObject({ status: 'done', ended_at: local(TODAY, '10:35') });
    expect(plan.next?.kind).toBe('update');
    if (plan.next?.kind !== 'update') return;
    expect(plan.next.id).toBe('o2');
    expect((plan.next.fields.agenda as { text: string }[]).map((a) => a.text)).toEqual(['B', 'Shared', 'C']);
  });

  it('creates the next 1:1 a cadence later when none is scheduled', () => {
    const cur = one('o1', local(TODAY, '10:00'), [{ text: 'B' }], 'in_progress');
    const plan = planEnd(cur, [cur], local(TODAY, '10:35'), 7);
    expect(plan.next).toMatchObject({ kind: 'create', fields: { person_id: 'p1', status: 'scheduled', scheduled_for: local('2026-11-17', '10:00') } });
  });

  it('needs no follow-up when everything was covered', () => {
    const cur = one('o1', local(TODAY, '10:00'), [{ text: 'B', done: true }], 'in_progress');
    expect(planEnd(cur, [cur], local(TODAY, '10:35'), 7).next).toBeNull();
  });

  it('caps a merged agenda at 50', () => {
    const many = Array.from({ length: 60 }, (_, i) => ({ text: `t${i}`, added_by: 'me' as const, carried_over: false, done: false }));
    expect(mergeAgenda(many, many)).toHaveLength(50);
  });
});

describe('meeting load', () => {
  const items = [
    meeting('Standup', '2026-11-09', '09:00', '09:15'),
    meeting('A', '2026-11-09', '13:00', '13:30', { category: 'cross-team' }),
    meeting('B', '2026-11-09', '13:30', '14:00', { category: 'external' }),
    meeting('C', '2026-11-09', '14:03', '14:45', { category: '1on1' }),
    meeting('Overlap', '2026-11-09', '13:15', '13:45'),
    meeting('Focus', '2026-11-10', '13:00', '15:00', { category: 'focus' }),
    meeting('Optional', '2026-11-10', '11:00', '12:00', { busy: false }),
    meeting('Late', '2026-11-11', '16:00', '17:00'),
    meeting('Next week', '2026-11-16', '10:00', '11:00'),
    item({ title: 'Dev meeting 1:1 with Kim', kind: 'bullet', type_id: 'dev.meeting', fields: { starts_at: local('2026-11-12', '10:00'), ends_at: local('2026-11-12', '10:30') } }),
  ];
  const load = meetingLoad(items, { today: TODAY, now: NOW, budgetHours: 10, dayStart: '09:00', dayEnd: '17:00' });

  it('aggregates this week by category, counting overlaps once and focus not at all', () => {
    expect(load.weekStart).toBe('2026-11-09');
    expect(load.byCategory).toMatchObject({ team: 15 + 30 + 60, 'cross-team': 30, external: 30, '1on1': 42 + 30, focus: 120 });
    // Union of busy non-focus spans: 15 + (13:00–14:00 = 60) + 42 + 60 + 30.
    expect(load.totalMinutes).toBe(15 + 60 + 42 + 60 + 30);
    expect(load.budgetMinutes).toBe(600);
    expect(load.days).toHaveLength(5);
    expect(load.days[0]!.hours[4]).toBe(60); // Monday 13:00–14:00
    expect(load.days[0]!.minutes).toBe(15 + 60 + 42);
  });

  it('finds back-to-back runs, longest first', () => {
    expect(load.chains[0]).toMatchObject({ date: '2026-11-09', count: 4 });
    const ms = items.map(readMeeting).filter((m): m is Meeting => !!m);
    expect(backToBack(ms, 0).find((c) => c.date === '2026-11-09')?.count).toBe(3);
  });

  it('counts only free blocks of an hour or more from now on as maker time', () => {
    // Tue from 09:30: 09:30–17:00 is free except focus (not busy for load) → 7.5 h.
    // Wed: 09:00–16:00 → 7 h. Thu: 09:00–10:00, 10:30–17:00 → 1 h + 6.5 h. Fri 8 h. Mon is past.
    expect(load.makerMinutesLeft).toBe((7.5 + 7 + 1 + 6.5 + 8) * 60);
  });
});

describe('roster order', () => {
  it('sorts only by name or next one-on-one, never by a metric', () => {
    expect(ROSTER_SORTS).toEqual(['name', 'next']);
    expect(readSort('score')).toBe('next');
    const people = readPeople([person('p1', 'Zoe'), person('p2', 'Adam'), person('p3', 'Mia')]);
    const ones = [readOneOnOne(item({ type_id: 'em.one_on_one', fields: { person_id: 'p1', scheduled_for: local('2026-11-11', '10:00') } }))!];
    expect(roster(people, ones, [], TODAY, NOW, 'next').map((r) => r.person.name)).toEqual(['Zoe', 'Adam', 'Mia']);
    expect(roster(people, ones, [], TODAY, NOW, 'name').map((r) => r.person.name)).toEqual(['Adam', 'Mia', 'Zoe']);
  });
});

describe('weekly update', () => {
  const people = peopleById([person('p1', 'Leo Brandt')]);
  const input = {
    today: TODAY,
    tasks: [
      item({ title: 'Publish roadmap', status: 'done' }),
      item({ title: 'Old done', status: 'done', updated_at: local('2026-11-02', '10:00') }),
      item({ title: 'Review scorecards', status: 'in_progress' }),
      item({ title: 'Headcount ask', due: '2026-11-08', tags: ['ask'] }),
      item({ title: 'Private note', kind: 'bullet', type_id: 'em.private_note', fields: { person_id: 'p1', body: 'secret' } }),
    ],
    commitments: [
      item({ type_id: 'em.commitment', status: 'done', fields: { direction: 'i_owe', what: 'Doc', person_id: 'p1', status: 'done' } }),
      item({ type_id: 'em.commitment', fields: { direction: 'i_owe', what: 'Intro', counterpart: 'Directors', due: '2026-11-09' } }),
      item({ type_id: 'em.commitment', fields: { direction: 'waiting_on', what: 'Postmortem', person_id: 'p1', due: '2026-11-11' } }),
    ],
    signals: [
      item({ kind: 'bullet', type_id: 'em.team_signal', fields: { team: 'Platform', period: '2026-W45', cycle_time_p50_hours: 10, open_incidents: 0 } }),
      item({ kind: 'bullet', type_id: 'em.team_signal', fields: { team: 'Platform', period: '2026-W46', cycle_time_p50_hours: 20, open_incidents: 2 } }),
    ],
    people,
  };

  it('assembles shipped, in progress, risks and asks', () => {
    const s = weeklySections(input);
    expect(s.shipped).toEqual(['Publish roadmap', 'Doc (for Leo Brandt)']);
    expect(s.inProgress).toEqual(['Review scorecards']);
    expect(s.risks).toEqual([
      'Late: Intro for Directors (due 2026-11-09)',
      'Overdue: Headcount ask (due 2026-11-08)',
      'Platform: 2 open incidents',
      'Platform: cycle time up to 20 h from 10 h last week',
    ]);
    expect(s.asks).toEqual(['Waiting on Leo Brandt: Postmortem (by 2026-11-11)', 'Headcount ask']);
  });

  it('puts risks first when there are any, and never includes private notes', () => {
    const text = weeklyUpdate(input);
    expect(text.startsWith('Weekly update, week of 2026-11-09\n\nRisks')).toBe(true);
    expect(text).not.toMatch(/secret|Private note/);
    const calm = weeklyUpdate({ ...input, tasks: [], commitments: [], signals: [] });
    expect(calm).toContain('Shipped\n- Nothing to report');
  });
});

describe('private notes stay private', () => {
  const note = item({ id: 'n1', title: 'Private note', notes: 'about Sam', kind: 'bullet', type_id: 'em.private_note', fields: { person_id: 'p1', body: 'stretched on-call' } });
  const task = item({ id: 't1', title: 'Talk to Sam about on-call' });

  it('are excluded from palette and picker search', () => {
    expect(searchItems([note, task], 'sam').map((i) => i.id)).toEqual(['t1']);
    expect(searchItems([note, task], 'private')).toEqual([]);
    expect(searchItems([note, task], 'stretched')).toEqual([]);
    expect(withoutPrivate([note, task])).toEqual([task]);
  });

  it('are excluded by a type marked private at runtime too', () => {
    const custom = item({ id: 'c1', title: 'Hidden', type_id: 'x.secret' });
    expect(searchItems([custom], 'hidden', new Set(['x.secret']))).toEqual([]);
  });

  it('show only a neutral summary in activity and agent strips', () => {
    const ev = (id: string, summary: string): HubEvent => ({ id: `e-${id}`, ts: local(TODAY, '09:00'), actor: 'user', command: 'tasks.update', entity: { kind: 'item', id }, summary });
    const events = [ev('n1', 'Updated bullet "stretched on-call"'), ev('t1', 'Updated task "Talk to Sam"')];
    const ids = new Set(['n1']);
    expect(redactEvents(events, ids).map((e) => e.summary)).toEqual([PRIVATE_SUMMARY, 'Updated task "Talk to Sam"']);
    expect(eventSummary(events[0]!, ids)).toBe(PRIVATE_SUMMARY);
    expect(eventSummary({ entity: { kind: 'widget', id: 'n1' }, summary: 'Moved' }, ids)).toBe('Moved');
  });
});

describe('archived items (removed on the synced side)', () => {
  it('are hidden from people, meetings and the weekly update', () => {
    const archived = { tags: ['archived'] };
    expect(readPeople([person('p1', 'Ada'), item({ ...archived, kind: 'bullet', type_id: 'em.person', fields: { name: 'Gone' } })]).map((p) => p.name)).toEqual(['Ada']);
    expect(readMeeting(item({ ...archived, kind: 'bullet', type_id: 'em.meeting', fields: { starts_at: local(TODAY, '09:00'), ends_at: local(TODAY, '10:00') } }))).toBeNull();
    // Inherited from an archived parent counts too.
    expect(readMeeting(item({ inherited_tags: ['archived'], kind: 'bullet', type_id: 'dev.meeting', fields: { starts_at: local(TODAY, '09:00'), ends_at: local(TODAY, '10:00') } }))).toBeNull();
    const s = weeklySections({
      today: TODAY,
      tasks: [item({ title: 'Live', status: 'in_progress' }), item({ ...archived, title: 'Removed', status: 'in_progress' })],
      commitments: [],
      signals: [],
      people: new Map(),
    });
    expect(s.inProgress).toEqual(['Live']);
  });
});
