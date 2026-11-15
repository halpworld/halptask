// j/k (and arrow) navigation between rows inside a list. Rows are elements
// with [data-row]; focus goes to the row's first focusable control.

export function listKeydown(e: KeyboardEvent): void {
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  const down = e.key === 'j' || e.key === 'ArrowDown';
  const up = e.key === 'k' || e.key === 'ArrowUp';
  if (!down && !up) return;
  const list = e.currentTarget as HTMLElement;
  const rows = [...list.querySelectorAll<HTMLElement>('[data-row]')];
  if (!rows.length) return;
  const current = rows.findIndex((r) => r.contains(document.activeElement));
  const next = current < 0 ? 0 : Math.max(0, Math.min(rows.length - 1, current + (down ? 1 : -1)));
  const target = rows[next]?.querySelector<HTMLElement>('[data-row-focus]') ?? rows[next];
  if (target) {
    e.preventDefault();
    target.focus();
  }
}
