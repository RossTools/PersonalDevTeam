import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { OpenTicketContext, StartTicketContext } from './actions';
import { DONE_WINDOW_MS } from './Board';
import { event, page, params, project, summary, ticket } from './fixtures';
import { TicketsView } from './TicketsView';
import { useBacklog } from './useBacklog';

beforeEach(() => {
  localStorage.clear();
  // The board segment, remembered per browser.
  localStorage.setItem('pdt.tickets.view', JSON.stringify({ projectId: null, mode: 'board', filters: {} }));
});
afterEach(() => vi.unstubAllGlobals());

const attempt = { id: 'a1', branch: 'pdt/ALP-2-cache', baseBranch: 'main', worktreeId: 'w1', state: 'active' as const, createdAt: 1, endedAt: null, lastHead: null };
const columns = {
  ready: [summary(1, { stage: 'ready', priority: 'high', labels: ['ui'] }), summary(4, { stage: 'ready', blockedBy: 1 })],
  in_progress: [summary(2, { stage: 'in_progress', title: 'Cache', attempt })],
  done: [summary(3, { stage: 'done', closedAt: Date.now() })],
} as Record<string, ReturnType<typeof summary>[]>;

const agent: Session = {
  id: 's1', projectId: project.id, worktreeId: 'w1', kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Claude · ALP-2',
  cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'needs_you', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: 'a1', ticket: { id: 't2', key: 'ALP-2' },
};

function stub(extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    'GET /api/tickets': (c: Call) => page(columns[params(c.path).stage] ?? []),
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/sessions`]: [agent],
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/tickets/t1': ticket(1, { stage: 'ready' }),
    'GET /api/tickets/t1/events': { items: [event('e1')], nextCursor: null },
    'PATCH /api/tickets/t1': (c: Call) => ticket(1, c.body as object),
    ...extra,
  });
}

function Harness({ onOpen = vi.fn(), onStart = vi.fn() }: { onOpen?: (t: { id: string }) => void; onStart?: (t: { id: string }) => void }) {
  const backlog = useBacklog();
  return (
    <OpenTicketContext.Provider value={onOpen}>
      <StartTicketContext.Provider value={onStart}>
        <TicketsView backlog={backlog} projects={[project]} onNewTicket={() => {}} />
      </StartTicketContext.Provider>
    </OpenTicketContext.Provider>
  );
}

const column = (name: string) => screen.getByRole('region', { name });
const card = async (name: RegExp) => (await screen.findAllByRole('listitem', { name })).find((el) => el.tagName === 'LI')!;

/** A DataTransfer for fireEvent (jsdom has none). */
function transfer() {
  const data: Record<string, string> = {};
  return {
    data,
    types: ['text/plain'],
    setData: (k: string, v: string) => { data[k] = v; },
    getData: (k: string) => data[k] ?? '',
    effectAllowed: 'all',
    dropEffect: 'none',
  };
}

describe('board', () => {
  it('shows Ready, In progress and Done with their own queries; Done is the last two weeks, newest first', async () => {
    const calls = stub();
    const before = Date.now();
    renderWithQuery(<Harness />);
    expect(await within(column('Ready')).findByText('Ticket 1')).toBeInTheDocument();
    expect(await within(column('In progress')).findByText('Cache')).toBeInTheDocument();
    expect(await within(column('Done')).findByText('Ticket 3')).toBeInTheDocument();
    expect(within(column('Ready')).getByText('Blocked')).toBeInTheDocument();
    const lists = calls.filter((c) => c.path.startsWith('/api/tickets?')).map((c) => params(c.path));
    expect(lists).toContainEqual({ stage: 'ready', limit: '50' });
    expect(lists).toContainEqual({ stage: 'in_progress', limit: '50' });
    const done = lists.find((q) => q.stage === 'done')!;
    expect(done.sort).toBe('updated');
    expect(Number(done.closedAfter)).toBeGreaterThanOrEqual(before - DONE_WINDOW_MS);
    expect(Number(done.closedAfter)).toBeLessThanOrEqual(Date.now() - DONE_WINDOW_MS);
    // The started card: its branch and the attempt's agent state.
    const started = await card(/ALP-2/);
    expect(within(started).getByText('pdt/ALP-2-cache')).toBeInTheDocument();
    expect(await within(started).findByLabelText('Needs you')).toBeInTheDocument();
  });

  it('follows the project chosen for the backlog', async () => {
    localStorage.setItem('pdt.tickets.view', JSON.stringify({ projectId: project.id, mode: 'board', filters: {} }));
    const calls = stub();
    renderWithQuery(<Harness />);
    await within(column('Ready')).findByText('Ticket 1');
    const lists = calls.filter((c) => c.path.startsWith('/api/tickets?')).map((c) => params(c.path));
    expect(lists.every((q) => q.project === project.id)).toBe(true);
  });

  it('moves a card to another column by dragging it there', async () => {
    const calls = stub();
    renderWithQuery(<Harness />);
    const c = await card(/ALP-1 Ticket 1/);
    expect(c).toHaveAttribute('draggable', 'true');
    const dt = transfer();
    fireEvent.dragStart(c, { dataTransfer: dt });
    expect(dt.data['text/plain']).toBe('pdt:ticket:t1');
    fireEvent.dragOver(column('In progress'), { dataTransfer: dt });
    fireEvent.drop(column('In progress'), { dataTransfer: dt });
    await waitFor(() => expect(calls.find((x) => x.method === 'PATCH')?.body).toEqual({ stage: 'in_progress' }));
  });

  it('ignores a drop on the column the card is already in', async () => {
    const calls = stub();
    renderWithQuery(<Harness />);
    const c = await card(/ALP-1 Ticket 1/);
    const dt = transfer();
    fireEvent.dragStart(c, { dataTransfer: dt });
    fireEvent.drop(column('Ready'), { dataTransfer: dt });
    await new Promise((r) => setTimeout(r, 20));
    expect(calls.some((x) => x.method === 'PATCH')).toBe(false);
  });

  it('moves a card from its menu, and offers Start… for a Ready ticket', async () => {
    const onStart = vi.fn();
    const calls = stub();
    renderWithQuery(<Harness onStart={onStart} />);
    await card(/ALP-1 Ticket 1/);
    await userEvent.click(screen.getByRole('button', { name: 'Actions for ALP-1' }));
    let menu = await screen.findByRole('menu', { name: 'Actions for ALP-1' });
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Move to' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Done' }));
    await waitFor(() => expect(calls.find((x) => x.method === 'PATCH')?.body).toEqual({ stage: 'done' }));

    await userEvent.click(screen.getByRole('button', { name: 'Actions for ALP-1' }));
    menu = await screen.findByRole('menu', { name: 'Actions for ALP-1' });
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Start…' }));
    expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
  });

  it('opens the card menu on right-click', async () => {
    const onOpen = vi.fn();
    stub();
    renderWithQuery(<Harness onOpen={onOpen} />);
    const c = await card(/ALP-1 Ticket 1/);
    await userEvent.pointer({ keys: '[MouseRight]', target: within(c).getByRole('button', { name: /Ticket 1/ }) });
    const menu = await screen.findByRole('menu', { name: 'Ticket ALP-1' });
    for (const name of ['Open', 'Start…', 'Move to', 'Delete…']) expect(within(menu).getByRole('menuitem', { name })).toBeInTheDocument();
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Open' }));
    expect(onOpen).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
  });

  it('opens a card in the peek, which offers Start', async () => {
    const onStart = vi.fn();
    stub();
    renderWithQuery(<Harness onStart={onStart} />);
    const c = await card(/ALP-1 Ticket 1/);
    await userEvent.click(within(c).getByRole('button', { name: /Ticket 1/ }));
    const sheet = await screen.findByRole('dialog', { name: 'ALP-1 · Ticket 1' });
    await userEvent.click(within(sheet).getByRole('button', { name: 'Start' }));
    expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
  });

  it('shows one column at a time on a phone', async () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('max-width'), media: q, addEventListener: () => {}, removeEventListener: () => {} }));
    stub();
    renderWithQuery(<Harness />);
    expect(await screen.findByText('Ticket 1')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'In progress' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'In progress' }));
    expect(await screen.findByText('Cache')).toBeInTheDocument();
    expect(screen.queryByText('Ticket 1')).not.toBeInTheDocument();
    // Touch: no native drag.
    expect(await card(/ALP-2/)).toHaveAttribute('draggable', 'false');
  });
});
