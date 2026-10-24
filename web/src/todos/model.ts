// The todo list's pure rules (PDT-125): filtering, splitting open from done, and the position a moved row takes; due
// dates (PDT-140): the user's local day, the chip a card shows, the groups open todos fall into and the quick picks.
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
    const hm = `${String(now.getHours()).padStart(2, '0')}:${String(now.getMinutes()).padStart(2, '0')}`;
    return t.dueTime && t.dueTime <= hm ? 'overdue' : 'today';
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

/** The open list's groups, in this order; each keeps the manual order. */
export type DueGroupId = 'overdue' | 'today' | 'upcoming' | 'none';
export const DUE_GROUPS: readonly { id: DueGroupId; label: string }[] = [
  { id: 'overdue', label: 'Overdue' },
  { id: 'today', label: 'Today' },
  { id: 'upcoming', label: 'Upcoming' },
  { id: 'none', label: 'No date' },
];

export function dueGroup(t: Todo, now: Date): DueGroupId {
  const state = dueState(t, now);
  if (state === 'overdue' || state === 'today') return state;
  return state ? 'upcoming' : 'none';
}

/** Open todos (already in their order) by group, leaving out empty groups. */
export function grouped(open: readonly Todo[], now: Date): { id: DueGroupId; label: string; todos: Todo[] }[] {
  return DUE_GROUPS.map((g) => ({ ...g, todos: open.filter((t) => dueGroup(t, now) === g.id) })).filter((g) => g.todos.length > 0);
}

/** The menu's quick picks: today, tomorrow and next week (the coming Monday). */
export function quickDays(now: Date): { label: string; day: string }[] {
  const today = localDay(now);
  const toMonday = ((8 - now.getDay()) % 7) || 7;
  return [
    { label: 'Today', day: today },
    { label: 'Tomorrow', day: addDays(today, 1) },
    { label: 'Next week', day: addDays(today, toMonday) },
  ];
}
