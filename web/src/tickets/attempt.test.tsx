import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Ticket } from '../api/gen/Ticket';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { OpenSessionContext, StartTicketContext } from './actions';
import { project, ticket } from './fixtures';
import { TicketView } from './TicketView';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const attempt = (over: Partial<AttemptSummary> = {}): AttemptSummary => ({
  id: 'a1', branch: 'pdt/ALP-1-fix-login', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, pr: null, ...over,
});

const sessions = [
  { id: 's1', title: 'Claude · ALP-1', status: 'running' as const, agent: 'claude' as const, attemptId: 'a1', mode: 'terminal' as const },
  { id: 's0', title: 'Codex · ALP-1', status: 'finished' as const, agent: 'codex' as const, attemptId: 'a0', mode: 'terminal' as const },
];

const active = ticket(1, { title: 'Fix login', stage: 'in_progress', attempt: attempt(), attempts: [attempt()], sessions });

function stub(t: Ticket, extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    'GET /api/tickets/t1': t,
    'PATCH /api/tickets/t1': (c: Call) => ({ ...t, ...(c.body as object) }),
    'GET /api/tickets/t1/events': { items: [], nextCursor: null },
    'POST /api/tickets/t1/check-merge': t,
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/sessions`]: [],
    ...extra,
  });
}

function view({ onStart = vi.fn(), onSession = vi.fn() } = {}) {
  return renderWithQuery(
    <StartTicketContext.Provider value={onStart}>
      <OpenSessionContext.Provider value={onSession}>
        <TicketView ticketId="t1" />
      </OpenSessionContext.Provider>
    </StartTicketContext.Provider>,
  );
}

const count = (calls: Call[], method: string, path: string) => calls.filter((c) => c.method === method && c.path.split('?')[0] === path).length;

describe('ticket attempt', () => {
  it('offers Start in the header for a Ready ticket, placed where the tab is', async () => {
    stub(ticket(1, { stage: 'ready' }));
    const onStart = vi.fn();
    view({ onStart });
    await userEvent.click(await screen.findByRole('button', { name: 'Start' }));
    expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't1', projectId: project.id }), { here: true });
  });

  it('shows the attempt with its branch, state and sessions; a session opens its tab; no Start while active', async () => {
    stub(active);
    const onSession = vi.fn();
    view({ onSession });
    const section = await screen.findByRole('region', { name: 'Attempt' });
    expect(within(section).getByText('pdt/ALP-1-fix-login')).toBeInTheDocument();
    expect(within(section).getByText('Active')).toBeInTheDocument();
    const list = within(section).getByRole('list', { name: 'Sessions' });
    expect(within(list).getAllByRole('button').map((b) => b.textContent)).toEqual([
      expect.stringContaining('Claude · ALP-1'), expect.stringContaining('Codex · ALP-1'),
    ]);
    await userEvent.click(within(list).getByRole('button', { name: /Claude · ALP-1/ }));
    expect(onSession).toHaveBeenCalledWith({ id: 's1', projectId: project.id });
    expect(screen.queryByRole('button', { name: 'Start' })).not.toBeInTheDocument();
  });

  it('abandons the active attempt after confirming', async () => {
    const calls = stub(active, { 'POST /api/attempts/a1/abandon': { ...active, attempt: attempt({ state: 'abandoned' }) } });
    view();
    const section = await screen.findByRole('region', { name: 'Attempt' });
    await userEvent.click(within(section).getByRole('button', { name: 'Abandon…' }));
    const dialog = await screen.findByRole('alertdialog', { name: /Abandon ALP-1/ });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Abandon attempt' }));
    await waitFor(() => expect(count(calls, 'POST', '/api/attempts/a1/abandon')).toBe(1));
  });

  it('checks for a merge once when it opens on an active attempt', async () => {
    const calls = stub(active);
    const { rerender } = view();
    await screen.findByRole('region', { name: 'Attempt' });
    await waitFor(() => expect(count(calls, 'POST', '/api/tickets/t1/check-merge')).toBe(1));
    rerender(
      <StartTicketContext.Provider value={vi.fn()}>
        <OpenSessionContext.Provider value={vi.fn()}>
          <TicketView ticketId="t1" />
        </OpenSessionContext.Provider>
      </StartTicketContext.Provider>,
    );
    await new Promise((r) => setTimeout(r, 50));
    expect(count(calls, 'POST', '/api/tickets/t1/check-merge')).toBe(1);
  });

  it('does not check for a merge without an active attempt', async () => {
    const calls = stub(ticket(1, { stage: 'ready' }));
    view();
    await screen.findByRole('button', { name: 'Start' });
    await new Promise((r) => setTimeout(r, 50));
    expect(count(calls, 'POST', '/api/tickets/t1/check-merge')).toBe(0);
  });
});

describe('merged banner', () => {
  const merged = { ...active, attempt: attempt({ state: 'merged' }), attempts: [attempt({ state: 'merged' })] };

  const sess = (id: string, over: object = {}) => ({ id, projectId: project.id, worktreeId: 'w1', status: 'running', ...over });
  const tab = (id: string, item: object) => ({ id, workspaceId: 'ws1', sessionId: null, runId: null, ticketId: null, position: 0, ...item });

  it('moves to Done, stops everything in the worktree after asking once, then discards changes and removes it', async () => {
    let removals = 0;
    const calls = stub(merged, {
      'GET /api/workspaces': [],
      // A shell in the worktree that is not the attempt's, and an ended session of another branch.
      [`GET /api/projects/${project.id}/sessions`]: [sess('sh'), sess('x', { worktreeId: 'w2' })],
      'DELETE /api/worktrees/w1': () => {
        removals += 1;
        // Still busy while the stopped sessions exit, then dirty.
        if (removals === 1) return new Reply(409, { code: 'worktree_busy', message: 'busy' });
        if (removals === 2) return new Reply(409, { code: 'worktree_dirty', message: 'dirty' });
        return undefined;
      },
      'POST /api/sessions/s1/kill': undefined,
      'POST /api/sessions/sh/kill': undefined,
      'GET /api/runs': [{ id: 'r1', worktreeId: 'w1' }, { id: 'r2', worktreeId: 'other' }],
      'POST /api/runs/r1/stop': undefined,
    });
    view();
    const banner = (await screen.findByText('Merged into main.')).closest<HTMLElement>('[role=status]')!;
    await userEvent.click(within(banner).getByRole('button', { name: 'Move to Done and remove the worktree' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ stage: 'done' }));

    const stop = await screen.findByRole('alertdialog', { name: "Stop ALP-1's sessions?" });
    expect(stop).toHaveTextContent('2 sessions and 1 run are still running.');
    expect(removals).toBe(0);
    await userEvent.click(within(stop).getByRole('button', { name: 'Stop and remove' }));
    await waitFor(() => expect(count(calls, 'POST', '/api/runs/r1/stop')).toBe(1));
    expect(count(calls, 'POST', '/api/sessions/s1/kill')).toBe(1);
    expect(count(calls, 'POST', '/api/sessions/sh/kill')).toBe(1);
    for (const id of ['s0', 'x']) expect(count(calls, 'POST', `/api/sessions/${id}/kill`)).toBe(0);
    expect(count(calls, 'POST', '/api/runs/r2/stop')).toBe(0);

    const dirty = await screen.findByRole('alertdialog', { name: 'The worktree has uncommitted changes' });
    await userEvent.click(within(dirty).getByRole('button', { name: 'Discard and remove' }));
    await waitFor(() => expect(removals).toBe(3));
    const deletes = calls.filter((c) => c.method === 'DELETE').map((c) => c.path);
    expect(deletes[0]).toContain('force=false');
    expect(deletes[2]).toContain('force=true');
    expect(await screen.findByText('ALP-1 is done and its worktree is removed.')).toBeInTheDocument();
  });

  it("stops and removes every session in the ticket's workspace, then deletes the workspace with its tabs", async () => {
    const withWs = { ...merged, attempt: attempt({ state: 'merged', workspaceId: 'ws1' }) };
    const calls = stub(withWs, {
      'GET /api/workspaces': [{
        id: 'ws1', name: 'ALP-1', projectId: null, position: 1, createdAt: 1,
        tabs: [tab('tt', { ticketId: 't1' }), tab('t1', { sessionId: 's1' }), tab('t9', { sessionId: 's9' }), tab('tr', { runId: 'r3' })],
      }],
      'GET /api/sessions/s1': sess('s1'),
      // A session somewhere else, opened in the ticket's workspace.
      'GET /api/sessions/s9': sess('s9', { worktreeId: null }),
      'GET /api/runs': [{ id: 'r3', worktreeId: null }],
      'POST /api/sessions/s1/kill': undefined,
      'POST /api/sessions/s9/kill': undefined,
      'POST /api/runs/r3/stop': undefined,
      'DELETE /api/worktrees/w1': undefined,
      // Removing the worktree already deleted s1; s9 is still exiting at first.
      'DELETE /api/sessions/s1': new Reply(404, { code: 'not_found', message: 'gone' }),
      'DELETE /api/sessions/s9': (() => {
        let n = 0;
        return () => (++n === 1 ? new Reply(409, { code: 'session_running', message: 'running' }) : undefined);
      })(),
      'DELETE /api/workspaces/ws1': undefined,
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    const stop = await screen.findByRole('alertdialog', { name: "Stop ALP-1's sessions?" });
    expect(stop).toHaveTextContent('2 sessions and 1 run are still running. Stop and remove them, the worktree and the workspace?');
    await userEvent.click(within(stop).getByRole('button', { name: 'Stop and remove' }));
    expect(await screen.findByText('ALP-1 is done and its worktree and workspace are removed.')).toBeInTheDocument();
    expect(count(calls, 'POST', '/api/sessions/s9/kill')).toBe(1);
    expect(count(calls, 'POST', '/api/runs/r3/stop')).toBe(1);
    const deletes = calls.filter((c) => c.method === 'DELETE').map((c) => c.path.split('?')[0]);
    expect(deletes.filter((p) => p === '/api/sessions/s9')).toHaveLength(2);
    // Worktree, then the workspace's sessions, then the workspace and its tabs (the ticket's among them).
    expect(deletes[0]).toBe('/api/worktrees/w1');
    expect(deletes.at(-1)).toBe('/api/workspaces/ws1');
    expect(calls.find((c) => c.method === 'DELETE' && c.path.startsWith('/api/workspaces/'))?.path).toContain('closeTabs=true');
    expect(count(calls, 'DELETE', '/api/tabs/tt')).toBe(0);
  });

  it('keeps everything when the stop is declined', async () => {
    const withWs = { ...merged, attempt: attempt({ state: 'merged', workspaceId: 'ws1' }) };
    const calls = stub(withWs, {
      'GET /api/workspaces': [{ id: 'ws1', name: 'ALP-1', projectId: null, position: 1, createdAt: 1, tabs: [tab('tt', { ticketId: 't1' })] }],
      'GET /api/runs': [],
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    const stop = await screen.findByRole('alertdialog', { name: "Stop ALP-1's sessions?" });
    expect(stop).toHaveTextContent('1 session is still running.');
    await userEvent.click(within(stop).getByRole('button', { name: 'Cancel' }));
    expect(await screen.findByText('ALP-1 moved to Done; the worktree was kept.')).toBeInTheDocument();
    expect(count(calls, 'POST', '/api/sessions/s1/kill')).toBe(0);
    expect(calls.filter((c) => c.method === 'DELETE')).toEqual([]);
  });

  it('is not shown once the ticket is Done', async () => {
    stub({ ...merged, stage: 'done' });
    view();
    await screen.findByRole('region', { name: 'Attempt' });
    expect(screen.queryByText('Merged into main.')).not.toBeInTheDocument();
  });
});
