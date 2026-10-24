import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import { OpenTicketContext } from '../tickets/actions';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { TodoList, type TodoFocus } from './TodoList';

const todo = (id: string, position: number, more: Partial<Todo> = {}): Todo => ({
  id, title: `Todo ${id}`, position, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
  doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, dueOn: null, dueTime: null, ...more,
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

function show(projectId: string | null = 'p1', openTicket = vi.fn(), focus: TodoFocus | null = null) {
  renderWithQuery(
    <OpenTicketContext.Provider value={openTicket}>
      <TodoList projectId={projectId} projects={[project]} focus={focus} />
    </OpenTicketContext.Provider>,
  );
  return openTicket;
}

/** A DataTransfer for fireEvent (jsdom has none). */
function transfer() {
  const data: Record<string, string> = {};
  return { data, types: ['text/plain'], setData: (k: string, v: string) => { data[k] = v; }, getData: (k: string) => data[k] ?? '', effectAllowed: 'all', dropEffect: 'none' };
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

describe('TodoList with due dates', () => {
  // Friday 23 October 2026, 10:00 local.
  beforeEach(() => {
    vi.useFakeTimers({ toFake: ['Date'] });
    vi.setSystemTime(new Date(2026, 9, 23, 10, 0));
  });
  afterEach(() => vi.useRealTimers());

  const dated = [
    todo('late', 1, { dueOn: '2026-10-20' }),
    todo('none', 2),
    todo('today', 3, { dueOn: '2026-10-23', dueTime: '17:00' }),
    todo('tmrw', 4, { dueOn: '2026-10-24' }),
    todo('later', 5, { dueOn: '2026-11-03' }),
    todo('late2', 6, { dueOn: '2026-10-22' }),
    todo('old', 0, { done: true, doneAt: 5, dueOn: '2026-10-01' }),
  ];
  const group = (name: string) => screen.getByRole('list', { name: `${name} todos` });
  const names = (name: string) => within(group(name)).getAllByRole('listitem').map((li) => li.getAttribute('aria-label'));
  const chip = (title: string) => within(screen.getByRole('listitem', { name: title })).queryByLabelText(/^Due /);

  function stubDated() {
    calls = stubApi({
      'GET /api/todos': dated,
      'POST /api/todos': (c: Call) => todo('new', -1, c.body as Partial<Todo>),
      'PATCH /api/todos/late': (c: Call) => ({ ...dated[0], ...(c.body as object) }),
      'PATCH /api/todos/late2': (c: Call) => ({ ...dated[5], ...(c.body as object) }),
      'PATCH /api/todos/none': (c: Call) => ({ ...dated[1], ...(c.body as object) }),
    });
  }

  it('groups as Overdue, Today, Upcoming and No date, in their order', async () => {
    stubDated();
    show();
    await screen.findByText('Todo late');
    expect(screen.getAllByRole('heading', { level: 3 }).map((h) => h.textContent)).toEqual(['Overdue (2)', 'Today (1)', 'Upcoming (2)', 'No date (1)']);
    expect(names('Overdue')).toEqual(['Todo late', 'Todo late2']);
    expect(names('Today')).toEqual(['Todo today']);
    expect(names('Upcoming')).toEqual(['Todo tmrw', 'Todo later']);
    expect(names('No date')).toEqual(['Todo none']);
  });

  it('chips overdue in red, today, tomorrow and the date; none on done cards', async () => {
    stubDated();
    show();
    await screen.findByText('Todo late');
    expect(chip('Todo late')).toHaveTextContent('Overdue');
    expect(chip('Todo late')?.className).toMatch(/failed|destructive/);
    expect(chip('Todo today')).toHaveTextContent('Today 17:00');
    expect(chip('Todo tmrw')).toHaveTextContent('Tomorrow');
    expect(chip('Todo later')?.textContent).toMatch(/3/);
    expect(chip('Todo none')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Done (1)' }));
    expect(chip('Todo old')).toBeNull();
  });

  it('drags to reorder within a group, never across', async () => {
    stubDated();
    show();
    await screen.findByText('Todo late');
    const card = (title: string) => screen.getByRole('listitem', { name: title });
    // jsdom has no layout: a drop lands on the target's bottom half.
    let dt = transfer();
    fireEvent.dragStart(card('Todo late'), { dataTransfer: dt });
    fireEvent.dragOver(card('Todo late2'), { dataTransfer: dt });
    fireEvent.drop(card('Todo late2'), { dataTransfer: dt });
    await waitFor(() => expect(sent('PATCH', '/api/todos/late')).toEqual([{ position: 7 }]));

    dt = transfer();
    fireEvent.dragStart(card('Todo none'), { dataTransfer: dt });
    fireEvent.drop(card('Todo late2'), { dataTransfer: dt });
    expect(sent('PATCH', '/api/todos/none')).toEqual([]);
  });

  it('sets a due date from the card menu: quick picks, a picked date, or none', async () => {
    stubDated();
    show();
    await screen.findByText('Todo none');
    await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo none' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Due date' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: /^Tomorrow/ }));
    await waitFor(() => expect(sent('PATCH', '/api/todos/none')).toEqual([{ dueOn: '2026-10-24' }]));

    await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo late' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Due date' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'No due date' }));
    await waitFor(() => expect(sent('PATCH', '/api/todos/late')).toEqual([{ dueOn: '' }]));

    await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo late2' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Due date' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Pick a date…' }));
    const dialog = await screen.findByRole('dialog');
    fireEvent.change(within(dialog).getByLabelText('Date'), { target: { value: '2026-12-01' } });
    fireEvent.change(within(dialog).getByLabelText(/^Time/), { target: { value: '08:15' } });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(sent('PATCH', '/api/todos/late2')).toEqual([{ dueOn: '2026-12-01', dueTime: '08:15' }]));
  });

  it('adds a todo with a due date', async () => {
    stubDated();
    show();
    await screen.findByText('Todo late');
    await userEvent.click(screen.getByRole('button', { name: 'Due date for the new todo' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: /^Today/ }));
    expect(screen.getByText(/^Due /)).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Add a todo' }), 'Pay rent{Enter}');
    await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Pay rent', dueOn: '2026-10-23' }]));
  });

  it('shows a todo a reminder opened, even when the filter hid it', async () => {
    stubDated();
    localStorage.setItem('pdt.todos.filter', 'project');
    show('p1', vi.fn(), { id: 'late', seq: 1 });
    const card = await screen.findByRole('listitem', { name: 'Todo late' });
    expect(card.className).toMatch(/ring-2/);
  });
});
