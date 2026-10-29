// The todo list's pure rules (PDT-125): filtering, splitting open from done, and the position a moved row takes; due
// dates (PDT-140): the user's local day, the chip a card shows, the groups open todos fall into and the quick picks;
// repeating (PDT-141): a rule as text; the redesign (PDT-143): six groups, Move to today, the due picker's quick picks
// and repeat presets.
import type { Recurrence } from '../api/gen/Recurrence';
import type { RepeatRule } from '../api/gen/RepeatRule';
import type { Todo } from '../api/gen/Todo';

/** The list's filter: every todo, or the current project's. */
export type TodoFilter = 'all' | 'project';

/** The todos the filter shows, open ones in their order and done ones latest first. */
export function shown(todos: readonly Todo[], filter: TodoFilter, projectId: string | null): { open: Todo[]; done: Todo[] } {
  const list = filter === 'project' && projectId ? todos.filter((t) => t.projectId === projectId) : todos;
  return {
    open: list.filter((t) => !t.done).sort((a, b) => a.position - b.position),
    done: list.filter((t) => t.done).sort((a, b) => (b.doneAt ?? 0) - (a.doneAt ?? 0)),
  };
}

/**
 * The position for `id` dropped before or after `targetId` in `open` (the rows on screen, in order): halfway between
 * the target and its neighbour on that side, or one past the target at an end. Null when nothing moves, or `id` isn't
 * in `open`.
 */
export function positionFor(open: readonly Todo[], id: string, targetId: string, where: 'before' | 'after'): number | null {
  if (id === targetId) return null;
  const rest = open.filter((t) => t.id !== id);
  const i = rest.findIndex((t) => t.id === targetId);
  if (i < 0) return null;
  const target = rest[i]!;
  const neighbour = where === 'before' ? rest[i - 1] : rest[i + 1];
  const from = open.findIndex((t) => t.id === id);
  // From another group (or list): a drop only reorders within one.
  if (from < 0) return null;
  const to = where === 'before' ? i : i + 1;
  if (from === to) return null;
  if (!neighbour) return where === 'before' ? target.position - 1 : target.position + 1;
  return (target.position + neighbour.position) / 2;
}

/** A day as the API keeps it: `YYYY-MM-DD` in the user's local time. */
export function localDay(d: Date): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** The local day `n` days after `day` (`YYYY-MM-DD`). */
export function addDays(day: string, n: number): string {
  const [y, m, d] = day.split('-').map(Number);
  return localDay(new Date(y!, m! - 1, d! + n));
}

/** How a todo's due date stands now: none (no date, or done), overdue, today, tomorrow or later. */
export type DueState = 'overdue' | 'today' | 'tomorrow' | 'later';

export function dueState(t: Pick<Todo, 'done' | 'dueOn' | 'dueTime'>, now: Date): DueState | null {
  if (t.done || !t.dueOn) return null;
  const today = localDay(now);
  if (t.dueOn < today) return 'overdue';
  if (t.dueOn === today) {
    return t.dueTime && t.dueTime <= hhmm(now) ? 'overdue' : 'today';
  }
  return t.dueOn === addDays(today, 1) ? 'tomorrow' : 'later';
}

/** A date for people: "Mon 2 Nov", with the year when it isn't this one. */
export function formatDay(day: string, now: Date): string {
  const [y, m, d] = day.split('-').map(Number);
  const date = new Date(y!, m! - 1, d!);
  return date.toLocaleDateString(undefined, {
    weekday: 'short', day: 'numeric', month: 'short', ...(y !== now.getFullYear() ? { year: 'numeric' } : {}),
  });
}

/** The due chip's text: Overdue, Today, Tomorrow or the date, with the time when there is one. Null: no chip. */
export function dueLabel(t: Pick<Todo, 'done' | 'dueOn' | 'dueTime'>, now: Date): string | null {
  const state = dueState(t, now);
  if (!state || !t.dueOn) return null;
  const day = { overdue: 'Overdue', today: 'Today', tomorrow: 'Tomorrow', later: formatDay(t.dueOn, now) }[state];
  return t.dueTime && state !== 'overdue' ? `${day} ${t.dueTime}` : day;
}

/** `HH:MM` of a moment, as the API keeps times. */
export function hhmm(d: Date): string {
  return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
}

/** The open list's groups, in this order; each keeps the manual order. */
export type DueGroupId = 'overdue' | 'today' | 'tomorrow' | 'week' | 'later' | 'none';
export const DUE_GROUPS: readonly { id: DueGroupId; label: string }[] = [
  { id: 'overdue', label: 'Overdue' },
  { id: 'today', label: 'Today' },
  { id: 'tomorrow', label: 'Tomorrow' },
  { id: 'week', label: 'Next 7 days' },
  { id: 'later', label: 'Later' },
  { id: 'none', label: 'No date' },
];

/** An open todo's group: Next 7 days is 2–7 days out, Later further. */
export function dueGroup(t: Todo, now: Date): DueGroupId {
  const state = dueState(t, now);
  if (!state) return 'none';
  if (state !== 'later') return state;
  return t.dueOn! <= addDays(localDay(now), 7) ? 'week' : 'later';
}

/** Open todos (already in their order) by group, leaving out empty groups. */
export function grouped(open: readonly Todo[], now: Date): { id: DueGroupId; label: string; todos: Todo[] }[] {
  return DUE_GROUPS.map((g) => ({ ...g, todos: open.filter((t) => dueGroup(t, now) === g.id) })).filter((g) => g.todos.length > 0);
}

/** Move to today's patch for an overdue todo: today, keeping its time only if that is still ahead. */
export function moveToToday(t: Pick<Todo, 'dueTime'>, now: Date): { dueOn: string; dueTime: string } {
  return { dueOn: localDay(now), dueTime: t.dueTime && t.dueTime > hhmm(now) ? t.dueTime : '' };
}

/** "In an hour": now + 60 minutes, to the nearest 5, as a day and a time (late at night, tomorrow's). */
export function inAnHour(now: Date): { dueOn: string; dueTime: string } {
  const five = 5 * 60_000;
  const at = new Date(Math.round((now.getTime() + 60 * 60_000) / five) * five);
  return { dueOn: localDay(at), dueTime: hhmm(at) };
}

/** The weekend pick: the coming Saturday, or on a Saturday or Sunday the following one ("Next weekend"). */
export function weekendPick(now: Date): { label: string; day: string } {
  const dow = now.getDay();
  const weekend = dow === 6 || dow === 0;
  const toSat = weekend ? (dow === 6 ? 7 : 6) : 6 - dow;
  return { label: weekend ? 'Next weekend' : 'This weekend', day: addDays(localDay(now), toSat) };
}

/** A quick pick in the due picker: a day, and a time only for In an hour. */
export type QuickPick = { id: 'hour' | 'today' | 'tomorrow' | 'weekend' | 'week'; label: string; dueOn: string; dueTime?: string };

/** The due picker's quick picks: in an hour, today, tomorrow, the weekend and next week (the coming Monday). */
export function quickPicks(now: Date): QuickPick[] {
  const today = localDay(now);
  const toMonday = ((8 - now.getDay()) % 7) || 7;
  const weekend = weekendPick(now);
  return [
    { id: 'hour', label: 'In an hour', ...inAnHour(now) },
    { id: 'today', label: 'Today', dueOn: today },
    { id: 'tomorrow', label: 'Tomorrow', dueOn: addDays(today, 1) },
    { id: 'weekend', label: weekend.label, dueOn: weekend.day },
    { id: 'week', label: 'Next week', dueOn: addDays(today, toMonday) },
  ];
}

/** Weekday names by the API's numbers (1 = Monday … 7 = Sunday), index 0 unused. */
export const WEEKDAYS = ['', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'] as const;

/** The weekday (1 = Monday … 7 = Sunday) of a `YYYY-MM-DD` day. */
export function weekdayOf(day: string): number {
  const [y, m, d] = day.split('-').map(Number);
  return ((new Date(y!, m! - 1, d!).getDay() + 6) % 7) + 1;
}

function ordinal(n: number): string {
  const tens = n % 100;
  const suffix = tens >= 11 && tens <= 13 ? 'th' : ({ 1: 'st', 2: 'nd', 3: 'rd' } as Record<number, string>)[n % 10] ?? 'th';
  return `${n}${suffix}`;
}

/** A repeat rule for people (PDT-141): "Every day", "Every 3 days", "Every weekday", "Every Mon, Wed", "Every 2 weeks on Fri", "Every month on the 31st". */
export function repeatText(r: Recurrence): string {
  const rule = r.rule;
  switch (rule.every) {
    case 'day':
      return rule.interval === 1 ? 'Every day' : `Every ${rule.interval} days`;
    case 'weekday':
      return 'Every weekday';
    case 'week': {
      const days = rule.days.map((d) => WEEKDAYS[d] ?? '?').join(', ');
      return rule.interval === 1 ? `Every ${days}` : `Every ${rule.interval} weeks on ${days}`;
    }
    case 'month':
      return `Every month on the ${ordinal(rule.day)}`;
  }
}

/** The due picker's repeat presets, built from the chosen day (`YYYY-MM-DD`). */
export function repeatPresets(day: string): { label: string; detail: string; rule: RepeatRule }[] {
  const wd = weekdayOf(day);
  const on = `on ${WEEKDAYS[wd]}`;
  return [
    { label: 'Every day', detail: '', rule: { every: 'day', interval: 1 } },
    { label: 'Every weekday', detail: 'Mon–Fri', rule: { every: 'weekday' } },
    { label: 'Every week', detail: on, rule: { every: 'week', interval: 1, days: [wd] } },
    { label: 'Every 2 weeks', detail: on, rule: { every: 'week', interval: 2, days: [wd] } },
    { label: 'Every month', detail: `on the ${ordinal(Number(day.slice(8)))}`, rule: { every: 'month', day: Number(day.slice(8)) } },
  ];
}

/** Whether two rules are the same. */
export function sameRule(a: RepeatRule, b: RepeatRule): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/** The rule the Repeat editor starts from for a todo without one: weekly on its due day's weekday (today's if undated). */
export function defaultRecurrence(dueOn: string | null, now: Date): Recurrence {
  return { rule: { every: 'week', interval: 1, days: [weekdayOf(dueOn ?? localDay(now))] }, from: 'due' };
}

/** A body's first line with text, without its Markdown marks (heading, list, quote, task box), for the row (PDT-227). */
export function bodyPreview(body: string): string {
  const line = body.split(/\r?\n/).find((l) => l.trim()) ?? '';
  return line
    .trim()
    .replace(/^(#{1,6}\s+|>\s*|[-*+]\s+(\[[ xX]\]\s+)?|\d+[.)]\s+(\[[ xX]\]\s+)?)+/, '')
    .replace(/[*_`~]/g, '')
    .trim();
}
