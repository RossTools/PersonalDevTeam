import { describe, expect, it } from 'vitest';
import type { Todo } from '../api/gen/Todo';
import { positionFor, shown } from './model';

function todo(id: string, position: number, more: Partial<Todo> = {}): Todo {
  return {
    id, title: id.toUpperCase(), position, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
    doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, ...more,
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
  });
});
