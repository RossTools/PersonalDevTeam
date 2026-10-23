import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import { OpenTicketContext } from '../tickets/actions';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { TodoList } from './TodoList';

const todo = (id: string, position: number, more: Partial<Todo> = {}): Todo => ({
  id, title: `Todo ${id}`, position, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
  doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, ...more,
});
const project = { id: 'p1', key: 'PX', name: 'P' } as Project;
const list = [
  todo('a', 1),
  todo('b', 2, { projectId: 'p1', projectKey: 'PX', ticketId: 't1', ticketKey: 'PX-7', ticketTitle: 'Fix the login' }),
  todo('c', 3, { projectId: 'p1', projectKey: 'PX' }),
  todo('d', 0, { done: true, doneAt: 5 }),
];

let calls: Call[];
beforeEach(() => {
  localStorage.clear();
  calls = stubApi({
    'GET /api/todos': list,
    'POST /api/todos': (c: Call) => todo('new', -1, { title: (c.body as { title: string }).title }),
    'PATCH /api/todos/a': (c: Call) => ({ ...list[0], ...(c.body as object) }),
    'PATCH /api/todos/c': (c: Call) => ({ ...list[2], ...(c.body as object) }),
    'DELETE /api/todos/a': undefined,
    'DELETE /api/todos/d': undefined,
  });
});
afterEach(() => vi.unstubAllGlobals());

const sent = (method: string, path: string) => calls.filter((c) => c.method === method && c.path === path).map((c) => c.body);
const open = () => screen.getByRole('list', { name: 'Todos' });
const titles = () => within(open()).getAllByRole('listitem').map((li) => li.getAttribute('aria-label'));

function show(projectId: string | null = 'p1', openTicket = vi.fn()) {
  renderWithQuery(
    <OpenTicketContext.Provider value={openTicket}>
      <TodoList projectId={projectId} projects={[project]} />
    </OpenTicketContext.Provider>,
  );
  return openTicket;
}

describe('TodoList', () => {
  it('adds a todo with Enter', async () => {
    show();
    await screen.findByText('Todo a');
    await userEvent.type(screen.getByRole('textbox', { name: 'Add a todo' }), 'Call Sam{Enter}');
    await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Call Sam' }]));
    expect(screen.getByRole('textbox', { name: 'Add a todo' })).toHaveValue('');
  });

  it('checks off, edits in place, reorders and deletes', async () => {
    show();
    expect(await screen.findByText('Todo a')).toBeInTheDocument();
    expect(titles()).toEqual(['Todo a', 'Todo b', 'Todo c']);

    await userEvent.click(screen.getByRole('checkbox', { name: 'Done: Todo a' }));
    expect(sent('PATCH', '/api/todos/a')).toEqual([{ done: true }]);

    await userEvent.click(screen.getByRole('button', { name: 'Todo c' }));
    const input = screen.getByRole('textbox', { name: 'Todo' });
    await userEvent.clear(input);
    await userEvent.type(input, 'Todo see{Enter}');
    expect(sent('PATCH', '/api/todos/c')).toEqual([{ title: 'Todo see' }]);

    // Alt+↑ moves c above b: halfway between a and b.
    const row = within(open()).getByRole('listitem', { name: /^Todo (c|see)$/ });
    within(row).getByRole('button', { name: /^Todo (c|see)$/ }).focus();
    await userEvent.keyboard('{Alt>}{ArrowUp}{/Alt}');
    await waitFor(() => expect(sent('PATCH', '/api/todos/c')).toContainEqual({ position: 1.5 }));

    await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo a' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/todos/a')).toBe(true));
  });

  it('shows project and ticket chips, and the ticket chip opens the ticket', async () => {
    const openTicket = show();
    await screen.findByText('Todo a');
    const b = within(open()).getByRole('listitem', { name: 'Todo b' });
    await userEvent.click(within(b).getByRole('button', { name: 'PX-7' }));
    expect(openTicket).toHaveBeenCalledWith({ id: 't1', projectId: 'p1' });
    const c = within(open()).getByRole('listitem', { name: 'Todo c' });
    expect(within(c).getByText('PX')).toBeInTheDocument();
  });

  it('filters to the current project, where new todos join it', async () => {
    show();
    await screen.findByText('Todo a');
    await userEvent.click(screen.getByRole('radio', { name: 'PX' }));
    expect(titles()).toEqual(['Todo b', 'Todo c']);
    await userEvent.type(screen.getByRole('textbox', { name: 'Add a todo' }), 'Ship{Enter}');
    await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Ship', project: 'p1' }]));
  });

  it('folds done todos into "Done (n)" and clears them', async () => {
    show();
    const toggle = await screen.findByRole('button', { name: 'Done (1)' });
    expect(screen.queryByText('Todo d')).toBeNull();
    await userEvent.click(toggle);
    expect(within(screen.getByRole('list', { name: 'Done todos' })).getByText('Todo d')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Clear done' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/todos/d')).toBe(true));
  });

  it('links a todo to a ticket from its menu', async () => {
    calls = stubApi({
      'GET /api/todos': list,
      'GET /api/tickets': { items: [{ id: 't9', key: 'PX-9', title: 'Add todos', stage: 'todo', projectId: 'p1' }], total: 1 },
      'PATCH /api/todos/a': list[0],
    });
    show();
    await screen.findByText('Todo a');
    await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo a' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Link to a ticket…' }));
    await userEvent.click(await within(await screen.findByRole('dialog')).findByText('PX-9 · Add todos'));
    await waitFor(() => expect(sent('PATCH', '/api/todos/a')).toEqual([{ ticket: 't9' }]));
  });
});
