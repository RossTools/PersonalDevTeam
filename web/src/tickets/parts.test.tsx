import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Ticket } from '../api/gen/Ticket';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { OpenTicketContext, StartTicketContext } from './actions';
import { page, project, summary, ticket } from './fixtures';
import { nextPart, scopeParts } from './model';
import { TicketView } from './TicketView';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const progress = { total: 3, done: 1, merged: 0, review: 0, progress: 1, todo: 0, backlog: 1, segments: ['done', 'progress', 'backlog'] as const };
const active: AttemptSummary = {
  id: 'a2', branch: 'b', baseBranch: 'main', worktreeId: 'w2', state: 'active', createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, pr: null,
};
const parentRef = { id: 't1', key: 'ALP-1', title: 'Phase 5' };
const parts = [
  summary(2, { title: '5a', stage: 'done', parent: parentRef }),
  summary(3, { title: '5b', stage: 'in_progress', attempt: active, parent: parentRef }),
  summary(4, { title: '5c', stage: 'draft', blockedBy: 1, parent: parentRef }),
];
const parent = ticket(1, { title: 'Phase 5', stage: 'in_progress', parts: { ...progress, segments: [...progress.segments] }, partList: parts });

function stub(t: Ticket, extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    [`GET /api/tickets/${t.id}`]: t,
    [`PATCH /api/tickets/${t.id}`]: (c: Call) => ({ ...t, ...(c.body as object) }),
    [`GET /api/tickets/${t.id}/events`]: { items: [], nextCursor: null },
    [`POST /api/tickets/${t.id}/check-merge`]: t,
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/sessions`]: [],
    'GET /api/tickets': page([summary(7, { title: 'Accounts' })]),
    ...extra,
  });
}

function view(id = 't1', { onStart = vi.fn(), onOpen = vi.fn() } = {}) {
  return renderWithQuery(
    <StartTicketContext.Provider value={onStart}>
      <OpenTicketContext.Provider value={onOpen}>
        <TicketView ticketId={id} />
      </OpenTicketContext.Provider>
    </StartTicketContext.Provider>,
  );
}

const posts = (calls: Call[], path: string) => calls.filter((c) => c.method === 'POST' && c.path.split('?')[0] === path).map((c) => c.body);

describe('scopeParts', () => {
  it('reads the top-level items under a Scope or Parts heading, bold leads only', () => {
    const md = [
      'Intro.', '', '## Scope (one PR each)',
      '- **5a — Claude conversations**: adapter trait, events.',
      '  - nested detail', '* **5b — Codex**: app-server.', '1. The rest of it:', '',
      '## Guardrails', '- not a part',
    ].join('\n');
    expect(scopeParts(md)).toEqual(['5a — Claude conversations', '5b — Codex', 'The rest of it']);
    expect(scopeParts('### Parts\n- one\n- two\n#### sub\n- three\n## Next\n- no')).toEqual(['one', 'two', 'three']);
    expect(scopeParts('- loose\n- items')).toEqual([]);
  });

  it('nextPart picks the first open, unstarted, unblocked part or says why', () => {
    expect(nextPart(parts)).toBe('its next parts are blocked: ALP-4');
    expect((nextPart([...parts, summary(5, { title: '5d' })]) as { id: string }).id).toBe('t5');
    expect(nextPart([parts[0]])).toBe('all its parts are closed');
    expect(nextPart([])).toBe('it has no parts');
  });
});

describe('a parent ticket', () => {
  it('lists its parts in order with the stepper, and hides Start', async () => {
    stub(parent);
    view();
    const list = await screen.findByRole('list', { name: 'Parts' });
    expect(within(list).getAllByRole('listitem').map((li) => li.getAttribute('aria-label'))).toEqual(['ALP-2 5a', 'ALP-3 5b', 'ALP-4 5c']);
    expect(screen.getAllByRole('img', { name: /1 of 3 parts done, 1 in progress, 1 not started/ }).length).toBeGreaterThan(0);
    expect(screen.queryByRole('button', { name: 'Start' })).not.toBeInTheDocument();
  });

  it('adds a part after the last one, blocked by it unless unticked', async () => {
    const calls = stub(parent, { [`POST /api/projects/${project.id}/tickets`]: ticket(9) });
    view();
    const input = await screen.findByRole('textbox', { name: 'New part title' });
    expect(screen.getByRole('checkbox', { name: 'Starts after ALP-4' })).toBeChecked();
    await userEvent.type(input, '5d{Enter}');
    await waitFor(() => expect(posts(calls, `/api/projects/${project.id}/tickets`)).toEqual([{ title: '5d', parent: 't1', blockedByPrevious: true }]));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Starts after ALP-4' }));
    await userEvent.type(input, '5e{Enter}');
    await waitFor(() => expect(posts(calls, `/api/projects/${project.id}/tickets`)[1]).toEqual({ title: '5e', parent: 't1' }));
  });

  it('reorders parts with Alt+arrows', async () => {
    const calls = stub(parent, { 'POST /api/tickets/t3/rank': ticket(3) });
    view();
    const row = await screen.findByRole('listitem', { name: 'ALP-3 5b' });
    fireEvent.keyDown(row, { key: 'ArrowDown', altKey: true });
    await waitFor(() => expect(posts(calls, '/api/tickets/t3/rank')).toEqual([{ after: 't4' }]));
    fireEvent.keyDown(row, { key: 'ArrowUp', altKey: true });
    await waitFor(() => expect(posts(calls, '/api/tickets/t3/rank')[1]).toEqual({ before: 't2' }));
  });

  it('starts the next part, or says why there is none', async () => {
    const onStart = vi.fn();
    stub(parent);
    view('t1', { onStart });
    await userEvent.click(await screen.findByRole('button', { name: /Start next part/ }));
    expect(await screen.findByText('ALP-1 has no part to start: its next parts are blocked: ALP-4.')).toBeInTheDocument();
    expect(onStart).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
    const ready = { ...parent, partList: [...parts, summary(5, { title: '5d', parent: parentRef })] };
    stub(ready);
    await userEvent.click(screen.getByRole('button', { name: /Start next part/ }));
    await waitFor(() => expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't5' })));
  });

  it('offers to close the open parts when moved to Done', async () => {
    let refused = false;
    const calls = stub(parent, {
      'PATCH /api/tickets/t1': (c: Call) => {
        if (!refused) {
          refused = true;
          return new Reply(409, { code: 'parts_open', message: 'ALP-1 still has open parts: ALP-3, ALP-4' });
        }
        return { ...parent, ...(c.body as object) };
      },
      'POST /api/tickets/bulk': { count: 2 },
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Move to' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Done' }));
    const ask = await screen.findByRole('alertdialog', { name: 'ALP-1 still has open parts' });
    expect(ask).toHaveTextContent('ALP-3 5b, ALP-4 5c are not closed.');
    await userEvent.click(within(ask).getByRole('button', { name: 'Close them and move' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PATCH')).toHaveLength(2));
    expect(posts(calls, '/api/tickets/bulk')).toEqual([{ ids: ['t3', 't4'], action: 'stage', value: 'wont_do' }]);
    expect(calls.filter((c) => c.method === 'PATCH').at(-1)?.body).toEqual({ stage: 'done' });
  });
});

describe('a part', () => {
  const part = ticket(3, { title: '5b', parent: parentRef });

  it('shows its parent chip, and detaches or picks a new parent', async () => {
    const onOpen = vi.fn();
    const calls = stub(part);
    view('t3', { onOpen });
    await userEvent.click((await screen.findAllByRole('button', { name: 'ALP-1 ›' }))[0]);
    expect(onOpen).toHaveBeenCalledWith({ id: 't1', projectId: project.id });
    await userEvent.click(screen.getByRole('button', { name: 'Detach' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ parent: '' }));
    await userEvent.click(screen.getByRole('button', { name: 'Change…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Make ALP-3 a part of…' });
    await userEvent.click(await within(dialog).findByRole('menuitem', { name: /ALP-7 · Accounts/ }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PATCH').at(-1)?.body).toEqual({ parent: 't7' }));
    // Only tickets that aren't parts are offered.
    expect(calls.some((c) => c.method === 'GET' && c.path.startsWith('/api/tickets?') && c.path.includes('parent=none'))).toBe(true);
  });

  it('shows the API refusing a parent', async () => {
    stub(part, {
      'PATCH /api/tickets/t3': new Reply(409, { code: 'invalid_parent', message: 'the parent is itself a part; parts are one level deep' }),
    });
    view('t3');
    await userEvent.click(await screen.findByRole('button', { name: 'Change…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Make ALP-3 a part of…' });
    await userEvent.click(await within(dialog).findByRole('menuitem', { name: /ALP-7 · Accounts/ }));
    expect(await within(dialog).findByText('the parent is itself a part; parts are one level deep')).toBeInTheDocument();
  });

  it('closes its own tab when finished, leaving the shared workspace', async () => {
    const merged = { ...active, state: 'merged' as const };
    const done = ticket(3, { title: '5b', parent: parentRef, stage: 'in_progress', attempt: merged, attempts: [merged] });
    const calls = stub(done, {
      'GET /api/workspaces': [{
        id: 'ws1', name: 'ALP-1', projectId: null, position: 1, createdAt: 1,
        tabs: [{ id: 'tp', workspaceId: 'ws1', sessionId: null, runId: null, ticketId: 't1', position: 0 }, { id: 'tt', workspaceId: 'ws1', sessionId: null, runId: null, ticketId: 't3', position: 1 }],
      }],
      'GET /api/runs': [],
      'DELETE /api/worktrees/w2': undefined,
      'DELETE /api/tabs/tt': undefined,
      [`POST /api/projects/${project.id}/pull`]: { branch: 'main', outcome: 'up_to_date', message: null },
    });
    view('t3');
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tt')).toBe(true));
    expect(calls.some((c) => c.method === 'DELETE' && c.path.startsWith('/api/workspaces/'))).toBe(false);
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tp')).toBe(false);
  });
});

describe('make into parts', () => {
  it('prefills the parts from the Scope list and makes them in one call', async () => {
    const leaf = ticket(1, { title: 'Phase 5', description: '## Scope\n- **5a**: x\n- **5b**: y' });
    const calls = stub(leaf, { 'POST /api/tickets/t1/parts': { items: [] } });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Make into parts…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Make ALP-1 into parts' });
    expect(within(dialog).getByRole('textbox', { name: 'Part 1 title' })).toHaveValue('5a');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Add part' }));
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Part 3 title' }), '5c');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Make 3 parts' }));
    await waitFor(() => expect(posts(calls, '/api/tickets/t1/parts')).toEqual([
      { parts: [{ title: '5a' }, { title: '5b' }, { title: '5c' }], blockedByPrevious: true },
    ]));
  });

  it('is not offered for a part', async () => {
    stub(ticket(3, { parent: parentRef }));
    view('t3');
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await screen.findByRole('menuitem', { name: 'Move to' });
    expect(screen.queryByRole('menuitem', { name: 'Make into parts…' })).not.toBeInTheDocument();
  });
});
