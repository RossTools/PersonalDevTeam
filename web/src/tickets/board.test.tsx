import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { OpenTicketContext, StartTicketContext } from './actions';
import { agentLine, boardSplits, cardActivity, displayColumns, DONE_WINDOW_MS, inColumn } from './Board';
import { COLLAPSED_KEY } from './BoardLanes';
import { event, page, params, project, summary, ticket } from './fixtures';
import { TicketsView } from './TicketsView';
import { useBacklog } from './useBacklog';

beforeEach(() => {
  localStorage.clear();
  // The board segment, remembered per browser.
  localStorage.setItem('pdt.tickets.view', JSON.stringify({ projectId: null, mode: 'board', filters: {} }));
});
afterEach(() => vi.unstubAllGlobals());

const attempt = { id: 'a1', branch: 'pdt/ALP-2-cache', baseBranch: 'main', worktreeId: 'w1', state: 'active' as const, createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, ci: null, pr: null };
const columns = {
  todo: [summary(1, { stage: 'todo', priority: 'high', labels: ['ui'] }), summary(4, { stage: 'todo', blockedBy: 1 })],
  in_progress: [summary(2, { stage: 'in_progress', title: 'Cache', attempt })],
  done: [summary(3, { stage: 'done', closedAt: Date.now() })],
} as Record<string, ReturnType<typeof summary>[]>;

const agent: Session = {
  id: 's1', projectId: project.id, worktreeId: 'w1', kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Claude · ALP-2',
  cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'needs_you', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: 'a1', ticket: { id: 't2', key: 'ALP-2' },
  mode: 'terminal', model: null, permissionMode: null, persona: null,
};

function stub(extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    'GET /api/tickets': (c: Call) => page(columns[params(c.path).stage] ?? []),
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/sessions`]: [agent],
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/tickets/t1': ticket(1, { stage: 'todo' }),
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
  it('shows To do, In progress and Done with their own queries; Done is the last two weeks, newest first', async () => {
    const calls = stub();
    const before = Date.now();
    renderWithQuery(<Harness />);
    expect(await within(column('To do')).findByText('Ticket 1')).toBeInTheDocument();
    expect(await within(column('In progress')).findByText('Cache')).toBeInTheDocument();
    expect(await within(column('Done')).findByText('Ticket 3')).toBeInTheDocument();
    expect(within(column('To do')).getByText('Blocked')).toBeInTheDocument();
    const lists = calls.filter((c) => c.path.startsWith('/api/tickets?')).map((c) => params(c.path));
    // Cards never include parents: they head lanes.
    expect(lists).toContainEqual({ stage: 'todo', hasParts: 'false', limit: '50' });
    expect(lists).toContainEqual({ stage: 'in_progress', hasParts: 'false', limit: '50' });
    expect(lists).toContainEqual({ stage: 'todo,in_progress', hasParts: 'true', limit: '100' });
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
    await within(column('To do')).findByText('Ticket 1');
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
    fireEvent.drop(column('To do'), { dataTransfer: dt });
    await new Promise((r) => setTimeout(r, 20));
    expect(calls.some((x) => x.method === 'PATCH')).toBe(false);
  });

  it('moves a card from its menu, and offers Start… for a To do ticket', async () => {
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
    await userEvent.click(screen.getByRole('button', { name: /^In progress/ }));
    expect(await screen.findByText('Cache')).toBeInTheDocument();
    expect(screen.queryByText('Ticket 1')).not.toBeInTheDocument();
    // Touch: no native drag.
    expect(await card(/ALP-2/)).toHaveAttribute('draggable', 'false');
  });

  it('picks the column from chips with counts, or by a swipe sideways, on a phone', async () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('max-width'), media: q, addEventListener: () => {}, removeEventListener: () => {} }));
    stub();
    renderWithQuery(<Harness />);
    const chips = await screen.findByRole('group', { name: 'Board column' });
    await waitFor(() => expect(within(chips).getByRole('button', { name: 'To do (2)' })).toHaveAttribute('aria-pressed', 'true'));
    expect(within(chips).getByRole('button', { name: 'In progress (1)' })).toHaveAttribute('aria-pressed', 'false');
    expect(within(chips).getByRole('button', { name: 'Done (1)' })).toBeInTheDocument();
    const board = chips.parentElement!;
    const swipe = (from: number, to: number, y = 300) => {
      fireEvent.touchStart(board, { touches: [{ clientX: from, clientY: y }] });
      fireEvent.touchEnd(board, { changedTouches: [{ clientX: to, clientY: y + 5 }] });
    };
    // Left: the next column.
    swipe(300, 150);
    expect(await screen.findByText('Cache')).toBeInTheDocument();
    expect(within(chips).getByRole('button', { name: 'In progress (1)' })).toHaveAttribute('aria-pressed', 'true');
    // Too short, mostly vertical, or from the screen's edge (the system Back gesture): nothing.
    swipe(300, 270);
    fireEvent.touchStart(board, { touches: [{ clientX: 300, clientY: 100 }] });
    fireEvent.touchEnd(board, { changedTouches: [{ clientX: 220, clientY: 400 }] });
    swipe(10, 200);
    expect(within(chips).getByRole('button', { name: 'In progress (1)' })).toHaveAttribute('aria-pressed', 'true');
    // On the chips (which scroll sideways on their own): the column stays.
    fireEvent.touchStart(within(chips).getByRole('button', { name: 'Done (1)' }), { touches: [{ clientX: 300, clientY: 140 }] });
    fireEvent.touchEnd(within(chips).getByRole('button', { name: 'Done (1)' }), { changedTouches: [{ clientX: 100, clientY: 142 }] });
    expect(within(chips).getByRole('button', { name: 'In progress (1)' })).toHaveAttribute('aria-pressed', 'true');
    // Right: the previous one.
    swipe(100, 250);
    expect(await screen.findByText('Ticket 1')).toBeInTheDocument();
    expect(within(chips).getByRole('button', { name: 'To do (2)' })).toHaveAttribute('aria-pressed', 'true');
  });

  it("moves a card to another column from its long-press menu on a phone", async () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('max-width'), media: q, addEventListener: () => {}, removeEventListener: () => {} }));
    const calls = stub();
    renderWithQuery(<Harness />);
    const c = await card(/ALP-1 Ticket 1/);
    await userEvent.pointer({ keys: '[MouseRight]', target: within(c).getByRole('button', { name: /Ticket 1/ }) });
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Move to' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'In progress' }));
    await waitFor(() => expect(calls.find((x) => x.method === 'PATCH')).toMatchObject({ path: '/api/tickets/t1', body: { stage: 'in_progress' } }));
  });
});

describe('card activity', () => {
  it("rolls up the attempt's sessions and the ones steps started on the ticket, not an earlier attempt's", () => {
    const card = { id: 't2', attempt };
    const idle = { ...agent, activity: 'idle' as const };
    // A Reviewer's session belongs to the ticket without the attempt.
    const reviewer = { ...agent, id: 's2', title: 'Reviewer · ALP-2', attemptId: null, activity: 'needs_you' as const };
    const old = { ...agent, id: 's0', attemptId: 'a0', activity: 'needs_you' as const };
    expect(cardActivity(card, [idle, old])).toEqual(cardActivity(card, [idle]));
    expect(cardActivity(card, [idle, reviewer])).toEqual(cardActivity(card, [reviewer]));
    expect(cardActivity(card, [idle, reviewer])).not.toEqual(cardActivity(card, [idle]));
  });
});

describe('board pipeline stages', () => {
  it('splits In progress into Code, Review, QA and Accept once a card has a stage, with its owner on the card', async () => {
    const calls = stub({
      'GET /api/tickets': (c: Call) => page(({
        todo: columns.todo,
        in_progress: [
          summary(2, { stage: 'in_progress', title: 'Cache', attempt }),
          summary(5, { stage: 'in_progress', title: 'Reviewing', pipelineStage: 'review',
            agent: { persona: 'Reviewer', state: 'working', sessionId: 's9', busy: true, runId: 'r1' } }),
          summary(6, { stage: 'in_progress', title: 'Accepting', pipelineStage: 'accept',
            agent: { persona: null, state: 'yours', sessionId: null, busy: false, runId: 'r2' } }),
        ],
        done: columns.done,
      } as Record<string, ReturnType<typeof summary>[]>)[params(c.path).stage] ?? []),
    });
    renderWithQuery(<Harness />);
    expect(await within(await screen.findByRole('region', { name: 'Review' })).findByText('Reviewing')).toBeInTheDocument();
    expect(within(column('Review')).getByText('Reviewer · working')).toBeInTheDocument();
    expect(within(column('Accept')).getByText('Your turn')).toBeInTheDocument();
    // No stage yet: under Code.
    expect(within(column('Code')).getByText('Cache')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'In progress' })).toBeNull();
    // Dropping on a stage moves the card to In progress; the workflow owns the stage.
    const c = await card(/ALP-1 Ticket 1/);
    const dt = transfer();
    fireEvent.dragStart(c, { dataTransfer: dt });
    fireEvent.dragOver(column('QA'), { dataTransfer: dt });
    fireEvent.drop(column('QA'), { dataTransfer: dt });
    await waitFor(() => expect(calls.find((x) => x.method === 'PATCH')?.body).toEqual({ stage: 'in_progress' }));
  });

  it('places cards and words their owner', () => {
    expect(boardSplits([{ pipelineStage: null }, { pipelineStage: 'refine' }])).toBe(false);
    expect(boardSplits([{ pipelineStage: 'qa' }])).toBe(true);
    expect(displayColumns(true).map((c) => c.title)).toEqual(['To do', 'Code', 'Review', 'QA', 'Accept', 'Merge', 'Done']);
    const [, code, review, , , merge] = displayColumns(true);
    expect(inColumn({ pipelineStage: 'merge' }, merge)).toBe(true);
    expect(inColumn({ pipelineStage: 'merge_blocked' }, merge)).toBe(true);
    expect(inColumn({ pipelineStage: 'merge_blocked' }, code)).toBe(false);
    expect(inColumn({ pipelineStage: null }, code)).toBe(true);
    expect(inColumn({ pipelineStage: 'review' }, code)).toBe(false);
    expect(inColumn({ pipelineStage: 'review' }, review)).toBe(true);
    const a = { persona: 'Coder', sessionId: null, busy: false, runId: null };
    expect(agentLine({ ...a, state: 'asking' }).text).toBe('Coder asks you');
    expect(agentLine({ ...a, state: 'stalled' }).dot.dot).toBe('failed');
    expect(agentLine({ ...a, state: 'queued' }).text).toBe('Coder · waiting for a slot');
    expect(agentLine({ ...a, state: 'yours' }).text).toBe('Your turn · Coder');
    expect(agentLine({ ...a, state: 'working', busy: true }).dot.pulse).toBe(true);
  });
});

describe('board lanes', () => {
  const progress = (segments: ('done' | 'progress' | 'todo' | 'backlog')[]) => ({
    total: segments.length, done: segments.filter((s) => s === 'done').length, merged: 0, review: 0,
    progress: segments.filter((s) => s === 'progress').length, todo: segments.filter((s) => s === 'todo').length,
    backlog: segments.filter((s) => s === 'backlog').length, segments,
  });
  // PDT-like: parent ALP-10 (rank b) with parts in To do and In progress; parent ALP-20 (rank a, picked, nothing on
  // the board); ALP-1 is not a part.
  const p10 = summary(10, { title: 'Phase five', stage: 'in_progress', rank: 'b', parts: progress(['done', 'progress', 'todo']) });
  const p20 = summary(20, { title: 'Phase seven', stage: 'todo', rank: 'a', parts: progress(['backlog']) });
  const asPart = (n: number, stage: 'todo' | 'in_progress' | 'done', parent = p10) =>
    summary(n, { stage, title: `Part ${n}`, parent: { id: parent.id, key: parent.key, title: parent.title } });
  const lanes = {
    todo: [summary(1, { stage: 'todo' }), asPart(12, 'todo')],
    in_progress: [asPart(11, 'in_progress')],
    done: [],
  } as Record<string, ReturnType<typeof summary>[]>;

  function laneStub(): Call[] {
    return stubApi({
      'GET /api/tickets': (c: Call) => {
        const q = params(c.path);
        if (q.hasParts === 'true') return page(q.stage === 'todo,in_progress' ? [p10, p20] : []);
        return page(lanes[q.stage] ?? []);
      },
      'GET /api/projects': [project],
      [`GET /api/projects/${project.id}/sessions`]: [],
      'PATCH /api/tickets/t11': (c: Call) => ticket(11, c.body as object),
      'PATCH /api/tickets/t1': (c: Call) => ticket(1, c.body as object),
    });
  }

  const lane = (key: string) => screen.findByRole('group', { name: new RegExp(`^${key} `) });

  it('heads a lane per parent, by rank, after the tickets that are not parts', async () => {
    laneStub();
    renderWithQuery(<Harness />);
    const seven = await lane('ALP-20');
    const five = await lane('ALP-10');
    // ALP-20 ranks first; a picked parent with nothing on the board still has its lane.
    expect(seven.compareDocumentPosition(five) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(screen.getByText('No parts on the board yet.')).toBeInTheDocument();
    expect(within(five).getByRole('img', { name: /1 of 3 parts done/ })).toBeInTheDocument();
    expect(within(five).getByText('1/3')).toBeInTheDocument();
    // The parts sit in their parent's lane, in their columns; the others in the first lane.
    expect(within(column('ALP-10 · To do')).getByText('Part 12')).toBeInTheDocument();
    expect(within(column('ALP-10 · In progress')).getByText('Part 11')).toBeInTheDocument();
    expect(within(column('To do')).getByText('Ticket 1')).toBeInTheDocument();
    expect(within(column('To do')).queryByText('Part 12')).not.toBeInTheDocument();
    // A long first lane (Done) stops at half the screen and scrolls on its own, so the lanes below stay in reach.
    expect(column('Done')).toHaveClass('max-h-[50dvh]');
    expect(column('ALP-10 · Done')).not.toHaveClass('max-h-[50dvh]');
    // Parents are lane headers, never cards.
    expect(screen.queryByRole('listitem', { name: /ALP-10 Phase five/ })).not.toBeInTheDocument();
  });

  it('starts a lane parent’s next part from its header', async () => {
    const onStart = vi.fn();
    stubApi({
      'GET /api/tickets': (c: Call) => {
        const q = params(c.path);
        if (q.hasParts === 'true') return page(q.stage === 'todo,in_progress' ? [p10, p20] : []);
        return page(lanes[q.stage] ?? []);
      },
      'GET /api/projects': [project],
      [`GET /api/projects/${project.id}/sessions`]: [],
      'GET /api/tickets/t10': ticket(10, { partList: [asPart(13, 'done'), { ...asPart(12, 'todo') }] }),
    });
    renderWithQuery(<Harness onStart={onStart} />);
    const five = await lane('ALP-10');
    await userEvent.click(within(five).getByRole('button', { name: 'Start next part' }));
    await waitFor(() => expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't12' })));
  });

  it('remembers a collapsed lane', async () => {
    laneStub();
    const { unmount } = renderWithQuery(<Harness />);
    const five = await lane('ALP-10');
    await userEvent.click(within(five).getByRole('button', { name: 'Collapse ALP-10' }));
    expect(screen.queryByRole('region', { name: 'ALP-10 · To do' })).not.toBeInTheDocument();
    expect(JSON.parse(localStorage.getItem(COLLAPSED_KEY)!)).toEqual(['t10']);
    unmount();
    renderWithQuery(<Harness />);
    expect(within(await lane('ALP-10')).getByRole('button', { name: 'Expand ALP-10' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'ALP-10 · To do' })).not.toBeInTheDocument();
  });

  it('moves a card only within its own lane', async () => {
    const calls = laneStub();
    renderWithQuery(<Harness />);
    const part = await card(/ALP-11 Part 11/);
    // Dropped on the first lane's Done: ignored.
    let dt = transfer();
    fireEvent.dragStart(part, { dataTransfer: dt });
    fireEvent.drop(column('Done'), { dataTransfer: dt });
    // A first-lane card dropped in the parent's lane: ignored.
    const plain = await card(/ALP-1 Ticket 1/);
    dt = transfer();
    fireEvent.dragStart(plain, { dataTransfer: dt });
    fireEvent.drop(column('ALP-10 · Done'), { dataTransfer: dt });
    await new Promise((r) => setTimeout(r, 20));
    expect(calls.some((x) => x.method === 'PATCH')).toBe(false);
    // Within its lane: moved.
    dt = transfer();
    fireEvent.dragStart(part, { dataTransfer: dt });
    fireEvent.dragOver(column('ALP-10 · Done'), { dataTransfer: dt });
    fireEvent.drop(column('ALP-10 · Done'), { dataTransfer: dt });
    await waitFor(() => expect(calls.find((x) => x.method === 'PATCH')).toMatchObject({ path: '/api/tickets/t11', body: { stage: 'done' } }));
  });

  it('shows parents as group headings in the one column on a phone', async () => {
    vi.stubGlobal('matchMedia', (q: string) => ({ matches: q.includes('max-width'), media: q, addEventListener: () => {}, removeEventListener: () => {} }));
    laneStub();
    renderWithQuery(<Harness />);
    const five = await lane('ALP-10');
    expect(within(five).getByText('1/3')).toBeInTheDocument();
    expect(within(column('ALP-10 · To do')).getByText('Part 12')).toBeInTheDocument();
    expect(screen.queryByText('Part 11')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /^In progress/ }));
    expect(await within(column('ALP-10 · In progress')).findByText('Part 11')).toBeInTheDocument();
  });
});
