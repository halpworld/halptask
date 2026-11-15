// The engineering manager's mock day: the pack's types (exactly as in
// hub/packs/eng-manager/pack.json), six fictional people, a week of meetings,
// commitments both ways, one-on-ones, team signals and private notes.
//
// Everything is relative to "today" (and the 1:1 about to start to "now"), so
// `pnpm dev` always shows a live week. tests/em-seed.test.ts checks every
// item against the schemas, like the core would.

import type { Item, TaskType, Widget } from '../types';
import { addDays, endOfWeek, parseISODate } from '../../util/dates';

const SCHEMA = 'https://json-schema.org/draft/2020-12/schema';
const ID = { type: 'string', pattern: '^[A-Za-z0-9_-]+$', minLength: 1, maxLength: 64 };
const DATE = { type: 'string', format: 'date', maxLength: 10 };
const STAMP = { type: 'string', format: 'date-time', maxLength: 40 };
const text = (max: number) => ({ type: 'string', minLength: 1, maxLength: max });

/** The eng-manager pack's task types, exactly as in the pack. */
export const EM_TYPES: TaskType[] = [
  {
    id: 'em.person',
    title: 'Person',
    icon: 'user',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        name: text(120),
        role: text(120),
        team: text(120),
        start_date: DATE,
        timezone: { type: 'string', pattern: '^[A-Za-z][A-Za-z0-9_+/-]*$', maxLength: 64 },
        cadence_days: { type: 'integer', minimum: 1, maximum: 90 },
        last_one_on_one: DATE,
        growth_goals: { type: 'array', maxItems: 10, items: text(200) },
        email: { type: 'string', format: 'email', maxLength: 254 },
      },
      required: ['name'],
      additionalProperties: false,
    },
    display: { primary_fields: ['role', 'team'], badge_field: 'team' },
  },
  {
    id: 'em.commitment',
    title: 'Commitment',
    icon: 'handshake',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        direction: { enum: ['i_owe', 'waiting_on'] },
        person_id: ID,
        counterpart: text(120),
        what: text(300),
        promised_on: DATE,
        due: DATE,
        source: { enum: ['meeting', 'slack', 'email', 'manual'] },
        status: { enum: ['open', 'done', 'dropped'] },
      },
      required: ['direction', 'what'],
      additionalProperties: false,
    },
    display: { primary_fields: ['direction', 'counterpart', 'due'], badge_field: 'direction' },
  },
  {
    id: 'em.one_on_one',
    title: 'One-on-one',
    icon: 'users',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        person_id: ID,
        scheduled_for: STAMP,
        agenda: {
          type: 'array',
          maxItems: 50,
          items: {
            type: 'object',
            properties: { text: text(300), added_by: { enum: ['me', 'them', 'agent'] }, carried_over: { type: 'boolean' }, done: { type: 'boolean' } },
            required: ['text'],
            additionalProperties: false,
          },
        },
        status: { enum: ['scheduled', 'in_progress', 'done', 'cancelled'] },
        started_at: STAMP,
        ended_at: STAMP,
      },
      required: ['person_id'],
      additionalProperties: false,
      'x-halp-ext-writable': ['scheduled_for'],
    },
    display: { primary_fields: ['person_id', 'scheduled_for'], badge_field: 'status' },
  },
  {
    id: 'em.meeting',
    title: 'Meeting',
    icon: 'calendar-clock',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        starts_at: STAMP,
        ends_at: STAMP,
        location: { type: 'string', maxLength: 200 },
        url: { type: 'string', format: 'uri', maxLength: 2048 },
        busy: { type: 'boolean' },
        attendees_count: { type: 'integer', minimum: 0, maximum: 10000 },
        organizer_is_me: { type: 'boolean' },
        recurring: { type: 'boolean' },
        category: { enum: ['1on1', 'team', 'cross-team', 'external', 'focus'] },
      },
      required: ['starts_at', 'ends_at'],
      additionalProperties: false,
    },
    display: { primary_fields: ['starts_at', 'category'], badge_field: 'category' },
  },
  {
    id: 'em.team_signal',
    title: 'Team signal',
    icon: 'activity',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        team: text(120),
        period: { type: 'string', pattern: '^[0-9]{4}-W[0-9]{2}$', maxLength: 8 },
        merged_prs: { type: 'integer', minimum: 0, maximum: 100000 },
        cycle_time_p50_hours: { type: 'number', minimum: 0, maximum: 10000 },
        open_incidents: { type: 'integer', minimum: 0, maximum: 10000 },
        review_wait_p50_hours: { type: 'number', minimum: 0, maximum: 10000 },
      },
      required: ['team', 'period'],
      additionalProperties: false,
    },
    display: { primary_fields: ['team', 'period'], badge_field: 'period' },
  },
  {
    id: 'em.private_note',
    title: 'Private note',
    icon: 'lock',
    schema: {
      $schema: SCHEMA,
      'x-halp-data-class': 'notes.private',
      type: 'object',
      properties: { person_id: ID, body: { type: 'string', minLength: 1, maxLength: 4000 }, created_at: STAMP },
      required: ['person_id', 'body'],
      additionalProperties: false,
    },
    display: { primary_fields: ['person_id'] },
  },
];

export const EM_WIDGETS = ['now', 'people-roster', 'commitments', 'one-on-one', 'meeting-load', 'team-pulse', 'weekly-update', 'agenda', 'task-list', 'activity', 'quick-capture'];

/** The pack's default layout. */
export function emLayoutWidgets(): Widget[] {
  const w = (id: string, kind: string, title: string, x: number, y: number, ww: number, h: number, config: Widget['config'] = {}): Widget => ({ id, kind, title, x, y, w: ww, h, config });
  return [
    w('now', 'now', 'Now', 0, 0, 4, 3),
    w('people-roster', 'people-roster', 'People', 4, 0, 5, 4, { sort: 'next' }),
    w('meeting-load', 'meeting-load', 'Meeting load', 9, 0, 3, 4, { budget_hours: 16, day_start: '09:00', day_end: '17:00' }),
    w('commitments', 'commitments', 'Commitments', 0, 3, 4, 5),
    w('one-on-one', 'one-on-one', 'One-on-one', 4, 4, 5, 5),
    w('agenda', 'agenda', 'Agenda', 9, 4, 3, 4, { days: 7 }),
    w('weekly-update', 'weekly-update', 'Weekly update', 0, 8, 4, 4),
    w('team-pulse', 'team-pulse', 'Team pulse', 9, 8, 3, 4, { weeks: 6 }),
    w('task-list', 'task-list', 'My tasks', 4, 9, 5, 3, { preset: 'open' }),
  ];
}

/** ISO 8601 week label (`2026-W46`) of a local date. */
export function isoWeek(date: string): string {
  const d = parseISODate(date);
  const day = (d.getDay() + 6) % 7;
  d.setDate(d.getDate() - day + 3); // Thursday of this week decides the year
  const year = d.getFullYear();
  const jan4 = new Date(year, 0, 4);
  const week = 1 + Math.round(((d.getTime() - jan4.getTime()) / 86_400_000 - 3 + ((jan4.getDay() + 6) % 7)) / 7);
  return `${year}-W${String(week).padStart(2, '0')}`;
}

export interface EmSeed {
  id: string;
  title: string;
  status?: Item['status'];
  due?: number | null;
  tags?: string[];
  notes?: string;
  type_id?: string;
  fields?: Item['fields'];
  kind?: Item['kind'];
  origin?: string;
}

export function emSeeds(today: string, nowIso: string, slot: (mins: number) => string): EmSeed[] {
  const weekStart = addDays(endOfWeek(today), -6);
  /** Local time `hh:mm` on a weekday of this week (0 = Monday), as an ISO stamp. */
  const wk = (day: number, hhmm: string) => {
    const d = parseISODate(addDays(weekStart, day));
    const [h, m] = hhmm.split(':').map(Number);
    d.setHours(h ?? 0, m ?? 0, 0, 0);
    return d.toISOString();
  };
  /** Local time on a day relative to today. */
  const rel = (days: number, hhmm: string) => {
    const d = parseISODate(addDays(today, days));
    const [h, m] = hhmm.split(':').map(Number);
    d.setHours(h ?? 0, m ?? 0, 0, 0);
    return d.toISOString();
  };
  const plus = (iso: string, mins: number) => new Date(Date.parse(iso) + mins * 60_000).toISOString();

  const person = (id: string, name: string, role: string, start: number, tz: string, cadence: number, last: number | null, goals: string[]): EmSeed => ({
    id,
    title: name,
    kind: 'bullet',
    origin: 'agent:hr',
    type_id: 'em.person',
    fields: {
      name,
      role,
      team: 'Platform',
      start_date: addDays(today, -start),
      timezone: tz,
      cadence_days: cadence,
      ...(last === null ? {} : { last_one_on_one: addDays(today, -last) }),
      growth_goals: goals,
    },
  });

  type Cat = '1on1' | 'team' | 'cross-team' | 'external' | 'focus';
  let n = 0;
  const meeting = (start: string, mins: number, title: string, category: Cat, attendees: number, opts: { mine?: boolean; recurring?: boolean; location?: string; url?: string } = {}): EmSeed => ({
    id: `em-m-${++n}`,
    title,
    kind: 'bullet',
    origin: 'agent:calendar',
    type_id: 'em.meeting',
    fields: {
      starts_at: start,
      ends_at: plus(start, mins),
      busy: true,
      attendees_count: attendees,
      organizer_is_me: !!opts.mine,
      recurring: !!opts.recurring,
      category,
      ...(opts.location ? { location: opts.location } : {}),
      ...(opts.url ? { url: opts.url } : {}),
    },
  });

  const one = (id: string, personId: string, name: string, when: string, agenda: Item['fields'][string], status = 'scheduled'): EmSeed => ({
    id,
    title: `1:1 with ${name}`,
    kind: 'bullet',
    origin: 'agent:calendar',
    type_id: 'em.one_on_one',
    fields: { person_id: personId, scheduled_for: when, status, agenda },
  });

  const commit = (id: string, title: string, f: Item['fields'], due: number | null, origin = 'user', status: Item['status'] = 'todo'): EmSeed => ({
    id,
    title,
    due,
    status,
    tags: ['commitment'],
    origin,
    type_id: 'em.commitment',
    fields: f,
  });

  const note = (id: string, personId: string, body: string, daysAgo: number): EmSeed => ({
    id,
    // Private items only ever carry the neutral title; the body is a field.
    title: 'Private note',
    kind: 'bullet',
    type_id: 'em.private_note',
    fields: { person_id: personId, body, created_at: rel(-daysAgo, '17:10') },
  });

  // The 1:1 that is about to start follows "now"; the rest sit on real days.
  const samAt = slot(30);
  const ones = [
    one('em-o-sam-prev', 'em-p-sam', 'Sam Okafor', rel(-7, '10:00'), [
      { text: 'Rotation change: how is on-call going?', added_by: 'me', done: true },
      { text: 'Importer rewrite: who else should be involved?', added_by: 'them', done: true },
    ], 'done'),
    one('em-o-sam', 'em-p-sam', 'Sam Okafor', samAt, [
      { text: 'On-call load since the rotation change', added_by: 'me', carried_over: true },
      { text: 'Importer rewrite: scope for Q1', added_by: 'them' },
      { text: 'Conference talk proposal: review the abstract', added_by: 'me' },
      { text: 'Intro to the infra guild lead (I owe this)', added_by: 'agent' },
    ]),
    one('em-o-ines', 'em-p-ines', 'Ines Duarte', rel(1, '15:30'), [
      { text: 'First month: what has been confusing?', added_by: 'me' },
      { text: 'Pairing setup with Priya', added_by: 'agent' },
    ]),
    one('em-o-leo', 'em-p-leo', 'Leo Brandt', rel(2, '11:00'), [{ text: 'Postmortem: what support do you need to run it?', added_by: 'me' }]),
    one('em-o-priya', 'em-p-priya', 'Priya Nair', rel(6, '13:00'), [{ text: 'Security review process: next steps', added_by: 'them' }]),
    one('em-o-dana', 'em-p-dana', 'Dana Wills', rel(8, '14:00'), [{ text: 'Promo packet: impact story for the sandbox work', added_by: 'me' }]),
  ];
  // The calendar copies of this week's one-on-ones.
  const oneMeetings = ones
    .filter((o) => o.fields!.status === 'scheduled' && (o.fields!.scheduled_for as string) < wk(7, '00:00') && (o.fields!.scheduled_for as string) >= wk(0, '00:00'))
    .map((o) => meeting(o.fields!.scheduled_for as string, 30, o.title, '1on1', 2, { mine: true, recurring: true }));

  const standup = (day: number) => meeting(wk(day, '09:00'), 15, 'Platform standup', 'team', 7, { recurring: true, location: 'Huddle' });

  const signals = [
    [31, 26, 9, 1],
    [28, 24, 8.5, 0],
    [35, 22, 7, 2],
    [33, 23, 7.5, 1],
    [38, 19, 6, 0],
    [41, 18, 5.5, 1],
  ].map(([merged, cycle, review, incidents], i, all): EmSeed => {
    const period = isoWeek(addDays(today, -7 * (all.length - 1 - i)));
    return {
      id: `em-s-${i + 1}`,
      title: `Platform · ${period}`,
      kind: 'bullet',
      origin: 'agent:github',
      type_id: 'em.team_signal',
      fields: { team: 'Platform', period, merged_prs: merged!, cycle_time_p50_hours: cycle!, open_incidents: incidents!, review_wait_p50_hours: review! },
    };
  });

  return [
    // Your own tasks.
    { id: 'em-1', title: 'Prep 1:1 with Sam: growth plan check-in', status: 'in_progress', due: 0, tags: ['one-on-one'], notes: 'Bring the importer scope doc.' },
    { id: 'em-6', title: 'Review hiring loop scorecards', status: 'in_progress', due: 1, tags: ['hiring'] },
    { id: 'em-7', title: 'Draft headcount ask for platform team', due: -2, tags: ['planning', 'ask'] },
    { id: 'em-8', title: 'Plan Q1 offsite agenda', due: 6, tags: ['planning'] },
    { id: 'em-9', title: 'Approve on-call swap for Thursday', status: 'done', due: 0 },
    { id: 'em-10', title: 'Publish the Q4 platform roadmap', status: 'done', due: -1, tags: ['planning'] },

    // People (HR roster sync).
    person('em-p-sam', 'Sam Okafor', 'Senior engineer', 980, 'Europe/Stockholm', 7, 7, ['Lead the importer rewrite', 'Mentor the next new hire']),
    person('em-p-priya', 'Priya Nair', 'Staff engineer', 1620, 'Europe/London', 14, 6, ['Own the security review process']),
    person('em-p-leo', 'Leo Brandt', 'Engineer', 410, 'Europe/Berlin', 7, 11, ['Run an incident review end to end', 'Present at the architecture forum']),
    person('em-p-dana', 'Dana Wills', 'Senior engineer', 1210, 'America/New_York', 14, 3, ['Promotion to staff: the sandbox design', 'Grow a second reviewer for the hub']),
    person('em-p-ines', 'Ines Duarte', 'Engineer', 20, 'Europe/Lisbon', 7, null, ['Ship a first feature end to end']),
    person('em-p-jonas', 'Jonas Berg', 'Senior engineer', 760, 'Europe/Stockholm', 14, 16, ['Lead the wasmtime upgrade']),

    // Commitments, both ways (tasks: you close them; the sync may too).
    commit('em-c-1', 'Weekly platform update for directors', { direction: 'i_owe', counterpart: 'Directors', what: 'Weekly platform update', promised_on: addDays(today, -7), due: addDays(today, 0), source: 'manual', status: 'open' }, 0),
    commit('em-c-2', 'Feedback on Dana’s promo packet', { direction: 'i_owe', person_id: 'em-p-dana', what: 'Feedback on the promo packet', promised_on: addDays(today, -4), due: addDays(today, 2), source: 'meeting', status: 'open' }, 2, 'agent:calendar'),
    commit('em-c-3', 'Intro Sam to the infra guild lead', { direction: 'i_owe', person_id: 'em-p-sam', what: 'Intro to the infra guild lead', promised_on: addDays(today, -7), due: addDays(today, -1), source: 'meeting', status: 'open' }, -1, 'agent:calendar'),
    commit('em-c-4', 'Scorecards for the backend loop', { direction: 'i_owe', counterpart: 'Recruiting', what: 'Scorecards for the backend loop', promised_on: addDays(today, -2), due: addDays(today, 1), source: 'email', status: 'open' }, 1, 'agent:email'),
    commit('em-c-5', 'Approve Ines’s conference budget', { direction: 'i_owe', person_id: 'em-p-ines', what: 'Approve the conference budget', promised_on: addDays(today, -1), source: 'slack', status: 'open' }, null, 'agent:slack'),
    commit('em-c-6', 'Priya: Q1 capacity estimate', { direction: 'waiting_on', person_id: 'em-p-priya', what: 'Q1 capacity estimate', promised_on: addDays(today, -6), due: addDays(today, -1), source: 'slack', status: 'open' }, -1, 'agent:slack'),
    commit('em-c-7', 'Leo: incident postmortem draft', { direction: 'waiting_on', person_id: 'em-p-leo', what: 'Incident postmortem draft', promised_on: addDays(today, -3), due: addDays(today, 1), source: 'meeting', status: 'open' }, 1, 'agent:calendar'),
    commit('em-c-8', 'Finance: contractor budget sign-off', { direction: 'waiting_on', counterpart: 'Finance (Mo)', what: 'Contractor budget sign-off', promised_on: addDays(today, -9), due: addDays(today, -2), source: 'email', status: 'open' }, -2, 'agent:email'),
    commit('em-c-9', 'Jonas: design review notes', { direction: 'waiting_on', person_id: 'em-p-jonas', what: 'Design review notes for the sandbox', promised_on: addDays(today, -1), source: 'meeting', status: 'open' }, null, 'agent:calendar'),
    commit('em-c-10', 'Share the on-call rotation doc with Sam', { direction: 'i_owe', person_id: 'em-p-sam', what: 'Share the on-call rotation doc', promised_on: addDays(today, -7), source: 'meeting', status: 'done' }, -1, 'agent:calendar', 'done'),

    // One-on-ones and the week's calendar.
    ...ones,
    ...oneMeetings,
    standup(0),
    meeting(wk(0, '10:00'), 60, 'Q1 roadmap review', 'cross-team', 14, { url: 'https://meet.example.com/roadmap' }),
    meeting(wk(0, '13:00'), 30, 'Hiring sync', 'team', 4, { mine: true, recurring: true }),
    meeting(wk(0, '13:30'), 30, 'Design crit: sandbox host', 'cross-team', 9),
    meeting(wk(0, '14:00'), 45, 'Vendor call: observability', 'external', 5),
    meeting(wk(0, '15:30'), 90, 'Focus: planning doc', 'focus', 1, { mine: true }),
    standup(1),
    meeting(wk(1, '11:30'), 45, 'Interview: backend loop', 'external', 3),
    meeting(wk(1, '13:30'), 60, 'Incident review', 'cross-team', 10, { url: 'https://meet.example.com/incident' }),
    meeting(wk(1, '14:30'), 30, 'Staff engineers sync', 'team', 6, { recurring: true }),
    meeting(wk(1, '15:00'), 30, 'Eng managers sync', 'cross-team', 8, { recurring: true }),
    standup(2),
    meeting(wk(2, '10:00'), 90, 'Platform planning', 'team', 7, { mine: true }),
    meeting(wk(2, '13:00'), 60, 'Focus: headcount ask', 'focus', 1, { mine: true }),
    meeting(wk(2, '14:00'), 30, 'Recruiting debrief', 'team', 5),
    meeting(wk(2, '16:00'), 45, '1:1 with Mo (director)', '1on1', 2, { recurring: true }),
    standup(3),
    meeting(wk(3, '09:30'), 60, 'Architecture forum', 'cross-team', 20, { recurring: true }),
    meeting(wk(3, '10:30'), 30, 'Security review', 'cross-team', 6),
    meeting(wk(3, '11:00'), 45, 'Customer call: Nordwind', 'external', 6),
    meeting(wk(3, '14:00'), 90, 'Focus: offsite plan', 'focus', 1, { mine: true }),
    standup(4),
    meeting(wk(4, '11:00'), 60, 'Team demo', 'team', 15, { mine: true, recurring: true }),
    meeting(wk(4, '15:00'), 30, 'Weekly retro', 'team', 7, { mine: true, recurring: true }),

    // Team-level delivery signals (GitHub and incident integrations).
    ...signals,

    // Private notes: never in search, activity or agent summaries.
    note('em-n-1', 'em-p-sam', 'Mentioned feeling stretched by on-call since the rotation change. Check in again in two weeks.', 7),
    note('em-n-2', 'em-p-leo', 'Wants more ownership after the incident; a bit nervous about running the postmortem meeting.', 11),
    note('em-n-3', 'em-p-dana', 'Packet is strong on design. Needs a clearer impact story for the sandbox work before the committee.', 3),
  ];
}
