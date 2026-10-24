import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import type { NewTodo } from '../api/gen/NewTodo';
import type { Todo } from '../api/gen/Todo';
import type { TodoPatch } from '../api/gen/TodoPatch';
import { runAction } from '../common/useAction';
import { POLL } from '../tickets/queries';

export const TODOS_KEY = ['todos'] as const;

/** Every todo (the list filters on the client), polled like the rest of the UI. */
export function useTodos() {
  return useQuery({ queryKey: TODOS_KEY, queryFn: () => api.todos.list(), refetchInterval: POLL });
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
    remove: (ids: string[]) => {
      edit((list) => list.filter((t) => !ids.includes(t.id)));
      return runAction(async () => {
        try {
          await Promise.all(ids.map((id) => api.todos.remove(id)));
        } finally {
          refresh(qc);
        }
      });
    },
  };
}

/** What a patch looks like before the server answers (links wait for it: their keys come from the server). */
function local(t: Todo, p: TodoPatch): Todo {
  return {
    ...t,
    ...(p.title !== undefined ? { title: p.title } : {}),
    ...(p.position !== undefined ? { position: p.position } : {}),
    ...(p.dueOn !== undefined ? { dueOn: p.dueOn || null, ...(p.dueOn ? {} : { dueTime: null }) } : {}),
    ...(p.dueTime !== undefined ? { dueTime: p.dueTime || null } : {}),
    ...(p.done !== undefined ? { done: p.done, doneAt: p.done ? (t.doneAt ?? Date.now()) : null } : {}),
  };
}
