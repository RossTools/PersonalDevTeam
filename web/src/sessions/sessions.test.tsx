import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi } from '../test-utils';
import { SessionList } from './SessionList';
import { SessionView } from './SessionView';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

afterEach(() => vi.unstubAllGlobals());

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const session = (over: Partial<Session>): Session => ({
  id: 's1', projectId: 'p', worktreeId: null, kind: 'shell', agent: null, title: 'Shell', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null, ...over,
});
const worktree = { id: 'w', projectId: 'p', branch: 'feature/x', path: '/r/.worktrees/feature-x', baseCommit: 'c', createdAt: 1 };

describe('SessionList', () => {
  it('groups sessions under the repository and worktrees', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [worktree],
      'GET /api/projects/p/sessions': [session({}), session({ id: 's2', worktreeId: 'w', kind: 'agent_tui', agent: 'claude', title: 'Claude' })],
    });
    renderWithQuery(<SessionList project={project} selectedId="s2" onSelect={() => {}} />);
    const repo = await screen.findByRole('region', { name: 'Repository' });
    expect(repo).toHaveTextContent('Shell');
    const wt = screen.getByRole('region', { name: 'feature/x' });
    expect(wt).toHaveTextContent('Claude');
    expect(screen.getByRole('button', { name: /Claude/, current: true })).toBeInTheDocument();
  });

  it('starts sessions in the chosen place and selects them', async () => {
    const calls = stubApi({
      'GET /api/projects/p/worktrees': [worktree],
      'GET /api/projects/p/sessions': [],
      'POST /api/sessions': (c) => session({ id: 'new', ...(c.body as object) }),
    });
    const onSelect = vi.fn();
    renderWithQuery(<SessionList project={project} selectedId={null} onSelect={onSelect} />);
    await userEvent.click(await screen.findByRole('button', { name: 'New Shell in Repository' }));
    await userEvent.click(screen.getByRole('button', { name: 'New Claude in feature/x' }));
    const posts = calls.filter((c) => c.method === 'POST').map((c) => c.body);
    expect(posts).toEqual([
      { projectId: 'p', kind: 'shell', cols: 120, rows: 32 },
      { projectId: 'p', worktreeId: 'w', kind: 'agent_tui', agent: 'claude', cols: 120, rows: 32 },
    ]);
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('new'));
  });

  it('shows server errors', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/sessions': [],
    });
    renderWithQuery(<SessionList project={project} selectedId={null} onSelect={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'New Codex in Repository' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('POST /api/sessions');
  });
});

describe('SessionView', () => {
  it('stops a running session', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/kill': undefined, 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({})} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('s1');
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions/s1/kill')).toBe(true));
  });

  it('offers restart for an interrupted agent and shows no terminal', async () => {
    const calls = stubApi({
      'POST /api/sessions/s1/restart': session({ kind: 'agent_tui', agent: 'claude' }),
      'GET /api/projects/p/sessions': [],
    });
    renderWithQuery(<SessionView session={session({ status: 'interrupted', kind: 'agent_tui', agent: 'claude', title: 'Claude' })} onRemoved={() => {}} />);
    expect(screen.queryByTestId('terminal-view')).not.toBeInTheDocument();
    expect(screen.getByText(/resumes its last conversation/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s1/restart')?.body).toEqual({ cols: 120, rows: 32 }));
  });

  it('removes an ended session', async () => {
    stubApi({ 'DELETE /api/sessions/s1': undefined, 'GET /api/projects/p/sessions': [] });
    const onRemoved = vi.fn();
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={onRemoved} />);
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
  });
});
