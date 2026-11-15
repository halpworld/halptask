// Mirrors the Rust tests in halp-domain/src/dates.rs and capture.rs, so the
// TS port stays exact. Add a case here whenever the core adds one.
import { describe, expect, it } from 'vitest';
import { CaptureError, parseCaptureText, parseDue, resolveCapture } from '../src/lib/capture/parse';

// Reference date of the Rust/Go tests: Tuesday 2026-08-25.
const T = '2026-08-25';

describe('parseDue (dates.rs)', () => {
  it.each([
    ['today', '2026-08-25'],
    ['tdy', '2026-08-25'],
    ['tomorrow', '2026-08-26'],
    ['tmrw', '2026-08-26'],
    ['yesterday', '2026-08-24'],
    ['due:today', '2026-08-25'],
    ['@due(tomorrow)', '2026-08-26'],
    ['+1d', '2026-08-26'],
    ['+3d', '2026-08-28'],
    ['+1w', '2026-09-01'],
    ['2026-09-15', '2026-09-15'],
    ['09/15', '2026-09-15'],
    ['friday', '2026-08-28'],
    ['fri', '2026-08-28'],
    ['tuesday', '2026-09-01'],
    ['3d', '2026-08-28'],
    ['-2d', '2026-08-23'],
    ['2w', '2026-09-08'],
    ['+1m', '2026-09-25'],
    ['2026/9/5', '2026-09-05'],
    ['2026-9-5', '2026-09-05'],
    ['9-5', '2026-09-05'],
    ['Jan 2', '2026-01-02'],
    ['jan 02', '2026-01-02'],
    ['2 January', '2026-01-02'],
    ['02 Jan', '2026-01-02'],
    ['September 30', '2026-09-30'],
    ['WED', '2026-08-26'],
    ['Sunday', '2026-08-30'],
    ['due="2026-12-24"', '2026-12-24'],
  ])('%s -> %s', (raw, want) => {
    expect(parseDue(raw, T)).toBe(want);
  });

  it('clamps months to the end of the month', () => {
    expect(parseDue('+1m', '2026-01-31')).toBe('2026-02-28');
    expect(parseDue('-1m', '2026-03-31')).toBe('2026-02-28');
  });

  it.each(['invalid-date-xyz', '0d', '99999999999d', '2026-02-30', '02/30', '26-09-15', '', 'Sept 3', '13/45', 'eow', 'nextweek', 'tod', 'tom'])(
    'rejects %j',
    (raw) => {
      expect(parseDue(raw, T)).toBeNull();
    },
  );
});

describe('parseCaptureText (capture.rs)', () => {
  it.each([
    ['Fix Redis connection timeout', 'Fix Redis connection timeout', [], null, 'todo', false],
    ['Fix Redis timeout #ops #backend due:tomorrow', 'Fix Redis timeout', ['ops', 'backend'], 'tomorrow', 'todo', false],
    ['- [~] Deploy staging hotfix #dev', 'Deploy staging hotfix', ['dev'], null, 'in_progress', false],
    ['- [x] Submit expense report', 'Submit expense report', [], null, 'done', false],
    ['- General system note', 'General system note', [], null, 'todo', true],
    ['Quarterly review #planning due=2026-09-01', 'Quarterly review', ['planning'], '2026-09-01', 'todo', false],
  ] as const)('%s', (input, title, tags, rawDue, status, bulletPrefix) => {
    expect(parseCaptureText(input)).toEqual({ title, tags, rawDue, status, bulletPrefix });
  });

  it('handles the contract example and edges', () => {
    expect(parseCaptureText('Fix login #bug due:friday')).toMatchObject({ title: 'Fix login', tags: ['bug'], rawDue: 'friday' });
    expect(parseCaptureText('Close #123 and #Bug, @due(2026-09-15) #bug')).toMatchObject({ title: 'Close #123 and', tags: ['bug'], rawDue: '2026-09-15' });
    expect(parseCaptureText('Weird #tag;drop me').title).toBe('Weird #tag;drop me');
    const u = parseCaptureText('• Unicode bullet 日本語 #i18n');
    expect(u.title).toBe('Unicode bullet 日本語');
    expect(u.bulletPrefix).toBe(true);
    expect(parseCaptureText('   ').title).toBe('');
    expect(parseCaptureText('due: alone').title).toBe('due: alone');
  });
});

describe('resolveCapture (capture.parse)', () => {
  it('resolves the due date and keeps an unparseable one in the title', () => {
    expect(resolveCapture('Fix login #bug #Auth due:fri', T)).toEqual({ title: 'Fix login', tags: ['bug', 'auth'], due: '2026-08-28', kind: 'task', status: 'todo' });
    expect(resolveCapture('Call Ana #x #X due:someday', T)).toMatchObject({ title: 'Call Ana due:someday', tags: ['x'], due: null });
  });
  it('bullets are always todo; extra tags are merged', () => {
    expect(resolveCapture('- [x] Note', T, ['Extra'], 'bullet')).toMatchObject({ kind: 'bullet', status: 'todo', tags: ['extra'] });
  });
  it('refuses empty text like the core', () => {
    expect(() => resolveCapture('   ', T)).toThrow(CaptureError);
    expect(() => resolveCapture('#only', T)).toThrow('nothing to capture');
  });
});
