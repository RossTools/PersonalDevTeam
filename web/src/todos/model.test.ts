import { describe, expect, it } from 'vitest';
import type { Todo } from '../api/gen/Todo';
import { addDays, dueLabel, dueState, grouped, localDay, positionFor, quickDays, shown } from './model';

function todo(id: string, position: number, more: Partial<Todo> = {}): Todo {
  return {
    id, title: id.toUpperCase(), position, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
    doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, dueOn: null, dueTime: null, ...more,
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

  it('groups open todos as Overdue, Today, Upcoming and No date, keeping their order', () => {
    const open = [
      todo('a', 1, { dueOn: '2026-11-02' }),
      todo('b', 2),
      todo('c', 3, { dueOn: '2026-10-23' }),
      todo('d', 4, { dueOn: '2026-10-01' }),
      todo('e', 5, { dueOn: '2026-10-24' }),
      todo('f', 6, { dueOn: '2026-10-23', dueTime: '08:00' }),
    ];
    expect(grouped(open, now).map((g) => [g.label, g.todos.map((t) => t.id)])).toEqual([
      ['Overdue', ['d', 'f']],
      ['Today', ['c']],
      ['Upcoming', ['a', 'e']],
      ['No date', ['b']],
    ]);
    expect(grouped([todo('x', 1)], now).map((g) => g.id)).toEqual(['none']);
  });

  it('offers today, tomorrow and the coming Monday', () => {
    expect(quickDays(now).map((q) => q.day)).toEqual(['2026-10-23', '2026-10-24', '2026-10-26']);
    expect(quickDays(new Date(2026, 9, 26, 9)).map((q) => q.day)[2], 'a Monday picks the next one').toBe('2026-11-02');
  });
});
