import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { AppShell } from './AppShell';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, bypassPermissions: false, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: 1, activity: null, attentionAt: null, lastViewedAt: null, unseen: false };

// The shell always renders the update banner and the first-run checklist; a fully set-up host shows neither.
const hostTool = { found: true, ok: true, detail: 'ok' };
const hostRoutes = {
  'GET /api/host/settings': {
    settings: { remoteAccess: true, ownerLogin: 'me@example.com', updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 }, maxConcurrentRuns: 4, agentCredentials: 'subscription' },
    ownerFromEnv: null,
    tokenPresent: true,
    remote: { enabled: true, active: true, url: 'https://box.ts.net', dnsName: 'box.ts.net', error: null, tailscaleLogin: 'me@example.com' },
  },
  'GET /api/host/update': {
    current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
    error: null, token: { state: 'valid', expiresAt: null }, lastResult: null,
  },
  'GET /api/host/checks': { claude: hostTool, codex: hostTool, git: hostTool, tailscale: hostTool, checkedAt: 1 },
  'GET /api/host/whoami': { caller: 'local', login: null },
};

function stubAlpha() {
  stubApi({
    ...hostRoutes,
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: [shell],
    [`GET /api/projects/${project.id}/runs`]: [],
    'GET /api/workspaces': [{ id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] }],
  });
}

describe('AppShell', () => {
  it('renders the six rail views and marks the active one', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    for (const name of ['Inbox', 'Board', 'Sessions', 'Personas', 'Files', 'Settings']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Board' }));
    expect(screen.getByRole('button', { name: 'Board' })).toHaveAttribute('aria-current', 'page');
  });

  it('shows the host settings in the Settings view', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByRole('heading', { name: 'This host' })).toBeInTheDocument();
    expect(await screen.findByRole('checkbox', { name: 'Remote access' })).toBeChecked();
  });

  it('toggles the bottom panel with Ctrl+` and with the on-screen button', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toBeInTheDocument();
    await userEvent.keyboard('{Control>}`{/Control}');
    expect(screen.queryByRole('region', { name: 'Terminal panel' })).not.toBeInTheDocument();
    const toggle = screen.getByRole('button', { name: 'Toggle terminal panel' });
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('region', { name: 'Terminal panel' })).toHaveTextContent('Select a project to open a terminal.');
  });


  it('shows the empty state when the project workspace has no tabs', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('button', { name: 'Workspace: Alpha' })).toBeInTheDocument();
    expect(screen.getByRole('main')).toHaveTextContent('No tabs here.');
  });
});

const sess = (id: string, title: string, extra: Record<string, unknown> = {}) => ({ ...shell, id, title, ...extra });
const tabOf = (id: string, workspaceId: string, sessionId: string, position: number) => ({ id, workspaceId, sessionId, runId: null, position });

/** Alpha with sessions a, b (tabs in Alpha's workspace) and c (no tab), and a free-form workspace "Release". */
function stubWorkspaces(extra: Record<string, unknown> = {}): Call[] {
  localStorage.clear();
  const sessions = [
    sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude', activity: 'working' }),
    sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', activity: 'needs_you', unseen: true }),
    sess('c', 'Shell C'),
  ];
  const workspaces = [
    { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [tabOf('ta', 'wa', 'a', 0), tabOf('tb', 'wa', 'b', 1)] },
    { id: 'wr', name: 'Release', projectId: null, position: 1, createdAt: 2, tabs: [] },
  ];
  return stubApi({
    ...hostRoutes,
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: sessions,
    [`GET /api/projects/${project.id}/runs`]: [],
    'GET /api/workspaces': workspaces,
    ...extra,
  });
}

describe('workspaces', () => {
  beforeEach(() => { document.title = 'PDT'; });
  afterEach(() => { document.title = ''; });

  it('shows the workspace tabs with their status and the current tab', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const tabs = await screen.findByRole('tablist', { name: 'Tabs' });
    await within(tabs).findByRole('tab', { name: /Claude A/ });
    const b = within(tabs).getByRole('tab', { name: /Codex B/ });
    expect(within(b).getByRole('img', { name: 'unseen' })).toBeInTheDocument();
    expect(within(tabs).getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('main').querySelector('[data-testid="terminal-view"]')).toHaveTextContent('a');
    await userEvent.click(b);
    expect(b).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('main').querySelector('[data-testid="terminal-view"]')).toHaveTextContent('b');
    await waitFor(() => expect(document.title).toBe('(1) PDT'));
  });

  it('focuses a tab from the tree, and opens one in the project workspace for an item without a tab', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wa/tabs': tabOf('tc', 'wa', 'c', 2) });
    renderWithQuery(<AppShell />);
    const tree = await screen.findByRole('region', { name: 'Project Alpha' });
    await userEvent.click(await within(tree).findByRole('button', { name: /Codex B/ }));
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(within(tree).getByRole('button', { name: /Shell C/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wa/tabs', body: { sessionId: 'c' } }));
  });

  it('closes a tab without stopping its session', async () => {
    const calls = stubWorkspaces({ 'DELETE /api/tabs/tb': undefined });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Close Codex B' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tb')).toBe(true));
    expect(calls.some((c) => c.path.includes('/kill'))).toBe(false);
  });

  it('switches tabs and opens the key help with the prefix keys', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await userEvent.keyboard('{Control>}]{/Control}2');
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.keyboard('{Control>}]{/Control}p');
    expect(screen.getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.keyboard('{Control>}]{/Control}?');
    expect(screen.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeInTheDocument();
  });

  it('jumps to the next session that needs you', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await userEvent.keyboard('{Control>}]{/Control}a');
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
  });

  it('launches an agent from the new-tab chooser right after the current tab', async () => {
    const created = sess('d', 'Claude D', { kind: 'agent_tui', agent: 'claude' });
    const calls = stubWorkspaces({ 'POST /api/sessions': created });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}c');
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await waitFor(() => expect(chooser).toHaveFocus());
    await userEvent.keyboard('c');
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body).toMatchObject({
      projectId: project.id, kind: 'agent_tui', agent: 'claude', tab: { workspaceId: 'wa', position: 1 },
    }));
  });

  it('brings an existing session into the workspace from the chooser', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wr/tabs': tabOf('ta', 'wr', 'a', 0) });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Workspace: Alpha' }));
    await userEvent.click(within(screen.getByRole('dialog', { name: 'Workspaces' })).getByRole('button', { name: /Release/ }));
    expect(await screen.findByRole('button', { name: 'Workspace: Release' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const list = within(screen.getByRole('dialog', { name: 'New tab' })).getByRole('list', { name: 'Other sessions and runs' });
    await userEvent.click(within(list).getByRole('button', { name: /Claude A/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wr/tabs', body: { sessionId: 'a', position: 0 } }));
  });

  it('creates a workspace from the switcher', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces': { id: 'wn', name: 'Spike', projectId: null, position: 2, createdAt: 3, tabs: [] } });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Workspace: Alpha' }));
    const dialog = screen.getByRole('dialog', { name: 'Workspaces' });
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'New workspace name' }), 'Spike{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces', body: { name: 'Spike' } }));
  });

  it('moves tabs from the tab menu and by dragging', async () => {
    const calls = stubWorkspaces({ 'PATCH /api/tabs/tb': tabOf('tb', 'wa', 'b', 0), 'PATCH /api/tabs/ta': tabOf('ta', 'wr', 'a', 0) });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Tab menu for Claude A' }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Tab menu for Claude A' })).getByRole('menuitem', { name: 'Release' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/tabs/ta', body: { workspaceId: 'wr' } }));

    const data: Record<string, string> = {};
    const dataTransfer = {
      setData: (k: string, v: string) => { data[k] = v; },
      getData: (k: string) => data[k] ?? '',
      get types() { return Object.keys(data); },
      effectAllowed: 'move',
    };
    fireEvent.dragStart(screen.getByRole('tab', { name: /Codex B/ }), { dataTransfer });
    const target = screen.getByRole('tab', { name: /Claude A/ }).parentElement!;
    fireEvent.dragOver(target, { dataTransfer });
    fireEvent.drop(target, { dataTransfer });
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/tabs/tb', body: { workspaceId: 'wa', position: 0 } }));
  });

  it('marks the project in the tree with the most urgent activity', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const header = await screen.findByRole('button', { name: /^Alpha/ });
    await waitFor(() => expect(within(header).getByRole('img', { name: 'waiting' })).toBeInTheDocument());
    await userEvent.click(header);
    expect(header).toHaveAttribute('aria-expanded', 'false');
  });
});
