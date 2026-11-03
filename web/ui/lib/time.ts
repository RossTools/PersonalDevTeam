// Times, dates and durations for people. One home for each format, so the same thing reads the same everywhere.
// In render, show a relative time with `<RelativeTime>` (it ticks), not `relativeTime()` (its text goes stale).

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

/** "12 Mar", or "12 Mar 2025" in another year. */
export function shortDate(time: number, now: number = Date.now()): string {
  const date = new Date(time);
  const sameYear = date.getFullYear() === new Date(now).getFullYear();
  return date.toLocaleDateString(undefined, { day: 'numeric', month: 'short', ...(sameYear ? {} : { year: 'numeric' }) });
}

/** "just now", "5m ago", "2h ago", "3d ago", then a date ("12 Mar", or "12 Mar 2025" in another year). */
export function relativeTime(time: number, now: number = Date.now()): string {
  const d = Math.max(0, now - time);
  if (d < MINUTE) return 'just now';
  if (d < HOUR) return `${Math.floor(d / MINUTE)}m ago`;
  if (d < DAY) return `${Math.floor(d / HOUR)}h ago`;
  if (d < 7 * DAY) return `${Math.floor(d / DAY)}d ago`;
  return shortDate(time, now);
}

/** An age in a list row's corner: "now", "10m", "3h", "2d". */
export function shortAge(time: number, now: number = Date.now()): string {
  const m = Math.floor(Math.max(0, now - time) / MINUTE);
  if (m < 1) return 'now';
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 24 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

/** The full local date and time, for tooltips: "12 Mar 2026, 3:40 PM". */
export function absoluteTime(time: number): string {
  return new Date(time).toLocaleString(undefined, { dateStyle: 'medium', timeStyle: 'short' });
}

/** The hour and minute, "3:40 PM"; with `weekday`, "Tue 3:40 PM" when `time` isn't on `now`'s day. */
export function clockTime(time: number, { weekday = false, now = Date.now() }: { weekday?: boolean; now?: number } = {}): string {
  const date = new Date(time);
  const otherDay = weekday && new Date(now).toDateString() !== date.toDateString();
  return date.toLocaleString(undefined, otherDay ? { weekday: 'short', hour: 'numeric', minute: '2-digit' } : { hour: 'numeric', minute: '2-digit' });
}

/** How long something took: "850 ms", "12 s", "3 min 4 s", "2 h 5 min". */
export function formatDuration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return s % 60 ? `${m} min ${s % 60} s` : `${m} min`;
  const h = Math.floor(m / 60);
  return m % 60 ? `${h} h ${m % 60} min` : `${h} h`;
}
