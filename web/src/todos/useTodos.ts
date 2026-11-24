import { useMemo } from 'react';
import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import type { NewTodo } from '../api/gen/NewTodo';
import type { Todo } from '../api/gen/Todo';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { runAction } from '../common/useAction';
import { POLL } from '../common/poll';
import { holdAnswer, undoToast, useHeldTodoIds } from '../inbox/held';

export const TODOS_KEY = ['todos'] as const;

/**
 * Every todo (the list filters on the client), polled like the rest of the UI; those whose delete is held for Undo are
 * left out.
 */
export function useTodos() {
  const held = useHeldTodoIds();
  const q = useQuery({ queryKey: TODOS_KEY, queryFn: () => api.todos.list(), refetchInterval: POLL.live });
  const data = useMemo(() => (held.size && q.data ? q.data.filter((t) => !held.has(t.id)) : q.data), [q.data, held]);
  return { ...q, data };
}

/** How many todos are still open (the collapsed panel's count). */
export function useOpenTodoCount(): number {
  return useTodos().data?.filter((t) => !t.done).length ?? 0;
}

function refresh(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: TODOS_KEY });
}

/** The list's changes: each shows at once, then the server's answer replaces it (errors toast and refetch). */
export function useTodoActions() {
  const qc = useQueryClient();
  const edit = (fn: (list: Todo[]) => Todo[]) => qc.setQueryData<Todo[]>(TODOS_KEY, (old) => (old ? fn(old) : old));
  return {
    add: (t: NewTodo) =>
      runAction(async () => {
        const made = await api.todos.create(t);
        edit((list) => [made, ...list]);
        refresh(qc);
        return made;
      }),
    patch: (id: string, p: TodoPatch) => {
      edit((list) => list.map((t) => (t.id === id ? local(t, p) : t)));
      return runAction(async () => {
        try {
          const saved = await api.todos.patch(id, p);
          edit((list) => list.map((t) => (t.id === id ? saved : t)));
        } finally {
          refresh(qc);
        }
      });
    },
    /**
     * Deletes `ids` after the Undo window (PDT-715): they leave the list at once, with a toast offering Undo, and go
     * out together (one todo alone takes `series`, for a repeating one: `only` makes the next one in its place,
     * `series` stops it repeating). `label` is the toast's ("Deleted", "Cleared completed").
     */
    remove: (ids: string[], label: string, series?: 'only' | 'series') => {
      if (ids.length === 0) return;
      const title = ids.length === 1 ? (qc.getQueryData<Todo[]>(TODOS_KEY)?.find((t) => t.id === ids[0])?.title ?? '') : `${ids.length} todos`;
      const key = `todos:${ids.join(',')}`;
      holdAnswer({
        itemId: key, title, label, scope: 'todos', hides: ids,
        send: () => runAction(async () => {
          try {
            await (ids.length === 1 ? api.todos.remove(ids[0]!, series) : api.todos.removeMany(ids));
            edit((list) => list.filter((t) => !ids.includes(t.id)));
          } finally {
            refresh(qc);
          }
        }),
      });
      undoToast(key, title, label);
    },
  };
}

/** What a patch looks like before the server answers (links wait for it: their keys come from the server). */
function local(t: Todo, p: TodoPatch): Todo {
  return {
    ...t,
    ...(p.title !== undefined ? { title: p.title } : {}),
    ...(p.position !== undefined ? { position: p.position } : {}),
    ...(p.dueOn !== undefined ? { dueOn: p.dueOn || null, ...(p.dueOn ? {} : { dueTime: null, recurrence: null }) } : {}),
    ...(p.recurrence !== undefined ? { recurrence: p.recurrence } : {}),
    ...(p.dueTime !== undefined ? { dueTime: p.dueTime || null } : {}),
    ...(p.done !== undefined ? { done: p.done, doneAt: p.done ? (t.doneAt ?? Date.now()) : null } : {}),
  };
}
