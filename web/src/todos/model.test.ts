import { describe, expect, it } from 'vitest';
import type { Todo } from '../api/gen/Todo';
import { addDays, defaultRecurrence, dueLabel, dueState, grouped, inAnHour, localDay, moveToToday, positionFor, quickPicks, repeatPresets, repeatText, shown, weekdayOf, weekendPick } from './model';

function todo(id: string, position: number, more: Partial<Todo> = {}): Todo {
  return {
    id, title: id.toUpperCase(), position, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
    doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, dueOn: null, dueTime: null, recurrence: null, ...more,
  };
}

describe('shown', () => {
  const list = [
    todo('b', 2, { projectId: 'p' }),
    todo('a', 1),
    todo('c', 0, { done: true, doneAt: 5, projectId: 'p' }),
    todo('d', 3, { done: true, doneAt: 9 }),
  ];
  it('splits open todos in order from done ones, latest first', () => {
    const s = shown(list, 'all', 'p');
    expect(s.open.map((t) => t.id)).toEqual(['a', 'b']);
    expect(s.done.map((t) => t.id)).toEqual(['d', 'c']);
  });
  it('keeps to the current project', () => {
    const s = shown(list, 'project', 'p');
    expect([...s.open, ...s.done].map((t) => t.id)).toEqual(['b', 'c']);
    expect(shown(list, 'project', null).open).toHaveLength(2);
  });
});

describe('positionFor', () => {
  const open = [todo('a', 1), todo('b', 2), todo('c', 3)];
  it('goes halfway to the neighbour', () => {
    expect(positionFor(open, 'c', 'b', 'before')).toBe(1.5);
    expect(positionFor(open, 'a', 'b', 'after')).toBe(2.5);
  });
  it('goes one past the target at an end', () => {
    expect(positionFor(open, 'c', 'a', 'before')).toBe(0);
    expect(positionFor(open, 'a', 'c', 'after')).toBe(4);
  });
  it('is null when nothing moves', () => {
    expect(positionFor(open, 'a', 'a', 'before')).toBeNull();
    expect(positionFor(open, 'a', 'b', 'before')).toBeNull();
    expect(positionFor(open, 'b', 'a', 'after')).toBeNull();
    expect(positionFor(open, 'a', 'gone', 'after')).toBeNull();
    expect(positionFor(open, 'elsewhere', 'b', 'after'), 'from another group').toBeNull();
  });
});

describe('due dates', () => {
  // Friday 23 October 2026, 10:00 local.
  const now = new Date(2026, 9, 23, 10, 0);
  const due = (dueOn: string | null, dueTime: string | null = null, done = false) => ({ dueOn, dueTime, done });

  it('works in local days', () => {
    expect(localDay(now)).toBe('2026-10-23');
    expect(addDays('2026-10-31', 1)).toBe('2026-11-01');
    expect(addDays('2026-12-31', 1)).toBe('2027-01-01');
  });

  it('chips overdue, today, tomorrow, then the date', () => {
    expect(dueState(due('2026-10-22'), now)).toBe('overdue');
    expect(dueLabel(due('2026-10-22'), now)).toBe('Overdue');
    expect(dueLabel(due('2026-10-23'), now)).toBe('Today');
    expect(dueLabel(due('2026-10-23', '14:30'), now)).toBe('Today 14:30');
    expect(dueLabel(due('2026-10-23', '09:30'), now), 'its time has passed').toBe('Overdue');
    expect(dueLabel(due('2026-10-24'), now)).toBe('Tomorrow');
    expect(dueLabel(due('2026-11-02'), now)).toMatch(/2/);
    expect(dueLabel(due('2026-11-02'), now)).not.toMatch(/2026/);
    expect(dueLabel(due('2027-01-05'), now)).toMatch(/2027/);
  });

  it('shows no chip on done or undated todos', () => {
    expect(dueLabel(due('2026-10-22', null, true), now)).toBeNull();
    expect(dueLabel(due(null), now)).toBeNull();
  });

  it('groups open todos as Overdue, Today, Tomorrow, Next 7 days, Later and No date, keeping their order', () => {
    const open = [
      todo('a', 1, { dueOn: '2026-11-02' }),
      todo('b', 2),
      todo('c', 3, { dueOn: '2026-10-23' }),
      todo('d', 4, { dueOn: '2026-10-01' }),
      todo('e', 5, { dueOn: '2026-10-24' }),
      todo('f', 6, { dueOn: '2026-10-23', dueTime: '08:00' }),
      todo('g', 7, { dueOn: '2026-10-25' }),
      todo('h', 8, { dueOn: '2026-10-30' }),
      todo('i', 9, { dueOn: '2026-10-31' }),
      todo('j', 10, { dueOn: '2026-10-23', dueTime: '10:00' }),
      todo('k', 11, { dueOn: '2026-10-23', dueTime: '10:01' }),
    ];
    expect(grouped(open, now).map((g) => [g.label, g.todos.map((t) => t.id)])).toEqual([
      ['Overdue', ['d', 'f', 'j']],
      ['Today', ['c', 'k']],
      ['Tomorrow', ['e']],
      ['Next 7 days', ['g', 'h']],
      ['Later', ['a', 'i']],
      ['No date', ['b']],
    ]);
    expect(grouped([todo('x', 1)], now).map((g) => g.id)).toEqual(['none']);
  });

  it('moves an overdue todo to today, keeping a time still ahead and dropping a past one', () => {
    expect(moveToToday(due('2026-10-20'), now)).toEqual({ dueOn: '2026-10-23', dueTime: '' });
    expect(moveToToday(due('2026-10-20', '15:00'), now)).toEqual({ dueOn: '2026-10-23', dueTime: '15:00' });
    expect(moveToToday(due('2026-10-20', '09:00'), now)).toEqual({ dueOn: '2026-10-23', dueTime: '' });
    expect(moveToToday(due('2026-10-23', '10:00'), now), 'due this very minute').toEqual({ dueOn: '2026-10-23', dueTime: '' });
  });

  it('puts "In an hour" an hour on, to the nearest 5 minutes, rolling into tomorrow late at night', () => {
    expect(inAnHour(new Date(2026, 9, 23, 14, 43))).toEqual({ dueOn: '2026-10-23', dueTime: '15:45' });
    expect(inAnHour(new Date(2026, 9, 23, 14, 42))).toEqual({ dueOn: '2026-10-23', dueTime: '15:40' });
    expect(inAnHour(new Date(2026, 9, 23, 14, 58))).toEqual({ dueOn: '2026-10-23', dueTime: '16:00' });
    expect(inAnHour(new Date(2026, 9, 23, 23, 20))).toEqual({ dueOn: '2026-10-24', dueTime: '00:20' });
    expect(inAnHour(new Date(2026, 9, 23, 22, 58)), 'rounds up past midnight').toEqual({ dueOn: '2026-10-24', dueTime: '00:00' });
    expect(inAnHour(new Date(2026, 11, 31, 23, 30))).toEqual({ dueOn: '2027-01-01', dueTime: '00:30' });
  });

  it('picks this weekend on a weekday and next weekend on a Saturday or Sunday', () => {
    // Mon 19 … Sun 25 October 2026.
    const picks = [19, 20, 21, 22, 23, 24, 25].map((d) => weekendPick(new Date(2026, 9, d, 9)));
    expect(picks.map((p) => p.day)).toEqual(['2026-10-24', '2026-10-24', '2026-10-24', '2026-10-24', '2026-10-24', '2026-10-31', '2026-10-31']);
    expect(picks.map((p) => p.label)).toEqual([...Array(5).fill('This weekend'), 'Next weekend', 'Next weekend']);
  });

  it('offers in an hour, today, tomorrow, the weekend and the coming Monday', () => {
    expect(quickPicks(now).map((q) => [q.label, q.dueOn, q.dueTime ?? null])).toEqual([
      ['In an hour', '2026-10-23', '11:00'],
      ['Today', '2026-10-23', null],
      ['Tomorrow', '2026-10-24', null],
      ['This weekend', '2026-10-24', null],
      ['Next week', '2026-10-26', null],
    ]);
    expect(quickPicks(new Date(2026, 9, 26, 9)).at(-1)?.dueOn, 'a Monday picks the next one').toBe('2026-11-02');
  });
});

describe('repeatText', () => {
  it('says each rule', () => {
    const text = (rule: Parameters<typeof repeatText>[0]['rule']) => repeatText({ rule, from: 'due' });
    expect(text({ every: 'day', interval: 1 })).toBe('Every day');
    expect(text({ every: 'day', interval: 3 })).toBe('Every 3 days');
    expect(text({ every: 'weekday' })).toBe('Every weekday');
    expect(text({ every: 'week', interval: 1, days: [1, 3] })).toBe('Every Mon, Wed');
    expect(text({ every: 'week', interval: 2, days: [5] })).toBe('Every 2 weeks on Fri');
    expect(text({ every: 'month', day: 31 })).toBe('Every month on the 31st');
    expect(text({ every: 'month', day: 2 })).toBe('Every month on the 2nd');
    expect(text({ every: 'month', day: 13 })).toBe('Every month on the 13th');
    expect(text({ every: 'month', day: 23 })).toBe('Every month on the 23rd');
  });
  it('starts a new rule on the due day', () => {
    expect(weekdayOf('2026-10-25')).toBe(7);
    expect(weekdayOf('2026-10-26')).toBe(1);
    expect(defaultRecurrence('2026-10-28', new Date(2026, 9, 23))).toEqual({ rule: { every: 'week', interval: 1, days: [3] }, from: 'due' });
    expect(defaultRecurrence(null, new Date(2026, 9, 23)).rule).toEqual({ every: 'week', interval: 1, days: [5] });
  });
  it('offers presets built from the chosen day', () => {
    expect(repeatPresets('2026-10-28').map((p) => [p.label, p.detail, p.rule])).toEqual([
      ['Every day', '', { every: 'day', interval: 1 }],
      ['Every weekday', 'Mon–Fri', { every: 'weekday' }],
      ['Every week', 'on Wed', { every: 'week', interval: 1, days: [3] }],
      ['Every 2 weeks', 'on Wed', { every: 'week', interval: 2, days: [3] }],
      ['Every month', 'on the 28th', { every: 'month', day: 28 }],
    ]);
  });
});
