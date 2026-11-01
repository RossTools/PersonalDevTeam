import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply } from '../test-utils';
import { page, summary, ticket } from '../tickets/fixtures';
import { AppShell } from './AppShell';
import { project, sess, tabOf, stubWorkspaces } from './fixtures';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

describe('tickets', () => {
  const fixLogin = ticket(1, { projectId: project.id, title: 'Fix login' });
  const ticketRoutes = {
    'GET /api/tickets/t1': fixLogin,
    'GET /api/tickets/ALP-1': fixLogin,
    'GET /api/tickets/t1/events': { items: [], nextCursor: null },
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/tickets': page([summary(1, { projectId: project.id, title: 'Fix login' })]),
  };

  it('opens the backlog with Ctrl+] l', async () => {
    stubWorkspaces(ticketRoutes);
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}l');
    expect(screen.getByRole('button', { name: 'Tickets' })).toHaveAttribute('aria-current', 'page');
    expect(await within(screen.getByRole('main')).findByRole('heading', { name: 'Tickets' })).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Backlog' })).toBeChecked();
    expect(screen.getByRole('navigation', { name: 'Ticket lists' })).toBeInTheDocument();
  });

  it('shows a ticket tab in the strip as "KEY · title" and the ticket in the work area', async () => {
    localStorage.clear();
    const workspaces = [{
      id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1,
      tabs: [{ id: 'tk', workspaceId: 'wa', sessionId: null, runId: null, ticketId: 't1', position: 0 }],
    }];
    stubWorkspaces({ ...ticketRoutes, 'GET /api/workspaces': workspaces });
    renderWithQuery(<AppShell />);
    const tab = await screen.findByRole('tab', { name: 'ALP-1 · Fix login' });
    expect(tab).toHaveAttribute('aria-selected', 'true');
    expect(await within(screen.getByRole('main')).findByRole('heading', { name: /Fix login/ })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Overview' })).toBeInTheDocument();
  });

  it('shows the merged banner above every tab of the ticket\'s workspace, once (PDT-71)', async () => {
    localStorage.clear();
    const mergedAttempt = {
      id: 'at', branch: 'pdt/ALP-1-fix-login', baseBranch: 'main', worktreeId: null, state: 'merged', createdAt: 1, endedAt: 2,
      lastHead: null, workspaceId: 'wt', ci: null, pr: null,
    };
    const merged = { ...fixLogin, stage: 'in_progress', attempt: mergedAttempt, attempts: [mergedAttempt] };
    const workspaces = [
      { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [tabOf('ta', 'wa', 'a', 0)] },
      {
        id: 'wt', name: 'ALP-1', projectId: null, position: 1, createdAt: 2,
        tabs: [
          { id: 'tk', workspaceId: 'wt', sessionId: null, runId: null, ticketId: 't1', position: 0 },
          tabOf('td', 'wt', 'd', 1),
        ],
        ticket: { id: 't1', projectId: project.id, key: 'ALP-1', title: 'Fix login', stage: 'in_progress', attemptId: 'at', attemptState: 'merged', pr: null },
      },
    ];
    stubWorkspaces({ ...ticketRoutes, 'GET /api/tickets/t1': merged, 'GET /api/workspaces': workspaces });
    localStorage.setItem('pdt.workspaces.current', JSON.stringify({ workspaceId: 'wt', tabs: { wt: 'td' } }));
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('tab', { name: /Claude D/ })).toHaveAttribute('aria-selected', 'true');
    expect(await screen.findByRole('button', { name: 'Move to Done and remove the worktree' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('tab', { name: 'ALP-1 · Fix login' }));
    expect(await within(screen.getByRole('main')).findByRole('heading', { name: /Fix login/ })).toBeInTheDocument();
    expect(screen.getAllByRole('button', { name: 'Move to Done and remove the worktree' })).toHaveLength(1);
    // Another workspace: no banner.
    await userEvent.click(within(screen.getByRole('region', { name: 'Workspaces' })).getByRole('row', { name: /Alpha/ }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Move to Done and remove the worktree' })).toBeNull());
  });

  it('stays in the Inbox when an item\'s session can\'t open, rather than showing the current tab (PDT-93)', async () => {
    const q = {
      id: 'q', kind: 'question', group: 'question', status: 'open', projectId: project.id, ticketId: null, ticketKey: null,
      ticketTitle: null, attemptId: null, sessionId: 'gone', runId: null, stepId: null, personaId: null, personaName: null,
      title: 'Which file?', body: { questions: [{ question: 'Which file?', choices: [] }] }, choices: [], resolution: null,
      createdAt: 1, resolvedAt: null,
    };
    stubWorkspaces({
      'GET /api/inbox': { items: [q], total: 1 },
      'POST /api/workspaces/wa/tabs': new Reply(400, { code: 'invalid_input', message: 'that session, run, ticket or worktree does not exist' }),
    });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.click(screen.getByRole('button', { name: /^Inbox/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Open session' }));
    expect(await screen.findByText('that session, run, ticket or worktree does not exist')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^Inbox/ })).toHaveAttribute('aria-current', 'page');
  });

  it('goes to a ticket by key from the palette, opening its tab in the project workspace', async () => {
    const calls = stubWorkspaces({ ...ticketRoutes, 'POST /api/workspaces/wa/tabs': { id: 'tk', workspaceId: 'wa', sessionId: null, runId: null, ticketId: 't1', position: 2 } });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.click(screen.getByRole('button', { name: 'Tickets' }));
    await userEvent.keyboard('{Control>}]{/Control}k');
    const palette = await screen.findByRole('dialog', { name: 'Command palette' });
    await userEvent.keyboard('ALP-1');
    await userEvent.click(await within(palette).findByRole('menuitem', { name: 'ALP-1 · Fix login' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/workspaces/wa/tabs')?.body).toEqual({ ticketId: 't1' }));
    expect(calls.some((c) => c.path.startsWith('/api/tickets?') && c.path.includes('q=ALP-1'))).toBe(true);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page'));
  });

  it('opens the New ticket form from the palette', async () => {
    stubWorkspaces(ticketRoutes);
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}k');
    const palette = await screen.findByRole('dialog', { name: 'Command palette' });
    await userEvent.click(within(palette).getByRole('menuitem', { name: 'New ticket' }));
    expect(await screen.findByRole('dialog', { name: 'New ticket' })).toBeInTheDocument();
  });

  it('opens the board with Ctrl+] b', async () => {
    stubWorkspaces(ticketRoutes);
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}b');
    expect(screen.getByRole('button', { name: 'Tickets' })).toHaveAttribute('aria-current', 'page');
    expect(await screen.findByRole('radio', { name: 'Board' })).toBeChecked();
    expect(await screen.findByRole('region', { name: 'In progress' })).toBeInTheDocument();
  });

  it('shows the ticket key on a started session\'s agent row and tab; the key opens the ticket', async () => {
    const started = sess('a', 'Claude · ALP-1', { kind: 'agent_tui', agent: 'claude', activity: 'working', attemptId: 'at', ticket: { id: 't1', key: 'ALP-1' } });
    const calls = stubWorkspaces({
      ...ticketRoutes,
      [`GET /api/projects/${project.id}/sessions`]: [started, sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex' })],
      'POST /api/workspaces/wa/tabs': { id: 'tk', workspaceId: 'wa', sessionId: null, runId: null, ticketId: 't1', position: 2 },
    });
    renderWithQuery(<AppShell />);
    const tabs = await screen.findByRole('tablist', { name: 'Tabs' });
    await within(tabs).findByRole('tab', { name: /Claude · ALP-1/ });
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const row = await within(agents).findByRole('row', { name: /Claude · ALP-1/ });
    expect(within(within(agents).getByRole('row', { name: /Codex B/ })).queryByRole('button', { name: /ALP-/ })).not.toBeInTheDocument();
    await userEvent.click(within(row).getByRole('button', { name: 'ALP-1' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/workspaces/wa/tabs')?.body).toEqual({ ticketId: 't1' }));
    expect(within(tabs).getByRole('button', { name: 'ALP-1' })).toBeInTheDocument();
  });

  it('starts a ticket from its tab: the tabs open where it is and the session becomes current', async () => {
    localStorage.clear();
    const ready = ticket(1, { projectId: project.id, title: 'Fix login', stage: 'ready' });
    const ticketTab = { id: 'tk', workspaceId: 'wa', sessionId: null, runId: null, ticketId: 't1', position: 0 };
    const sessionTab = { id: 'tn', workspaceId: 'wa', sessionId: 'n', runId: null, ticketId: null, position: 1 };
    let workspaces = [{ id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [ticketTab] as object[] }];
    const newSession = sess('n', 'Claude · ALP-1', { kind: 'agent_tui', agent: 'claude', attemptId: 'at', ticket: { id: 't1', key: 'ALP-1' } });
    let sessions: unknown[] = [];
    const calls = stubWorkspaces({
      ...ticketRoutes,
      'GET /api/tickets/t1': ready,
      'GET /api/workspaces': () => workspaces,
      [`GET /api/projects/${project.id}/sessions`]: () => sessions,
      'POST /api/tickets/t1/start': () => {
        workspaces = [{ ...workspaces[0], tabs: [ticketTab, sessionTab] }];
        sessions = [newSession];
        return { ticket: { ...ready, stage: 'in_progress' }, attempt: { id: 'at' }, session: newSession };
      },
    });
    renderWithQuery(<AppShell />);
    await within(await screen.findByRole('main')).findByRole('heading', { name: /Fix login/ });
    await userEvent.click(within(screen.getByRole('main')).getByRole('button', { name: 'Start' }));
    const dialog = await screen.findByRole('dialog', { name: 'Start ALP-1' });
    // A new workspace is the default; off here, so the tabs go where the ticket tab is.
    await userEvent.click(within(dialog).getByRole('switch', { name: 'Open in a new workspace' }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Start' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/tickets/t1/start')?.body).toMatchObject({
      agent: 'claude', mode: 'check', tab: { workspaceId: 'wa', position: 0 },
    }));
    await waitFor(() => expect(screen.getByRole('tab', { name: /Claude · ALP-1/ })).toHaveAttribute('aria-selected', 'true'));
    expect(screen.queryByRole('dialog', { name: 'Start ALP-1' })).not.toBeInTheDocument();
  });

  it('opens a ticket here from the new-tab chooser\'s Ticket…', async () => {
    const calls = stubWorkspaces({ ...ticketRoutes, 'POST /api/workspaces/wa/tabs': { id: 'tk', workspaceId: 'wa', sessionId: null, runId: null, ticketId: 't1', position: 1 } });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await userEvent.click(within(chooser).getByRole('menuitem', { name: 'Ticket…' }));
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: /ALP-1 · Fix login/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/workspaces/wa/tabs')?.body).toEqual({ ticketId: 't1', position: 1 }));
  });
});
