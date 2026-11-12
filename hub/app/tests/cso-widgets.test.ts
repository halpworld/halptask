// CSO widget logic: deadlines that are never shown as certain unless they
// are, the data chase, evidence chains and gaps, a trajectory that never
// interpolates, grid freshness and the board brief's source gate. Pure
// functions, no DOM.

import { describe, expect, it } from 'vitest';
import type { Item } from '../src/lib/api/types';
import {
  bandOf,
  briefSections,
  briefStatusPatch,
  chainState,
  chaseBoard,
  chaseCard,
  chasePatch,
  checkBrief,
  citedSourceIds,
  lookupSources,
  deadlineRows,
  energyView,
  energyViews,
  evidenceGaps,
  evidenceRows,
  pathwayAt,
  radarRows,
  readBrief,
  readDataRequest,
  readDisclosure,
  readEnergy,
  readEvidence,
  readTarget,
  scopeLabel,
  shortHash,
  sourceSummary,
  statusPatch,
  trajectoryModel,
  type DataRequest,
  type Evidence,
} from '../src/lib/widgets/cso/logic';
import { CSO_CONFIG } from '../src/lib/widgets/cso/config';
import { ageLabel, daysLabel, fmtBytes, fmtPct } from '../src/lib/widgets/cso/format';

const TODAY = '2026-11-10';
const NOW = new Date('2026-11-10T09:30:00').getTime();
const HASH = 'a'.repeat(60) + 'beef';

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
    roles: ['cso'],
    origin: 'user',
    created_at: '2026-11-01T00:00:00Z',
    updated_at: '2026-11-01T00:00:00Z',
    version: 1,
    ...p,
  } as Item;
}
const disc = (f: Item['fields'], p: Partial<Item> = {}) => item({ type_id: 'cso.disclosure', fields: f, ...p });
const req = (f: Item['fields'], p: Partial<Item> = {}) => item({ type_id: 'cso.data_request', fields: { supplier: 'Acme', metric: 'tkm', status: 'requested', ...f }, ...p });
const ev = (f: Item['fields'], p: Partial<Item> = {}) => item({ type_id: 'cso.evidence', fields: { file_name: 'a.pdf', sha256: HASH, status: 'draft', ...f }, ...p });

describe('deadline wall', () => {
  it('sorts by days left, marks state, and only confirmed is certain', () => {
    const rows = deadlineRows(
      [
        disc({ framework: 'sb253', due: '2026-11-10', regulatory_certainty: 'confirmed' }, { id: 'a' }),
        disc({ framework: 'sb261', due: '2026-12-01', regulatory_certainty: 'litigated' }, { id: 'b' }),
        disc({ framework: 'csrd_esrs', due: '2028-05-01', regulatory_certainty: 'delayed' }, { id: 'c' }),
        disc({ framework: 'other', due: '2026-11-07', regulatory_certainty: 'confirmed' }, { id: 'd' }),
      ],
      TODAY,
    );
    expect(rows.map((r) => r.d.item.id)).toEqual(['d', 'a', 'b', 'c']);
    expect(rows.map((r) => r.state)).toEqual(['overdue', 'today', 'soon', 'later']);
    expect(rows.map((r) => r.certain)).toEqual([true, true, false, false]);
  });

  it('never treats a missing or unknown certainty as confirmed', () => {
    expect(readDisclosure(disc({ framework: 'sb253', due: '2027-01-01' }))?.certainty).toBe('proposed');
    expect(readDisclosure(disc({ framework: 'sb253', due: '2027-01-01', regulatory_certainty: 'CONFIRMED' }))?.certainty).toBe('proposed');
    const [r] = deadlineRows([disc({ framework: 'sb253', due: '2027-01-01', regulatory_certainty: 'yes' })], TODAY);
    expect(r?.certain).toBe(false);
  });

  it('applies the horizon and hides filed unless asked', () => {
    const items = [disc({ framework: 'sb253', due: '2029-01-01', regulatory_certainty: 'confirmed' }), disc({ framework: 'other', due: '2026-10-01', regulatory_certainty: 'confirmed', status: 'filed' })];
    expect(deadlineRows(items, TODAY, { horizonDays: 30 })).toHaveLength(0);
    const shown = deadlineRows(items, TODAY, { horizonDays: 1825, showFiled: true });
    expect(shown.map((r) => r.state)).toEqual(['later', 'filed']);
  });

  it('skips malformed items and refuses non-https sources', () => {
    expect(readDisclosure(disc({ framework: 'nope', due: '2027-01-01' }))).toBeNull();
    expect(readDisclosure(disc({ framework: 'sb253' }))).toBeNull();
    expect(readDisclosure(disc({ framework: 'sb253', due: '2027-01-01', source_url: 'javascript:alert(1)' }))?.sourceUrl).toBeNull();
    expect(readDisclosure(item({ type_id: 'cso.target', fields: {} }))).toBeNull();
  });
});

describe('data chase', () => {
  it('groups into five columns, overdue first', () => {
    const board = chaseBoard(
      [
        req({ supplier: 'B', due: '2026-11-08' }, { id: 'late2' }),
        req({ supplier: 'A', due: '2026-11-01', status: 'chased', chase_count: 2 }, { id: 'late9' }),
        req({ supplier: 'C', due: '2026-12-01' }, { id: 'ok' }),
        req({ status: 'validated' }, { id: 'v' }),
        req({ status: 'bogus' }, { id: 'x' }),
      ],
      TODAY,
      NOW,
    );
    expect(board.map((c) => c.status)).toEqual(['requested', 'chased', 'received', 'validated', 'rejected']);
    expect(board[0]!.cards.map((c) => c.r.item.id)).toEqual(['late2', 'ok', 'x']);
    expect(board[0]!.overdue).toBe(1);
    expect(board[1]!.cards[0]!.overdueDays).toBe(9);
    expect(board[3]!.cards[0]!.overdueDays).toBeNull();
  });

  it('marks a request due a chase after N quiet days', () => {
    const r = readDataRequest(req({ requested_on: '2026-11-01', due: '2026-12-01' }))!;
    expect(chaseCard(r, TODAY, NOW, 7).chaseDue).toBe(true);
    expect(chaseCard(r, TODAY, NOW, 14).chaseDue).toBe(false);
    const chased = readDataRequest(req({ status: 'chased', chase_count: 1, last_chased: new Date(NOW - 2 * 86_400_000).toISOString(), due: '2026-12-01' }))!;
    expect(chaseCard(chased, TODAY, NOW, 7)).toMatchObject({ chaseDue: false, sinceChase: 2 });
  });

  it('a chase increments chase_count, sets last_chased and keeps every other field', () => {
    const fields = { supplier: 'Acme', metric: 'tkm', status: 'requested', chase_count: 2, source_ref: 'x' };
    const nowIso = '2026-11-10T08:30:00.000Z';
    expect(chasePatch(fields, nowIso)).toEqual({ ...fields, status: 'chased', chase_count: 3, last_chased: nowIso });
    expect(chasePatch({ supplier: 'A', metric: 'm', status: 'chased' }, nowIso)).toMatchObject({ chase_count: 1 });
    expect(chasePatch({ ...fields, status: 'received' }, nowIso)).toBeNull();
    expect(chasePatch({ ...fields, chase_count: 1000 }, nowIso)?.chase_count).toBe(1000);
    expect(statusPatch(fields, 'validated')).toEqual({ ...fields, status: 'validated' });
  });

  it('labels scope 3 categories', () => {
    expect(scopeLabel({ scope: 'scope3', category: 4 })).toBe('Scope 3 · cat 4');
    expect(scopeLabel({ scope: 'scope1' })).toBe('Scope 1');
    expect(scopeLabel({})).toBeUndefined();
  });
});

describe('evidence locker', () => {
  const approvals = (...d: string[]) => d.map((decision, i) => ({ reviewer: `R${i}`, decision, at: `2026-11-0${i + 1}T10:00:00Z` }));

  it('reads the chain in time order and reports its state', () => {
    const e = readEvidence(ev({ status: 'approved', approvals: [...approvals('changes_requested', 'approved')].reverse() }))!;
    expect(e.approvals.map((a) => a.decision)).toEqual(['changes_requested', 'approved']);
    expect(chainState(e)).toBe('approved');
    expect(chainState(readEvidence(ev({ status: 'in_review', approvals: approvals('approved') }))!)).toBe('pending');
    expect(chainState(readEvidence(ev({ status: 'in_review', approvals: approvals('changes_requested') }))!)).toBe('changes');
    expect(chainState(readEvidence(ev({ status: 'draft' }))!)).toBe('none');
    expect(chainState(readEvidence(ev({ status: 'rejected' }))!)).toBe('rejected');
  });

  it('flags evidence marked approved without an approval or a hash', () => {
    expect(chainState(readEvidence(ev({ status: 'approved' }))!)).toBe('inconsistent');
    expect(chainState(readEvidence(ev({ status: 'approved', sha256: 'XYZ', approvals: approvals('approved') }))!)).toBe('inconsistent');
    expect(evidenceRows([readEvidence(ev({ status: 'approved', approvals: approvals('approved') }))!, readEvidence(ev({ status: 'approved' }))!]).map(chainState)).toEqual(['inconsistent', 'approved']);
  });

  it('shortens the hash and never invents one', () => {
    expect(shortHash(HASH)).toBe('aaaaaaaa…beef');
    expect(shortHash(null)).toBe('no hash');
    expect(readEvidence(ev({ sha256: 'A'.repeat(64) }))!.sha256).toBeNull();
  });

  it('finds data points with no approved evidence; rejected evidence does not count', () => {
    const reqs = [
      readDataRequest(req({ status: 'validated' }, { id: 'p1' }))!,
      readDataRequest(req({ status: 'received' }, { id: 'p2' }))!,
      readDataRequest(req({ status: 'received' }, { id: 'p3' }))!,
      readDataRequest(req({ status: 'requested' }, { id: 'p4' }))!,
    ] as DataRequest[];
    const evs = [
      readEvidence(ev({ data_point_id: 'p1', status: 'approved', approvals: approvals('approved') }))!,
      readEvidence(ev({ data_point_id: 'p2', status: 'in_review', approvals: approvals('approved') }))!,
      readEvidence(ev({ data_point_id: 'p3', status: 'rejected' }))!,
    ] as Evidence[];
    expect(evidenceGaps(reqs, evs).map((g) => [g.r.item.id, g.kind])).toEqual([
      ['p3', 'missing'],
      ['p2', 'unapproved'],
    ]);
  });
});

describe('trajectory', () => {
  const t = readTarget(
    item({
      type_id: 'cso.target',
      fields: {
        scope: 'scope1_2',
        base_year: 2021,
        base_value: 1000,
        target_year: 2030,
        target_value: 580,
        actuals: [
          { year: 2021, value: 1000, source_ref: 'inv21' },
          { year: 2022, value: 950, source_ref: 'inv22' },
          { year: 2024, value: 900, source_ref: 'inv24' },
          { year: 2025, value: 870 },
          { year: 2025, value: 1 },
        ],
      },
    }),
  )!;

  it('keeps one figure per year and never fills a missing year', () => {
    const m = trajectoryModel(t, 2026);
    expect(m.points.map((p) => p.year)).toEqual([2021, 2022, 2024, 2025]);
    expect(m.points.map((p) => p.value)).toEqual([1000, 950, 900, 870]);
    // The line breaks at 2023: two segments, neither crossing the gap.
    expect(m.segments).toHaveLength(2);
    expect(m.segments[0]!.split('L')).toHaveLength(2);
    expect(m.gaps.map((g) => g.year)).toEqual([2023]);
  });

  it('marks points without a source as unsourced and uses only sourced ones for the headline', () => {
    const m = trajectoryModel(t, 2026);
    expect(m.points.filter((p) => !p.sourced).map((p) => p.year)).toEqual([2025]);
    expect(m.latestSourced?.year).toBe(2024);
    expect(m.changeVsBase).toBeCloseTo(-10);
    expect(m.vsPathway).toBe(pathwayAt(t, 2024) >= 900 ? 'below' : 'above');
  });

  it('reports the years after the last figure up to last year as gaps', () => {
    const m = trajectoryModel(t, 2028);
    expect(m.gaps.map((g) => g.year)).toEqual([2023, 2026, 2027]);
  });

  it('refuses malformed targets', () => {
    expect(readTarget(item({ type_id: 'cso.target', fields: { scope: 'scope1', base_year: 2030, base_value: 1, target_year: 2020, target_value: 0 } }))).toBeNull();
    expect(readTarget(item({ type_id: 'cso.target', fields: { scope: 'scope1', base_year: 2020, base_value: -1, target_year: 2030, target_value: 0 } }))).toBeNull();
  });

  it('keeps geometry inside the plot', () => {
    const m = trajectoryModel(t, 2026, { width: 300, height: 150 });
    for (const p of m.points) {
      expect(p.x).toBeGreaterThanOrEqual(m.plot.left);
      expect(p.x).toBeLessThanOrEqual(m.plot.right);
      expect(p.y).toBeGreaterThanOrEqual(m.plot.top);
      expect(p.y).toBeLessThanOrEqual(m.plot.bottom);
    }
  });
});

describe('grid carbon', () => {
  const reading = (minsAgo: number, extra: Item['fields'] = {}) =>
    item({ type_id: 'cso.energy_reading', fields: { source: 'electricity_maps', zone: 'SE-SE3', at: new Date(NOW - minsAgo * 60_000).toISOString(), gco2_per_kwh: 40, ...extra } });

  it('is live within the stale window and stale after it', () => {
    expect(energyView(readEnergy(reading(5))!, NOW).freshness).toBe('live');
    expect(energyView(readEnergy(reading(31))!, NOW).freshness).toBe('stale');
    expect(readEnergy(reading(5))!.estimated).toBe(false);
    const est = reading(5);
    expect(readEnergy({ ...est, fields: { ...est.fields, estimated: true } })!.estimated).toBe(true);
    expect(readEnergy({ ...est, fields: { ...est.fields, estimated: 'yes' } })!.estimated).toBe(false);
    expect(energyView(readEnergy(reading(31))!, NOW, { staleMinutes: 60 }).freshness).toBe('live');
  });

  it('bands intensity by the configured thresholds', () => {
    expect(bandOf(100)).toBe('low');
    expect(bandOf(200)).toBe('mid');
    expect(bandOf(500)).toBe('high');
    expect(bandOf(120, { low: 100, high: 110 })).toBe('high');
  });

  it('keeps the series in time order, last 24 h only, without filling gaps', () => {
    const at = (m: number) => new Date(NOW - m * 60_000).toISOString();
    const r = readEnergy(reading(5, { series: [{ at: at(30), gco2_per_kwh: 45 }, { at: at(60 * 30), gco2_per_kwh: 99 }, { at: at(90) }, { at: at(120), gco2_per_kwh: 50 }, { at: at(30), gco2_per_kwh: 7 }] }))!;
    const v = energyView(r, NOW);
    expect(v.intensity).toEqual([50, 45]);
    expect(v.low).toBe(45);
    expect(v.high).toBe(50);
  });

  it('has no data without a timestamp, and filters by zone', () => {
    expect(readEnergy(item({ type_id: 'cso.energy_reading', fields: { source: 'mqtt', zone: 'x' } }))).toBeNull();
    expect(energyViews([reading(5), reading(5, { zone: 'PL' })], NOW, { zone: 'PL' }).map((v) => v.r.zone)).toEqual(['PL']);
  });
});

describe('regulatory radar', () => {
  const reg = (f: Item['fields']) => item({ type_id: 'cso.reg_change', fields: { title: 't', jurisdiction: 'EU', source_url: 'https://eur-lex.europa.eu/', ...f } });
  it('sorts by impact, hides withdrawn, and flags stale reviews', () => {
    const rows = radarRows([reg({ title: 'low', stage: 'proposed', impact: 'low', last_reviewed: '2026-11-01' }), reg({ title: 'high', stage: 'litigated', impact: 'high' }), reg({ title: 'gone', stage: 'withdrawn', impact: 'high' })], TODAY);
    expect(rows.map((r) => r.c.title)).toEqual(['high', 'low']);
    expect(rows.map((r) => r.stale)).toEqual([true, false]);
    expect(radarRows([reg({ stage: 'proposed', impact: 'low' })], TODAY, { minImpact: 'medium' })).toHaveLength(0);
  });
});

describe('board brief', () => {
  const src = item({ id: 'src-1', title: 'Scope 1+2', type_id: 'cso.target', fields: { base_year: 2021, base_value: 1 } });
  const sources = new Map<string, Item | 'missing' | 'unknown'>([[src.id, src], ['gone', 'missing']]);
  const brief = (figures: unknown[], status = 'draft') => readBrief(item({ type_id: 'cso.board_brief', fields: { section: 'Climate', status, meeting_date: '2026-11-19', figures } as Item['fields'] }))!;

  it('blocks ready while any figure is unsourced or cites a missing item', () => {
    const c = checkBrief(brief([{ label: 'a', value: 1, unit: 't', source_item_id: 'src-1' }, { label: 'b', value: 2, unit: 't' }, { label: 'c', value: 3, unit: 't', source_item_id: 'gone' }]), sources);
    expect(c.figures.map((f) => f.state)).toEqual(['sourced', 'unsourced', 'broken']);
    expect(c.blocking).toBe(2);
    expect(c.canBeReady).toBe(false);
    expect(briefStatusPatch(c, 'ready')).toBeNull();
    expect(briefStatusPatch(c, 'presented')).toBeNull();
    expect(briefStatusPatch(c, 'draft')).toMatchObject({ status: 'draft' });
  });

  it('treats a source that could not be looked up as unknown, not broken', () => {
    const c = checkBrief(brief([{ label: 'a', value: 1, unit: 't', source_item_id: 'src-1' }, { label: 'b', value: 2, unit: 't', source_item_id: 'flaky' }]), sources);
    expect(c.figures.map((f) => f.state)).toEqual(['sourced', 'unknown']);
    expect(c.blocking).toBe(0);
    expect(c.unchecked).toBe(1);
    expect(c.canBeReady).toBe(false);
  });

  it('looks every cited source up by id: found, missing on not-found, unknown on other errors', async () => {
    const notFound = new Error('nf');
    const get = async (id: string): Promise<Item> => {
      if (id === 'src-1') return src;
      if (id === 'src-600') return item({ id: 'src-600' });
      if (id === 'gone') throw notFound;
      throw new Error('boom');
    };
    const b = brief([1, 2, 3, 4].map((n, i) => ({ label: 'l', value: n, unit: 't', source_item_id: ['src-1', 'src-600', 'gone', 'flaky'][i] })));
    expect(citedSourceIds([b, b])).toEqual(['src-1', 'src-600', 'gone', 'flaky']);
    const got = await lookupSources(citedSourceIds([b]), get, (e) => e === notFound);
    expect(got.get('src-600')).toMatchObject({ id: 'src-600' });
    expect(got.get('gone')).toBe('missing');
    expect(got.get('flaky')).toBe('unknown');
    expect(checkBrief(b, got).figures.map((f) => f.state)).toEqual(['sourced', 'sourced', 'broken', 'unknown']);
  });

  it('allows ready when every figure has a source (or there are none)', () => {
    expect(briefStatusPatch(checkBrief(brief([{ label: 'a', value: 1, unit: 't', source_item_id: 'src-1' }]), sources), 'ready')).toMatchObject({ status: 'ready', section: 'Climate' });
    expect(checkBrief(brief([]), sources).canBeReady).toBe(true);
  });

  it('drops malformed figures rather than guessing values', () => {
    expect(brief([{ label: 'a', value: '12', unit: 't' }, { label: 'b', unit: 't' }, { label: 'c', value: 1, unit: 't' }]).figures.map((f) => f.label)).toEqual(['c']);
  });

  it('picks the next meeting, or the configured one', () => {
    const at = (d: string, s: string) => item({ type_id: 'cso.board_brief', fields: { section: s, status: 'draft', meeting_date: d } });
    const items = [at('2026-09-01', 'old'), at('2026-11-19', 'b'), at('2026-11-19', 'a'), at('2027-02-01', 'later')];
    expect(briefSections(items, TODAY)).toMatchObject({ meeting: '2026-11-19' });
    expect(briefSections(items, TODAY).briefs.map((b) => b.section)).toEqual(['a', 'b']);
    expect(briefSections(items, TODAY, '2026-09-01').briefs.map((b) => b.section)).toEqual(['old']);
    expect(briefSections(items, '2028-01-01').meeting).toBe('2027-02-01');
  });

  it('summarises a source without inventing anything', () => {
    expect(sourceSummary(src)).toEqual({ type: 'cso.target', lines: ['Base year: 2021', 'Base value: 1'] });
  });
});

describe('config and format', () => {
  it('every config schema is closed', () => {
    for (const s of Object.values(CSO_CONFIG)) expect(s).toMatchObject({ type: 'object', additionalProperties: false });
  });
  it('formats', () => {
    expect(daysLabel(0)).toBe('today');
    expect(daysLabel(-3)).toBe('3 days late');
    expect(daysLabel(12)).toBe('in 12 days');
    expect(fmtPct(-12.34)).toBe('−12.3 %');
    expect(fmtBytes(4_218_331)).toBe('4.2 MB');
    expect(ageLabel(185)).toBe('3 h ago');
  });
});
