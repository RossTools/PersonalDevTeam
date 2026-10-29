import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { useState } from 'react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Todo } from '../api/gen/Todo';
import { OpenTicketContext } from '../tickets/actions';
import { editorText, renderWithQuery, setEditorText, stubApi, type Call } from '../test-utils';
import { AUTOSAVE_MS } from '../notes/useNoteDraft';
import { TodoList, type TodoFocus } from './TodoList';

const todo = (id: string, position: number, more: Partial<Todo> = {}): Todo => ({
  id, title: `Todo ${id}`, position, done: false, projectId: null, ticketId: null, createdAt: 1, updatedAt: 1,
  doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, body: '', dueOn: null, dueTime: null, recurrence: null, ...more,
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

/** Picks a scope from the header's dropdown. */
async function scope(name: string) {
  await userEvent.click(screen.getByRole('button', { name: /^Scope: / }));
  await userEvent.click(await screen.findByRole('menuitem', { name: new RegExp(`^(✓ )?${name}$`) }));
}

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

    await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo c' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Edit' }));
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

  it('scopes to the current project, where new todos join it', async () => {
    show();
    await screen.findByText('Todo a');
    await scope('PX');
    expect(titles()).toEqual(['Todo b', 'Todo c']);
    await userEvent.type(screen.getByRole('textbox', { name: 'Add a todo' }), 'Ship{Enter}');
    await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Ship', project: 'p1' }]));
  });

  it('folds done todos into "Completed · n" and clears them', async () => {
    show();
    const toggle = await screen.findByRole('button', { name: 'Completed · 1' });
    expect(screen.queryByText('Todo d')).toBeNull();
    await userEvent.click(toggle);
    expect(within(screen.getByRole('list', { name: 'Completed todos' })).getByText('Todo d')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Clear' }));
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

describe('An open todo (PDT-227)', () => {
  const noted = [
    todo('a', 1, { body: '## Bring\n\n- the **forms**' }),
    todo('b', 2),
  ];
  let server: Todo[];
  beforeEach(() => {
    server = noted.map((t) => ({ ...t }));
    calls = stubApi({
      'GET /api/todos': () => server,
      'PATCH /api/todos/a': (c: Call) => (server[0] = { ...server[0], ...(c.body as object) }),
      'PATCH /api/todos/b': (c: Call) => (server[1] = { ...server[1], ...(c.body as object) }),
      'DELETE /api/todos/b': undefined,
    });
  });

  it('shows a notes icon and the first line of the notes in the list', async () => {
    show();
    const a = await screen.findByRole('listitem', { name: 'Todo a' });
    expect(within(a).getByText('Bring')).toBeInTheDocument();
    expect(within(a).getByLabelText('Has notes')).toBeInTheDocument();
    const b = screen.getByRole('listitem', { name: 'Todo b' });
    expect(within(b).queryByLabelText('Has notes')).toBeNull();
  });

  it('opens from its title, saves its notes as you type, and goes back to the list', async () => {
    show();
    await userEvent.click(await screen.findByRole('button', { name: 'Todo b' }));
    const view = await screen.findByRole('article', { name: 'Todo: Todo b' });
    expect(screen.queryByRole('textbox', { name: 'Add a todo' }), 'in place of the list').toBeNull();
    const notes = await within(view).findByRole('textbox', { name: 'Notes' });
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    setEditorText(notes, '# Agenda\n\n- *dates*');
    act(() => vi.advanceTimersByTime(AUTOSAVE_MS - 1));
    expect(sent('PATCH', '/api/todos/b')).toEqual([]);
    expect(screen.getByRole('status')).toHaveTextContent('Saving…');
    await act(async () => vi.advanceTimersByTime(1));
    vi.useRealTimers();
    await waitFor(() => expect(sent('PATCH', '/api/todos/b')).toEqual([{ body: '# Agenda\n\n- *dates*' }]));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('Saved'));

    await userEvent.click(screen.getByRole('button', { name: 'Todos' }));
    const b = await screen.findByRole('listitem', { name: 'Todo b' });
    expect(within(b).getByText('Agenda')).toBeInTheDocument();
    expect(within(b).getByLabelText('Has notes')).toBeInTheDocument();

    // Opened again, the notes are there.
    await userEvent.click(within(b).getByRole('button', { name: 'Todo b' }));
    expect(editorText(await screen.findByRole('textbox', { name: 'Notes' }))).toBe('# Agenda\n\n- *dates*');
  });

  it('saves what is unsaved on Back, renames, and keeps a title', async () => {
    show();
    await userEvent.click(await screen.findByRole('button', { name: 'Todo a' }));
    setEditorText(await screen.findByRole('textbox', { name: 'Notes' }), 'Changed');
    const title = screen.getByRole('textbox', { name: 'Title' });
    await userEvent.clear(title);
    await userEvent.tab();
    expect(title, 'an emptied title comes back').toHaveValue('Todo a');
    await userEvent.clear(title);
    await userEvent.type(title, 'Bring forms{Enter}');
    await userEvent.click(screen.getByRole('button', { name: 'Todos' }));
    await waitFor(() => {
      const patches = sent('PATCH', '/api/todos/a') as Partial<Todo>[];
      expect(patches.some((p) => p.body === 'Changed')).toBe(true);
      expect(patches.some((p) => p.title === 'Bring forms')).toBe(true);
      expect(patches.every((p) => p.title === undefined || p.title.trim() !== '')).toBe(true);
    });
  });

  it('checks off, links and deletes from the open view', async () => {
    show();
    await userEvent.click(await screen.findByRole('button', { name: 'Todo b' }));
    const view = await screen.findByRole('article', { name: 'Todo: Todo b' });
    await userEvent.click(within(view).getByRole('checkbox', { name: 'Done: Todo b' }));
    await waitFor(() => expect(sent('PATCH', '/api/todos/b')).toEqual([{ done: true }]));
    await userEvent.click(screen.getByRole('button', { name: /^Actions for Todo b/ }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/todos/b')).toBe(true));
    expect(await screen.findByRole('textbox', { name: 'Add a todo' }), 'back to the list').toBeInTheDocument();
  });

  it('tells the shell when a todo opens and closes (a phone goes full screen)', async () => {
    const onOpenChange = vi.fn();
    renderWithQuery(
      <OpenTicketContext.Provider value={vi.fn()}>
        <TodoList projectId="p1" projects={[project]} onOpenChange={onOpenChange} />
      </OpenTicketContext.Provider>,
    );
    await userEvent.click(await screen.findByRole('button', { name: 'Todo a' }));
    await waitFor(() => expect(onOpenChange).toHaveBeenLastCalledWith(true));
    await userEvent.click(screen.getByRole('button', { name: 'Todos' }));
    await waitFor(() => expect(onOpenChange).toHaveBeenLastCalledWith(false));
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
    todo('week', 7, { dueOn: '2026-10-27' }),
  ];
  const group = (name: string) => screen.getByRole('list', { name: `${name} todos` });
  const names = (name: string) => within(group(name)).getAllByRole('listitem').map((li) => li.getAttribute('aria-label'));
  const chip = (title: string) => within(screen.getByRole('listitem', { name: title })).queryByLabelText(/^Due (?!date)/);

  function stubDated() {
    calls = stubApi({
      'GET /api/todos': dated,
      'POST /api/todos': (c: Call) => todo('new', -1, c.body as Partial<Todo>),
      'PATCH /api/todos/late': (c: Call) => ({ ...dated[0], ...(c.body as object) }),
      'PATCH /api/todos/late2': (c: Call) => ({ ...dated[5], ...(c.body as object) }),
      'PATCH /api/todos/none': (c: Call) => ({ ...dated[1], ...(c.body as object) }),
      'PATCH /api/todos/today': (c: Call) => ({ ...dated[2], ...(c.body as object) }),
    });
  }

  it('groups as Overdue, Today, Tomorrow, Next 7 days, Later and No date, in their order', async () => {
    stubDated();
    show();
    await screen.findByText('Todo late');
    expect(screen.getAllByRole('heading', { level: 3 }).map((h) => h.textContent)).toEqual([
      'Overdue2Move to today', 'Today1', 'Tomorrow1', 'Next 7 days1', 'Later1', 'No date1',
    ]);
    expect(names('Overdue')).toEqual(['Todo late', 'Todo late2']);
    expect(names('Today')).toEqual(['Todo today']);
    expect(names('Tomorrow')).toEqual(['Todo tmrw']);
    expect(names('Next 7 days')).toEqual(['Todo week']);
    expect(names('Later')).toEqual(['Todo later']);
    expect(names('No date')).toEqual(['Todo none']);
  });

  it('says the day where its heading doesn\'t: overdue in red, a time today in amber; nothing on done rows', async () => {
    stubDated();
    show();
    await screen.findByText('Todo late');
    expect(chip('Todo late')).toHaveTextContent(/20/);
    expect(chip('Todo late')?.className).toMatch(/destructive/);
    expect(within(screen.getByRole('listitem', { name: 'Todo late' })).getByRole('checkbox').closest('label')!.querySelector('.border-destructive')).not.toBeNull();
    expect(chip('Todo today')).toHaveTextContent('17:00');
    expect(chip('Todo today')?.className).toMatch(/waiting/);
    expect(chip('Todo tmrw'), 'the heading says Tomorrow').toBeNull();
    expect(chip('Todo later')).toHaveTextContent(/3/);
    expect(chip('Todo none')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Completed · 1' }));
    expect(chip('Todo old')).toBeNull();
  });

  it('moves every overdue todo to today, keeping a time only if it is still ahead', async () => {
    calls = stubApi({
      'GET /api/todos': [todo('late', 1, { dueOn: '2026-10-20', dueTime: '09:00' }), todo('late2', 2, { dueOn: '2026-10-21', dueTime: '16:00' }), todo('none', 3)],
      'PATCH /api/todos/late': (c: Call) => todo('late', 1, c.body as Partial<Todo>),
      'PATCH /api/todos/late2': (c: Call) => todo('late2', 2, c.body as Partial<Todo>),
    });
    show();
    await screen.findByText('Todo late');
    await userEvent.click(screen.getByRole('button', { name: 'Move to today' }));
    await waitFor(() => {
      expect(sent('PATCH', '/api/todos/late')).toEqual([{ dueOn: '2026-10-23', dueTime: '' }]);
      expect(sent('PATCH', '/api/todos/late2')).toEqual([{ dueOn: '2026-10-23', dueTime: '16:00' }]);
    });
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

  describe('the due picker', () => {
    const picker = () => screen.getByRole('dialog', { name: /^Due date: / });
    const openPicker = async (title: string) => {
      await userEvent.click(within(screen.getByRole('listitem', { name: title })).getByRole('button', { name: `Due date for ${title}` }));
      return screen.findByRole('dialog', { name: `Due date: ${title}` });
    };

    it('sets a day from each quick pick, closing; No date clears it', async () => {
      stubDated();
      show();
      await screen.findByText('Todo none');
      const picks: [string, object][] = [
        ['In an hour', { dueOn: '2026-10-23', dueTime: '11:00' }],
        ['Today', { dueOn: '2026-10-23', dueTime: '' }],
        ['Tomorrow', { dueOn: '2026-10-24', dueTime: '' }],
        ['This weekend', { dueOn: '2026-10-24', dueTime: '' }],
        ['Next week', { dueOn: '2026-10-26', dueTime: '' }],
      ];
      for (const [name] of picks) {
        const dialog = await openPicker('Todo none');
        expect(within(dialog).queryByRole('button', { name: 'No date' }), 'nothing to clear').toBeNull();
        await userEvent.click(within(dialog).getByRole('button', { name: new RegExp(`^${name}(?!,)`) }));
        await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      }
      expect(sent('PATCH', '/api/todos/none')).toEqual(picks.map(([, p]) => p));

      const dialog = await openPicker('Todo late');
      await userEvent.click(within(dialog).getByRole('button', { name: 'No date' }));
      await waitFor(() => expect(sent('PATCH', '/api/todos/late')).toEqual([{ dueOn: '' }]));
    });

    it('opens from the row\'s date and the menu, and picks a calendar day keeping the time', async () => {
      stubDated();
      show();
      await screen.findByText('Todo late');
      await userEvent.click(chip('Todo late')!);
      expect(picker()).toBeInTheDocument();
      await userEvent.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

      await userEvent.click(screen.getByRole('button', { name: 'Actions for Todo late2' }));
      await userEvent.click(await screen.findByRole('menuitem', { name: 'Due date…' }));
      const dialog = await screen.findByRole('dialog', { name: 'Due date: Todo late2' });
      // Past days can't be picked.
      await userEvent.click(within(dialog).getByRole('button', { name: /October 21, 2026/ }));
      expect(sent('PATCH', '/api/todos/late2')).toEqual([]);
      await userEvent.click(within(dialog).getByRole('button', { name: /October 29, 2026/ }));
      await waitFor(() => expect(sent('PATCH', '/api/todos/late2')).toEqual([{ dueOn: '2026-10-29' }]));
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });

    it('takes a typed date', async () => {
      stubDated();
      show();
      await screen.findByText('Todo none');
      const dialog = await openPicker('Todo none');
      await userEvent.type(within(dialog).getByRole('textbox', { name: 'Type a date' }), 'fri 9am{Enter}');
      await waitFor(() => expect(sent('PATCH', '/api/todos/none')).toEqual([{ dueOn: '2026-10-30', dueTime: '09:00' }]));
    });

    it('sets a time from a slot or typed, and clears it, staying open', async () => {
      stubDated();
      show();
      await screen.findByText('Todo today');
      const dialog = await openPicker('Todo today');
      await userEvent.click(within(dialog).getByRole('button', { name: 'Time: 17:00' }));
      expect(within(picker()).getByRole('heading', { name: /^Time · / })).toBeInTheDocument();
      await userEvent.click(within(picker()).getByRole('button', { name: /^09:00/ }));
      fireEvent.change(within(picker()).getByLabelText('Time'), { target: { value: '07:45' } });
      fireEvent.blur(within(picker()).getByLabelText('Time'));
      await userEvent.click(within(picker()).getByRole('button', { name: 'No time' }));
      await waitFor(() => expect(sent('PATCH', '/api/todos/today')).toEqual([{ dueTime: '09:00' }, { dueTime: '07:45' }, { dueTime: '' }]));
      expect(picker()).toBeInTheDocument();
    });

    it('sets a repeat from the presets, dating an undated todo; Custom… opens the editor', async () => {
      stubDated();
      show();
      await screen.findByText('Todo none');
      let dialog = await openPicker('Todo none');
      await userEvent.click(within(dialog).getByRole('button', { name: 'Repeat' }));
      expect(within(picker()).getByRole('heading', { name: 'Repeat' })).toBeInTheDocument();
      // Built from today (a Friday, the 23rd).
      expect(within(picker()).getByRole('button', { name: /^Every week(?!day)/ })).toHaveTextContent('on Fri');
      await userEvent.click(within(picker()).getByRole('button', { name: /^Every month/ }));
      await waitFor(() => expect(sent('PATCH', '/api/todos/none')).toEqual([
        { recurrence: { rule: { every: 'month', day: 23 }, from: 'due' }, dueOn: '2026-10-23' },
      ]));
      await userEvent.click(within(picker()).getByRole('button', { name: /^Doesn’t repeat/ }));
      await waitFor(() => expect(sent('PATCH', '/api/todos/none')).toHaveLength(2));
      expect(sent('PATCH', '/api/todos/none')[1]).toEqual({ recurrence: null });
      await userEvent.click(within(picker()).getByRole('button', { name: 'Custom…' }));
      await waitFor(() => expect(screen.queryByRole('dialog', { name: /^Due date/ })).toBeNull());
      dialog = await screen.findByRole('dialog', { name: 'Repeat' });
      expect(within(dialog).getByRole('button', { name: 'Save' })).toBeInTheDocument();
    });
  });

  describe('quick add', () => {
    const box = () => screen.getByRole('textbox', { name: 'Add a todo' });

    it('picks dates and repeats out of the text as tags; × keeps the words; Enter sends the parsed fields', async () => {
      stubDated();
      show(null);
      await screen.findByText('Todo late');
      await userEvent.click(box());
      expect(screen.getByRole('button', { name: 'Date' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Repeat' })).toBeInTheDocument();
      expect(screen.getByRole('button', { name: 'Ticket' })).toBeInTheDocument();
      await userEvent.type(box(), 'Call the dentist tomorrow 3pm every month');
      expect(screen.getByText('Tomorrow 15:00')).toBeInTheDocument();
      expect(screen.getByText('Every month on the 24th')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Date' })).toBeNull();
      await userEvent.click(screen.getByRole('button', { name: 'Keep the repeat words in the title' }));
      expect(screen.queryByText('Every month on the 24th')).toBeNull();
      expect(box(), 'focus goes back to the text').toHaveFocus();
      await userEvent.keyboard('{Enter}');
      await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([
        { title: 'Call the dentist every month', dueOn: '2026-10-24', dueTime: '15:00' },
      ]));
      expect(box()).toHaveValue('');
    });

    it('keeps a dismissed match dismissed while typing on, until its words are gone', async () => {
      stubDated();
      show(null);
      await screen.findByText('Todo late');
      await userEvent.type(box(), 'Call Sam tomorrow');
      await userEvent.click(screen.getByRole('button', { name: 'Keep the date words in the title' }));
      await userEvent.type(box(), ' about lunch');
      expect(screen.queryByRole('button', { name: 'Keep the date words in the title' }), 'still dismissed after more typing').toBeNull();
      expect(screen.getByRole('button', { name: 'Date' })).toBeInTheDocument();
      await userEvent.keyboard('{Enter}');
      await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Call Sam tomorrow about lunch' }]));

      // Once the dismissed words are edited away, a new phrase matches again.
      await userEvent.type(box(), 'Pay tomorrow');
      await userEvent.click(screen.getByRole('button', { name: 'Keep the date words in the title' }));
      await userEvent.clear(box());
      await userEvent.type(box(), 'Pay fri');
      expect(screen.getByRole('button', { name: 'Keep the date words in the title' }), 'fri is a date tag').toBeInTheDocument();
    });

    it('keeps a day picked by hand over typed date words as typing goes on', async () => {
      stubDated();
      show(null);
      await screen.findByText('Todo late');
      await userEvent.type(box(), 'Call Sam tomorrow');
      // The date tag comes from the words; open the picker (from the Repeat tag) and pick Next week by hand.
      await userEvent.click(screen.getByRole('button', { name: 'Repeat' }));
      const picker = await screen.findByRole('dialog', { name: /^Due date: / });
      await userEvent.click(within(picker).getByRole('button', { name: 'Back' }));
      await userEvent.click(within(screen.getByRole('dialog', { name: /^Due date: / })).getByRole('button', { name: /^Next week(?!,)/ }));
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      await userEvent.type(box(), ' soon');
      await userEvent.keyboard('{Enter}');
      await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Call Sam tomorrow soon', dueOn: '2026-10-26' }]));
    });

    it('puts the cursor in the text from a click on the + circle', async () => {
      stubDated();
      show(null);
      await screen.findByText('Todo late');
      expect(box()).not.toHaveFocus();
      expect(screen.queryByRole('button', { name: 'Date' }), 'closed: no tags yet').toBeNull();
      // The line the input sits on: the + circle is its first child.
      await userEvent.click(box().parentElement!.parentElement!.querySelector('svg')!);
      expect(box()).toHaveFocus();
      expect(box()).toHaveAttribute('placeholder', 'What needs doing?');
      expect(screen.getByRole('button', { name: 'Date' }), 'opened into the quick-add box').toBeInTheDocument();
      await userEvent.keyboard('Pay rent');
      expect(box()).toHaveValue('Pay rent');
    });

    it('sets a date with the Date tag', async () => {
      stubDated();
      show(null);
      await screen.findByText('Todo late');
      await userEvent.click(box());
      await userEvent.click(screen.getByRole('button', { name: 'Date' }));
      await userEvent.click(within(await screen.findByRole('dialog', { name: 'Due date: New todo' })).getByRole('button', { name: /^Today(?!,)/ }));
      expect(await screen.findByRole('button', { name: 'No date' }), 'the Date tag, set').toBeInTheDocument();
      await userEvent.type(box(), 'Pay rent{Enter}');
      await waitFor(() => expect(sent('POST', '/api/todos')).toEqual([{ title: 'Pay rent', dueOn: '2026-10-23' }]));
    });
  });

  describe('completing', () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['Date', 'setTimeout', 'clearTimeout'], shouldAdvanceTime: true });
      vi.setSystemTime(new Date(2026, 9, 23, 10, 0));
    });

    /** The server's list, kept as patches change it. */
    function stubLive() {
      let live = [...dated];
      calls = stubApi({
        'GET /api/todos': () => live,
        'PATCH /api/todos/none': (c: Call) => {
          const body = c.body as Partial<Todo>;
          live = live.map((t) => (t.id === 'none' ? { ...t, ...body, doneAt: body.done ? 9 : null } : t));
          return live.find((t) => t.id === 'none');
        },
      });
    }

    it('keeps a ticked row in place, struck through, then moves it to Completed', async () => {
      stubLive();
      show();
      await screen.findByText('Todo none');
      fireEvent.click(screen.getByRole('checkbox', { name: 'Done: Todo none' }));
      await waitFor(() => expect(sent('PATCH', '/api/todos/none')).toEqual([{ done: true }]));
      const row = within(group('No date')).getByRole('listitem', { name: 'Todo none' });
      expect(row).toHaveAttribute('data-completing');
      expect(within(row).getByRole('button', { name: 'Todo none' }).className).toMatch(/line-through/);
      await act(async () => { vi.advanceTimersByTime(900); });
      await waitFor(() => expect(screen.queryByRole('list', { name: 'No date todos' })).toBeNull());
      expect(screen.getByRole('button', { name: 'Completed · 2' })).toBeInTheDocument();
    });

    it('stays put when unticked within the delay', async () => {
      stubLive();
      show();
      await screen.findByText('Todo none');
      fireEvent.click(screen.getByRole('checkbox', { name: 'Done: Todo none' }));
      await waitFor(() => expect(screen.getByRole('checkbox', { name: 'Not done: Todo none' })).toBeChecked());
      fireEvent.click(screen.getByRole('checkbox', { name: 'Not done: Todo none' }));
      await act(async () => { vi.advanceTimersByTime(900); });
      await waitFor(() => expect(sent('PATCH', '/api/todos/none')).toEqual([{ done: true }, { done: false }]));
      expect(within(group('No date')).getByRole('listitem', { name: 'Todo none' })).not.toHaveAttribute('data-completing');
    });
  });

  it('shows a reminder\'s todo once: reopening the list keeps the filter and rings nothing', async () => {
    stubDated();
    localStorage.setItem('pdt.todos.filter', 'project');
    const onFocused = vi.fn();
    let toggle = () => {};
    // As AppShell holds it: the focus is dropped once the list has used it; closing the panel unmounts the list.
    function Harness() {
      const [focus, setFocus] = useState<TodoFocus | null>({ id: 'late', seq: 1 });
      const [shown, setShown] = useState(true);
      toggle = () => setShown((v) => !v);
      return shown ? <TodoList projectId="p1" projects={[project]} focus={focus} onFocused={() => { onFocused(); setFocus(null); }} /> : null;
    }
    renderWithQuery(<OpenTicketContext.Provider value={vi.fn()}><Harness /></OpenTicketContext.Provider>);
    expect(await screen.findByRole('article', { name: 'Todo: Todo late' })).toBeInTheDocument();
    expect(onFocused).toHaveBeenCalledTimes(1);
    // Back: the list shows it (All: the filter hid it), with a ring.
    await userEvent.click(screen.getByRole('button', { name: 'Todos' }));
    expect((await screen.findByRole('listitem', { name: 'Todo late' })).className).toMatch(/ring-2/);
    expect(screen.getByRole('button', { name: 'Scope: All projects' })).toBeInTheDocument();

    act(() => toggle());
    act(() => toggle());
    expect(await screen.findByRole('button', { name: 'Scope: PX' }), 'the chosen scope stays').toBeInTheDocument();
    expect(screen.queryByRole('listitem', { name: 'Todo late' }), 'not in PX, and not pulled back in').toBeNull();
    expect(onFocused).toHaveBeenCalledTimes(1);
  });

  it('opens the todo a reminder is for, even when the filter hid it', async () => {
    stubDated();
    localStorage.setItem('pdt.todos.filter', 'project');
    show('p1', vi.fn(), { id: 'late', seq: 1 });
    const view = await screen.findByRole('article', { name: 'Todo: Todo late' });
    expect(within(view).getByRole('textbox', { name: 'Title' })).toHaveValue('Todo late');
    expect(screen.queryByRole('list', { name: /todos$/i })).toBeNull();
  });

  describe('repeating (PDT-141)', () => {
    const weekly = { rule: { every: 'week' as const, interval: 1, days: [1, 3] }, from: 'due' as const };
    const repeating = [
      todo('rep', 1, { dueOn: '2026-10-26', recurrence: weekly }),
      todo('plain', 2, { dueOn: '2026-10-26' }),
      todo('undated', 3),
      todo('was', 0, { done: true, doneAt: 5, dueOn: '2026-10-19', recurrence: weekly }),
    ];
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['Date'] });
      vi.setSystemTime(new Date(2026, 9, 23, 10, 0));
      calls = stubApi({
        'GET /api/todos': repeating,
        'PATCH /api/todos/rep': (c: Call) => ({ ...repeating[0], ...(c.body as object) }),
        'PATCH /api/todos/plain': (c: Call) => ({ ...repeating[1], ...(c.body as object) }),
        'PATCH /api/todos/undated': (c: Call) => ({ ...repeating[2], ...(c.body as object) }),
        'DELETE /api/todos/rep': undefined,
        'DELETE /api/todos/plain': undefined,
      });
    });
    afterEach(() => vi.useRealTimers());
    const card = (name: string) => screen.getByRole('listitem', { name });
    const menu = async (name: string, item: string) => {
      await userEvent.click(within(card(name)).getByRole('button', { name: `Actions for ${name}` }));
      await userEvent.click(await screen.findByRole('menuitem', { name: item }));
    };

    it('shows the repeat icon and rule on open cards only', async () => {
      show(null);
      await screen.findByText('Todo rep');
      expect(within(card('Todo rep')).getByLabelText('Repeats: Every Mon, Wed')).toHaveTextContent('Mon, Wed');
      expect(within(card('Todo plain')).queryByLabelText(/^Repeats/)).toBeNull();
      await userEvent.click(screen.getByRole('button', { name: 'Completed · 1' }));
      expect(within(card('Todo was')).queryByLabelText(/^Repeats/), 'not on a done card').toBeNull();
    });

    it('sets a rule with the Repeat editor, and stops it', async () => {
      show(null);
      await screen.findByText('Todo plain');
      await menu('Todo plain', 'Repeat…');
      let dialog = await screen.findByRole('dialog');
      // Starts weekly on the due day (2026-10-26 is a Monday).
      expect(within(dialog).getByRole('checkbox', { name: 'Mon' })).toBeChecked();
      await userEvent.click(within(dialog).getByRole('checkbox', { name: 'Fri' }));
      fireEvent.change(within(dialog).getByLabelText('Every … weeks'), { target: { value: '2' } });
      await userEvent.click(within(dialog).getByRole('radio', { name: 'When it’s done' }));
      await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
      expect(sent('PATCH', '/api/todos/plain')).toEqual([{ recurrence: { rule: { every: 'week', interval: 2, days: [1, 5] }, from: 'done' } }]);

      // Monthly; a todo without a date gets one with its rule.
      await menu('Todo undated', 'Repeat…');
      dialog = await screen.findByRole('dialog');
      await userEvent.click(within(dialog).getByRole('button', { name: /Weekly/ }));
      await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Monthly' }));
      fireEvent.change(within(dialog).getByLabelText(/^On day/), { target: { value: '31' } });
      fireEvent.change(within(dialog).getByLabelText(/^First due/), { target: { value: '2026-10-31' } });
      await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
      expect(sent('PATCH', '/api/todos/undated')).toEqual([{ recurrence: { rule: { every: 'month', day: 31 }, from: 'due' }, dueOn: '2026-10-31' }]);

      // A weekly rule needs a day.
      await menu('Todo rep', 'Repeat…');
      dialog = await screen.findByRole('dialog');
      await userEvent.click(within(dialog).getByRole('checkbox', { name: 'Mon' }));
      await userEvent.click(within(dialog).getByRole('checkbox', { name: 'Wed' }));
      expect(within(dialog).getByRole('button', { name: 'Save' })).toBeDisabled();
      await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));

      await menu('Todo rep', 'Stop repeating');
      expect(sent('PATCH', '/api/todos/rep')).toEqual([{ recurrence: null }]);
    });

    it('asks how to delete a repeating todo: only this one, or the series', async () => {
      show(null);
      await screen.findByText('Todo rep');
      await menu('Todo plain', 'Delete');
      expect(screen.queryByRole('dialog'), 'a plain todo just goes').toBeNull();
      expect(calls.filter((c) => c.method === 'DELETE').map((c) => c.path)).toEqual(['/api/todos/plain']);

      await menu('Todo rep', 'Delete');
      let dialog = await screen.findByRole('dialog');
      await userEvent.click(within(dialog).getByRole('button', { name: 'Only this one' }));
      await menu('Todo rep', 'Delete');
      dialog = await screen.findByRole('dialog');
      await userEvent.click(within(dialog).getByRole('button', { name: 'Stop the series' }));
      await waitFor(() => expect(calls.filter((c) => c.method === 'DELETE').map((c) => c.path)).toEqual([
        '/api/todos/plain', '/api/todos/rep?series=only', '/api/todos/rep?series=series',
      ]));
    });
  });
});
