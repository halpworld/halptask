// Realistic seed data for the three first-party roles. Dates are relative to
// "today" so the mock always looks like a live day.
//
// Like the real core, each root item belongs to a role through its `roles`
// field (children inherit it), so one store holds three people's days and
// widgets scope with the `role` filter of tasks.list.

import type { Item, Layout, Role, TaskType, TaskTypeView, Widget } from '../types';
import { addDays } from '../../util/dates';
import { EM_TYPES, EM_WIDGETS, emLayoutWidgets, emSeeds } from './seed-em';
import { CSO_TYPES, CSO_WIDGETS, csoLayoutWidgets, csoSeeds } from './seed-cso';

export { EM_TYPES } from './seed-em';
export { CSO_TYPES } from './seed-cso';

const SCHEMA = 'https://json-schema.org/draft/2020-12/schema';
const REPO = { type: 'string', pattern: '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$', maxLength: 200 };
const URI = { type: 'string', format: 'uri', maxLength: 2048 };
const STAMP = { type: 'string', format: 'date-time', maxLength: 40 };
const PR_NUMBER = { type: 'integer', minimum: 1, maximum: 1000000000 };
const COUNT = { type: 'integer', minimum: 0, maximum: 100000000 };
const text = (max: number, min = 1) => ({ type: 'string', minLength: min, maxLength: max });

/** The core's built-in types (halp-domain task_type.rs builtin_types). */
export const BASE_TYPES: TaskType[] = [
  {
    id: 'base.task',
    title: 'Task',
    icon: 'check',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: { priority: { enum: ['low', 'normal', 'high', 'urgent'] }, estimate_minutes: { type: 'integer', minimum: 0, maximum: 100000 }, url: URI },
      additionalProperties: false,
    },
    display: { primary_fields: ['priority'], badge_field: 'priority' },
  },
  { id: 'base.note', title: 'Note', icon: 'note', schema: { $schema: SCHEMA, type: 'object', additionalProperties: false }, display: { primary_fields: [] } },
  {
    id: 'base.link',
    title: 'Link',
    icon: 'link',
    schema: { $schema: SCHEMA, type: 'object', properties: { url: URI }, required: ['url'], additionalProperties: false },
    display: { primary_fields: ['url'] },
  },
];

/** The developer pack's types, exactly as in hub/packs/developer/pack.json. */
export const DEV_TYPES: TaskType[] = [
  {
    id: 'dev.pull_request',
    title: 'Pull request',
    icon: 'git-pull-request',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        repo: REPO,
        number: PR_NUMBER,
        url: URI,
        state: { enum: ['draft', 'open', 'review_requested', 'changes_requested', 'approved', 'merged', 'closed'] },
        ci: { enum: ['unknown', 'pending', 'passing', 'failing'] },
        author: text(100),
        reviewers: { type: 'array', maxItems: 30, items: text(100) },
        requested_from_me: { type: 'boolean' },
        authored_by_me: { type: 'boolean' },
        requested_at: STAMP,
        updated_at: STAMP,
        additions: COUNT,
        deletions: COUNT,
        branch: text(255),
        mergeable: { enum: ['clean', 'conflicting', 'blocked', 'unknown'] },
      },
      required: ['repo', 'number'],
      additionalProperties: false,
    },
    display: { primary_fields: ['repo', 'number', 'author'], badge_field: 'ci' },
  },
  {
    id: 'dev.ci_run',
    title: 'CI run',
    icon: 'radar',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        repo: REPO,
        branch: text(255),
        workflow: text(200),
        status: { enum: ['queued', 'running', 'passing', 'failing', 'cancelled'] },
        url: URI,
        started_at: STAMP,
        finished_at: STAMP,
        pr_number: PR_NUMBER,
      },
      required: ['repo', 'branch', 'workflow', 'status'],
      additionalProperties: false,
    },
    display: { primary_fields: ['repo', 'branch', 'workflow'], badge_field: 'status' },
  },
  {
    id: 'dev.ask',
    title: 'Ask',
    icon: 'message-circle-question',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        source: { enum: ['slack', 'email', 'github', 'manual'] },
        from: text(200),
        channel: text(200),
        permalink: URI,
        received_at: STAMP,
        reply_by: STAMP,
      },
      required: ['source', 'from', 'received_at'],
      additionalProperties: false,
    },
    display: { primary_fields: ['from', 'source'], badge_field: 'source' },
  },
  {
    id: 'dev.context',
    title: 'Context snapshot',
    icon: 'history',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: {
        repo: REPO,
        branch: text(255),
        files: { type: 'array', maxItems: 100, items: text(500) },
        note: { type: 'string', maxLength: 2000 },
        last_command: { type: 'string', maxLength: 500 },
        open_prs: { type: 'array', maxItems: 20, items: PR_NUMBER },
        task_id: { type: 'string', pattern: '^[A-Za-z0-9_-]+$', maxLength: 64 },
        captured_at: STAMP,
      },
      required: ['repo', 'branch', 'captured_at'],
      additionalProperties: false,
    },
    display: { primary_fields: ['repo', 'branch'] },
  },
  {
    id: 'dev.meeting',
    title: 'Meeting',
    icon: 'calendar-clock',
    schema: {
      $schema: SCHEMA,
      type: 'object',
      properties: { starts_at: STAMP, ends_at: STAMP, location: { type: 'string', maxLength: 200 }, url: URI, busy: { type: 'boolean' } },
      required: ['starts_at', 'ends_at'],
      additionalProperties: false,
    },
    display: { primary_fields: ['starts_at', 'location'] },
  },
];

export function seedRoles(): Role[] {
  return [
    {
      id: 'developer',
      title: 'Developer',
      accent: '#0e7490',
      layout_id: 'layout-developer',
      task_types: ['base.task', 'base.note', 'base.link', ...DEV_TYPES.map((t) => t.id)],
      // Mirrors hub/packs/developer/pack.json (tests/dev-seed.test.ts checks it).
      widgets: ['now', 'pr-queue', 'my-prs', 'ci-radar', 'context-restore', 'focus-protector', 'ask-catcher', 'day-plan', 'agenda', 'task-list', 'activity', 'quick-capture'],
    },
    {
      id: 'eng-manager',
      title: 'Engineering Manager',
      accent: '#b4531f',
      layout_id: 'layout-eng-manager',
      task_types: ['base.task', 'base.note', 'base.link', ...EM_TYPES.map((t) => t.id)],
      // Mirrors hub/packs/eng-manager/pack.json (tests/em-seed.test.ts checks it).
      widgets: EM_WIDGETS,
    },
    {
      id: 'cso',
      title: 'Chief Sustainability Officer',
      accent: '#4f7a1f',
      layout_id: 'layout-cso',
      task_types: ['base.task', 'base.note', 'base.link', ...CSO_TYPES.map((t) => t.id)],
      // Mirrors hub/packs/cso/pack.json (tests/cso-seed.test.ts checks it).
      widgets: CSO_WIDGETS,
    },
  ];
}

export function seedTypes(): TaskTypeView[] {
  return [
    ...BASE_TYPES.map((t) => ({ ...t, origin: 'builtin' })),
    ...DEV_TYPES.map((t) => ({ ...t, origin: 'developer' })),
    ...EM_TYPES.map((t) => ({ ...t, origin: 'eng-manager' })),
    ...CSO_TYPES.map((t) => ({ ...t, origin: 'cso' })),
  ];
}

interface ItemSeed {
  id: string;
  title: string;
  role: string;
  status?: Item['status'];
  due?: number | null;
  tags?: string[];
  notes?: string;
  type_id?: string;
  fields?: Item['fields'];
  parent_id?: string;
  kind?: Item['kind'];
  /** Who wrote it; synced items come from an integration's agent. */
  origin?: string;
}

const MINUTE = 60_000;

/** `now` rounded down to a quarter hour, plus `mins`: meeting times look like a calendar's. */
export function quarterSlot(nowIso: string, mins: number): string {
  const q = 15 * MINUTE;
  return new Date(Math.floor(Date.parse(nowIso) / q) * q + mins * MINUTE).toISOString();
}

export function seedItems(today: string, nowIso: string): Item[] {
  // Integration timestamps and meetings are relative to now, so `pnpm dev`
  // shows a live developer day at any hour.
  const at = (mins: number) => new Date(Date.parse(nowIso) + mins * MINUTE).toISOString();
  const slot = (mins: number) => quarterSlot(nowIso, mins);
  const seeds: ItemSeed[] = [
    // Developer: their own work...
    { id: 'dev-1', role: 'developer', title: 'Ship OAuth PKCE loopback flow', status: 'in_progress', due: 0, tags: ['auth', 'hub'], type_id: 'base.task', fields: { priority: 'high', estimate_minutes: 90 }, notes: 'Branch feat/pkce-loopback, 3 dirty files. Next step: retry on loopback port collision.' },
    { id: 'dev-1a', role: 'developer', parent_id: 'dev-1', title: 'Retry on loopback port collision', due: 0 },
    { id: 'dev-1b', role: 'developer', parent_id: 'dev-1', title: 'Store refresh token in OS keychain', status: 'done', due: 0 },
    // ...review requests (tasks: the integration closes them when the review lands)...
    { id: 'dev-2', role: 'developer', title: 'Review #482: event log compaction', due: 0, tags: ['review'], origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 482, url: 'https://github.com/halptask/hub/pull/482', state: 'review_requested', ci: 'passing', author: 'mira', reviewers: ['kenth', 'oskar'], requested_from_me: true, requested_at: at(-26 * 60), updated_at: at(-95), additions: 312, deletions: 120, branch: 'mira/log-compaction', mergeable: 'clean' } },
    { id: 'dev-3', role: 'developer', title: 'Fix flaky store migration test on Windows', due: -1, tags: ['ci', 'bug'] },
    { id: 'dev-4', role: 'developer', title: 'Triage Sentry spike in TUI importer', due: 0, tags: ['incident'], type_id: 'base.task', fields: { priority: 'urgent', estimate_minutes: 30 } },
    { id: 'dev-5', role: 'developer', title: 'Pair with Jonas on wasmtime fuel limits', due: 1, tags: ['pairing'] },
    { id: 'dev-6', role: 'developer', title: 'Write ADR: sandboxed widget iframe protocol', due: 2, tags: ['adr'] },
    { id: 'dev-7', role: 'developer', title: 'Profile day.state latency under 10k items', status: 'in_progress', due: 3, tags: ['perf'] },
    { id: 'dev-8', role: 'developer', title: 'Bump rusqlite and re-run SQLCipher benchmarks', due: 4, tags: ['deps'] },
    { id: 'dev-9', role: 'developer', title: 'Update CONTRIBUTING for hub/', status: 'done', due: -1, tags: ['docs'] },
    { id: 'dev-10', role: 'developer', title: 'Review #479: keyring fallback on Linux', due: 1, tags: ['review'], origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 479, url: 'https://github.com/halptask/hub/pull/479', state: 'review_requested', ci: 'failing', author: 'oskar', reviewers: ['kenth'], requested_from_me: true, requested_at: at(-52 * 60), updated_at: at(-14), additions: 45, deletions: 12, branch: 'oskar/keyring-linux', mergeable: 'clean' } },
    { id: 'dev-11', role: 'developer', title: 'Review #490: widget sandbox host', tags: ['review'], origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 490, url: 'https://github.com/halptask/hub/pull/490', state: 'review_requested', ci: 'pending', author: 'jonas', reviewers: ['kenth', 'mira'], requested_from_me: true, requested_at: at(-3 * 60), additions: 1840, deletions: 410, branch: 'jonas/widget-host', mergeable: 'unknown' } },
    { id: 'dev-12', role: 'developer', title: 'Review #212: shell status line colours', tags: ['review'], origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/halp-cli', number: 212, url: 'https://github.com/halptask/halp-cli/pull/212', state: 'open', ci: 'passing', author: 'priya', reviewers: ['kenth'], requested_from_me: true, requested_at: at(-9 * 60), additions: 64, deletions: 30, branch: 'priya/statusline', mergeable: 'clean' } },
    // ...asks people are waiting on (tasks)...
    { id: 'dev-ask-1', role: 'developer', title: 'Can you confirm the compaction threshold before I merge?', origin: 'agent:slack', type_id: 'dev.ask', fields: { source: 'slack', from: 'Mira Holm', channel: '#hub-dev', permalink: 'https://halptask.slack.com/archives/C01HUB/p1731', received_at: at(-3 * 60), reply_by: at(70) } },
    { id: 'dev-ask-2', role: 'developer', title: 'Got 10 minutes to look at the fuel-limit numbers?', origin: 'agent:slack', type_id: 'dev.ask', fields: { source: 'slack', from: 'Jonas Berg', channel: 'DM', received_at: at(-50) } },
    { id: 'dev-ask-3', role: 'developer', title: 'Security review notes for the MCP socket', origin: 'agent:email', type_id: 'dev.ask', fields: { source: 'email', from: 'Priya Nair', received_at: at(-20 * 60), reply_by: at(-60) } },
    { id: 'dev-ask-4', role: 'developer', title: 'Is the Linux keyring fallback safe without a session bus?', origin: 'agent:github', type_id: 'dev.ask', fields: { source: 'github', from: 'oskar', channel: 'halptask/hub#479', permalink: 'https://github.com/halptask/hub/pull/479#discussion_r1', received_at: at(-5 * 60), reply_by: at(6 * 60) } },
    { id: 'dev-ask-5', role: 'developer', title: 'Look into the flaky Windows runner (from standup)', type_id: 'dev.ask', fields: { source: 'manual', from: 'Standup', received_at: at(-2 * 60) } },
    // ...and what the integrations keep in sync (bullets: no status of their own).
    { id: 'dev-mine-1', role: 'developer', kind: 'bullet', title: 'PKCE loopback flow for desktop sign-in', origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 488, url: 'https://github.com/halptask/hub/pull/488', state: 'review_requested', ci: 'passing', author: 'kenth', reviewers: ['mira', 'oskar'], authored_by_me: true, updated_at: at(-40), additions: 268, deletions: 54, branch: 'feat/pkce-loopback', mergeable: 'clean' } },
    { id: 'dev-mine-2', role: 'developer', kind: 'bullet', title: 'Store refresh tokens in the OS keychain', origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 484, url: 'https://github.com/halptask/hub/pull/484', state: 'approved', ci: 'passing', author: 'kenth', reviewers: ['mira'], authored_by_me: true, updated_at: at(-2 * 60), additions: 140, deletions: 22, branch: 'feat/keychain-refresh', mergeable: 'clean' } },
    { id: 'dev-mine-3', role: 'developer', kind: 'bullet', title: 'Consent dialog: show argument hash', origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 480, url: 'https://github.com/halptask/hub/pull/480', state: 'changes_requested', ci: 'passing', author: 'kenth', reviewers: ['priya'], authored_by_me: true, updated_at: at(-6 * 60), additions: 88, deletions: 9, branch: 'feat/consent-hash', mergeable: 'conflicting' } },
    { id: 'dev-mine-4', role: 'developer', kind: 'bullet', title: 'Widget config schemas in the registry', origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 491, url: 'https://github.com/halptask/hub/pull/491', state: 'draft', ci: 'pending', author: 'kenth', authored_by_me: true, updated_at: at(-15), additions: 520, deletions: 130, branch: 'feat/widget-config' } },
    { id: 'dev-mine-5', role: 'developer', kind: 'bullet', title: 'Undo for layout moves', origin: 'agent:github', type_id: 'dev.pull_request', fields: { repo: 'halptask/hub', number: 476, url: 'https://github.com/halptask/hub/pull/476', state: 'merged', ci: 'passing', author: 'kenth', reviewers: ['oskar'], authored_by_me: true, updated_at: at(-5 * 60), additions: 96, deletions: 40, branch: 'feat/layout-undo' } },
    { id: 'dev-ci-1', role: 'developer', kind: 'bullet', title: 'ci on main', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/hub', branch: 'main', workflow: 'ci', status: 'passing', url: 'https://github.com/halptask/hub/actions/runs/101', started_at: at(-52), finished_at: at(-41) } },
    { id: 'dev-ci-2', role: 'developer', kind: 'bullet', title: 'release on main', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/hub', branch: 'main', workflow: 'release', status: 'passing', url: 'https://github.com/halptask/hub/actions/runs/97', started_at: at(-26 * 60), finished_at: at(-26 * 60 + 18) } },
    { id: 'dev-ci-3', role: 'developer', kind: 'bullet', title: 'ci on oskar/keyring-linux', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/hub', branch: 'oskar/keyring-linux', workflow: 'ci', status: 'failing', url: 'https://github.com/halptask/hub/actions/runs/103', started_at: at(-26), finished_at: at(-14), pr_number: 479 } },
    { id: 'dev-ci-4', role: 'developer', kind: 'bullet', title: 'ci on feat/pkce-loopback', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/hub', branch: 'feat/pkce-loopback', workflow: 'ci', status: 'passing', url: 'https://github.com/halptask/hub/actions/runs/102', started_at: at(-48), finished_at: at(-37), pr_number: 488 } },
    { id: 'dev-ci-5', role: 'developer', kind: 'bullet', title: 'ci on main (earlier)', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/halp-cli', branch: 'main', workflow: 'ci', status: 'failing', url: 'https://github.com/halptask/halp-cli/actions/runs/88', started_at: at(-3 * 60), finished_at: at(-3 * 60 + 9) } },
    { id: 'dev-ci-6', role: 'developer', kind: 'bullet', title: 'ci on main', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/halp-cli', branch: 'main', workflow: 'ci', status: 'running', url: 'https://github.com/halptask/halp-cli/actions/runs/89', started_at: at(-4) } },
    { id: 'dev-ci-7', role: 'developer', kind: 'bullet', title: 'pages on main', origin: 'agent:github', type_id: 'dev.ci_run', fields: { repo: 'halptask/docs', branch: 'main', workflow: 'pages', status: 'passing', url: 'https://github.com/halptask/docs/actions/runs/12', started_at: at(-7 * 60), finished_at: at(-7 * 60 + 3) } },
    { id: 'dev-ctx-1', role: 'developer', kind: 'bullet', title: 'feat/pkce-loopback', origin: 'agent:git', type_id: 'dev.context', fields: { repo: 'halptask/hub', branch: 'feat/pkce-loopback', files: ['crates/halp-auth/src/loopback.rs', 'crates/halp-auth/src/pkce.rs', 'crates/halp-auth/tests/loopback.rs'], note: 'Port collision: bind 0 and read the port back instead of retrying a fixed range.', last_command: 'cargo test -p halp-auth loopback -- --nocapture', open_prs: [488], task_id: 'dev-1', captured_at: at(-25) } },
    { id: 'dev-ctx-2', role: 'developer', kind: 'bullet', title: 'feat/pkce-loopback (earlier)', origin: 'agent:git', type_id: 'dev.context', fields: { repo: 'halptask/hub', branch: 'feat/pkce-loopback', files: ['crates/halp-auth/src/pkce.rs'], captured_at: at(-4 * 60), task_id: 'dev-1' } },
    { id: 'dev-ctx-3', role: 'developer', kind: 'bullet', title: 'fix/windows-migration', origin: 'agent:git', type_id: 'dev.context', fields: { repo: 'halptask/halp-cli', branch: 'fix/windows-migration', files: ['src/store/migrate.rs', 'tests/migrate_windows.rs'], note: 'Fails only when the temp dir has a space in it.', last_command: 'cargo test migrate_windows', task_id: 'dev-3', captured_at: at(-22 * 60) } },
    { id: 'dev-ctx-4', role: 'developer', kind: 'bullet', title: 'perf/day-state', origin: 'agent:git', type_id: 'dev.context', fields: { repo: 'halptask/bench', branch: 'perf/day-state', files: ['benches/day_state.rs'], last_command: 'cargo bench day_state', task_id: 'dev-7', captured_at: at(-50 * 60) } },
    { id: 'dev-mtg-1', role: 'developer', kind: 'bullet', title: 'Hub standup', origin: 'agent:calendar', type_id: 'dev.meeting', fields: { starts_at: slot(-120), ends_at: slot(-105), location: 'Huddle', busy: true } },
    { id: 'dev-mtg-2', role: 'developer', kind: 'bullet', title: 'Design review: widget sandbox', origin: 'agent:calendar', type_id: 'dev.meeting', fields: { starts_at: slot(45), ends_at: slot(90), location: 'Room Fika', url: 'https://meet.example.com/hub-design', busy: true } },
    { id: 'dev-mtg-3', role: 'developer', kind: 'bullet', title: '1:1 with Kim', origin: 'agent:calendar', type_id: 'dev.meeting', fields: { starts_at: slot(150), ends_at: slot(180), busy: true } },
    { id: 'dev-mtg-4', role: 'developer', kind: 'bullet', title: 'Lunch talk (optional)', origin: 'agent:calendar', type_id: 'dev.meeting', fields: { starts_at: slot(195), ends_at: slot(225), location: 'Kitchen', busy: false } },
    { id: 'dev-mtg-5', role: 'developer', kind: 'bullet', title: 'Release sync', origin: 'agent:calendar', type_id: 'dev.meeting', fields: { starts_at: slot(270), ends_at: slot(300), url: 'https://meet.example.com/release', busy: true } },

    // CSO: a fictional manufacturer's sustainability office (seed-cso.ts, demo data).
    ...csoSeeds(today, nowIso).map((e) => ({ ...e, role: 'cso' })),
    // Engineering manager: people, meetings, commitments, 1:1s (seed-em.ts).
    ...emSeeds(today, nowIso, slot).map((e) => ({ ...e, role: 'eng-manager' })),
  ];

  return seeds.map((s, i) => ({
    id: s.id,
    parent_id: s.parent_id ?? null,
    order: i,
    kind: s.kind ?? 'task',
    status: s.status ?? 'todo',
    title: s.title,
    notes: s.notes ?? '',
    tags: s.tags ?? [],
    inherited_tags: [],
    due: s.due === null || s.due === undefined ? null : addDays(today, s.due),
    type_id: s.type_id ?? null,
    fields: s.fields ?? {},
    // Children inherit their parent's roles, as in the core.
    roles: s.parent_id ? [] : [s.role],
    origin: s.origin ?? 'user',
    created_at: nowIso,
    updated_at: nowIso,
    version: 1,
  }));
}

function w(id: string, kind: string, title: string, x: number, y: number, ww: number, h: number, config: Widget['config'] = {}): Widget {
  return { id, kind, title, x, y, w: ww, h, config };
}

/** Local `HH:MM` of `iso` moved by `mins`, kept inside the same day. */
function hhmm(iso: string, mins: number): string {
  const base = new Date(iso);
  const d = new Date(base.getTime() + mins * MINUTE);
  if (d.toDateString() !== base.toDateString()) return mins < 0 ? '00:00' : '23:59';
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

export function seedLayouts(today: string, nowIso = `${today}T09:30:00`): Layout[] {
  return [
    {
      // The developer pack's default layout (hub/packs/developer/pack.json).
      // Only the day plan's window and the running focus session differ: they
      // follow "now" so the mock shows a working day at any hour.
      id: 'layout-developer',
      role_id: 'developer',
      columns: 12,
      widgets: [
        w('now', 'now', 'Now', 0, 0, 4, 3),
        w('pr-queue', 'pr-queue', 'Review queue', 4, 0, 5, 4, { stale_hours: 24 }),
        w('focus-protector', 'focus-protector', 'Deep work', 9, 0, 3, 3, {
          minutes: 50,
          session: { item_id: 'dev-1', started_at: new Date(Date.parse(nowIso) - 18 * MINUTE).toISOString(), minutes: 50 },
        }),
        w('context-restore', 'context-restore', 'Where was I?', 0, 3, 4, 3),
        w('day-plan', 'day-plan', 'Day plan', 9, 3, 3, 7, { day_start: hhmm(quarterSlot(nowIso, 0), -150), day_end: hhmm(quarterSlot(nowIso, 0), 330) }),
        w('my-prs', 'my-prs', 'My pull requests', 4, 4, 5, 3),
        w('ask-catcher', 'ask-catcher', 'Asks', 0, 6, 4, 4),
        w('ci-radar', 'ci-radar', 'CI radar', 4, 7, 5, 3),
      ],
    },
    {
      id: 'layout-eng-manager',
      role_id: 'eng-manager',
      columns: 12,
      // The eng-manager pack's default layout (hub/packs/eng-manager/pack.json).
      widgets: emLayoutWidgets(),
    },
    {
      id: 'layout-cso',
      role_id: 'cso',
      columns: 12,
      // The CSO pack's default layout (hub/packs/cso/pack.json).
      widgets: csoLayoutWidgets(),
    },
  ];
}
