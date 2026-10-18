import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { AppShell } from './AppShell';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'ALP', githubRepo: 'o/alpha' };
const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, bypassPermissions: false, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: 1, activity: null, attentionAt: null, lastViewedAt: null, unseen: false };

// The shell always renders the update banner; a fully set-up host shows none.
const hostTool = { found: true, ok: true, detail: 'ok' };
const hostRoutes = {
  'GET /api/host/settings': {
    settings: { remoteAccess: true, ownerLogin: 'me@example.com', updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 }, maxConcurrentRuns: 4, agentCredentials: 'subscription' },
    ownerFromEnv: null,
    tokenPresent: true,
    tokenError: null,
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
    expect(await screen.findByRole('switch', { name: 'Remote access' })).toBeChecked();
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
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    expect(await within(list).findByRole('row', { name: /Alpha/ })).toHaveAttribute('data-current', 'true');
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
    sess('d', 'Claude D', { kind: 'agent_tui', agent: 'claude', activity: 'idle', unseen: true }),
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

  it('lists agents in the sidebar; clicking one focuses its tab, or opens one when it has none', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wa/tabs': tabOf('td', 'wa', 'd', 2) });
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const b = await within(agents).findByRole('row', { name: /Codex B/ });
    expect(b).toHaveTextContent('Needs you');
    expect(b).toHaveTextContent('Alpha');
    expect(within(agents).queryByRole('row', { name: /Shell C/ })).not.toBeInTheDocument();
    await userEvent.click(b);
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    const d = within(agents).getByRole('row', { name: /Claude D/ });
    expect(d).toHaveTextContent('Done');
    expect(d).toHaveTextContent('no tab');
    await userEvent.click(d);
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wa/tabs', body: { sessionId: 'd' } }));
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

  it('keeps focus in the chooser\'s menus when the screen refreshes', async () => {
    stubWorkspaces();
    const { rerender } = renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await waitFor(() => expect(chooser).toHaveFocus());
    const place = within(chooser).getByRole('button', { name: /^Place:/ });
    place.focus();
    rerender(<AppShell />);
    expect(place).toHaveFocus();
  });

  it('brings an existing session into the workspace from the chooser', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wr/tabs': tabOf('ta', 'wr', 'a', 0) });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    await userEvent.click(await within(list).findByRole('row', { name: /Release/ }));
    expect(within(list).getByRole('row', { name: /Release/ })).toHaveAttribute('data-current', 'true');
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const others = within(screen.getByRole('dialog', { name: 'New tab' })).getByRole('grid', { name: 'Other sessions and runs' });
    await userEvent.click(within(others).getByRole('row', { name: /Claude A/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wr/tabs', body: { sessionId: 'a', position: 0 } }));
  });
  it('creates, renames and deletes workspaces from the sidebar', async () => {
    const calls = stubWorkspaces({
      'POST /api/workspaces': { id: 'wn', name: 'Spike', projectId: null, position: 2, createdAt: 3, tabs: [] },
      'PATCH /api/workspaces/wr': { id: 'wr', name: 'Ship', projectId: null, position: 1, createdAt: 2, tabs: [] },
      'DELETE /api/workspaces/wr': undefined,
    });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    await userEvent.click(within(list).getByRole('button', { name: 'New workspace' }));
    await userEvent.type(within(list).getByRole('textbox', { name: 'New workspace name' }), 'Spike{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces', body: { name: 'Spike' } }));

    fireEvent.contextMenu(within(list).getByRole('row', { name: /Release/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Release' })).getByRole('menuitem', { name: 'Rename' }));
    const name = within(list).getByRole('textbox', { name: 'Workspace name' });
    await userEvent.clear(name);
    await userEvent.type(name, 'Ship{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/workspaces/wr', body: { name: 'Ship' } }));

    fireEvent.contextMenu(within(list).getByRole('row', { name: /Release/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Release' })).getByRole('menuitem', { name: /Delete/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/workspaces/wr')).toBe(true));

    // A project's own workspace can be neither renamed nor deleted.
    fireEvent.contextMenu(within(list).getByRole('row', { name: /Alpha/ }));
    const menu = screen.getByRole('menu', { name: 'Workspace Alpha' });
    expect(within(menu).queryByRole('menuitem', { name: 'Rename' })).not.toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /Delete/ })).not.toBeInTheDocument();
  });
  it('moves tabs from the right-click menu and by dragging', async () => {
    const calls = stubWorkspaces({ 'PATCH /api/tabs/tb': tabOf('tb', 'wa', 'b', 0), 'PATCH /api/tabs/ta': tabOf('ta', 'wr', 'a', 0) });
    renderWithQuery(<AppShell />);
    fireEvent.contextMenu(await screen.findByRole('tab', { name: /Claude A/ }));
    const menu = screen.getByRole('menu', { name: 'Tab Claude A' });
    for (const item of ['Rename', 'Move left', 'Move right', 'Release', 'Close tab']) {
      expect(within(menu).getByRole('menuitem', { name: item })).toBeInTheDocument();
    }
    expect(within(menu).getByRole('menuitem', { name: 'New workspace…' })).toBeInTheDocument();
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Release' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/tabs/ta', body: { workspaceId: 'wr' } }));
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

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
  it('opens project settings, and the add-project modal, from the sidebar', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    fireEvent.contextMenu(await within(list).findByRole('row', { name: /Alpha/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Alpha' })).getByRole('menuitem', { name: 'Project settings…' }));
    expect(await within(screen.getByRole('main')).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
    await userEvent.click(screen.getByRole('button', { name: 'Sessions' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Add project' }));
    const dialog = screen.getByRole('dialog', { name: 'Add project' });
    expect(within(dialog).getByRole('radio', { name: 'Clone URL' })).toBeInTheDocument();
    // The modal hides the page behind it from assistive tech; the Sessions view is still underneath.
    expect(screen.getByRole('region', { name: 'Workspaces', hidden: true })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    expect(screen.queryByRole('dialog', { name: 'Add project' })).not.toBeInTheDocument();
  });

  it('rolls up each workspace to its most urgent agent', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    const alpha = await within(list).findByRole('row', { name: /Alpha/ });
    await waitFor(() => expect(alpha).toHaveTextContent('Needs you'));
    expect(within(alpha).getByRole('img', { name: 'Needs you' })).toBeInTheDocument();
    expect(within(list).getByRole('row', { name: /Release/ })).not.toHaveTextContent('Needs you');
  });

  it('focuses the workspace list with Ctrl+] w', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    await within(list).findByRole('row', { name: /Alpha/ });
    await userEvent.keyboard('{Control>}]{/Control}w');
    await waitFor(() => expect(within(list).getByRole('row', { name: /Alpha/ })).toHaveFocus());
    await userEvent.keyboard('{ArrowDown}');
    expect(within(list).getByRole('row', { name: /Release/ })).toHaveFocus();
  });

  it('starts an agent with skip-permissions in another place from the chooser', async () => {
    const calls = stubWorkspaces({
      [`GET /api/projects/${project.id}/worktrees`]: [{ id: 'wt1', projectId: project.id, branch: 'feat', path: '/r/.worktrees/feat', baseCommit: 'x', createdAt: 2 }],
      'POST /api/sessions': sess('e', 'Codex E', { kind: 'agent_tui', agent: 'codex' }),
    });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await userEvent.click(within(chooser).getByRole('button', { name: /^Place:/ }));
    await userEvent.click(await within(screen.getByRole('menu', { name: 'Places' })).findByRole('menuitem', { name: 'feat' }));
    await within(chooser).findByRole('button', { name: 'Place: Alpha › feat' });
    await userEvent.click(within(chooser).getByRole('switch', { name: /Skip permission prompts/ }));
    await userEvent.click(within(chooser).getByRole('button', { name: /^Codex/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body).toMatchObject({
      worktreeId: 'wt1', agent: 'codex', bypassPermissions: true,
    }));
  });

  it('manages projects in the Settings view', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { overrides: null, agentCredentials: null, effective: null, error: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    await userEvent.click(await within(nav).findByRole('row', { name: /Alpha/ }));
    expect(within(nav).getByRole('row', { name: /Alpha/ })).toHaveAttribute('data-current', 'true');
    expect(within(nav).getByRole('row', { name: /Alpha/ })).toHaveTextContent('ALP');
    expect(await within(screen.getByRole('main')).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
    expect(screen.getByRole('button', { name: 'Add project' })).toBeInTheDocument();
  });

  it('shows one state dot per tab, plus the unseen dot', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const a = await screen.findByRole('tab', { name: /Claude A/ });
    expect(within(a).getAllByRole('img').map((e) => e.getAttribute('aria-label'))).toEqual(['Working']);
    const b = screen.getByRole('tab', { name: /Codex B/ });
    expect(within(b).getAllByRole('img').map((e) => e.getAttribute('aria-label'))).toEqual(['Needs you', 'unseen']);
  });

  it('toasts when a tab action fails', async () => {
    stubWorkspaces({ 'DELETE /api/tabs/tb': new Reply(500, { code: 'internal', message: 'Tab store is locked' }) });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Close Codex B' }));
    expect(await screen.findByText('Tab store is locked')).toBeInTheDocument();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Claude A/ }));
    stubWorkspaces({ 'PATCH /api/tabs/ta': new Reply(409, { code: 'conflict', message: 'Workspace is gone' }) });
    await userEvent.click(within(screen.getByRole('menu', { name: 'Tab Claude A' })).getByRole('menuitem', { name: 'Release' }));
    expect(await screen.findByText('Workspace is gone')).toBeInTheDocument();
  });

  it('does not launch an agent on type-ahead in the chooser\'s run picker', async () => {
    const config = {
      filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null,
      effective: { runs: [{ name: 'compile', setup: null, command: 'x', web: false, data: false, env: {} }], tests: [] },
    };
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: config });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await userEvent.click(await within(chooser).findByRole('button', { name: /Start a run/ }));
    const listbox = await screen.findByRole('listbox');
    await waitFor(() => expect(listbox).toContainElement(document.activeElement as HTMLElement));
    await userEvent.keyboard('c');
    await userEvent.keyboard('{Escape}');
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions')).toBe(false);
  });
});
