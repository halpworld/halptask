import { describe, expect, it } from 'vitest';
import { keyStatus, passphraseProblem, windowLabel, MIN_PASSPHRASE_CHARS } from '../src/lib/api/shell';

describe('passphrase form', () => {
  it('unlocking only needs a passphrase', () => {
    expect(passphraseProblem('', '', false)).toMatch(/Enter/);
    expect(passphraseProblem('x', '', false)).toBeNull();
  });
  it('creating needs a long enough, repeated passphrase', () => {
    expect(passphraseProblem('short', 'short', true)).toMatch(String(MIN_PASSPHRASE_CHARS));
    expect(passphraseProblem('correct horse', 'correct hose', true)).toMatch(/match/);
    expect(passphraseProblem('correct horse', 'correct horse', true)).toBeNull();
  });
  it('counts characters, not UTF-16 units', () => {
    expect(passphraseProblem('🔑🔑🔑🔑', '🔑🔑🔑🔑', true)).not.toBeNull();
  });
});

describe('outside Tauri', () => {
  it('is ready at once and is the main window', async () => {
    expect(await keyStatus()).toEqual({ state: 'ready', kind: 'mock' });
    expect(await windowLabel()).toBe('main');
  });
});
