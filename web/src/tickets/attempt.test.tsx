import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { api } from '../api/client';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Ticket } from '../api/gen/Ticket';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { QueryClient } from '@tanstack/react-query';
import { moveTicket, OpenSessionContext, StartTicketContext } from './actions';
import { project, ticket } from './fixtures';
import { TicketView } from './TicketView';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const attempt = (over: Partial<AttemptSummary> = {}): AttemptSummary => ({
  id: 'a1', branch: 'pdt/ALP-1-fix-login', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null, pr: null, ...over,
});

const sessions = [
  { id: 's1', title: 'Claude · ALP-1', status: 'running' as const, agent: 'claude' as const, attemptId: 'a1', persona: null, mode: 'terminal' as const },
  { id: 's0', title: 'Codex · ALP-1', status: 'finished' as const, agent: 'codex' as const, attemptId: 'a0', persona: null, mode: 'terminal' as const },
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
    [`POST /api/projects/${project.id}/pull`]: (c: Call) => ({ branch: (c.body as { branch?: string }).branch ?? 'main', outcome: 'up_to_date', message: null }),
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
    const list = within(section).getByRole('grid', { name: 'Sessions' });
    expect(within(list).getAllByRole('row').map((b) => b.textContent)).toEqual([
      expect.stringContaining('Claude · ALP-1'), expect.stringContaining('Codex · ALP-1'),
    ]);
    await userEvent.click(within(list).getByRole('row', { name: /Claude · ALP-1/ }));
    expect(onSession).toHaveBeenCalledWith({ id: 's1', projectId: project.id });
    expect(screen.queryByRole('button', { name: 'Start' })).not.toBeInTheDocument();
  });

  it("lists a workflow step's sessions on the ticket before there is an attempt", async () => {
    const refiner = { id: 's7', title: 'Refiner · ALP-1', status: 'running' as const, agent: 'claude' as const, attemptId: null, persona: 'Refiner', mode: 'conversation' as const };
    stub(ticket(1, { stage: 'draft', sessions: [refiner] }));
    const onSession = vi.fn();
    view({ onSession });
    const section = await screen.findByRole('region', { name: 'Sessions' });
    await userEvent.click(within(section).getByRole('row', { name: /Refiner · ALP-1/ }));
    expect(onSession).toHaveBeenCalledWith({ id: 's7', projectId: project.id });
    expect(screen.queryByRole('region', { name: 'Attempt' })).not.toBeInTheDocument();
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

  it('offers Clean up on an abandoned attempt whose worktree was kept, and only then (PDT-339)', async () => {
    const kept = { ...active, stage: 'wont_do' as const, attempt: attempt({ state: 'abandoned' }), attempts: [attempt({ state: 'abandoned' })] };
    const calls = stub(kept, { 'POST /api/attempts/a1/clean-up': { ...kept, attempt: attempt({ state: 'abandoned', worktreeId: null }) } });
    view();
    const section = await screen.findByRole('region', { name: 'Attempt' });
    expect(within(section).queryByRole('button', { name: 'Abandon…' })).toBeNull();
    await userEvent.click(within(section).getByRole('button', { name: 'Clean up…' }));
    const dialog = await screen.findByRole('alertdialog', { name: /Clean up ALP-1/ });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Clean up' }));
    await waitFor(() => expect(count(calls, 'POST', '/api/attempts/a1/clean-up')).toBe(1));
  });

  it('offers no Clean up once the worktree is gone', async () => {
    const gone = attempt({ state: 'abandoned', worktreeId: null });
    stub({ ...active, stage: 'wont_do', attempt: gone, attempts: [gone] });
    view();
    const section = await screen.findByRole('region', { name: 'Attempt' });
    expect(within(section).queryByRole('button', { name: 'Clean up…' })).toBeNull();
  });

  it('asks before moving a ticket at work back before work, which stops it (PDT-159)', async () => {
    const calls = stub(active);
    const qc = new QueryClient();
    const no = vi.fn().mockResolvedValue({ ok: false });
    // Without its attempt at hand, it looks the ticket up.
    expect(await moveTicket(qc, { id: 't1' }, 'todo', no)).toBeNull();
    expect(no).toHaveBeenCalledWith(expect.objectContaining({ title: "Stop ALP-1's work?", action: 'Stop and move to To do' }));
    expect(count(calls, 'PATCH', '/api/tickets/t1')).toBe(0);
    const yes = vi.fn().mockResolvedValue({ ok: true });
    await moveTicket(qc, active, 'ready', yes);
    expect(yes).toHaveBeenCalledOnce();
    expect(count(calls, 'PATCH', '/api/tickets/t1')).toBe(1);
    // Closing it, or a ticket whose attempt already ended: nothing to ask.
    const ask = vi.fn();
    await moveTicket(qc, active, 'done', ask);
    await moveTicket(qc, { ...active, attempt: attempt({ state: 'abandoned' }) }, 'todo', ask);
    expect(ask).not.toHaveBeenCalled();
    expect(count(calls, 'PATCH', '/api/tickets/t1')).toBe(3);
  });

  it('checks for a merge once when it opens on an active attempt', async () => {
    const calls = stub(active);
    // The check starts from an effect, which a render runs before it returns.
    const check = vi.spyOn(api, 'checkMerge');
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
    expect(check).toHaveBeenCalledOnce();
  });

  it('does not check for a merge without an active attempt', async () => {
    stub(ticket(1, { stage: 'ready' }));
    const check = vi.spyOn(api, 'checkMerge');
    view();
    // Shown with its ticket loaded: the effect has run.
    await screen.findByRole('button', { name: 'Start' });
    expect(check).not.toHaveBeenCalled();
  });
});

describe('merged banner', () => {
  const merged = { ...active, attempt: attempt({ state: 'merged' }), attempts: [attempt({ state: 'merged' })] };

  type Finish = { stopRunning: boolean; discardChanges: boolean; deleteBranch: boolean; keepWorktree: boolean };
  const pulled = (outcome = 'up_to_date', message: string | null = null) => ({ branch: 'main', outcome, message });
  const result = (over: object = {}) => ({
    outcome: 'finished', blocked: null, worktreeRemoved: true, workspaceRemoved: false, branchNote: null, pull: pulled(), ...over,
  });
  const blocked = (b: object) => result({ outcome: 'blocked', blocked: { sessions: 0, runs: 0, dirty: false, ...b }, worktreeRemoved: false, pull: null });
  const finishes = (calls: Call[]) => calls.filter((c) => c.method === 'POST' && c.path === '/api/tickets/t1/finish').map((c) => c.body as Finish);

  it('asks once to stop what still runs, then to discard changes, then finishes on the server', async () => {
    const calls = stub(merged, {
      'POST /api/tickets/t1/finish': (c: Call) => {
        const r = c.body as Finish;
        if (!r.stopRunning) return blocked({ sessions: 2, runs: 1, dirty: true });
        if (!r.discardChanges) return blocked({ dirty: true });
        return result();
      },
    });
    view();
    const banner = (await screen.findByText('Merged into main.')).closest<HTMLElement>('[role=status]')!;
    await userEvent.click(within(banner).getByRole('button', { name: 'Move to Done and remove the worktree' }));
    const stop = await screen.findByRole('alertdialog', { name: "Stop ALP-1's sessions?" });
    expect(stop).toHaveTextContent('2 sessions and 1 run are still running. Stop and remove them, the worktree?');
    await userEvent.click(within(stop).getByRole('button', { name: 'Stop and remove' }));
    const dirty = await screen.findByRole('alertdialog', { name: 'The worktree has uncommitted changes' });
    await userEvent.click(within(dirty).getByRole('button', { name: 'Discard and remove' }));
    expect(await screen.findByText('ALP-1 is done and its worktree is removed. main is up to date.')).toBeInTheDocument();
    expect(finishes(calls)).toEqual([
      { stopRunning: false, discardChanges: false, deleteBranch: false, keepWorktree: false },
      { stopRunning: true, discardChanges: false, deleteBranch: false, keepWorktree: false },
      { stopRunning: true, discardChanges: true, deleteBranch: false, keepWorktree: false },
    ]);
    expect(calls.filter((c) => c.method === 'DELETE' || c.path.endsWith('/kill'))).toEqual([]);
  });

  it("names the workspace and says when it went too", async () => {
    stub({ ...merged, attempt: attempt({ state: 'merged', workspaceId: 'ws1' }) }, {
      'POST /api/tickets/t1/finish': (c: Call) =>
        (c.body as Finish).stopRunning ? result({ workspaceRemoved: true }) : blocked({ sessions: 1 }),
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    const stop = await screen.findByRole('alertdialog', { name: "Stop ALP-1's sessions?" });
    expect(stop).toHaveTextContent('1 session is still running. Stop and remove it, the worktree and the workspace?');
    await userEvent.click(within(stop).getByRole('button', { name: 'Stop and remove' }));
    expect(await screen.findByText('ALP-1 is done and its worktree and workspace are removed. main is up to date.')).toBeInTheDocument();
  });

  it('keeps everything when the stop is declined', async () => {
    const calls = stub(merged, {
      'POST /api/tickets/t1/finish': (c: Call) =>
        (c.body as Finish).keepWorktree ? result({ outcome: 'kept', worktreeRemoved: false }) : blocked({ sessions: 1 }),
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    const stop = await screen.findByRole('alertdialog', { name: "Stop ALP-1's sessions?" });
    await userEvent.click(within(stop).getByRole('button', { name: 'Cancel' }));
    expect(await screen.findByText('ALP-1 moved to Done; the worktree was kept. main is up to date.')).toBeInTheDocument();
    expect(finishes(calls).at(-1)).toEqual({ stopRunning: false, discardChanges: false, deleteBranch: false, keepWorktree: true });
  });

  it('pulls the base branch and says so', async () => {
    stub(merged, { 'POST /api/tickets/t1/finish': result({ pull: pulled('updated') }) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    expect(await screen.findByText('ALP-1 is done and its worktree is removed. main is pulled.')).toBeInTheDocument();
  });

  it('tells when the base branch could not be pulled, and when the branch stayed', async () => {
    stub(merged, {
      'POST /api/tickets/t1/finish': result({
        branchNote: 'the worktree was removed, but its branch was not deleted: nope',
        pull: pulled('blocked', 'error: Your local changes to the following files would be overwritten by merge:\n\ta.txt'),
      }),
    });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' }));
    expect(await screen.findByText('ALP-1 is done and its worktree is removed.')).toBeInTheDocument();
    expect(
      await screen.findByText('main was not pulled: error: Your local changes to the following files would be overwritten by merge:'),
    ).toBeInTheDocument();
    expect(await screen.findByText('the worktree was removed, but its branch was not deleted: nope')).toBeInTheDocument();
  });

  it('is not shown once the ticket is Done and its worktree is gone', async () => {
    const gone = attempt({ state: 'merged', worktreeId: null });
    stub({ ...merged, stage: 'done', attempt: gone, attempts: [gone] });
    view();
    await screen.findByRole('region', { name: 'Attempt' });
    expect(screen.queryByText('Merged into main.')).not.toBeInTheDocument();
  });

  it('offers to remove the worktree of a Done ticket that still has one (PDT-97)', async () => {
    const calls = stub({ ...merged, stage: 'done' }, { 'POST /api/tickets/t1/finish': result() });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Remove the worktree' }));
    await waitFor(() =>
      expect(finishes(calls)).toEqual([{ stopRunning: false, discardChanges: false, deleteBranch: false, keepWorktree: false }]),
    );
  });

  it("is not shown for a Won't do ticket", async () => {
    stub({ ...merged, stage: 'wont_do' });
    view();
    await screen.findByRole('region', { name: 'Attempt' });
    expect(screen.queryByText('Merged into main.')).not.toBeInTheDocument();
  });
});
