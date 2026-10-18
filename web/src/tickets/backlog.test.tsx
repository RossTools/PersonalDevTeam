import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { OpenTicketContext } from './actions';
import { event, page, params, project, summary, ticket } from './fixtures';
import { TicketsList } from './TicketsList';
import { TicketsView } from './TicketsView';
import { useBacklog } from './useBacklog';

// jsdom has no layout: give elements a viewport so the virtualised list renders its first screen of rows.
const sizes = [['clientWidth', 900], ['clientHeight', 600], ['offsetWidth', 900], ['offsetHeight', 600]] as const;
const restore: (() => void)[] = [];
let observed: ((entries: { isIntersecting: boolean }[]) => void)[] = [];
beforeAll(() => {
  for (const [prop, value] of sizes) {
    const spy = vi.spyOn(HTMLElement.prototype, prop, 'get').mockImplementation(() => value);
    restore.push(() => spy.mockRestore());
  }
});
afterAll(() => restore.forEach((r) => r()));
beforeEach(() => {
  localStorage.clear();
  observed = [];
  vi.stubGlobal('IntersectionObserver', class {
    constructor(cb: (entries: { isIntersecting: boolean }[]) => void) { observed.push(cb); }
    observe() {}
    unobserve() {}
    disconnect() {}
  });
});
afterEach(() => vi.unstubAllGlobals());

const rows = [summary(1, { stage: 'ready', priority: 'high', labels: ['ui'] }), summary(2), summary(3, { blockedBy: 1 })];

function Harness({ onNewTicket = () => {}, onOpen = () => {} }: { onNewTicket?: (p: string | null, title?: string) => void; onOpen?: (t: { id: string }) => void }) {
  const backlog = useBacklog();
  return (
    <OpenTicketContext.Provider value={onOpen}>
      <TicketsList backlog={backlog} projects={[project]} />
      <TicketsView backlog={backlog} projects={[project]} onNewTicket={onNewTicket} />
    </OpenTicketContext.Provider>
  );
}

function stub(extra: Parameters<typeof stubApi>[0] = {}, list: (c: Call) => unknown = () => page(rows, null, 3)): Call[] {
  return stubApi({
    'GET /api/tickets': (c: Call) => (params(c.path).limit === '1' ? page([], null, 7) : list(c)),
    [`GET /api/projects/${project.id}/labels`]: ['backend', 'ui'],
    'GET /api/tickets/t1': ticket(1, { stage: 'ready', priority: 'high', labels: ['ui'], areas: ['web/ui'], description: '**Bold** text' }),
    [`GET /api/projects/${project.id}/areas`]: { areas: [{ name: 'web/ui', paths: ['web/ui/**'], exclusive: false }], error: null },
    'GET /api/tickets/t1/events': { items: [event('e1')], nextCursor: null },
    ...extra,
  });
}

const listCalls = (calls: Call[]) => calls.filter((c) => c.method === 'GET' && c.path.startsWith('/api/tickets?') && params(c.path).limit !== '1');
const lastList = (calls: Call[]) => params(listCalls(calls).at(-1)!.path);
const pickProject = async () => {
  await userEvent.click(await screen.findByRole('row', { name: /Alpha/ }));
};

describe('backlog', () => {
  it('lists open tickets of every project by default, with key, stage, priority, labels and blocked', async () => {
    const calls = stub();
    renderWithQuery(<Harness />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    const first = await within(grid).findByRole('row', { name: /ALP-1 Ticket 1/ });
    expect(first).toHaveTextContent('Ready');
    expect(first).toHaveTextContent('High');
    expect(first).toHaveTextContent('ui');
    expect(within(grid).getByRole('row', { name: /ALP-3/ })).toHaveTextContent('Blocked');
    expect(lastList(calls)).toEqual({ limit: '100', stage: 'draft,ready' });
    expect(screen.getByText(/All projects · 3 tickets/)).toBeInTheDocument();
    // The list column: projects with their open-ticket count.
    expect(await screen.findByLabelText('7 open')).toBeInTheDocument();
  });

  it('scopes to a project, and filters by stage, Show closed and search', async () => {
    const calls = stub();
    renderWithQuery(<Harness />);
    await pickProject();
    await waitFor(() => expect(lastList(calls).project).toBe(project.id));
    await userEvent.click(screen.getByRole('switch', { name: 'Show closed' }));
    await waitFor(() => expect(lastList(calls).stage).toBeUndefined());
    await userEvent.click(screen.getByRole('button', { name: 'Stage filter' }));
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Done' }));
    await waitFor(() => expect(lastList(calls).stage).toBe('done'));
    await userEvent.keyboard('{Escape}');
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search tickets' }), 'login');
    await waitFor(() => expect(lastList(calls).q).toBe('login'));
  });

  it('loads the next page near the end of the list', async () => {
    const calls = stub({}, (c) => (params(c.path).cursor === 'c2' ? page([summary(4)], null, 4) : page(rows, 'c2', 4)));
    renderWithQuery(<Harness />);
    await screen.findByRole('row', { name: /ALP-1 / });
    act(() => observed.forEach((cb) => cb([{ isIntersecting: true }])));
    expect(await screen.findByRole('row', { name: /ALP-4 / })).toBeInTheDocument();
    expect(listCalls(calls).some((c) => params(c.path).cursor === 'c2')).toBe(true);
  });

  it('shows the empty states: no tickets, and nothing matching the filters', async () => {
    stub({}, () => page([]));
    renderWithQuery(<Harness />);
    expect(await screen.findByText('No Draft or Ready tickets.')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search tickets' }), 'zzz');
    expect(await screen.findByText('No tickets match these filters.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Clear filters' }));
    expect(await screen.findByText('No Draft or Ready tickets.')).toBeInTheDocument();
  });

  it('switches to the board', async () => {
    stub();
    renderWithQuery(<Harness />);
    await userEvent.click(screen.getByRole('radio', { name: 'Board' }));
    for (const name of ['To do', 'In progress', 'Done']) expect(await screen.findByRole('region', { name })).toBeInTheDocument();
    expect(screen.queryByRole('grid', { name: 'Tickets' })).not.toBeInTheDocument();
  });

  it('quick-adds a Draft above the first row with Enter; Shift+Enter opens the full form with the title', async () => {
    const onNewTicket = vi.fn();
    const calls = stub({ [`POST /api/projects/${project.id}/tickets`]: ticket(9) });
    renderWithQuery(<Harness onNewTicket={onNewTicket} />);
    // Quick add needs a project.
    expect(screen.queryByRole('textbox', { name: 'New ticket title' })).not.toBeInTheDocument();
    await pickProject();
    const input = await screen.findByRole('textbox', { name: 'New ticket title' });
    await userEvent.type(input, 'Write docs{Enter}');
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ title: 'Write docs', before: 't1' }));
    await waitFor(() => expect(input).toHaveValue(''));
    await userEvent.type(input, 'Bigger one{Shift>}{Enter}{/Shift}');
    expect(onNewTicket).toHaveBeenCalledWith(project.id, 'Bigger one');
  });

  it('acts on selected rows from the bulk bar: Move to Ready, priority, label, delete', async () => {
    const calls = stub({ 'POST /api/tickets/bulk': { count: 2 } });
    renderWithQuery(<Harness />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    await userEvent.click(within(within(grid).getByRole('row', { name: /ALP-1 / })).getByRole('checkbox'));
    await userEvent.click(within(within(grid).getByRole('row', { name: /ALP-2 / })).getByRole('checkbox'));
    const bar = screen.getByRole('toolbar', { name: 'Bulk actions' });
    expect(bar).toHaveTextContent('2 tickets selected');
    expect(screen.queryByRole('search')).not.toBeInTheDocument();
    const bulk = () => calls.filter((c) => c.path === '/api/tickets/bulk').map((c) => c.body);

    await userEvent.click(within(bar).getByRole('button', { name: 'Move to Ready' }));
    await waitFor(() => expect(bulk().at(-1)).toEqual({ ids: ['t1', 't2'], action: 'stage', value: 'ready' }));

    await userEvent.click(within(bar).getByRole('button', { name: /Priority/ }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Urgent' }));
    await waitFor(() => expect(bulk().at(-1)).toEqual({ ids: ['t1', 't2'], action: 'priority', value: 'urgent' }));

    await userEvent.click(within(bar).getByRole('button', { name: /Label/ }));
    // Every project: the labels on the loaded rows (one project's view also offers the project's labels).
    await userEvent.click((await screen.findAllByRole('menuitem', { name: 'ui' }))[0]);
    await waitFor(() => expect(bulk().at(-1)).toEqual({ ids: ['t1', 't2'], action: 'addLabel', value: 'ui' }));

    await userEvent.click(within(bar).getByRole('button', { name: 'Move to top' }));
    await waitFor(() => expect(bulk().at(-1)).toEqual({ ids: ['t1', 't2'], action: 'top' }));

    await userEvent.click(within(bar).getByRole('button', { name: 'Delete…' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete 2 tickets?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Delete tickets' }));
    await waitFor(() => expect(bulk().at(-1)).toEqual({ ids: ['t1', 't2'], action: 'delete' }));
    await waitFor(() => expect(screen.queryByRole('toolbar', { name: 'Bulk actions' })).not.toBeInTheDocument());
  });

  it('ranks with Alt+Up/Down in one project sorted by rank, and not otherwise', async () => {
    const calls = stub({ 'POST /api/tickets/t2/rank': ticket(2) });
    renderWithQuery(<Harness />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    // Every project: ranks are per project, so no handle.
    expect(within(grid).getByRole('row', { name: /ALP-1 / }).querySelector('[data-drag-handle]')).toBeNull();
    await pickProject();
    // A new list (React Aria can't add drag and drop in place).
    const ranked = () => screen.getByRole('grid', { name: 'Tickets' });
    await waitFor(() => expect(within(ranked()).getByRole('row', { name: /ALP-1 / }).querySelector('[data-drag-handle]')).not.toBeNull());
    act(() => within(ranked()).getByRole('row', { name: /ALP-2 / }).focus());
    await userEvent.keyboard('{Alt>}{ArrowUp}{/Alt}');
    await waitFor(() => expect(calls.find((c) => c.path === '/api/tickets/t2/rank')?.body).toEqual({ before: 't1' }));
  });

  it('opens a row in the peek sheet with its details and areas, and Open opens its tab', async () => {
    const onOpen = vi.fn();
    stub();
    renderWithQuery(<Harness onOpen={onOpen} />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    await userEvent.click(within(grid).getByRole('row', { name: /ALP-1 / }));
    const sheet = await screen.findByRole('dialog', { name: 'ALP-1 · Ticket 1' });
    expect(within(sheet).getByText('Bold')).toBeInTheDocument();
    expect(within(sheet).getByRole('button', { name: /Stage/ })).toHaveTextContent('Ready');
    expect(within(within(sheet).getByRole('group', { name: 'Areas' })).getByText('web/ui')).toBeInTheDocument();
    expect(await within(sheet).findByText('Created')).toBeInTheDocument();
    await userEvent.click(within(sheet).getByRole('button', { name: 'Open' }));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 't1', projectId: project.id }));
  });

  it('opens a row menu on right-click with Open, Move to, Priority and Delete', async () => {
    const onOpen = vi.fn();
    stub();
    renderWithQuery(<Harness onOpen={onOpen} />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    const row = within(grid).getByRole('row', { name: /ALP-2 / });
    await userEvent.pointer({ keys: '[MouseRight]', target: row });
    const menu = await screen.findByRole('menu', { name: 'Ticket ALP-2' });
    for (const name of ['Open', 'Move to', 'Priority', 'Delete…']) expect(within(menu).getByRole('menuitem', { name })).toBeInTheDocument();
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Open' }));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 't2' }));
  });

  it('saves the filter on screen by name and applies it again', async () => {
    const calls = stub();
    renderWithQuery(<Harness />);
    await pickProject();
    await userEvent.click(screen.getByRole('switch', { name: 'Show closed' }));
    await userEvent.click(screen.getAllByRole('button', { name: 'Save filter…' })[0]);
    const dialog = await screen.findByRole('dialog', { name: 'Save filter' });
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'Everything{Enter}');
    const saved = await screen.findByRole('grid', { name: 'Saved filters' });
    expect(JSON.parse(localStorage.getItem('pdt.tickets.savedFilters')!)[0]).toMatchObject({ name: 'Everything', projectId: project.id });
    await userEvent.click(screen.getByRole('row', { name: /All projects/ }));
    await userEvent.click(screen.getByRole('switch', { name: 'Show closed' }));
    await waitFor(() => expect(lastList(calls).project).toBeUndefined());
    await userEvent.click(within(saved).getByRole('row', { name: /Everything/ }));
    await waitFor(() => expect(lastList(calls)).toEqual({ limit: '100', project: project.id }));
  });
});
