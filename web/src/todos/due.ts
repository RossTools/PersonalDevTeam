import type { Recurrence } from '../api/gen/Recurrence';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { addDays, formatDay, localDay } from './model';

/** What the picker edits: a todo's due day, time and repeat rule. */
export type DueValue = { dueOn: string | null; dueTime: string | null; recurrence: Recurrence | null };
export type DuePanel = 'date' | 'time' | 'repeat';

/** A patch applied to a value, as the server would (clearing the day clears its time and rule). */
export function applyDuePatch(v: DueValue, p: TodoPatch): DueValue {
  const cleared = p.dueOn === '';
  return {
    dueOn: p.dueOn !== undefined ? p.dueOn || null : v.dueOn,
    dueTime: cleared ? null : p.dueTime !== undefined ? p.dueTime || null : v.dueTime,
    recurrence: cleared ? null : p.recurrence !== undefined ? p.recurrence : v.recurrence,
  };
}

/** A day for people: Today, Tomorrow, or "Wed 28 Oct". */
export function dayText(day: string, now: Date): string {
  const today = localDay(now);
  if (day === today) return 'Today';
  if (day === addDays(today, 1)) return 'Tomorrow';
  return formatDay(day, now);
}
