// Display helpers for the CSO widgets. Numbers are shown as reported:
// grouping and a fixed number of decimals, never rounded to a "nicer" story.

import { formatTime, parseISODate } from '../../util/dates';

const monthDay = new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric' });
const monthYear = new Intl.DateTimeFormat(undefined, { month: 'short', year: 'numeric' });
const longDate = new Intl.DateTimeFormat(undefined, { day: 'numeric', month: 'short', year: 'numeric' });

export const dateShort = (d: string) => monthDay.format(parseISODate(d));
export const dateLong = (d: string) => longDate.format(parseISODate(d));
export const monthLabel = (d: string) => monthYear.format(parseISODate(d));
export const clockLabel = (ms: number) => formatTime(new Date(ms));

/** 48 200, 1 234.5, 0.42: up to `max` decimals, grouping on. */
export function fmtNum(v: number, max = 1): string {
  return new Intl.NumberFormat(undefined, { maximumFractionDigits: Math.abs(v) >= 1000 ? 0 : max }).format(v);
}

/** "−12.4 %" / "+3.0 %": sign always shown. */
export function fmtPct(v: number): string {
  const s = new Intl.NumberFormat(undefined, { maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(Math.abs(v));
  return `${v < 0 ? '−' : v > 0 ? '+' : '±'}${s} %`;
}

/** 4.2 MB, 812 kB. */
export function fmtBytes(n: number): string {
  const units = ['B', 'kB', 'MB', 'GB', 'TB'];
  let v = n;
  let i = 0;
  while (v >= 1000 && i < units.length - 1) {
    v /= 1000;
    i++;
  }
  return `${i === 0 ? v : v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

/** "in 12 d", "today", "3 d late", "in 1 y 2 mo"-ish compact day counts. */
export function daysLabel(days: number): string {
  if (days === 0) return 'today';
  if (days === 1) return 'tomorrow';
  if (days === -1) return '1 day late';
  if (days < 0) return `${-days} days late`;
  return `in ${days} days`;
}

/** "12 min ago", "3 h ago", "2 d ago". */
export function ageLabel(minutes: number): string {
  if (minutes < 1) return 'just now';
  if (minutes < 60) return `${minutes} min ago`;
  if (minutes < 60 * 48) return `${Math.floor(minutes / 60)} h ago`;
  return `${Math.floor(minutes / 1440)} d ago`;
}

export const humanize = (s: string) => s.replace(/_/g, ' ');
