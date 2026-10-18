import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { SessionView } from './SessionView';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId }: { sessionId: string }) => <div data-testid="terminal-view">{sessionId}</div>,
}));

afterEach(() => vi.unstubAllGlobals());

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', githubRepo: null };
const session = (over: Partial<Session>): Session => ({
  id: 's1', projectId: 'p', worktreeId: null, kind: 'shell', agent: null, bypassPermissions: false, title: 'Shell', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null,
  externalSessionId: null, freshStartedAt: 1,
  activity: null, attentionAt: null, lastViewedAt: null, unseen: false, ...over,
});
const worktree = { id: 'w', projectId: 'p', branch: 'feature/x', path: '/r/.worktrees/feature-x', baseCommit: 'c', createdAt: 1 };

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

  it('disables the session actions while one is in flight', async () => {
    let respond!: (v: Session) => void;
    stubApi({
      'POST /api/sessions/s1/restart': () => new Promise<Session>((r) => { respond = r; }),
      'GET /api/projects/p/sessions': [],
    });
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restart' })).toBeDisabled());
    expect(screen.getByRole('button', { name: 'Remove' })).toBeDisabled();
    respond(session({}));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Restart' })).toBeEnabled());
    expect(screen.getByRole('button', { name: 'Remove' })).toBeEnabled();
  });

  it('removes an ended session after asking, naming what is lost', async () => {
    const calls = stubApi({ 'DELETE /api/sessions/s1': undefined, 'GET /api/projects/p/sessions': [] });
    const onRemoved = vi.fn();
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={onRemoved} />);
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Remove Shell?' });
    expect(dialog).toHaveTextContent(/terminal history is deleted/);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove session' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
  });

  it('shows a failed action\'s error', async () => {
    stubApi({ 'POST /api/sessions/s1/kill': new Reply(500, { code: 'internal', message: 'kill failed' }), 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({})} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('kill failed');
  });

  it('shows the agent state in one vocabulary: dot, label and unseen', () => {
    stubApi({ 'POST /api/sessions/s1/view': undefined });
    renderWithQuery(<SessionView session={session({ kind: 'agent_tui', agent: 'claude', activity: 'needs_you', unseen: true })} onRemoved={() => {}} />);
    const header = screen.getByRole('banner');
    expect(within(header).getByRole('img', { name: 'Needs you' })).toHaveAttribute('data-status', 'waiting');
    expect(within(header).getByText('Needs you')).toBeInTheDocument();
    expect(within(header).getByRole('img', { name: 'unseen' })).toBeInTheDocument();
  });
});
