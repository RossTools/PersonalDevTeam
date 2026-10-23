// The todo list's pure rules (PDT-125): filtering, splitting open from done, and the position a moved row takes.
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
 * the target and its neighbour on that side, or one past the target at an end. Null when nothing moves.
 */
export function positionFor(open: readonly Todo[], id: string, targetId: string, where: 'before' | 'after'): number | null {
  if (id === targetId) return null;
  const rest = open.filter((t) => t.id !== id);
  const i = rest.findIndex((t) => t.id === targetId);
  if (i < 0) return null;
  const target = rest[i]!;
  const neighbour = where === 'before' ? rest[i - 1] : rest[i + 1];
  const from = open.findIndex((t) => t.id === id);
  const to = where === 'before' ? i : i + 1;
  if (from === to) return null;
  if (!neighbour) return where === 'before' ? target.position - 1 : target.position + 1;
  return (target.position + neighbour.position) / 2;
}
