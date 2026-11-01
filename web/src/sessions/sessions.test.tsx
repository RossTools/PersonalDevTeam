import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { toast } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Session } from '../api/gen/Session';
import { githubLinks, renderWithQuery, Reply, stubApi } from '../test-utils';
import { SessionView } from './SessionView';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, dimmed }: { sessionId: string; dimmed?: boolean }) => (
    <div data-testid="terminal-view" data-dimmed={dimmed ? 'true' : 'false'}>{sessionId}</div>
  ),
}));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  act(() => { toast.dismiss(); });
});

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', forge: null, kind: 'repo' };
const session = (over: Partial<Session>): Session => ({
  id: 's1', projectId: 'p', worktreeId: null, kind: 'shell', agent: null, bypassPermissions: false, title: 'Shell', cwd: '/r',
  status: 'running', exitCode: null, createdAt: 1, endedAt: null,
  externalSessionId: null, freshStartedAt: 1,
  activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'terminal', model: null, permissionMode: null, persona: null, background: [], ...over,
});
const worktree = { id: 'w', projectId: 'p', branch: 'feature/x', path: '/r/.worktrees/feature-x', baseCommit: 'c', createdAt: 1, missing: false };

const header = () => screen.getByRole('banner');
async function openMenu() {
  await userEvent.click(within(header()).getByRole('button', { name: 'Session actions' }));
  return screen.findByRole('menu', { name: 'Session actions' });
}
async function choose(item: string | RegExp) {
  const menu = await openMenu();
  await userEvent.click(within(menu).getByRole('menuitem', { name: item }));
}

describe('SessionView header', () => {
  it('is one line on a phone (state, title, branch) with every action under ⋯', async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('max-width'), media: query, addEventListener: () => {}, removeEventListener: () => {} }));
    const calls = stubApi({
      'GET /api/projects': [project],
      'GET /api/projects/p/worktrees': [worktree],
      'GET /api/worktrees/w/changes': { worktreeId: 'w', branch: 'feature/x', base: 'main', mergeBase: 'm', head: 'h', dirty: false, files: [], attemptId: null, ticketId: null, attemptActive: false, approved: false },
      'POST /api/sessions/s1/view': undefined,
      'POST /api/sessions/s1/mode': session({}),
    });
    renderWithQuery(<SessionView session={session({ kind: 'agent_tui', agent: 'claude', title: 'Claude', worktreeId: 'w', activity: 'working' })} onRemoved={() => {}} />);
    const h = screen.getByRole('banner', { name: 'Session' });
    expect(within(h).getByText('Working')).toBeInTheDocument();
    expect(within(h).getByRole('heading', { name: 'Claude' })).toBeInTheDocument();
    expect(await within(h).findByText('feature/x')).toBeInTheDocument();
    // Only ⋯: Changes, the view switch, Stop and Restart moved under it.
    expect(within(h).getAllByRole('button').map((b) => b.getAttribute('aria-label'))).toEqual(['Session actions']);
    expect(within(h).queryByRole('radiogroup')).not.toBeInTheDocument();
    await userEvent.click(within(h).getByRole('button', { name: 'Session actions' }));
    const menu = await screen.findByRole('menu', { name: 'Session actions' });
    expect(within(menu).getAllByRole('menuitem').map((m) => m.textContent))
      .toEqual(['Changes', 'Show as conversation', 'Paste', 'Compose', 'Font size (13)', 'Stop', 'Restart', 'Copy path', 'Remove…']);
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Show as conversation' }));
    // The agent is working: switching asks first, as the desktop switch does.
    await userEvent.click(await screen.findByRole('button', { name: 'Close terminal and switch' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions/s1/mode' && (c.body as { mode: string }).mode === 'conversation')).toBe(true));
  });

  it('shows the dot, title, project key for the main checkout and the label; the path only in the tooltip', async () => {
    stubApi({ 'GET /api/projects': [project], 'POST /api/sessions/s1/view': undefined });
    renderWithQuery(<SessionView session={session({ kind: 'agent_tui', agent: 'claude', title: 'Claude', activity: 'needs_you', unseen: true })} onRemoved={() => {}} />);
    const h = header();
    expect(within(h).getByRole('img', { name: 'Needs you' })).toHaveAttribute('data-status', 'waiting');
    expect(within(h).getByText('Needs you')).toBeInTheDocument();
    expect(within(h).getByRole('img', { name: 'unseen' })).toBeInTheDocument();
    expect(within(h).getByRole('heading', { name: 'Claude' })).toHaveAttribute('title', 'Claude · /r');
    expect(await within(h).findByText('PDT')).toBeInTheDocument();
    expect(within(h).queryByText('/r')).not.toBeInTheDocument();
    // Running: no primary action, only the menu.
    expect(within(h).queryByRole('button', { name: 'Restart' })).not.toBeInTheDocument();
  });

  it('shows the worktree branch as a chip with GitHub links', async () => {
    stubApi({
      'GET /api/projects': [{ ...project, forge: githubLinks('o/r'), kind: 'repo' }],
      'GET /api/projects/p/worktrees': [worktree],
      'POST /api/sessions/s1/view': undefined,
    });
    renderWithQuery(<SessionView session={session({ worktreeId: 'w', cwd: '/r/.worktrees/feature-x' })} onRemoved={() => {}} />);
    expect(await within(header()).findByRole('button', { name: 'Branch feature/x' })).toBeInTheDocument();
    expect(within(header()).queryByText('PDT')).not.toBeInTheDocument();
    const menu = await openMenu();
    expect(within(menu).getByRole('menuitem', { name: 'Open on GitHub' })).toBeInTheDocument();
  });

  it('takes the branch and repo from props', () => {
    stubApi({ 'POST /api/sessions/s1/view': undefined });
    renderWithQuery(<SessionView session={session({ worktreeId: 'w' })} branch="feature/y" forge={null} baseBranch="main" projectKey="PDT" onRemoved={() => {}} />);
    expect(within(header()).getByText('feature/y')).toBeInTheDocument();
  });
});

describe('SessionView actions', () => {
  it('stops an idle session without asking', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/kill': undefined, 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({ kind: 'agent_tui', agent: 'claude', activity: 'idle' })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-dimmed', 'false');
    await choose('Stop');
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions/s1/kill')).toBe(true));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('asks before stopping a working agent', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/kill': undefined, 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({ kind: 'agent_tui', agent: 'claude', title: 'Claude', activity: 'working' })} onRemoved={() => {}} />);
    await choose('Stop');
    const dialog = await screen.findByRole('alertdialog', { name: 'Stop Claude?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(calls.some((c) => c.path === '/api/sessions/s1/kill')).toBe(false);
    await choose('Stop');
    await userEvent.click(await screen.findByRole('button', { name: 'Stop agent' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/sessions/s1/kill')).toBe(true));
  });

  it('copies the path and says so', async () => {
    stubApi({});
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
    renderWithQuery(<SessionView session={session({ cwd: '/r/sub' })} onRemoved={() => {}} />);
    await choose('Copy path');
    expect(writeText).toHaveBeenCalledWith('/r/sub');
    expect(await screen.findByText('Path copied')).toBeInTheDocument();
  });

  it('removes a running session after a modal that says it is stopped first', async () => {
    const calls = stubApi({ 'DELETE /api/sessions/s1': undefined, 'GET /api/projects/p/sessions': [] });
    const onRemoved = vi.fn();
    renderWithQuery(<SessionView session={session({})} onRemoved={onRemoved} />);
    await choose('Remove…');
    const dialog = await screen.findByRole('alertdialog', { name: 'Remove Shell?' });
    expect(dialog).toHaveTextContent(/still running and will be stopped/);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);
    await choose('Remove…');
    await userEvent.click(await screen.findByRole('button', { name: 'Stop and remove' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/sessions/s1')).toBe(true);
    expect(calls.some((c) => c.path === '/api/sessions/s1/kill')).toBe(false);
  });

  it('warns that removing a working agent interrupts its turn', async () => {
    stubApi({ 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({ kind: 'agent_tui', agent: 'claude', title: 'Claude', activity: 'working' })} onRemoved={() => {}} />);
    await choose('Remove…');
    const dialog = await screen.findByRole('alertdialog', { name: 'Remove Claude?' });
    expect(dialog).toHaveTextContent(/stops it mid-turn/);
  });

  it('toasts a failed action', async () => {
    stubApi({ 'POST /api/sessions/s1/kill': new Reply(500, { code: 'internal', message: 'kill failed' }), 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({})} onRemoved={() => {}} />);
    await choose('Stop');
    expect(await screen.findByText('kill failed')).toBeInTheDocument();
  });
});

describe('SessionView ended', () => {
  it('keeps the terminal, dimmed, under a bar that restarts', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/restart': session({}), 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-dimmed', 'true');
    const bar = screen.getByRole('group', { name: 'Session ended' });
    expect(within(bar).getByRole('button', { name: 'Remove…' })).toBeInTheDocument();
    expect(within(header()).getByRole('button', { name: 'Restart' })).toBeInTheDocument();
    await userEvent.click(within(bar).getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s1/restart')?.body).toEqual({ cols: 120, rows: 32 }));
  });

  it('offers Resume for an interrupted agent, with the terminal still shown', async () => {
    const calls = stubApi({ 'POST /api/sessions/s1/restart': session({ kind: 'agent_tui', agent: 'claude' }), 'GET /api/projects/p/sessions': [] });
    renderWithQuery(<SessionView session={session({ status: 'interrupted', kind: 'agent_tui', agent: 'claude', title: 'Claude' })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-dimmed', 'true');
    const bar = screen.getByRole('group', { name: 'Interrupted when the daemon stopped' });
    expect(bar).toHaveTextContent(/resumes its last conversation/);
    expect(within(bar).queryByRole('button', { name: 'Remove…' })).not.toBeInTheDocument();
    await userEvent.click(within(bar).getByRole('button', { name: 'Resume' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/sessions/s1/restart')).toBe(true));
  });

  it('disables the actions while one is in flight', async () => {
    let respond!: (v: Session) => void;
    stubApi({
      'POST /api/sessions/s1/restart': () => new Promise<Session>((r) => { respond = r; }),
      'GET /api/projects/p/sessions': [],
    });
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={() => {}} />);
    const bar = screen.getByRole('group', { name: 'Session ended' });
    await userEvent.click(within(header()).getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(within(bar).getByRole('button', { name: 'Remove…' })).toBeDisabled());
    respond(session({}));
    await waitFor(() => expect(within(bar).getByRole('button', { name: 'Remove…' })).toBeEnabled());
  });

  it('removes an ended session after asking, naming what is lost', async () => {
    const calls = stubApi({ 'DELETE /api/sessions/s1': undefined, 'GET /api/projects/p/sessions': [] });
    const onRemoved = vi.fn();
    renderWithQuery(<SessionView session={session({ status: 'finished', exitCode: 0 })} onRemoved={onRemoved} />);
    await choose('Remove…');
    const dialog = await screen.findByRole('alertdialog', { name: 'Remove Shell?' });
    expect(dialog).toHaveTextContent(/terminal history is deleted/);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);
    await userEvent.click(within(screen.getByRole('group', { name: 'Session ended' })).getByRole('button', { name: 'Remove…' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove session' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
  });
});
