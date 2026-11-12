// Display helpers for the engineering manager widgets.

import { diffDays, formatTime, parseISODate, toISODate } from '../../util/dates';

const weekday = new Intl.DateTimeFormat(undefined, { weekday: 'short' });
const monthDay = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });

/** "today 10:00", "tomorrow 14:30", "Thu 09:00", "Nov 24", or "Not scheduled". */
export function whenLabel(ms: number | undefined, now: number): string {
  if (ms === undefined) return 'Not scheduled';
  const d = new Date(ms);
  const days = diffDays(toISODate(d), toISODate(new Date(now)));
  if (days === 0) return `today ${formatTime(d)}`;
  if (days === 1) return `tomorrow ${formatTime(d)}`;
  if (days > 1 && days < 7) return `${weekday.format(d)} ${formatTime(d)}`;
  return monthDay.format(d);
}

export function weekdayShort(date: string): string {
  return weekday.format(parseISODate(date));
}

export function dateShort(date: string): string {
  return monthDay.format(parseISODate(date));
}

export function clockLabel(ms: number): string {
  return formatTime(new Date(ms));
}
