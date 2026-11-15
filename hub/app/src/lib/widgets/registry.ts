import ListChecks from '@lucide/svelte/icons/list-checks';
import CalendarRange from '@lucide/svelte/icons/calendar-range';
import Crosshair from '@lucide/svelte/icons/crosshair';
import PenLine from '@lucide/svelte/icons/pen-line';
import Gauge from '@lucide/svelte/icons/gauge';
import Timer from '@lucide/svelte/icons/timer';
import CircleDot from '@lucide/svelte/icons/circle-dot';
import Columns3 from '@lucide/svelte/icons/columns-3';
import Clock from '@lucide/svelte/icons/clock';
import Activity from '@lucide/svelte/icons/activity';
import GitPullRequest from '@lucide/svelte/icons/git-pull-request';
import Radar from '@lucide/svelte/icons/radar';
import Users from '@lucide/svelte/icons/users';
import MessagesSquare from '@lucide/svelte/icons/messages-square';
import HeartPulse from '@lucide/svelte/icons/heart-pulse';
import FileText from '@lucide/svelte/icons/file-text';
import Handshake from '@lucide/svelte/icons/handshake';
import CalendarClock from '@lucide/svelte/icons/calendar-clock';
import Landmark from '@lucide/svelte/icons/landmark';
import Send from '@lucide/svelte/icons/send';
import ShieldCheck from '@lucide/svelte/icons/shield-check';
import TrendingDown from '@lucide/svelte/icons/trending-down';
import Zap from '@lucide/svelte/icons/zap';
import Puzzle from '@lucide/svelte/icons/puzzle';
import GitPullRequestArrow from '@lucide/svelte/icons/git-pull-request-arrow';
import History from '@lucide/svelte/icons/history';
import Brain from '@lucide/svelte/icons/brain';
import MessageCircleQuestion from '@lucide/svelte/icons/message-circle-question';
import CalendarDays from '@lucide/svelte/icons/calendar-days';
import Presentation from '@lucide/svelte/icons/presentation';
import ScanSearch from '@lucide/svelte/icons/scan-search';
import Sigma from '@lucide/svelte/icons/sigma';

import type { WidgetComponent, WidgetEntry } from './types';
import TaskList from './TaskList.svelte';
import Agenda from './Agenda.svelte';
import Now from './Now.svelte';
import QuickCapture from './QuickCapture.svelte';
import MetricTile from './MetricTile.svelte';
import CountdownWidget from './CountdownWidget.svelte';
import RingRoster from './RingRoster.svelte';
import Kanban from './Kanban.svelte';
import Timeline from './Timeline.svelte';
import ActivityFeed from './Activity.svelte';
import Fallback from './Fallback.svelte';
import ExtFrame from './ext/ExtFrame.svelte';
import Computed from './ext/Computed.svelte';
import { COMPUTED_CONFIG } from './ext/computed';
import { parseExtKind } from './ext/bridge';
import PrQueue from './dev/PrQueue.svelte';
import MyPrs from './dev/MyPrs.svelte';
import CiRadar from './dev/CiRadar.svelte';
import ContextRestore from './dev/ContextRestore.svelte';
import FocusProtector from './dev/FocusProtector.svelte';
import AskCatcher from './dev/AskCatcher.svelte';
import DayPlan from './dev/DayPlan.svelte';
import { DEV_CONFIG } from './dev/config';
import PeopleRoster from './em/PeopleRoster.svelte';
import Commitments from './em/Commitments.svelte';
import OneOnOne from './em/OneOnOne.svelte';
import MeetingLoad from './em/MeetingLoad.svelte';
import TeamPulse from './em/TeamPulse.svelte';
import WeeklyUpdate from './em/WeeklyUpdate.svelte';
import { EM_CONFIG } from './em/config';
import DeadlineWall from './cso/DeadlineWall.svelte';
import DataChase from './cso/DataChase.svelte';
import EvidenceLocker from './cso/EvidenceLocker.svelte';
import Trajectory from './cso/Trajectory.svelte';
import GridCarbon from './cso/GridCarbon.svelte';
import RegRadar from './cso/RegRadar.svelte';
import BoardBrief from './cso/BoardBrief.svelte';
import { CSO_CONFIG } from './cso/config';

type Def = Omit<WidgetEntry, 'kind' | 'minW' | 'minH' | 'ambient'> & Partial<Pick<WidgetEntry, 'minW' | 'minH' | 'ambient'>>;

const defs: Record<string, Def> = {
  // Generic kinds
  'task-list': { title: 'Tasks', icon: ListChecks, component: TaskList, minH: 3 },
  agenda: { title: 'Agenda', icon: CalendarRange, component: Agenda, minH: 3 },
  now: { title: 'Now', icon: Crosshair, component: Now, minW: 3 },
  'quick-capture': { title: 'Capture', icon: PenLine, component: QuickCapture, ambient: 'hide', minH: 2 },
  'metric-tile': { title: 'Metric', icon: Gauge, component: MetricTile },
  countdown: { title: 'Countdown', icon: Timer, component: CountdownWidget },
  'ring-roster': { title: 'Roster', icon: CircleDot, component: RingRoster },
  kanban: { title: 'Board', icon: Columns3, component: Kanban, minW: 4, minH: 3 },
  timeline: { title: 'Timeline', icon: Clock, component: Timeline },
  activity: { title: 'Activity', icon: Activity, component: ActivityFeed },
  // Developer pack (hub/packs/developer)
  'pr-queue': { title: 'Review queue', icon: GitPullRequest, component: PrQueue, minW: 3, config: DEV_CONFIG['pr-queue'] },
  'my-prs': { title: 'My pull requests', icon: GitPullRequestArrow, component: MyPrs, minW: 3, config: DEV_CONFIG['my-prs'] },
  'ci-radar': { title: 'CI radar', icon: Radar, component: CiRadar, minW: 3, config: DEV_CONFIG['ci-radar'] },
  'context-restore': { title: 'Where was I?', icon: History, component: ContextRestore, minW: 3, minH: 3, config: DEV_CONFIG['context-restore'] },
  'focus-protector': { title: 'Deep work', icon: Brain, component: FocusProtector, minH: 3, config: DEV_CONFIG['focus-protector'] },
  'ask-catcher': { title: 'Asks', icon: MessageCircleQuestion, component: AskCatcher, minW: 3, minH: 3, config: DEV_CONFIG['ask-catcher'] },
  'day-plan': { title: 'Day plan', icon: CalendarDays, component: DayPlan, minH: 4, config: DEV_CONFIG['day-plan'] },
  // Engineering manager pack (hub/packs/eng-manager)
  'people-roster': { title: 'People', icon: Users, component: PeopleRoster, minW: 3, minH: 3, config: EM_CONFIG['people-roster'] },
  commitments: { title: 'Commitments', icon: Handshake, component: Commitments, minW: 3, minH: 3, config: EM_CONFIG.commitments },
  'one-on-one': { title: 'One-on-one', icon: MessagesSquare, component: OneOnOne, minW: 4, minH: 4, config: EM_CONFIG['one-on-one'] },
  'meeting-load': { title: 'Meeting load', icon: CalendarClock, component: MeetingLoad, minW: 3, minH: 3, config: EM_CONFIG['meeting-load'] },
  'team-pulse': { title: 'Team pulse', icon: HeartPulse, component: TeamPulse, minW: 3, minH: 3, config: EM_CONFIG['team-pulse'] },
  'weekly-update': { title: 'Weekly update', icon: FileText, component: WeeklyUpdate, minW: 3, minH: 3, config: EM_CONFIG['weekly-update'] },
  // Chief Sustainability Officer pack (hub/packs/cso)
  'deadline-wall': { title: 'Disclosure deadlines', icon: Landmark, component: DeadlineWall, minW: 3, minH: 3, config: CSO_CONFIG['deadline-wall'] },
  'data-chase': { title: 'Data chase', icon: Send, component: DataChase, minW: 4, minH: 3, config: CSO_CONFIG['data-chase'] },
  evidence: { title: 'Evidence locker', icon: ShieldCheck, component: EvidenceLocker, minW: 3, minH: 3, config: CSO_CONFIG.evidence },
  trajectory: { title: 'Trajectory', icon: TrendingDown, component: Trajectory, minW: 3, minH: 3, config: CSO_CONFIG.trajectory },
  'grid-carbon': { title: 'Grid carbon', icon: Zap, component: GridCarbon, minW: 2, minH: 3, config: CSO_CONFIG['grid-carbon'] },
  'reg-radar': { title: 'Regulatory radar', icon: ScanSearch, component: RegRadar, minW: 3, minH: 3, config: CSO_CONFIG['reg-radar'] },
  'board-brief': { title: 'Board brief', icon: Presentation, component: BoardBrief, minW: 3, minH: 3, config: CSO_CONFIG['board-brief'] },
};

export const registry: ReadonlyMap<string, WidgetEntry> = new Map(
  Object.entries(defs).map(([kind, d]) => [kind, { kind, ambient: 'show', minW: 2, minH: 2, ...d }]),
);

export const FALLBACK: WidgetEntry = {
  kind: '__fallback',
  title: 'Unknown widget',
  icon: Puzzle,
  component: Fallback as WidgetComponent,
  ambient: 'hide',
  minW: 2,
  minH: 2,
};

/**
 * Tier C widgets (`x.<pack_id>.<name>`, P5-EXTENSIONS.md §3.1, §6): a pack's
 * page in a sandboxed frame. One entry for all of them; ExtFrame reads the
 * pack and widget from the kind. On Windows it renders a placeholder.
 */
export const EXT_ENTRY: WidgetEntry = {
  kind: '__ext',
  title: 'Custom widget',
  icon: Puzzle,
  component: ExtFrame as WidgetComponent,
  ambient: 'show',
  minW: 2,
  minH: 2,
};

/**
 * Tier B widgets (`computed`, P5-EXTENSIONS.md §5): a pack transform's
 * output, rendered by the host as text. Not platform-limited. Its config
 * (`{transform: "<pack_id>/<name>", view}`) is checked again by the widget.
 */
export const COMPUTED_KIND = 'computed';
export const COMPUTED_ENTRY: WidgetEntry = {
  kind: COMPUTED_KIND,
  title: 'Computed',
  icon: Sigma,
  component: Computed as WidgetComponent,
  ambient: 'show',
  minW: 2,
  minH: 2,
  config: COMPUTED_CONFIG,
};

/** Look up a renderer. Unknown or malformed kinds get the safe fallback. */
export function resolveWidget(kind: unknown): WidgetEntry {
  if (typeof kind !== 'string') return FALLBACK;
  // Own-property lookup only: no prototype keys like "constructor".
  const known = registry.get(kind);
  if (known) return known;
  if (kind === COMPUTED_KIND) return COMPUTED_ENTRY;
  if (parseExtKind(kind)) return EXT_ENTRY;
  return FALLBACK;
}

export function isKnownKind(kind: string): boolean {
  return registry.has(kind) || kind === COMPUTED_KIND || parseExtKind(kind) !== null;
}

export const GENERIC_KINDS = ['task-list', 'agenda', 'now', 'quick-capture', 'metric-tile', 'countdown', 'ring-roster', 'kanban', 'timeline', 'activity'] as const;
export const DEV_KINDS = ['pr-queue', 'my-prs', 'ci-radar', 'context-restore', 'focus-protector', 'ask-catcher', 'day-plan'] as const;
export const EM_KINDS = ['people-roster', 'commitments', 'one-on-one', 'meeting-load', 'team-pulse', 'weekly-update'] as const;
export const CSO_KINDS = ['deadline-wall', 'data-chase', 'evidence', 'trajectory', 'grid-carbon', 'reg-radar', 'board-brief'] as const;
