// Pure logic for the CSO pack's widgets (hub/packs/cso).
//
// The CSO pack is an overlay: the carbon platform and the ERP stay the system
// of record. The Hub tracks who owes which data point, the evidence behind it,
// the deadlines and what the board will see. Integrations and agents write
// typed items; the widgets read them back with tasks.list and write only
// through existing commands. Values come from outside the app, so every
// reader re-checks types and skips malformed items.
//
// The one rule everything here keeps: no figure is ever made up. Nothing is
// estimated, interpolated or carried forward. A missing value is a gap and
// is shown as one; a value without a source is shown as unsourced.

import type { Item, JsonValue } from '../../api/types';
import { isArchived } from '../parts/archived';
import { diffDays } from '../../util/dates';
import { safeUrl, timeOf } from '../dev/logic';

export const CSO_TYPES = {
  dataRequest: 'cso.data_request',
  evidence: 'cso.evidence',
  disclosure: 'cso.disclosure',
  target: 'cso.target',
  regChange: 'cso.reg_change',
  brief: 'cso.board_brief',
  energy: 'cso.energy_reading',
} as const;

/** Data class of pre-disclosure figures (cso.board_brief). */
export const UNPUBLISHED_CLASS = 'cso.unpublished';

const MIN = 60_000;

// ------------------------------------------------------------ Field readers

type Fields = Record<string, JsonValue>;

const str = (f: Fields, k: string, max = 4000): string | undefined => {
  const v = f[k];
  return typeof v === 'string' && v.trim().length > 0 ? v.slice(0, max) : undefined;
};
const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v);
const num = (f: Fields, k: string): number | undefined => (finite(f[k]) ? (f[k] as number) : undefined);
const int = (f: Fields, k: string, lo: number, hi: number): number | undefined => {
  const v = f[k];
  return typeof v === 'number' && Number.isInteger(v) && v >= lo && v <= hi ? v : undefined;
};
function oneOf<T extends string>(f: Fields, k: string, allowed: readonly T[]): T | undefined {
  const v = f[k];
  return typeof v === 'string' && (allowed as readonly string[]).includes(v) ? (v as T) : undefined;
}
const DATE_RE = /^\d{4}-\d{2}-\d{2}$/;
const date = (f: Fields, k: string): string | undefined => {
  const v = f[k];
  return typeof v === 'string' && DATE_RE.test(v) ? v : undefined;
};
const stamp = (f: Fields, k: string): number | undefined => {
  const v = f[k];
  return typeof v === 'string' ? timeOf(v) : undefined;
};
const ITEM_ID_RE = /^[A-Za-z0-9_-]{1,64}$/;
const isRecord = (v: unknown): v is Fields => typeof v === 'object' && v !== null && !Array.isArray(v);
const usable = (item: Item, type: string) => item.type_id === type && !isArchived(item);

// ------------------------------------------------------------ Disclosures

export const FRAMEWORKS = ['csrd_esrs', 'sb253', 'sb261', 'issb', 'other'] as const;
export type Framework = (typeof FRAMEWORKS)[number];
export const FRAMEWORK_LABEL: Record<Framework, string> = {
  csrd_esrs: 'CSRD / ESRS',
  sb253: 'SB 253',
  sb261: 'SB 261',
  issb: 'ISSB',
  other: 'Other',
};

export const CERTAINTIES = ['confirmed', 'proposed', 'litigated', 'delayed'] as const;
export type Certainty = (typeof CERTAINTIES)[number];
export const DISCLOSURE_STATUSES = ['not_started', 'drafting', 'in_review', 'assured', 'filed'] as const;
export type DisclosureStatus = (typeof DISCLOSURE_STATUSES)[number];

export interface Disclosure {
  item: Item;
  framework: Framework;
  name: string;
  jurisdiction?: string;
  period?: string;
  due: string;
  certainty: Certainty;
  certaintyNote?: string;
  sourceUrl: string | null;
  owner?: string;
  status: DisclosureStatus;
}

export function readDisclosure(item: Item): Disclosure | null {
  if (!usable(item, CSO_TYPES.disclosure)) return null;
  const f = item.fields;
  const framework = oneOf(f, 'framework', FRAMEWORKS);
  const due = date(f, 'due') ?? item.due ?? undefined;
  // An unknown or missing certainty is never shown as confirmed.
  const certainty = oneOf(f, 'regulatory_certainty', CERTAINTIES) ?? 'proposed';
  if (!framework || !due) return null;
  return {
    item,
    framework,
    name: str(f, 'name', 200) ?? (item.title.trim() || FRAMEWORK_LABEL[framework]),
    jurisdiction: str(f, 'jurisdiction', 80),
    period: str(f, 'reporting_period', 7),
    due,
    certainty,
    certaintyNote: str(f, 'certainty_note', 500),
    sourceUrl: safeUrl(f.source_url),
    owner: str(f, 'owner', 120),
    status: item.status === 'done' ? 'filed' : (oneOf(f, 'status', DISCLOSURE_STATUSES) ?? 'not_started'),
  };
}

export type DeadlineState = 'overdue' | 'today' | 'soon' | 'later' | 'filed';

export interface DeadlineRow {
  d: Disclosure;
  daysLeft: number;
  state: DeadlineState;
  /** Only `confirmed` deadlines are ever presented as certain. */
  certain: boolean;
}

/** Days ahead that count as "soon" on the wall. */
export const SOON_DAYS = 30;

export function deadlineRows(items: Item[], today: string, opts: { horizonDays?: number; showFiled?: boolean } = {}): DeadlineRow[] {
  const horizon = opts.horizonDays ?? 800;
  return items
    .map(readDisclosure)
    .filter((d): d is Disclosure => !!d)
    .map((d) => {
      const daysLeft = diffDays(d.due, today);
      const state: DeadlineState = d.status === 'filed' ? 'filed' : daysLeft < 0 ? 'overdue' : daysLeft === 0 ? 'today' : daysLeft <= SOON_DAYS ? 'soon' : 'later';
      return { d, daysLeft, state, certain: d.certainty === 'confirmed' };
    })
    .filter((r) => (r.state === 'filed' ? !!opts.showFiled : r.daysLeft <= horizon))
    .sort((a, b) => Number(a.state === 'filed') - Number(b.state === 'filed') || a.daysLeft - b.daysLeft || a.d.name.localeCompare(b.d.name));
}

// ------------------------------------------------------------ Data requests

export const REQUEST_STATUSES = ['requested', 'chased', 'received', 'validated', 'rejected'] as const;
export type RequestStatus = (typeof REQUEST_STATUSES)[number];
export const SCOPES = ['scope1', 'scope2', 'scope3'] as const;
export type Scope = (typeof SCOPES)[number];

export interface DataRequest {
  item: Item;
  supplier: string;
  owner?: string;
  scope?: Scope;
  category?: number;
  metric: string;
  unit?: string;
  period?: string;
  due?: string;
  requestedOn?: string;
  status: RequestStatus;
  chaseCount: number;
  lastChased?: number;
  sourceRef?: string;
}

export function readDataRequest(item: Item): DataRequest | null {
  if (!usable(item, CSO_TYPES.dataRequest)) return null;
  const f = item.fields;
  const supplier = str(f, 'supplier', 200);
  const metric = str(f, 'metric', 200);
  if (!supplier || !metric) return null;
  return {
    item,
    supplier,
    owner: str(f, 'owner', 120),
    scope: oneOf(f, 'scope', SCOPES),
    category: int(f, 'category', 1, 15),
    metric,
    unit: str(f, 'unit', 40),
    period: str(f, 'period', 7),
    due: date(f, 'due') ?? item.due ?? undefined,
    requestedOn: date(f, 'requested_on'),
    status: oneOf(f, 'status', REQUEST_STATUSES) ?? 'requested',
    chaseCount: int(f, 'chase_count', 0, 1000) ?? 0,
    lastChased: stamp(f, 'last_chased'),
    sourceRef: str(f, 'source_ref', 200),
  };
}

export const isOpenRequest = (r: Pick<DataRequest, 'status'>) => r.status === 'requested' || r.status === 'chased';

/** "Scope 3 · cat 4", "Scope 1". */
export function scopeLabel(r: Pick<DataRequest, 'scope' | 'category'>): string | undefined {
  if (!r.scope) return undefined;
  const s = `Scope ${r.scope.slice(-1)}`;
  return r.scope === 'scope3' && r.category ? `${s} · cat ${r.category}` : s;
}

export interface ChaseCard {
  r: DataRequest;
  /** Days past due (positive) when still open, else null. */
  overdueDays: number | null;
  dueInDays: number | null;
  /** Whole days since the last chase, or null if never chased. */
  sinceChase: number | null;
  /** Open, and not chased within `chaseAfterDays`: worth a chase now. */
  chaseDue: boolean;
}

export interface ChaseColumn {
  status: RequestStatus;
  cards: ChaseCard[];
  overdue: number;
}

export const DEFAULT_CHASE_AFTER = 7;

export function chaseCard(r: DataRequest, today: string, now: number, chaseAfterDays = DEFAULT_CHASE_AFTER): ChaseCard {
  const open = isOpenRequest(r);
  const dueInDays = r.due ? diffDays(r.due, today) : null;
  const overdueDays = open && dueInDays !== null && dueInDays < 0 ? -dueInDays : null;
  const sinceChase = r.lastChased !== undefined ? Math.max(0, Math.floor((now - r.lastChased) / 86_400_000)) : null;
  const lastTouch = sinceChase ?? (r.requestedOn ? Math.max(0, diffDays(today, r.requestedOn)) : null);
  const chaseDue = open && (overdueDays !== null || (lastTouch !== null && lastTouch >= chaseAfterDays));
  return { r, overdueDays, dueInDays, sinceChase, chaseDue };
}

/** Most overdue first, then soonest due, then supplier. */
function cardOrder(a: ChaseCard, b: ChaseCard): number {
  const ao = a.overdueDays ?? -1;
  const bo = b.overdueDays ?? -1;
  if (ao !== bo) return bo - ao;
  const ad = a.dueInDays ?? Number.POSITIVE_INFINITY;
  const bd = b.dueInDays ?? Number.POSITIVE_INFINITY;
  return ad - bd || a.r.supplier.localeCompare(b.r.supplier) || a.r.metric.localeCompare(b.r.metric);
}

/** One column per status, in workflow order, always all five. */
export function chaseBoard(items: Item[], today: string, now: number, chaseAfterDays = DEFAULT_CHASE_AFTER): ChaseColumn[] {
  const cards = items
    .map(readDataRequest)
    .filter((r): r is DataRequest => !!r)
    .map((r) => chaseCard(r, today, now, chaseAfterDays));
  return REQUEST_STATUSES.map((status) => {
    const col = cards.filter((c) => c.r.status === status).sort(cardOrder);
    return { status, cards: col, overdue: col.filter((c) => c.overdueDays !== null).length };
  });
}

/**
 * The fields after one chase: chase_count + 1, last_chased = now, and a
 * `requested` request becomes `chased`. Every other field is kept.
 * Returns null for a request that is not open (nothing to chase).
 */
export function chasePatch(fields: Fields, nowIso: string): Fields | null {
  const status = oneOf(fields, 'status', REQUEST_STATUSES) ?? 'requested';
  if (status !== 'requested' && status !== 'chased') return null;
  const count = int(fields, 'chase_count', 0, 1000) ?? 0;
  return { ...fields, status: 'chased', chase_count: Math.min(1000, count + 1), last_chased: nowIso };
}

/** The fields with a new status (moving a card on the board). */
export function statusPatch(fields: Fields, status: RequestStatus): Fields {
  return { ...fields, status };
}

// ------------------------------------------------------------ Evidence

export const EVIDENCE_STATUSES = ['draft', 'in_review', 'approved', 'rejected'] as const;
export type EvidenceStatus = (typeof EVIDENCE_STATUSES)[number];
export const DECISIONS = ['approved', 'rejected', 'changes_requested'] as const;
export type Decision = (typeof DECISIONS)[number];

export interface Approval {
  reviewer: string;
  decision: Decision;
  at: number;
  note?: string;
}

export interface Evidence {
  item: Item;
  fileName: string;
  /** Lowercase hex, exactly 64 characters, or null when malformed. */
  sha256: string | null;
  sizeBytes?: number;
  sourceSystem?: string;
  dataPointId?: string;
  dataPoint?: string;
  status: EvidenceStatus;
  approvals: Approval[];
}

const SHA256_RE = /^[0-9a-f]{64}$/;

export function readEvidence(item: Item): Evidence | null {
  if (!usable(item, CSO_TYPES.evidence)) return null;
  const f = item.fields;
  const fileName = str(f, 'file_name', 255);
  if (!fileName) return null;
  const hash = typeof f.sha256 === 'string' && SHA256_RE.test(f.sha256) ? f.sha256 : null;
  const approvals: Approval[] = (Array.isArray(f.approvals) ? f.approvals : [])
    .filter(isRecord)
    .flatMap((a): Approval[] => {
      const reviewer = str(a, 'reviewer', 120);
      const decision = oneOf(a, 'decision', DECISIONS);
      const at = stamp(a, 'at');
      return reviewer && decision && at !== undefined ? [{ reviewer, decision, at, note: str(a, 'note', 500) }] : [];
    })
    .slice(0, 20)
    .sort((a, b) => a.at - b.at);
  const did = str(f, 'data_point_id', 64);
  return {
    item,
    fileName,
    sha256: hash,
    sizeBytes: int(f, 'size_bytes', 0, 1e12),
    sourceSystem: str(f, 'source_system', 120),
    dataPointId: did && ITEM_ID_RE.test(did) ? did : undefined,
    dataPoint: str(f, 'data_point', 200),
    status: oneOf(f, 'status', EVIDENCE_STATUSES) ?? 'draft',
    approvals,
  };
}

/** `9f2c41ab…e01d`: enough to compare by eye, the full hash is copied. */
export function shortHash(hash: string | null): string {
  if (!hash) return 'no hash';
  return `${hash.slice(0, 8)}…${hash.slice(-4)}`;
}

/** Approved evidence is immutable: the UI offers no edits, agents must not write it. */
export const isLocked = (e: Pick<Evidence, 'status'>) => e.status === 'approved';

export type ChainState = 'approved' | 'pending' | 'rejected' | 'changes' | 'none' | 'inconsistent';

/**
 * What the approval chain says. "inconsistent" when the status claims
 * approved but no reviewer approved it (or the hash is missing).
 */
export function chainState(e: Evidence): ChainState {
  const last = e.approvals.at(-1);
  if (e.status === 'approved') return last?.decision === 'approved' && e.sha256 ? 'approved' : 'inconsistent';
  if (!last) return e.status === 'rejected' ? 'rejected' : 'none';
  if (last.decision === 'rejected' || e.status === 'rejected') return 'rejected';
  if (last.decision === 'changes_requested') return 'changes';
  return 'pending';
}

export interface EvidenceGap {
  r: DataRequest;
  /** No evidence at all, or only evidence still waiting / rejected. */
  kind: 'missing' | 'unapproved';
}

/**
 * Data points (requests that delivered a value: received or validated) and
 * whether evidence backs them. Rejected evidence does not count.
 */
export function evidenceGaps(requests: DataRequest[], evidence: Evidence[]): EvidenceGap[] {
  const byPoint = new Map<string, Evidence[]>();
  for (const e of evidence) {
    if (!e.dataPointId || e.status === 'rejected') continue;
    byPoint.set(e.dataPointId, [...(byPoint.get(e.dataPointId) ?? []), e]);
  }
  const gaps: EvidenceGap[] = [];
  for (const r of requests) {
    if (r.status !== 'received' && r.status !== 'validated') continue;
    const ev = byPoint.get(r.item.id) ?? [];
    if (ev.length === 0) gaps.push({ r, kind: 'missing' });
    else if (!ev.some((e) => chainState(e) === 'approved')) gaps.push({ r, kind: 'unapproved' });
  }
  return gaps.sort((a, b) => Number(a.kind === 'unapproved') - Number(b.kind === 'unapproved') || a.r.supplier.localeCompare(b.r.supplier));
}

/** Evidence in review first, then the rest, newest decision first. */
export function evidenceRows(evidence: Evidence[]): Evidence[] {
  const rank: Record<ChainState, number> = { inconsistent: 0, changes: 1, pending: 2, none: 2, rejected: 3, approved: 4 };
  return [...evidence].sort((a, b) => rank[chainState(a)] - rank[chainState(b)] || (b.approvals.at(-1)?.at ?? 0) - (a.approvals.at(-1)?.at ?? 0) || a.fileName.localeCompare(b.fileName));
}

// ------------------------------------------------------------ Targets and trajectory

export const TARGET_SCOPES = ['scope1', 'scope2', 'scope1_2', 'scope3', 'scope1_2_3'] as const;
export type TargetScope = (typeof TARGET_SCOPES)[number];
export const TARGET_SCOPE_LABEL: Record<TargetScope, string> = {
  scope1: 'Scope 1',
  scope2: 'Scope 2',
  scope1_2: 'Scope 1 + 2',
  scope3: 'Scope 3',
  scope1_2_3: 'Scope 1 + 2 + 3',
};

export interface Actual {
  year: number;
  value: number;
  sourceRef?: string;
}

export interface Target {
  item: Item;
  name: string;
  scope: TargetScope;
  unit: string;
  validation?: 'none' | 'committed' | 'validated';
  baseYear: number;
  baseValue: number;
  targetYear: number;
  targetValue: number;
  /** One per year (the first given wins), sorted by year. */
  actuals: Actual[];
}

export function readTarget(item: Item): Target | null {
  if (!usable(item, CSO_TYPES.target)) return null;
  const f = item.fields;
  const scope = oneOf(f, 'scope', TARGET_SCOPES);
  const baseYear = int(f, 'base_year', 1990, 2100);
  const targetYear = int(f, 'target_year', 1990, 2100);
  const baseValue = num(f, 'base_value');
  const targetValue = num(f, 'target_value');
  if (!scope || baseYear === undefined || targetYear === undefined || baseValue === undefined || targetValue === undefined) return null;
  if (targetYear <= baseYear || baseValue < 0 || targetValue < 0) return null;
  const seen = new Set<number>();
  const actuals: Actual[] = [];
  for (const a of Array.isArray(f.actuals) ? f.actuals.filter(isRecord) : []) {
    const year = int(a, 'year', 1990, 2100);
    const value = num(a, 'value');
    if (year === undefined || value === undefined || value < 0 || seen.has(year)) continue;
    seen.add(year);
    actuals.push({ year, value, sourceRef: str(a, 'source_ref', 200) });
  }
  actuals.sort((a, b) => a.year - b.year);
  return {
    item,
    name: str(f, 'name', 120) ?? (item.title.trim() || TARGET_SCOPE_LABEL[scope]),
    scope,
    unit: str(f, 'unit', 40) ?? 'tCO2e',
    validation: oneOf(f, 'validation', ['none', 'committed', 'validated'] as const),
    baseYear,
    baseValue,
    targetYear,
    targetValue,
    actuals,
  };
}

export interface ChartPoint extends Actual {
  x: number;
  y: number;
  sourced: boolean;
}

export interface TrajectoryModel {
  width: number;
  height: number;
  /** Plot area inside the axes. */
  plot: { left: number; right: number; top: number; bottom: number };
  years: { year: number; x: number; label: boolean }[];
  ticks: { value: number; y: number }[];
  /** The declared pathway: a straight line from base to target. */
  pathway: { x1: number; y1: number; x2: number; y2: number };
  points: ChartPoint[];
  /** Polylines through consecutive years only: a missing year breaks the line. */
  segments: string[];
  /** Years between the first and last actual (and up to this year) with no figure. */
  gaps: { year: number; x: number }[];
  latestSourced?: ChartPoint;
  /** Latest sourced actual vs the base value, in percent (negative = lower). */
  changeVsBase?: number;
  /** Latest sourced actual vs the declared pathway in that year. */
  vsPathway?: 'below' | 'above';
}

const niceStep = (span: number, n: number) => {
  const raw = span / Math.max(1, n);
  const mag = 10 ** Math.floor(Math.log10(raw || 1));
  const norm = raw / mag;
  return (norm <= 1 ? 1 : norm <= 2 ? 2 : norm <= 5 ? 5 : 10) * mag;
};

/** Value on the declared base-to-target line (for the above/below flag only). */
export function pathwayAt(t: Pick<Target, 'baseYear' | 'baseValue' | 'targetYear' | 'targetValue'>, year: number): number {
  return t.baseValue + ((t.targetValue - t.baseValue) * (year - t.baseYear)) / (t.targetYear - t.baseYear);
}

/**
 * Geometry for the trajectory chart. Every point drawn is a reported figure;
 * the only line not made of reported figures is the declared pathway, which
 * is drawn dashed and labelled as the target.
 */
export function trajectoryModel(t: Target, currentYear: number, size: { width: number; height: number } = { width: 320, height: 160 }): TrajectoryModel {
  const { width, height } = size;
  const plot = { left: 40, right: width - 10, top: 10, bottom: height - 20 };
  const lastYear = Math.max(t.targetYear, ...t.actuals.map((a) => a.year));
  const firstYear = Math.min(t.baseYear, ...t.actuals.map((a) => a.year));
  const span = Math.max(1, lastYear - firstYear);
  const xOf = (y: number) => plot.left + ((y - firstYear) / span) * (plot.right - plot.left);
  const values = [t.baseValue, t.targetValue, ...t.actuals.map((a) => a.value)];
  const step = niceStep(Math.max(...values), 4);
  const top = Math.max(step, Math.ceil(Math.max(...values) / step) * step);
  const yOf = (v: number) => plot.bottom - (v / top) * (plot.bottom - plot.top);
  const every = span > 12 ? 5 : span > 6 ? 2 : 1;
  const years = Array.from({ length: span + 1 }, (_, i) => firstYear + i).map((year) => ({ year, x: xOf(year), label: year === firstYear || year === lastYear || (year - firstYear) % every === 0 }));
  const ticks: { value: number; y: number }[] = [];
  for (let v = 0; v <= top + step / 2; v += step) ticks.push({ value: v, y: yOf(v) });
  const points: ChartPoint[] = t.actuals.map((a) => ({ ...a, x: xOf(a.year), y: yOf(a.value), sourced: !!a.sourceRef }));
  const segments: string[] = [];
  let run: ChartPoint[] = [];
  const flush = () => {
    if (run.length > 1) segments.push(run.map((p, i) => `${i ? 'L' : 'M'}${p.x.toFixed(1)} ${p.y.toFixed(1)}`).join(' '));
    run = [];
  };
  for (const p of points) {
    if (run.length && p.year !== run[run.length - 1]!.year + 1) flush();
    run.push(p);
  }
  flush();
  const have = new Set(t.actuals.map((a) => a.year));
  const gapFrom = t.actuals.length ? t.actuals[0]!.year : t.baseYear;
  const gapTo = Math.min(currentYear - 1, t.targetYear);
  const gaps: { year: number; x: number }[] = [];
  for (let y = gapFrom; y <= gapTo; y++) if (!have.has(y)) gaps.push({ year: y, x: xOf(y) });
  const latestSourced = [...points].reverse().find((p) => p.sourced);
  const model: TrajectoryModel = {
    width,
    height,
    plot,
    years,
    ticks,
    pathway: { x1: xOf(t.baseYear), y1: yOf(t.baseValue), x2: xOf(t.targetYear), y2: yOf(t.targetValue) },
    points,
    segments,
    gaps,
    latestSourced,
  };
  if (latestSourced && t.baseValue > 0) {
    model.changeVsBase = ((latestSourced.value - t.baseValue) / t.baseValue) * 100;
    if (latestSourced.year >= t.baseYear) model.vsPathway = latestSourced.value <= pathwayAt(t, latestSourced.year) ? 'below' : 'above';
  }
  return model;
}

// ------------------------------------------------------------ Regulatory radar

export const REG_STAGES = ['proposed', 'consultation', 'adopted', 'in_force', 'delayed', 'litigated', 'withdrawn'] as const;
export type RegStage = (typeof REG_STAGES)[number];
export const IMPACTS = ['high', 'medium', 'low'] as const;
export type Impact = (typeof IMPACTS)[number];

export interface RegChange {
  item: Item;
  title: string;
  jurisdiction: string;
  instrument?: string;
  stage: RegStage;
  effectiveDate?: string;
  impact?: Impact;
  impactNote?: string;
  sourceUrl: string | null;
  lastReviewed?: string;
}

export function readRegChange(item: Item): RegChange | null {
  if (!usable(item, CSO_TYPES.regChange)) return null;
  const f = item.fields;
  const title = str(f, 'title', 200) ?? (item.title.trim() || undefined);
  const jurisdiction = str(f, 'jurisdiction', 80);
  const stage = oneOf(f, 'stage', REG_STAGES);
  if (!title || !jurisdiction || !stage) return null;
  return {
    item,
    title,
    jurisdiction,
    instrument: str(f, 'instrument', 200),
    stage,
    effectiveDate: date(f, 'effective_date'),
    impact: oneOf(f, 'impact', IMPACTS),
    impactNote: str(f, 'impact_note', 500),
    sourceUrl: safeUrl(f.source_url),
    lastReviewed: date(f, 'last_reviewed'),
  };
}

/** Settled law: in force or adopted. Everything else is still moving. */
export const isSettled = (s: RegStage) => s === 'in_force' || s === 'adopted';

export interface RadarRow {
  c: RegChange;
  daysToEffect: number | null;
  /** Not reviewed for this many days (null when never reviewed). */
  reviewAge: number | null;
  stale: boolean;
}

/** Days after which a change should be looked at again. */
export const REVIEW_STALE_DAYS = 30;

export function radarRows(items: Item[], today: string, opts: { minImpact?: Impact; showWithdrawn?: boolean } = {}): RadarRow[] {
  const rank: Record<Impact, number> = { high: 0, medium: 1, low: 2 };
  const min = opts.minImpact ? rank[opts.minImpact] : 2;
  return items
    .map(readRegChange)
    .filter((c): c is RegChange => !!c)
    .filter((c) => (opts.showWithdrawn || c.stage !== 'withdrawn') && rank[c.impact ?? 'low'] <= min)
    .map((c) => {
      const reviewAge = c.lastReviewed ? Math.max(0, diffDays(today, c.lastReviewed)) : null;
      return { c, daysToEffect: c.effectiveDate ? diffDays(c.effectiveDate, today) : null, reviewAge, stale: reviewAge === null || reviewAge > REVIEW_STALE_DAYS };
    })
    .sort((a, b) => rank[a.c.impact ?? 'low'] - rank[b.c.impact ?? 'low'] || (a.daysToEffect ?? 1e9) - (b.daysToEffect ?? 1e9) || a.c.title.localeCompare(b.c.title));
}

// ------------------------------------------------------------ Board brief

export const BRIEF_STATUSES = ['draft', 'ready', 'presented'] as const;
export type BriefStatus = (typeof BRIEF_STATUSES)[number];

export interface Figure {
  label: string;
  value: number;
  unit: string;
  sourceId?: string;
  note?: string;
}

export interface Brief {
  item: Item;
  section: string;
  meetingDate?: string;
  status: BriefStatus;
  narrative?: string;
  figures: Figure[];
}

export function readBrief(item: Item): Brief | null {
  if (!usable(item, CSO_TYPES.brief)) return null;
  const f = item.fields;
  const section = str(f, 'section', 120) ?? (item.title.trim() || undefined);
  if (!section) return null;
  const figures: Figure[] = (Array.isArray(f.figures) ? f.figures : [])
    .filter(isRecord)
    .slice(0, 40)
    .flatMap((x): Figure[] => {
      const sid = str(x, 'source_item_id', 64);
      const label = str(x, 'label', 120);
      const value = num(x, 'value');
      const unit = str(x, 'unit', 40);
      if (!label || value === undefined || !unit) return [];
      return [{ label, value, unit, sourceId: sid && ITEM_ID_RE.test(sid) ? sid : undefined, note: str(x, 'note', 300) }];
    });
  return {
    item,
    section,
    meetingDate: date(f, 'meeting_date'),
    status: oneOf(f, 'status', BRIEF_STATUSES) ?? 'draft',
    narrative: str(f, 'narrative', 4000),
    figures,
  };
}

export type FigureState = 'sourced' | 'unsourced' | 'broken' | 'unknown';

/** What a lookup of one cited source found. Absent from the map counts as 'unknown'. */
export type SourceLookup = Item | 'missing' | 'unknown';

export interface CheckedFigure {
  f: Figure;
  state: FigureState;
  source?: Item;
}

export interface BriefCheck {
  b: Brief;
  figures: CheckedFigure[];
  /** Figures that block "ready": no source, or a source id that is not in the Hub. */
  blocking: number;
  /** Figures whose source could not be checked (a failed lookup): not broken, but not confirmed. */
  unchecked: number;
  canBeReady: boolean;
}

/**
 * Every figure must point at an item that exists. A section with no figures
 * can be ready (pure narrative); one with any unsourced or broken figure
 * cannot. A source that could not be looked up is neither broken nor
 * confirmed ("unknown"): it does not count as broken, but blocks ready.
 */
export function checkBrief(b: Brief, sources: ReadonlyMap<string, SourceLookup>): BriefCheck {
  const figures = b.figures.map((f): CheckedFigure => {
    if (!f.sourceId) return { f, state: 'unsourced' };
    const hit = sources.get(f.sourceId) ?? 'unknown';
    if (hit === 'missing') return { f, state: 'broken' };
    if (hit === 'unknown') return { f, state: 'unknown' };
    return { f, state: 'sourced', source: hit };
  });
  const blocking = figures.filter((x) => x.state === 'unsourced' || x.state === 'broken').length;
  const unchecked = figures.filter((x) => x.state === 'unknown').length;
  return { b, figures, blocking, unchecked, canBeReady: blocking === 0 && unchecked === 0 };
}

/**
 * Looks each cited source up by id, so a valid source is never reported
 * missing because of a list limit. Only a not-found answer is 'missing';
 * any other failure is 'unknown'.
 */
export async function lookupSources(
  ids: Iterable<string>,
  get: (id: string) => Promise<Item>,
  isNotFound: (e: unknown) => boolean,
): Promise<Map<string, SourceLookup>> {
  const uniq = [...new Set(ids)];
  const out = new Map<string, SourceLookup>();
  await Promise.all(
    uniq.map(async (id) => {
      try {
        out.set(id, await get(id));
      } catch (e) {
        out.set(id, isNotFound(e) ? 'missing' : 'unknown');
      }
    }),
  );
  return out;
}

/** Every source id the briefs cite. */
export function citedSourceIds(briefs: readonly Brief[]): string[] {
  return [...new Set(briefs.flatMap((b) => b.figures.flatMap((f) => (f.sourceId ? [f.sourceId] : []))))];
}

/** The fields with a new status, or null when "ready"/"presented" is not allowed yet. */
export function briefStatusPatch(check: BriefCheck, status: BriefStatus): Fields | null {
  if (status !== 'draft' && !check.canBeReady) return null;
  return { ...check.b.item.fields, status };
}

// ------------------------------------------------------------ Energy

export const ENERGY_SOURCES = ['electricity_maps', 'mqtt', 'manual'] as const;
export type EnergySource = (typeof ENERGY_SOURCES)[number];

export interface Sample {
  at: number;
  gco2?: number;
  powerKw?: number;
}

export interface EnergyReading {
  item: Item;
  source: EnergySource;
  zone: string;
  zoneLabel?: string;
  site?: string;
  gco2?: number;
  powerKw?: number;
  renewablePct?: number;
  /** The provider marked the intensity as estimated (nowcast), not measured. */
  estimated: boolean;
  at: number;
  /** Oldest first, deduplicated by timestamp, at most 192. */
  series: Sample[];
}

export function readEnergy(item: Item): EnergyReading | null {
  if (!usable(item, CSO_TYPES.energy)) return null;
  const f = item.fields;
  const source = oneOf(f, 'source', ENERGY_SOURCES);
  const zone = str(f, 'zone', 64);
  const at = stamp(f, 'at');
  if (!source || !zone || at === undefined) return null;
  const gco2 = num(f, 'gco2_per_kwh');
  const pct = num(f, 'renewable_pct');
  const seen = new Set<number>();
  const series: Sample[] = [];
  for (const s of Array.isArray(f.series) ? f.series.filter(isRecord) : []) {
    const t = stamp(s, 'at');
    if (t === undefined || seen.has(t)) continue;
    seen.add(t);
    const g = num(s, 'gco2_per_kwh');
    series.push({ at: t, gco2: g !== undefined && g >= 0 ? g : undefined, powerKw: num(s, 'power_kw') });
  }
  series.sort((a, b) => a.at - b.at);
  return {
    item,
    source,
    zone,
    zoneLabel: str(f, 'zone_label', 120),
    site: str(f, 'site', 120),
    gco2: gco2 !== undefined && gco2 >= 0 ? gco2 : undefined,
    powerKw: num(f, 'power_kw'),
    renewablePct: pct !== undefined && pct >= 0 && pct <= 100 ? pct : undefined,
    estimated: f.estimated === true,
    at,
    series: series.slice(-192),
  };
}

export type Band = 'low' | 'mid' | 'high';
export type Freshness = 'live' | 'stale';

export interface EnergyView {
  r: EnergyReading;
  freshness: Freshness;
  ageMinutes: number;
  band?: Band;
  /** Carbon intensity samples of the last `windowHours`, oldest first; no gaps filled. */
  intensity: number[];
  power: number[];
  /** Lowest and highest reported intensity in the window. */
  low?: number;
  high?: number;
}

export const DEFAULT_STALE_MINUTES = 30;
export const DEFAULT_BANDS = { low: 150, high: 400 };

export function bandOf(g: number, bands = DEFAULT_BANDS): Band {
  return g < bands.low ? 'low' : g > bands.high ? 'high' : 'mid';
}

export function energyView(r: EnergyReading, now: number, opts: { staleMinutes?: number; low?: number; high?: number; windowHours?: number } = {}): EnergyView {
  const staleMinutes = opts.staleMinutes ?? DEFAULT_STALE_MINUTES;
  const bands = { low: opts.low ?? DEFAULT_BANDS.low, high: opts.high ?? DEFAULT_BANDS.high };
  const ageMinutes = Math.max(0, Math.floor((now - r.at) / MIN));
  const from = now - (opts.windowHours ?? 24) * 3_600_000;
  const recent = r.series.filter((s) => s.at >= from && s.at <= now + 5 * MIN);
  const intensity = recent.map((s) => s.gco2).filter((v): v is number => v !== undefined);
  const power = recent.map((s) => s.powerKw).filter((v): v is number => v !== undefined);
  return {
    r,
    freshness: ageMinutes > staleMinutes ? 'stale' : 'live',
    ageMinutes,
    band: r.gco2 !== undefined ? bandOf(r.gco2, bands) : undefined,
    intensity,
    power,
    low: intensity.length ? Math.min(...intensity) : undefined,
    high: intensity.length ? Math.max(...intensity) : undefined,
  };
}

/** Readings sorted: the configured zone first, then live before stale, then by zone. */
export function energyViews(items: Item[], now: number, opts: { zone?: string; staleMinutes?: number; low?: number; high?: number } = {}): EnergyView[] {
  return items
    .map(readEnergy)
    .filter((r): r is EnergyReading => !!r && (!opts.zone || r.zone === opts.zone || r.site === opts.zone))
    .map((r) => energyView(r, now, opts))
    .sort((a, b) => Number(a.freshness === 'stale') - Number(b.freshness === 'stale') || a.r.zone.localeCompare(b.r.zone));
}

// ------------------------------------------------------------ Config readers

export function intIn(v: unknown, lo: number, hi: number, dflt: number): number {
  return typeof v === 'number' && Number.isInteger(v) && v >= lo && v <= hi ? v : dflt;
}
export function numIn(v: unknown, lo: number, hi: number, dflt: number): number {
  return typeof v === 'number' && Number.isFinite(v) && v >= lo && v <= hi ? v : dflt;
}

/**
 * The sections for one board meeting: the configured date, else the next
 * meeting on or after today, else the most recent one. Sections with no
 * meeting date are only shown when no dated section exists.
 */
export function briefSections(items: Item[], today: string, meetingDate?: string): { meeting?: string; briefs: Brief[] } {
  const all = items.map(readBrief).filter((b): b is Brief => !!b);
  const dates = [...new Set(all.map((b) => b.meetingDate).filter((d): d is string => !!d))].sort();
  const meeting = meetingDate ?? dates.find((d) => d >= today) ?? dates.at(-1);
  const briefs = all.filter((b) => (meeting ? b.meetingDate === meeting : true)).sort((a, b) => a.section.localeCompare(b.section));
  return { meeting, briefs };
}

/** Short, factual description of a source item for the inline source card. */
export function sourceSummary(item: Item): { type: string; lines: string[] } {
  const f = item.fields;
  const lines: string[] = [];
  const add = (label: string, v: unknown) => {
    if (typeof v === 'string' && v.trim()) lines.push(`${label}: ${v.slice(0, 200)}`);
    else if (finite(v)) lines.push(`${label}: ${v}`);
  };
  switch (item.type_id) {
    case CSO_TYPES.dataRequest:
      add('Supplier', f.supplier);
      add('Metric', f.metric);
      add('Period', f.period);
      add('Status', f.status);
      add('Source', f.source_ref);
      break;
    case CSO_TYPES.evidence:
      add('File', f.file_name);
      add('Status', f.status);
      add('SHA-256', typeof f.sha256 === 'string' ? shortHash(f.sha256) : undefined);
      break;
    case CSO_TYPES.target:
      add('Base year', f.base_year);
      add('Base value', f.base_value);
      add('Target year', f.target_year);
      add('Target value', f.target_value);
      break;
    case CSO_TYPES.energy:
      add('Zone', f.zone_label ?? f.zone);
      add('Reading', f.gco2_per_kwh);
      add('At', f.at);
      break;
    default:
      add('Status', item.status);
  }
  return { type: item.type_id ?? 'item', lines: lines.slice(0, 6) };
}
