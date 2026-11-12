// Global keyboard shortcuts. Kept small and documented in ShortcutsHelp.
// Single keys are ignored while typing in a field; mod+ chords always work.

export interface Shortcut {
  keys: string;
  label: string;
  group: 'General' | 'View' | 'Navigate' | 'Edit';
}

export const SHORTCUTS: Shortcut[] = [
  { keys: 'mod+k', label: 'Command palette', group: 'General' },
  { keys: '?', label: 'Show shortcuts', group: 'General' },
  { keys: 'mod+,', label: 'Settings: agents and connections', group: 'General' },
  { keys: 'mod+z', label: 'Undo last change', group: 'General' },
  { keys: 'mod+shift+z', label: 'Redo', group: 'General' },
  { keys: 'n', label: 'Quick capture', group: 'General' },
  { keys: 'a', label: 'Ambient (wall) mode', group: 'View' },
  { keys: 'b', label: 'Privacy blur', group: 'View' },
  { keys: 't', label: 'Toggle light / dark', group: 'View' },
  { keys: '1', label: 'Developer role', group: 'View' },
  { keys: '2', label: 'Eng manager role', group: 'View' },
  { keys: '3', label: 'CSO role', group: 'View' },
  { keys: 'j', label: 'Next widget', group: 'Navigate' },
  { keys: 'k', label: 'Previous widget', group: 'Navigate' },
  { keys: 'g n', label: 'Go to Now', group: 'Navigate' },
  { keys: 'g d', label: 'Go to today’s tasks', group: 'Navigate' },
  { keys: 'g a', label: 'Go to agenda', group: 'Navigate' },
  { keys: 'e', label: 'Edit layout', group: 'Edit' },
  { keys: 'esc', label: 'Leave ambient / edit mode', group: 'Edit' },
];

export interface KeyActions {
  palette(): void;
  help(): void;
  settings(): void;
  undo(): void;
  redo(): void;
  capture(): void;
  ambient(): void;
  privacy(): void;
  theme(): void;
  role(index: number): void;
  nextWidget(delta: 1 | -1): void;
  goto(target: 'now' | 'day' | 'agenda'): void;
  edit(): void;
  escape(): boolean;
  /** True while a dialog (palette, consent, help) owns the keyboard. */
  blocked(): boolean;
}

export function isTyping(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  if (target.isContentEditable) return true;
  const tag = target.tagName;
  if (tag === 'TEXTAREA' || tag === 'SELECT') return true;
  if (tag === 'INPUT') {
    const type = (target as HTMLInputElement).type;
    return !['checkbox', 'radio', 'button', 'submit', 'range'].includes(type);
  }
  return false;
}

/** Returns a keydown listener. `g` starts a 1.2 s chord. */
export function createKeyHandler(a: KeyActions): (e: KeyboardEvent) => void {
  let chord: { key: string; until: number } | null = null;

  return (e: KeyboardEvent) => {
    const mod = e.metaKey || e.ctrlKey;
    const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;

    if (mod && key === 'k') {
      e.preventDefault();
      a.palette();
      return;
    }
    if (a.blocked() || e.defaultPrevented) return;
    if (mod && key === ',') {
      e.preventDefault();
      a.settings();
      return;
    }
    if (mod && key === 'z' && !isTyping(e.target)) {
      e.preventDefault();
      if (e.shiftKey) a.redo();
      else a.undo();
      return;
    }
    if (key === 'Escape') {
      if (a.escape()) e.preventDefault();
      return;
    }
    if (mod || e.altKey || isTyping(e.target)) return;

    const now = Date.now();
    if (chord && chord.until > now && chord.key === 'g') {
      chord = null;
      const target = ({ n: 'now', d: 'day', a: 'agenda' } as const)[key as 'n' | 'd' | 'a'];
      if (target) {
        e.preventDefault();
        a.goto(target);
      }
      return;
    }
    chord = null;

    const run = (fn: () => void) => {
      e.preventDefault();
      fn();
    };
    switch (key) {
      case 'g':
        chord = { key: 'g', until: now + 1200 };
        e.preventDefault();
        return;
      case '?':
        return run(a.help);
      case 'n':
        return run(a.capture);
      case 'a':
        return run(a.ambient);
      case 'b':
        return run(a.privacy);
      case 't':
        return run(a.theme);
      case 'e':
        return run(a.edit);
      case '1':
      case '2':
      case '3':
        return run(() => a.role(Number(key) - 1));
      case 'j':
        // Rows inside a focused list handle j/k themselves (they stop propagation).
        return run(() => a.nextWidget(1));
      case 'k':
        return run(() => a.nextWidget(-1));
    }
  };
}
