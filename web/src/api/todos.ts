import { enc, queryString, request } from './core';
import type { NewTodo } from './gen/NewTodo';
import type { Todo } from './gen/Todo';
import type { TodoPatch } from './gen/TodoPatch';

/** The user's todo list. */
export const todosApi = {
  /** The user's todo list (PDT-125): open ones in their order, then done ones (latest first). */
  todos: {
    list: (q: { project?: string; done?: boolean } = {}) => request<Todo[]>(`/api/todos${queryString(q)}`),
    create: (t: NewTodo) => request<Todo>('/api/todos', { method: 'POST', body: JSON.stringify(t) }),
    patch: (id: string, p: TodoPatch) => request<Todo>(`/api/todos/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }),
    /** `series` (a repeating todo): `only` skips to the next one, `series` stops it repeating. */
    remove: (id: string, series?: 'only' | 'series') => request<void>(`/api/todos/${enc(id)}${queryString({ series })}`, { method: 'DELETE' }),
  },
};
