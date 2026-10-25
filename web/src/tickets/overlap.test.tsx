// Phase 4c in the web UI: areas, overlap badges and the Overlaps section, waiting and clear to start, Start's Wait,
// and the overlap filters.
import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ProjectAreas } from '../api/gen/ProjectAreas';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketOverlap } from '../api/gen/TicketOverlap';
import { toast } from '@ui';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { OpenTicketContext, StartTicketContext } from './actions';
import { resetClearToasts } from './clearToStart';
import { event, page, params, project, summary, ticket } from './fixtures';
import { DEFAULT_FILTERS, eventTitle, isDefaultFilter, READY_TO_START, toQuery } from './model';
import { NewTicketDialog } from './NewTicketDialog';
import { StartDialog } from './StartDialog';
import { TicketsList } from './TicketsList';
import { TicketsView } from './TicketsView';
import { TicketView } from './TicketView';
import { useBacklog } from './useBacklog';

// jsdom has no layout: give elements a viewport so the virtualised list renders its rows.
const sizes = [['clientWidth', 900], ['clientHeight', 600], ['offsetWidth', 900], ['offsetHeight', 600]] as const;
const restore: (() => void)[] = [];
beforeAll(() => {
  // jsdom lacks pointer capture; Sonner calls it when a toast is pressed.
  Element.prototype.setPointerCapture ??= () => {};
  Element.prototype.releasePointerCapture ??= () => {};
  for (const [prop, value] of sizes) {
    const spy = vi.spyOn(HTMLElement.prototype, prop, 'get').mockImplementation(() => value);
    restore.push(() => spy.mockRestore());
  }
});
afterAll(() => restore.forEach((r) => r()));
beforeEach(() => {
  localStorage.clear();
  resetClearToasts();
  vi.stubGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} });
});
afterEach(() => {
  act(() => { toast.dismiss(); });
  vi.unstubAllGlobals();
});

const areas: ProjectAreas = {
  areas: [
    { name: 'migrations', paths: ['crates/pdt-store/migrations/**'], exclusive: true },
    { name: 'web/ui', paths: ['web/ui/**'], exclusive: false },
  ],
  error: null,
};
const areasRoute = { [`GET /api/projects/${project.id}/areas`]: areas };

const conflictWith: TicketOverlap = {
  ticketId: 't2', key: 'ALP-2', title: 'Schema change', stage: 'in_progress', level: 'conflict', areas: ['migrations'],
  files: ['a.sql', 'b.sql', 'c.sql', 'd.sql', 'e.sql', 'f.sql'], filesTotal: 9,
};
const overlapWith: TicketOverlap = {
  ticketId: 't3', key: 'ALP-3', title: 'Buttons', stage: 'in_progress', level: 'overlap', areas: ['web/ui'], files: [], filesTotal: 0,
};

const patches = (calls: Call[]) => calls.filter((c) => c.method === 'PATCH').map((c) => c.body);
const listCalls = (calls: Call[]) => calls.filter((c) => c.method === 'GET' && c.path.startsWith('/api/tickets?') && params(c.path).limit !== '1');

// ---------------------------------------------------------------------------------------------------------------

describe('model', () => {
  it('maps the blocked and overlap filters to the query, and counts them as filters', () => {
    expect(toQuery(null, { ...DEFAULT_FILTERS, blocked: false, conflicts: true })).toMatchObject({ blocked: false, conflicts: true });
    expect(toQuery(null, DEFAULT_FILTERS)).not.toHaveProperty('conflicts');
    expect(isDefaultFilter({ ...DEFAULT_FILTERS, conflicts: false })).toBe(false);
  });

  it('ships Ready to start: Ready, unblocked, no overlap', () => {
    expect(toQuery('p1', READY_TO_START.filters)).toEqual({ limit: 100, project: 'p1', stage: 'ready', blocked: false, conflicts: false });
  });

  it('describes waiting and overlap events', () => {
    expect(eventTitle(event('e', { kind: 'waiting', data: { waiting: true } }))).toBe('Waiting until clear');
    expect(eventTitle(event('e', { kind: 'waiting', data: { waiting: false } }))).toBe('Stopped waiting');
    expect(eventTitle(event('e', { kind: 'overlap', data: { ticketId: 't2', key: 'ALP-2', files: ['a', 'b'], level: 'conflict' } })))
      .toBe('Conflicts with ALP-2: 2 shared files');
    expect(eventTitle(event('e', { kind: 'overlap', data: { key: 'ALP-3', files: ['a'], level: 'overlap' } }))).toBe('Overlaps ALP-3: 1 shared file');
    expect(eventTitle(event('e', { kind: 'overlap', data: { key: 'ALP-3', files: [], level: 'none', cleared: true } })))
      .toBe('No longer overlaps ALP-3');
  });
});

// ---------------------------------------------------------------------------------------------------------------

function Backlog({ onStart = vi.fn() }: { onStart?: (t: { id: string }) => void }) {
  const backlog = useBacklog();
  return (
    <StartTicketContext.Provider value={onStart}>
      <TicketsList backlog={backlog} projects={[project]} />
      <TicketsView backlog={backlog} projects={[project]} onNewTicket={() => {}} />
    </StartTicketContext.Provider>
  );
}

const rows = [
  summary(1, { overlap: 'conflict', overlapWith: ['ALP-2', 'ALP-3'] }),
  summary(4, { stage: 'ready', waiting: true }),
  summary(5, { stage: 'ready', waiting: true, clearToStart: true }),
];

function stubBacklog(list: () => unknown = () => page(rows)) {
  return stubApi({
    'GET /api/tickets': (c: Call) => (params(c.path).limit === '1' ? page([], null, 0) : list()),
    [`GET /api/projects/${project.id}/labels`]: [],
    ...areasRoute,
    'PATCH /api/tickets/t4': (c: Call) => ticket(4, c.body as object),
  });
}

describe('backlog', () => {
  it('shows overlap, waiting and clear-to-start badges on rows', async () => {
    stubBacklog();
    renderWithQuery(<Backlog />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    const conflict = await within(grid).findByRole('row', { name: /ALP-1 / });
    expect(within(conflict).getByTitle('Conflicts with ALP-2, ALP-3')).toHaveTextContent('Conflicts with ALP-2');
    expect(within(grid).getByRole('row', { name: /ALP-4 / })).toHaveTextContent('Waiting');
    expect(within(grid).getByRole('row', { name: /ALP-5 / })).toHaveTextContent('Clear to start');
  });

  it('toasts a clear ticket once per browser session, with Start', async () => {
    const onStart = vi.fn();
    stubBacklog();
    const { unmount } = renderWithQuery(<Backlog onStart={onStart} />);
    expect(await screen.findByText('ALP-5 is clear to start')).toBeInTheDocument();
    expect(screen.queryByText('ALP-4 is clear to start')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't5' }));
    expect(JSON.parse(sessionStorage.getItem('pdt.tickets.clearToastShown')!)).toEqual(['t5']);
    unmount();
    // A fresh mount (another view, a reload of the page's state) in the same session does not toast again.
    const info = vi.spyOn(toast, 'info');
    renderWithQuery(<Backlog />);
    await screen.findByRole('row', { name: /ALP-5 / });
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(info).not.toHaveBeenCalled();
    info.mockRestore();
  });

  it('stops waiting from the row menu', async () => {
    const calls = stubBacklog();
    renderWithQuery(<Backlog />);
    const grid = await screen.findByRole('grid', { name: 'Tickets' });
    await userEvent.pointer({ keys: '[MouseRight]', target: await within(grid).findByRole('row', { name: /ALP-4 / }) });
    const menu = await screen.findByRole('menu', { name: 'Ticket ALP-4' });
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Stop waiting' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ waiting: false }]));
    expect(await screen.findByText('ALP-4 stopped waiting')).toBeInTheDocument();
  });

  it('filters by overlap', async () => {
    const calls = stubBacklog();
    renderWithQuery(<Backlog />);
    await screen.findByRole('grid', { name: 'Tickets' });
    await userEvent.click(screen.getByRole('button', { name: 'Overlap filter' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'No overlap' }));
    await waitFor(() => expect(params(listCalls(calls).at(-1)!.path).conflicts).toBe('false'));
    expect(screen.getByRole('button', { name: 'Overlap filter' })).toHaveTextContent('Overlap: No overlap');
    await userEvent.click(screen.getByRole('button', { name: 'Overlap filter' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Any' }));
    await waitFor(() => expect(params(listCalls(calls).at(-1)!.path)).not.toHaveProperty('conflicts'));
  });

  it('lists the built-in Ready to start filter, applies it to the project on screen and cannot delete it', async () => {
    const calls = stubBacklog();
    renderWithQuery(<Backlog />);
    await userEvent.click(await screen.findByRole('row', { name: /Alpha/ }));
    const saved = screen.getByRole('grid', { name: 'Saved filters' });
    const row = within(saved).getByRole('row', { name: /Ready to start/ });
    await userEvent.click(row);
    await waitFor(() => expect(params(listCalls(calls).at(-1)!.path)).toEqual({
      limit: '100', project: project.id, stage: 'ready', blocked: 'false', conflicts: 'false',
    }));
    await userEvent.pointer({ keys: '[MouseRight]', target: row });
    const menu = await screen.findByRole('menu', { name: 'Saved filter Ready to start' });
    expect(within(menu).queryByRole('menuitem', { name: 'Delete' })).not.toBeInTheDocument();
    // Never stored with the browser's own filters.
    expect(localStorage.getItem('pdt.tickets.savedFilters')).toBeNull();
  });
});

// ---------------------------------------------------------------------------------------------------------------

describe('board', () => {
  it('shows the overlap badge on cards and stops waiting from the card menu', async () => {
    localStorage.setItem('pdt.tickets.view', JSON.stringify({ projectId: null, mode: 'board', filters: {} }));
    const todo = [summary(4, { stage: 'todo', waiting: true, overlap: 'overlap', overlapWith: ['ALP-3'] })];
    const calls = stubApi({
      'GET /api/tickets': (c: Call) => page(params(c.path).stage === 'todo' ? todo : []),
      'GET /api/projects': [project],
      [`GET /api/projects/${project.id}/sessions`]: [],
      'PATCH /api/tickets/t4': (c: Call) => ticket(4, c.body as object),
    });
    renderWithQuery(<Backlog />);
    const col = screen.getByRole('region', { name: 'To do' });
    expect(await within(col).findByTitle('Overlaps ALP-3')).toBeInTheDocument();
    expect(within(col).getByText('Waiting')).toBeInTheDocument();
    await userEvent.click(within(col).getByRole('button', { name: 'Actions for ALP-4' }));
    const menu = await screen.findByRole('menu', { name: 'Actions for ALP-4' });
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Stop waiting' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ waiting: false }]));
  });
});

// ---------------------------------------------------------------------------------------------------------------

describe('ticket tab', () => {
  const base = ticket(1, {
    stage: 'ready', areas: ['migrations', 'old-area'], waiting: true,
    overlap: 'conflict', overlapWith: ['ALP-2', 'ALP-3'], overlaps: [conflictWith, overlapWith],
  });

  function stubTicket(t: Ticket = base, areasReply: ProjectAreas = areas) {
    return stubApi({
      'GET /api/tickets/t1': t,
      'PATCH /api/tickets/t1': (c: Call) => ({ ...t, ...(c.body as object) }),
      'GET /api/tickets/t1/events': { items: [], nextCursor: null },
      [`GET /api/projects/${project.id}/labels`]: [],
      [`GET /api/projects/${project.id}/areas`]: areasReply,
    });
  }

  const view = (onOpen = vi.fn()) =>
    renderWithQuery(
      <OpenTicketContext.Provider value={onOpen}>
        <TicketView ticketId="t1" />
      </OpenTicketContext.Provider>,
    );

  it('lists overlaps with the other ticket, its stage, shared areas and files', async () => {
    const onOpen = vi.fn();
    stubTicket();
    view(onOpen);
    const section = await screen.findByRole('region', { name: 'Overlaps' });
    const items = within(section).getAllByRole('listitem').filter((li) => li.getAttribute('aria-label'));
    expect(items.map((li) => li.getAttribute('aria-label'))).toEqual(['ALP-2 Schema change', 'ALP-3 Buttons']);
    const first = items[0];
    expect(first).toHaveTextContent('Conflict');
    expect(first).toHaveTextContent('In progress');
    expect(within(first).getByText('migrations')).toBeInTheDocument();
    const files = within(first).getByRole('list', { name: 'Shared files' });
    expect(within(files).getByText('a.sql')).toBeInTheDocument();
    expect(within(files).queryByText('f.sql')).not.toBeInTheDocument();
    expect(files).toHaveTextContent('and 4 more');
    expect(items[1]).toHaveTextContent('Overlap');
    await userEvent.click(within(first).getByRole('button', { name: 'ALP-2' }));
    expect(onOpen).toHaveBeenCalledWith({ id: 't2', projectId: project.id });
    // The header: the conflict badge and Waiting.
    expect(screen.getAllByTitle('Conflicts with ALP-2, ALP-3').length).toBeGreaterThan(0);
    expect(screen.getAllByText('Waiting').length).toBeGreaterThan(0);
  });

  it('has no Overlaps section without overlaps', async () => {
    stubTicket(ticket(1));
    view();
    await screen.findByRole('group', { name: 'Areas' });
    expect(screen.queryByRole('region', { name: 'Overlaps' })).not.toBeInTheDocument();
  });

  it('strikes through areas the config does not declare, marks exclusive ones and offers the configured ones', async () => {
    const calls = stubTicket();
    view();
    const section = await screen.findByRole('group', { name: 'Areas' });
    await waitFor(() => expect(within(section).getByText('old-area').closest('[title]')).toHaveAttribute('title', 'Not in .pdt/project.toml — ignored for overlap'));
    expect(within(section).getByText('old-area').closest('span[title]')!.className).toContain('line-through');
    expect(within(section).getByLabelText('Exclusive')).toBeInTheDocument();
    await userEvent.click(within(section).getByRole('combobox', { name: 'Add area' }));
    await userEvent.click(await screen.findByRole('option', { name: 'web/ui' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ areas: ['migrations', 'old-area', 'web/ui'] }]));
  });

  it('shows a configuration error quietly', async () => {
    stubTicket(ticket(1, { areas: ['web/ui'] }), { areas: [], error: 'area "x": no paths' });
    view();
    const section = await screen.findByRole('group', { name: 'Areas' });
    expect(await within(section).findByText(/Areas config: area "x": no paths/)).toBeInTheDocument();
    // Without a valid config nothing is struck through.
    expect(within(section).getByText('web/ui').closest('span[title], span')!.className).not.toContain('line-through');
  });

  it('stops waiting from the header menu', async () => {
    const calls = stubTicket();
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Stop waiting' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ waiting: false }]));
  });
});

// ---------------------------------------------------------------------------------------------------------------

describe('Start dialog', () => {
  const ready = ticket(1, { title: 'Fix login', stage: 'ready' });

  function stubStart(t: Ticket) {
    return stubApi({
      'GET /api/tickets/t1': t,
      'GET /api/projects': [project],
      ...areasRoute,
      'PATCH /api/tickets/t1': (c: Call) => ({ ...t, ...(c.body as object) }),
      'POST /api/tickets/t1/start': () => ({ ticket: t, attempt: { id: 'a1' }, session: { id: 's1', projectId: project.id } }),
    });
  }
  const open = () => {
    const onClose = vi.fn();
    const onStarted = vi.fn();
    renderWithQuery(<StartDialog ticketId="t1" onClose={onClose} onStarted={onStarted} />);
    return { onClose, onStarted };
  };

  it('names each overlap, and Wait marks the ticket waiting', async () => {
    const calls = stubStart({ ...ready, overlaps: [conflictWith, overlapWith] });
    const { onClose, onStarted } = open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    const conflict = within(dialog).getByRole('alert');
    expect(conflict).toHaveTextContent('Conflicts with');
    expect(conflict).toHaveTextContent('ALP-2');
    expect(conflict).toHaveTextContent('Shared areas:');
    expect(conflict).toHaveTextContent('and 4 more');
    const warning = within(dialog).getAllByRole('status').find((el) => el.textContent?.includes('ALP-3'))!;
    expect(warning).toHaveTextContent('Overlaps');
    expect(warning).toHaveTextContent('web/ui');
    expect(within(dialog).queryByRole('button', { name: 'Start' })).not.toBeInTheDocument();
    // A conflict makes Start anyway the danger variant.
    expect(within(dialog).getByRole('button', { name: 'Start anyway' }).className).toContain('text-destructive');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Wait' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ waiting: true }]));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
    expect(onStarted).not.toHaveBeenCalled();
    expect(await screen.findByText('ALP-1 will wait until clear')).toBeInTheDocument();
  });

  it('starts anyway with mode force; a plain overlap keeps it secondary', async () => {
    const calls = stubStart({ ...ready, overlaps: [overlapWith] });
    const { onStarted } = open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    const anyway = within(dialog).getByRole('button', { name: 'Start anyway' });
    expect(anyway.className).not.toContain('text-destructive');
    await userEvent.click(anyway);
    await waitFor(() => expect(onStarted).toHaveBeenCalled());
    expect(calls.find((c) => c.method === 'POST')?.body).toMatchObject({ mode: 'force' });
  });

  it('offers Wait after the server answers start_blocked', async () => {
    stubApi({
      'GET /api/tickets/t1': ready,
      'GET /api/projects': [project],
      ...areasRoute,
      'PATCH /api/tickets/t1': (c: Call) => ({ ...ready, ...(c.body as object) }),
      'POST /api/tickets/t1/start': new Reply(409, { code: 'start_blocked', message: 'Overlaps ALP-3 (web/ui)' }),
    });
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    expect(await within(dialog).findByText('Overlaps ALP-3 (web/ui)')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Wait' })).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'Start anyway' })).toBeInTheDocument();
  });

  it('says a waiting ticket is waiting and offers Stop waiting instead of Wait', async () => {
    const calls = stubStart({ ...ready, waiting: true, overlaps: [overlapWith] });
    open();
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    expect(within(dialog).getByText('Waiting until clear')).toBeInTheDocument();
    expect(within(dialog).queryByRole('button', { name: 'Wait' })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Stop waiting' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ waiting: false }]));
  });
});

// ---------------------------------------------------------------------------------------------------------------

describe('new ticket', () => {
  it('sends the areas picked from the configuration', async () => {
    const calls = stubApi({
      [`GET /api/projects/${project.id}/labels`]: [],
      ...areasRoute,
      [`POST /api/projects/${project.id}/tickets`]: (c: Call) => ticket(9, c.body as object),
    });
    renderWithQuery(<NewTicketDialog projects={[project]} projectId={project.id} title="Areas" onClose={vi.fn()} />);
    const dialog = screen.getByRole('dialog', { name: 'New ticket' });
    await waitFor(() => expect(calls.some((c) => c.path.endsWith('/areas'))).toBe(true));
    await userEvent.type(within(dialog).getByRole('combobox', { name: 'Add area' }), 'migrations{Enter}');
    // The other configured area is still offered: close the list first.
    if (screen.queryByRole('listbox')) await userEvent.keyboard('{Escape}');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Create ticket' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toMatchObject({ title: 'Areas', areas: ['migrations'] }));
  });
});
