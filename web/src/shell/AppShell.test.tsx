import { act, cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { page, summary, ticket } from '../tickets/fixtures';
import { AppShell } from './AppShell';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'ALP', githubRepo: 'o/alpha' };
const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, bypassPermissions: false, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: 1, activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'terminal', model: null, permissionMode: null };

// The shell always renders the update banner; a fully set-up host shows none.
const hostTool = { found: true, ok: true, detail: 'ok' };
const hostRoutes = {
  'GET /api/host/settings': {
    settings: { remoteAccess: true, ownerLogin: 'me@example.com', updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 }, maxConcurrentRuns: 4, agentCredentials: 'subscription', defaultAgent: 'claude', defaultSessionMode: 'conversation' },
    ownerFromEnv: null,
    tokenPresent: true,
    tokenError: null,
    remote: { enabled: true, active: true, url: 'https://box.ts.net', dnsName: 'box.ts.net', error: null, tailscaleLogin: 'me@example.com' },
  },
  'GET /api/host/update': {
    current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
    error: null, token: { state: 'valid', expiresAt: null }, lastResult: null,
  },
  'GET /api/host/checks': { claude: hostTool, codex: hostTool, devin: hostTool, git: hostTool, tailscale: hostTool, checkedAt: 1 },
  'GET /api/host/whoami': { caller: 'local', login: null },
};

const health = (dev: boolean, port: number) => ({
  status: 'ok', name: 'Personal Dev Team', version: '0.1.0-local', profile: dev ? 'dev' : 'stable', port, dev,
});

/** Below the md breakpoint: every max-width media query matches (the phone layout). */
const stubPhone = () =>
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: query.includes('max-width'),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));

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
  it('shows only the views that exist in the rail, and marks the active one', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    const rail = screen.getByRole('navigation', { name: 'Views' });
    expect(within(rail).getAllByRole('button').map((b) => b.getAttribute('aria-label'))).toEqual(['Project manager', 'Sessions', 'Inbox', 'Tickets', 'Workflows', 'Personas', 'Files', 'Settings', 'Expand side rail']);
    for (const name of ['Board', 'Search']) {
      expect(within(rail).queryByRole('button', { name })).not.toBeInTheDocument();
    }
    await userEvent.click(within(rail).getByRole('button', { name: 'Files' }));
    expect(within(rail).getByRole('button', { name: 'Files' })).toHaveAttribute('aria-current', 'page');
    expect(screen.queryByText(/arrives with/)).not.toBeInTheDocument();
  });

  it('shows the DEV badge at the top of the rail when /api/health says dev', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes, 'GET /api/health': health(true, 8512) });
    renderWithQuery(<AppShell />);
    const rail = screen.getByRole('navigation', { name: 'Views' });
    const badge = await within(rail).findByTitle(/^Development instance/);
    expect(badge).toHaveTextContent('DEV8512');
  });

  it('shows no DEV badge on the stable instance', async () => {
    const calls = stubApi({ 'GET /api/projects': [], ...hostRoutes, 'GET /api/health': health(false, 7420) });
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(calls.some((c) => c.path === '/api/health')).toBe(true));
    await act(async () => {});
    expect(screen.queryByTitle(/^Development instance/)).not.toBeInTheDocument();
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

  it('lists ticket workspaces apart, with the ticket, its PR and its agent state', async () => {
    const ticketWs = {
      id: 'wt', name: 'ALP-7', projectId: null, position: 1, createdAt: 3, tabs: [tabOf('td', 'wt', 'd', 0)],
      ticket: {
        id: 't7', projectId: project.id, key: 'ALP-7', title: 'Fix login', stage: 'in_progress', attemptId: 'a7', attemptState: 'active',
        pr: {
          number: 48, url: 'https://gh/pull/48', title: 'Fix login', state: 'open', draft: false, baseBranch: 'main', headSha: 'h',
          mergeable: 'mergeable', behind: false, autoMerge: true, inMergeQueue: false, queuePosition: null, queueCi: null, checkedAt: 1,
          ci: { state: 'failed', url: 'https://gh/runs/1', runs: [] },
        },
      },
    };
    stubWorkspaces({
      'GET /api/workspaces': [
        { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [tabOf('ta', 'wa', 'a', 0)] },
        ticketWs,
        { id: 'wr', name: 'Release', projectId: null, position: 2, createdAt: 2, tabs: [] },
      ],
    });
    renderWithQuery(<AppShell />);
    const tickets = await screen.findByRole('region', { name: 'Ticket workspaces' });
    const row = await within(tickets).findByRole('row', { name: /Fix login/ });
    expect(within(row).getByRole('button', { name: 'ALP-7' })).toBeInTheDocument();
    expect(within(row).getByText(/#48 · CI failed/)).toBeInTheDocument();
    expect(within(row).getByText('Done')).toBeInTheDocument();
    const general = screen.getByRole('region', { name: 'Workspaces' });
    expect(within(general).queryByRole('row', { name: /Fix login|ALP-7/ })).toBeNull();
    expect(within(general).getByRole('row', { name: /Release/ })).toBeInTheDocument();
  });

  it('shows the workspace tabs with their status and the current tab', async () => {
    // The title counts the Inbox's Needs you (PDT-108).
    stubWorkspaces({ 'GET /api/inbox': { items: [], total: 1 } });
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

  it('scrolls the tab strip to the current tab when it changes from the sidebar', async () => {
    const scroll = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scroll;
    try {
      stubWorkspaces({});
      renderWithQuery(<AppShell />);
      const agents = await screen.findByRole('region', { name: 'Agents' });
      await userEvent.click(await within(agents).findByRole('row', { name: /Codex B/ }));
      await waitFor(() => expect(scroll.mock.contexts.some((el) => (el as HTMLElement).getAttribute('aria-selected') === 'true'
        && (el as HTMLElement).textContent?.includes('Codex B'))).toBe(true));
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });
  it('lists agents in the sidebar; clicking one focuses its tab, or opens one when it has none', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wa/tabs': tabOf('td', 'wa', 'd', 2) });
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const b = await within(agents).findByRole('row', { name: /Codex B/ });
    expect(b).toHaveTextContent('Needs you');
    expect(within(agents).queryByRole('row', { name: /Shell C/ })).not.toBeInTheDocument();
    await userEvent.click(b);
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    const d = within(agents).getByRole('row', { name: /Claude D/ });
    expect(d).toHaveTextContent('Done');
    expect(d).toHaveTextContent('no tab');
    await userEvent.click(d);
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wa/tabs', body: { sessionId: 'd' } }));
  });
  it('shows each agent\'s branch, and its workspace only when it is not the current one', async () => {
    stubWorkspaces({
      [`GET /api/projects/${project.id}/worktrees`]: [{ id: 'wt1', projectId: project.id, branch: 'feat/login', path: '/r/.worktrees/feat-login', baseCommit: 'x', createdAt: 2 }],
      [`GET /api/projects/${project.id}/sessions`]: [
        sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude', activity: 'working' }),
        sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', worktreeId: 'wt1', cwd: '/r/.worktrees/feat-login' }),
        sess('d', 'Claude D', { kind: 'agent_tui', agent: 'claude' }),
      ],
    });
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const secondary = (name: RegExp) => within(agents).getByRole('row', { name }).querySelector('[data-slot=list-secondary]')?.textContent ?? null;
    await within(agents).findByRole('row', { name: /Codex B/ });
    // In the current workspace (Alpha): the main checkout says nothing, a worktree its branch.
    await waitFor(() => expect(secondary(/Codex B/)).toBe('feat/login'));
    expect(secondary(/Claude A/)).toBeNull();
    expect(secondary(/Claude D/)).toBe('no tab');
    // From another workspace, the agents' workspace follows.
    await userEvent.click(within(screen.getByRole('region', { name: 'Workspaces' })).getByRole('row', { name: /Release/ }));
    await waitFor(() => expect(secondary(/Codex B/)).toBe('feat/login · Alpha'));
    expect(secondary(/Claude A/)).toBe('Alpha');
  });

  it('opens a session\'s changes in the workspace on screen, after its tab', async () => {
    const calls = stubWorkspaces({
      [`GET /api/projects/${project.id}/worktrees`]: [{ id: 'wt1', projectId: project.id, branch: 'feat-login', path: '/r/.worktrees/feat-login', baseCommit: 'x', createdAt: 2 }],
      [`GET /api/projects/${project.id}/sessions`]: [sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', worktreeId: 'wt1', cwd: '/r/.worktrees/feat-login' })],
      'GET /api/workspaces': [
        { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] },
        { id: 'wr', name: 'Release', projectId: null, position: 1, createdAt: 2, tabs: [tabOf('tb', 'wr', 'b', 0)] },
      ],
      'POST /api/workspaces/wr/tabs': { id: 'tc', workspaceId: 'wr', sessionId: null, runId: null, worktreeId: 'wt1', position: 1 },
    });
    renderWithQuery(<AppShell />);
    await userEvent.click(await within(await screen.findByRole('region', { name: 'Workspaces' })).findByRole('row', { name: /Release/ }));
    await userEvent.click(await screen.findByRole('button', { name: /^Changes/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wr/tabs', body: { worktreeId: 'wt1', position: 1 } }));
    expect(calls.some((c) => c.path === '/api/workspaces/wa/tabs')).toBe(false);
  });

  it('keeps the agents list a plain view: no drag handles, and a menu without tab movement', async () => {
    stubWorkspaces({});
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const b = await within(agents).findByRole('row', { name: /Codex B/ });
    expect(agents.querySelector('[data-drag-handle]')).toBeNull();
    // The workspaces list above still has them (workspaces reorder by dragging).
    expect(screen.getByRole('region', { name: 'Workspaces' }).querySelector('[data-drag-handle]')).not.toBeNull();
    fireEvent.contextMenu(b);
    const menu = await screen.findByRole('menu', { name: 'Agent Codex B' });
    expect(within(menu).getAllByRole('menuitem').map((i) => i.textContent)).toEqual(['Open', 'Rename']);
  });
  it('asks before closing a running session\'s tab, which stops and removes it', async () => {
    const calls = stubWorkspaces({ 'DELETE /api/tabs/tb': undefined });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Close Codex B' }));
    let dialog = await screen.findByRole('alertdialog', { name: 'Close Codex B?' });
    expect(dialog).toHaveTextContent(/stops the agent and removes it/);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);

    await userEvent.click(screen.getByRole('button', { name: 'Close Codex B' }));
    dialog = await screen.findByRole('alertdialog', { name: 'Close Codex B?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Stop and close' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tb')).toBe(true));
  });

  it('closes an ended session\'s tab without asking', async () => {
    const calls = stubWorkspaces({
      [`GET /api/projects/${project.id}/sessions`]: [
        sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude' }),
        sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', status: 'finished' }),
      ],
      'DELETE /api/tabs/tb': undefined,
    });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await waitFor(() => expect(within(screen.getByRole('tab', { name: /Codex B/ })).queryByRole('img', { name: 'Ended' })).toBeTruthy());
    await userEvent.click(screen.getByRole('button', { name: 'Close Codex B' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tb')).toBe(true));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
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
    const choices = within(screen.getByRole('dialog', { name: 'New tab' })).getByRole('menu', { name: 'New tab choices' });
    // Its workspace follows the title.
    await userEvent.click(within(choices).getByRole('menuitem', { name: /Claude A Alpha/ }));
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
  it('opens project settings from the sidebar, which has no Add project link (that is in Settings)', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    expect(screen.queryByRole('button', { name: 'Add project' })).not.toBeInTheDocument();
    fireEvent.contextMenu(await within(list).findByRole('row', { name: /Alpha/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Alpha' })).getByRole('menuitem', { name: 'Project settings…' }));
    expect(await within(screen.getByRole('main')).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
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
    await userEvent.click(within(chooser).getByRole('menuitem', { name: /^Codex \(terminal\)/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body).toMatchObject({
      worktreeId: 'wt1', agent: 'codex', mode: 'terminal', bypassPermissions: true,
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

  it('shows one state dot per tab, plus the unseen dot, then what the session runs', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const a = await screen.findByRole('tab', { name: /Claude A/ });
    expect(within(a).getAllByRole('img').map((e) => e.getAttribute('aria-label'))).toEqual(['Working', 'Claude']);
    const b = screen.getByRole('tab', { name: /Codex B/ });
    expect(within(b).getAllByRole('img').map((e) => e.getAttribute('aria-label'))).toEqual(['Needs you', 'unseen', 'Codex']);
  });

  it('toasts when a tab action fails', async () => {
    stubWorkspaces({ 'DELETE /api/tabs/tb': new Reply(500, { code: 'internal', message: 'Tab store is locked' }) });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Close Codex B' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Stop and close' }));
    expect(await screen.findByText('Tab store is locked')).toBeInTheDocument();
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Claude A/ }));
    stubWorkspaces({ 'PATCH /api/tabs/ta': new Reply(409, { code: 'conflict', message: 'Workspace is gone' }) });
    await userEvent.click(within(screen.getByRole('menu', { name: 'Tab Claude A' })).getByRole('menuitem', { name: 'Release' }));
    expect(await screen.findByText('Workspace is gone')).toBeInTheDocument();
  });

  it('does not launch an agent on type-ahead in the chooser\'s run-another-branch pickers', async () => {
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig, [`GET /api/projects/${project.id}/branches`]: [] });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: 'Run another branch…' }));
    await userEvent.click(await within(chooser).findByRole('button', { name: /What to run/ }));
    const listbox = await screen.findByRole('listbox');
    await waitFor(() => expect(listbox).toContainElement(document.activeElement as HTMLElement));
    await userEvent.keyboard('c');
    await userEvent.keyboard('{Escape}');
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions')).toBe(false);
  });

});

const runConfig = {
  filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null,
  effective: {
    runs: [{ name: 'web', setup: null, command: 'x', web: true, data: true, env: {} }],
    tests: [{ name: 'check', setup: null, command: 'y', web: false, data: false, env: {} }],
  },
};
const runOf = (id: string) => ({
  id, projectId: project.id, worktreeId: null, branch: 'main', name: 'web', kind: 'run', state: 'starting',
  port: 8401, url: null, dataDir: '/d', cwd: '/r', exitCode: null, error: null, detached: false, createdAt: 1, updatedAt: 1,
});

describe('new-tab chooser', () => {
  const sessionPost = (calls: Call[]) => calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body;
  async function open() {
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await waitFor(() => expect(chooser).toHaveFocus());
    return chooser;
  }

  it('groups Start here, Run and Bring here, with the keys and the footer', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig });
    const chooser = await open();
    const list = within(chooser).getByRole('menu', { name: 'New tab choices' });
    for (const g of ['Start here', 'Run', 'Bring here']) expect(within(list).getByText(g)).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ })).toHaveTextContent('c');
    expect(within(list).getByRole('menuitem', { name: /^Claude \(terminal\)/ })).toHaveTextContent('t');
    expect(within(list).getByRole('menuitem', { name: /^Codex(?! \(terminal\))/ })).toHaveTextContent('x');
    expect(within(list).getByRole('menuitem', { name: /^Codex \(terminal\)/ })).toHaveTextContent('o');
    expect(await within(list).findByRole('menuitem', { name: 'Run web' })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: 'Test check' })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: 'Run another branch…' })).toBeInTheDocument();
    // Sessions without a tab here: c and d (no workspace).
    expect(within(list).getByRole('menuitem', { name: /Shell C/ })).toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: /Codex B/ })).not.toBeInTheDocument();
    expect(within(chooser).getByRole('switch', { name: /Skip permission prompts/ })).toBeInTheDocument();
    expect(within(chooser).getByRole('button', { name: /Permission mode for a new conversation/ })).toBeInTheDocument();
    expect(within(chooser).getByText('Esc to close')).toBeInTheDocument();
  });

  it.each([
    ['Shell', /^Shell/, 's', 'shell'],
    ['Codex', /^Codex(?! \(terminal\))/, 'x', 'agent_tui'],
    ['Codex (terminal)', /^Codex \(terminal\)/, 'o', 'agent_tui'],
  ] as const)('launches %s by click and by its key', async (_label, name, key, kind) => {
    const calls = stubWorkspaces({ 'POST /api/sessions': sess('e', 'New', { kind }) });
    const chooser = await open();
    await userEvent.click(within(chooser).getByRole('menuitem', { name }));
    await waitFor(() => expect(sessionPost(calls)).toMatchObject({ kind, tab: { workspaceId: 'wa', position: 1 } }));
    cleanup();
    calls.length = 0;
    await open();
    await userEvent.keyboard(key);
    await waitFor(() => expect(sessionPost(calls)).toMatchObject({ kind }));
  });

  it('types other keys into the filter, which filters every group and turns the shortcuts off', async () => {
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig });
    const chooser = await open();
    const list = within(chooser).getByRole('menu', { name: 'New tab choices' });
    await within(list).findByRole('menuitem', { name: 'Run web' });
    await userEvent.keyboard('hell');
    const filter = within(chooser).getByRole('searchbox');
    expect(filter).toHaveFocus();
    expect(filter).toHaveValue('hell');
    expect(within(list).getByRole('menuitem', { name: /^Shell(?! C)/ })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: /Shell C/ })).toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: /^Claude/ })).not.toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: 'Run web' })).not.toBeInTheDocument();
    expect(within(list).queryByText('Run')).not.toBeInTheDocument();
    // With text in the filter, c / x / s are just letters.
    await userEvent.keyboard('{Backspace}{Backspace}{Backspace}{Backspace}x');
    expect(filter).toHaveValue('x');
    expect(sessionPost(calls)).toBeUndefined();
  });

  it('starts a configured run after the current tab', async () => {
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig, [`POST /api/projects/${project.id}/runs`]: runOf('r9'),
      'POST /api/workspaces/wa/tabs': { id: 'tr', workspaceId: 'wa', sessionId: null, runId: 'r9', position: 1 },
    });
    const chooser = await open();
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: 'Test check' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === `/api/projects/${project.id}/runs`)?.body).toEqual({
      name: 'check', kind: 'test', tab: { workspaceId: 'wa', position: 1 },
    }));
  });

  it('swaps the list for the branch form and back', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig, [`GET /api/projects/${project.id}/branches`]: [] });
    const chooser = await open();
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: 'Run another branch…' }));
    expect(within(chooser).queryByRole('menu', { name: 'New tab choices' })).not.toBeInTheDocument();
    expect(await within(chooser).findByRole('button', { name: /^Branch/ })).toBeInTheDocument();
    await userEvent.click(within(chooser).getByRole('button', { name: 'Back' }));
    expect(within(chooser).getByRole('menu', { name: 'New tab choices' })).toBeInTheDocument();
  });

  it('shows the error from a failed launch', async () => {
    stubWorkspaces({ 'POST /api/sessions': new Reply(500, { code: 'spawn', message: 'claude not found' }) });
    const chooser = await open();
    await userEvent.click(within(chooser).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ }));
    expect(await within(chooser).findByText('claude not found')).toBeInTheDocument();
  });
});

describe('command palette', () => {
  it('opens with Ctrl+] k and runs the chosen item', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}k');
    const palette = await screen.findByRole('dialog', { name: 'Command palette' });
    await waitFor(() => expect(within(palette).getByRole('searchbox')).toHaveFocus());
    await userEvent.keyboard('Release');
    await userEvent.click(within(palette).getByRole('menuitem', { name: 'Workspace · Release' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Command palette' })).not.toBeInTheDocument());
    expect(within(screen.getByRole('region', { name: 'Workspaces' })).getByRole('row', { name: /Release/ })).toHaveAttribute('data-current', 'true');
  });

  it('closes the current tab through the workspaces API', async () => {
    const calls = stubWorkspaces({ 'DELETE /api/tabs/ta': undefined });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}k');
    const palette = await screen.findByRole('dialog', { name: 'Command palette' });
    await userEvent.click(within(palette).getByRole('menuitem', { name: /Close tab/ }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Stop and close' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/ta')).toBe(true));
  });

  it('switches to Settings for a project\'s settings', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}k');
    const palette = await screen.findByRole('dialog', { name: 'Command palette' });
    await userEvent.click(within(palette).getByRole('menuitem', { name: 'Settings · Alpha' }));
    expect(screen.getByRole('button', { name: 'Settings' })).toHaveAttribute('aria-current', 'page');
    expect(await within(screen.getByRole('main')).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
  });
});

describe('settings page', () => {
  it('opens the add-project dialog from the + next to PROJECTS; no inline form remains', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    expect(within(nav).getByRole('row', { name: /This host/ })).toBeInTheDocument();
    expect(screen.queryByRole('radio', { name: 'Clone URL' })).not.toBeInTheDocument();
    await userEvent.click(within(nav).getByRole('button', { name: 'Add project' }));
    const dialog = screen.getByRole('dialog', { name: 'Add project' });
    expect(within(dialog).getByRole('radio', { name: 'Clone URL' })).toBeInTheDocument();
  });

  it('has a Connections page beside This host', async () => {
    stubWorkspaces({ 'GET /api/connections': [] });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    await userEvent.click(within(nav).getByRole('row', { name: /Connections/ }));
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('heading', { name: 'Connections' })).toBeInTheDocument();
    expect(await within(main).findByRole('region', { name: 'Discord webhooks' })).toBeInTheDocument();
  });

  it('lays a project page out in sections', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    await userEvent.click(await within(nav).findByRole('row', { name: /Alpha/ }));
    const main = screen.getByRole('main');
    await within(main).findByRole('region', { name: 'Project' });
    expect(within(main).getAllByRole('region').map((r) => r.getAttribute('aria-labelledby') && document.getElementById(r.getAttribute('aria-labelledby')!)?.textContent))
      .toEqual(['Project', 'Worktrees', 'Runs on this host', 'Agent instructions', 'Agents', 'Review', 'Pipeline and policies', 'Jira and Buildkite', 'Danger zone']);
    expect(within(main).queryByText(/settings on this host/)).not.toBeInTheDocument();
  });
});

describe('phone', () => {
  // `/` opens the PM on a phone; these start in Sessions.
  beforeEach(() => window.history.replaceState(null, '', '/sessions'));

  it('opens the PM at /', async () => {
    window.history.replaceState(null, '', '/');
    stubPhone();
    stubAlpha();
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(window.location.pathname).toBe('/pm'));
  });

  const drawer = () => screen.findByRole('dialog', { name: 'Menu' });
  const openDrawer = async () => {
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    return drawer();
  };
  const here = () => window.location.pathname + window.location.search;

  it('has a top bar instead of the rail: ☰, the title, Needs you, Search and the DEV badge', async () => {
    stubPhone();
    stubWorkspaces({ 'GET /api/health': health(true, 8512), 'GET /api/inbox': { items: [], total: 3 } });
    renderWithQuery(<AppShell />);
    const bar = screen.getByRole('banner', { name: 'Top bar' });
    expect(screen.queryByRole('navigation', { name: 'Views' })).not.toBeInTheDocument();
    expect(within(bar).getByRole('button', { name: 'Menu' })).toHaveAttribute('aria-expanded', 'false');
    expect(within(bar).getByRole('heading', { name: 'Sessions' })).toBeInTheDocument();
    // Compact on phones; the port is in the tooltip.
    expect(await within(bar).findByText('DEV')).toHaveAttribute('title', 'Development instance on port 8512');
    await userEvent.click(within(bar).getByRole('button', { name: 'Search' }));
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    await userEvent.click(await within(bar).findByRole('button', { name: 'Needs you (3)' }));
    expect(here()).toBe('/inbox');
  });

  it("lists the views in the drawer in the rail's order, with the Inbox count, Settings and the host's state", async () => {
    stubPhone();
    stubWorkspaces({ 'GET /api/inbox': { items: [], total: 3 } });
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    const d = await openDrawer();
    const nav = within(d).getByRole('navigation', { name: 'Views' });
    await waitFor(() => expect(within(nav).getAllByRole('button').map((b) => b.getAttribute('aria-label')))
      .toEqual(['Project manager', 'Sessions', 'Inbox (3)', 'Tickets', 'Workflows', 'Personas', 'Files']));
    expect(within(nav).getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page');
    expect(within(d).getByRole('button', { name: 'Settings' })).toBeInTheDocument();
    expect(within(d).getByText('Host connected')).toBeInTheDocument();
    expect(within(d).getByRole('region', { name: 'Agents' })).toBeInTheDocument();
    await userEvent.click(within(nav).getByRole('button', { name: 'Tickets' }));
    expect(here()).toBe('/tickets');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Menu' })).not.toBeInTheDocument());
    // Choosing replaced the drawer's history entry: Back goes to the screen before.
    await act(() => window.history.back());
    await waitFor(() => expect(here()).toBe('/sessions'));
  });

  it('closes the drawer on Back, staying on the screen', async () => {
    stubPhone();
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    await openDrawer();
    await act(() => window.history.back());
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Menu' })).not.toBeInTheDocument());
    expect(here()).toBe('/sessions');
    // Closed by Escape, its entry goes too.
    await openDrawer();
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Menu' })).not.toBeInTheDocument());
    await waitFor(() => expect(window.history.state?.overlay).toBeFalsy());
    expect(here()).toBe('/sessions');
  });

  it('shows the workspaces at /sessions; a workspace opens on its tab, with ← back to them', async () => {
    stubPhone();
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    await userEvent.click(await within(within(main).getByRole('region', { name: 'Workspaces' })).findByRole('row', { name: /Alpha/ }));
    await waitFor(() => expect(here()).toBe('/sessions/wa/ta'));
    expect(await screen.findByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true');
    const bar = screen.getByRole('banner', { name: 'Top bar' });
    expect(within(bar).getByRole('heading', { name: 'Alpha' })).toBeInTheDocument();
    expect(within(bar).queryByRole('button', { name: 'Menu' })).not.toBeInTheDocument();
    await userEvent.click(within(bar).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/sessions'));
    expect(await within(main).findByRole('region', { name: 'Agents' })).toBeInTheDocument();
  });

  it("folds the drawer's workspaces open to their tabs; a tab opens it", async () => {
    stubPhone();
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    const d = await openDrawer();
    const tree = within(within(d).getByRole('region', { name: 'Workspaces' }));
    expect(await tree.findByRole('row', { name: /Alpha/ })).toHaveAttribute('aria-expanded', 'true');
    await userEvent.click(await tree.findByRole('row', { name: /Codex B/ }));
    await waitFor(() => expect(here()).toBe('/sessions/wa/tb'));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Menu' })).not.toBeInTheDocument());
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
  });

  it("creates a workspace from the drawer's ＋, closing the drawer", async () => {
    stubPhone();
    const calls = stubWorkspaces({ 'POST /api/workspaces': { id: 'wn', name: 'Spike', projectId: null, position: 2, createdAt: 3, tabs: [] } });
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    const d = await openDrawer();
    const section = within(within(d).getByRole('region', { name: 'Workspaces' }));
    await userEvent.click(section.getByRole('button', { name: 'New workspace' }));
    await userEvent.type(section.getByRole('textbox', { name: 'New workspace name' }), 'Spike{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces', body: { name: 'Spike' } }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Menu' })).not.toBeInTheDocument());
  });

  const topBar = () => screen.getByRole('banner', { name: 'Top bar' });
  const topMenu = async () => {
    await userEvent.click(within(topBar()).getByRole('button', { name: 'Screen actions' }));
    return screen.findByRole('menu', { name: 'Screen actions' });
  };

  it("puts a workspace's actions and the terminal panel under ⋯; the tab strip has ⧉ instead of ‹ ›", async () => {
    stubPhone();
    stubWorkspaces();
    window.history.replaceState(null, '', '/sessions/wa/ta');
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    expect(screen.queryByRole('button', { name: 'Toggle terminal panel' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Previous tab' })).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Next tab' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Tab overview' })).toBeInTheDocument();
    // A project's own workspace can't be renamed or closed.
    expect(within(await topMenu()).getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['New tab', 'Tab overview', 'Terminal panel']);
  });

  it('renames a free-form workspace in the top bar, and closes it back to /sessions', async () => {
    stubPhone();
    const calls = stubWorkspaces({ 'PATCH /api/workspaces/wr': {}, 'DELETE /api/workspaces/wr': undefined });
    window.history.replaceState(null, '', '/sessions');
    renderWithQuery(<AppShell />);
    await userEvent.click(await within(screen.getByRole('main')).findByRole('row', { name: /Release/ }));
    await waitFor(() => expect(here()).toBe('/sessions/wr'));
    let menu = await topMenu();
    expect(within(menu).getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['New tab', 'Tab overview', 'Rename', 'Terminal panel', 'Close workspace…']);
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Rename' }));
    const field = within(topBar()).getByRole('textbox', { name: 'Workspace name' });
    await userEvent.clear(field);
    await userEvent.type(field, 'Ship it{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/workspaces/wr', body: { name: 'Ship it' } }));
    menu = await topMenu();
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Close workspace…' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Close workspace' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/workspaces/wr')).toBe(true));
    await waitFor(() => expect(here()).toBe('/sessions'));
  });

  it('has New workspace and New conversation under ⋯ on /sessions, with 56px rows', async () => {
    stubPhone();
    const created = sess('n', 'Claude', { kind: 'agent_tui', agent: 'claude', mode: 'conversation' });
    // The session's tab exists once it is opened.
    let opened = false;
    const alpha = (tabs: unknown[]) => ({ id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs });
    const release = { id: 'wr', name: 'Release', projectId: null, position: 1, createdAt: 2, tabs: [] };
    const calls = stubWorkspaces({
      'GET /api/workspaces': () => [alpha([tabOf('ta', 'wa', 'a', 0), tabOf('tb', 'wa', 'b', 1), ...(opened ? [tabOf('tn', 'wa', 'n', 2)] : [])]), release],
      'POST /api/sessions': created,
      'POST /api/workspaces/wa/tabs': () => { opened = true; return tabOf('tn', 'wa', 'n', 2); },
    });
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('row', { name: /Alpha/ })).toHaveClass('min-h-14');
    let menu = await topMenu();
    expect(within(menu).getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['New workspace', 'New conversation']);
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'New workspace' }));
    expect(within(main).getByRole('textbox', { name: 'New workspace name' })).toHaveFocus();
    await userEvent.keyboard('{Escape}');
    menu = await topMenu();
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'New conversation' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body)
      .toMatchObject({ projectId: project.id, kind: 'agent_tui', agent: 'claude', mode: 'conversation' }));
    await waitFor(() => expect(here()).toBe('/sessions/wa/tn'));
  });

  it('opens the tab overview from ⧉: cards with a live snippet; a tap opens the tab, Back closes it', async () => {
    const question = {
      id: 'q', kind: 'question', group: 'question', status: 'open', projectId: project.id, ticketId: null, ticketKey: null,
      ticketTitle: null, attemptId: null, sessionId: 'b', runId: null, stepId: null, personaId: null, personaName: null,
      title: 'Which file?', body: {}, choices: [], resolution: null, createdAt: 1, resolvedAt: null,
    };
    const reply = { seq: 3, pos: 3, ts: 1, event: { kind: 'agent_message', id: 'm', markdown: 'Looking now.\n\n- Tests **pass**', phase: 'final' } };
    stubPhone();
    stubWorkspaces({
      [`GET /api/projects/${project.id}/sessions`]: [
        sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude', activity: 'working', mode: 'conversation' }),
        sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', activity: 'needs_you', unseen: true }),
      ],
      'GET /api/sessions/a/events': [reply],
      'GET /api/inbox': { items: [question], total: 1 },
    });
    window.history.replaceState(null, '', '/sessions/wa/ta');
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.click(screen.getByRole('button', { name: 'Tab overview' }));
    const overview = await screen.findByRole('region', { name: 'Tab overview' });
    const cardA = within(overview).getByRole('button', { name: /Claude A/ });
    expect(cardA).toHaveAttribute('aria-current', 'true');
    expect(await within(cardA).findByText('Tests pass')).toBeInTheDocument();
    const cardB = within(overview).getByRole('button', { name: /Codex B/ });
    expect(within(cardB).getByText('Needs you')).toBeInTheDocument();
    expect(await within(cardB).findByText('? Which file?')).toBeInTheDocument();
    // Long press (right-click) has the tab's menu.
    await userEvent.pointer({ keys: '[MouseRight]', target: cardB });
    const menu = await screen.findByRole('menu', { name: 'Tab Codex B' });
    expect(within(menu).getByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
    expect(within(menu).getByRole('menuitem', { name: 'Close tab' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    // Back closes the overview, staying on the tab.
    await act(() => window.history.back());
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Tab overview' })).not.toBeInTheDocument());
    expect(here()).toBe('/sessions/wa/ta');
    // A tap opens a tab.
    await userEvent.click(screen.getByRole('button', { name: 'Tab overview' }));
    await userEvent.click(within(await screen.findByRole('region', { name: 'Tab overview' })).getByRole('button', { name: /Codex B/ }));
    await waitFor(() => expect(here()).toBe('/sessions/wa/tb'));
    expect(screen.queryByRole('region', { name: 'Tab overview' })).not.toBeInTheDocument();
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
  });

  it('goes up from a detail screen opened from a link, and drills into Settings from its list', async () => {
    stubPhone();
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    window.history.replaceState(null, '', '/settings/projects/ALP');
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(here()).toBe('/settings');
    const list = await within(main).findByRole('navigation', { name: 'Settings sections' });
    expect(within(list).getAllByRole('row').map((r) => r.textContent)).toEqual(expect.arrayContaining(['This host', 'Connections', 'Notifications']));
    await userEvent.click(within(list).getByRole('row', { name: /Alpha/ }));
    expect(here()).toBe('/settings/projects/ALP');
    expect(await within(main).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/settings'));
  });

  it('shows the Inbox as a list; an item opens as its own screen, and ← returns to the list', async () => {
    const q = {
      id: 'q', kind: 'question', group: 'question', status: 'open', projectId: project.id, ticketId: null, ticketKey: null,
      ticketTitle: null, attemptId: null, sessionId: null, runId: null, stepId: null, personaId: null, personaName: null,
      title: 'Which file?', body: { questions: [{ question: 'Which file?', choices: [] }] }, choices: [], resolution: null,
      createdAt: 1, resolvedAt: null,
    };
    stubPhone();
    stubWorkspaces({ 'GET /api/inbox': { items: [q], total: 1 } });
    window.history.replaceState(null, '', '/inbox');
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    await userEvent.click(await within(main).findByText('Which file?'));
    expect(here()).toBe('/inbox/q');
    await userEvent.click(within(screen.getByRole('banner', { name: 'Top bar' })).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/inbox'));
    expect(within(screen.getByRole('banner', { name: 'Top bar' })).getByRole('button', { name: 'Menu' })).toBeInTheDocument();
  });

  it("has the PM's New session, Notes and earlier sessions under ⋯", async () => {
    stubPhone();
    stubAlpha();
    window.history.replaceState(null, '', '/pm');
    renderWithQuery(<AppShell />);
    await userEvent.click(within(screen.getByRole('banner', { name: 'Top bar' })).getByRole('button', { name: 'Screen actions' }));
    expect(screen.getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['New session…', 'Notes', 'Earlier sessions…']);
    await userEvent.click(screen.getByRole('menuitem', { name: 'Notes' }));
    expect(here()).toBe('/pm/notes');
    expect(within(screen.getByRole('banner', { name: 'Top bar' })).getByRole('button', { name: 'Back' })).toBeInTheDocument();
  });
});

describe('tickets', () => {
  const fixLogin = ticket(1, { projectId: project.id, title: 'Fix login' });
  const ticketRoutes = {
    'GET /api/tickets/t1': fixLogin,
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

describe('routes (PDT-111)', () => {
  const fixLogin = ticket(1, { projectId: project.id, title: 'Fix login' });
  const ticketRoutes = {
    'GET /api/tickets/t1': fixLogin,
    'GET /api/tickets/t1/events': { items: [], nextCursor: null },
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/tickets': page([summary(1, { projectId: project.id, title: 'Fix login' })]),
  };
  const at = (path: string) => window.history.replaceState(null, '', path);
  const here = () => window.location.pathname + window.location.search;
  const back = () => act(() => window.history.back());

  it('fills in the workspace and tab at / and /sessions', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/sessions/wa/ta'));
  });

  it('opens the workspace and tab in the path on reload, and Back/Forward walk the tabs chosen', async () => {
    stubWorkspaces();
    at('/sessions/wa/tb');
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('tab', { name: /Claude A/ }));
    expect(here()).toBe('/sessions/wa/ta');
    await userEvent.click(within(screen.getByRole('region', { name: 'Workspaces' })).getByRole('row', { name: /Release/ }));
    expect(here()).toBe('/sessions/wr');
    await back();
    await waitFor(() => expect(screen.getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true'));
    await back();
    await waitFor(() => expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true'));
    await act(() => window.history.forward());
    await waitFor(() => expect(screen.getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true'));
    // Remembered for when a path doesn't say.
    expect(JSON.parse(localStorage.getItem('pdt.workspaces.current')!).tabs.wa).toBe('ta');
  });

  it('puts the view in the path, and Back returns to the one before, as it was', async () => {
    stubWorkspaces(ticketRoutes);
    at('/sessions/wa/tb');
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await userEvent.click(screen.getByRole('button', { name: 'Tickets' }));
    expect(here()).toBe('/tickets');
    await userEvent.click(await screen.findByRole('radio', { name: 'Board' }));
    expect(here()).toBe('/tickets?view=board');
    await userEvent.click(screen.getByRole('button', { name: 'Sessions' }));
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    expect(here()).toBe('/sessions/wa/tb');
    // Tickets opens as it was left.
    await userEvent.click(screen.getByRole('button', { name: 'Tickets' }));
    expect(here()).toBe('/tickets?view=board');
    await back();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page'));
    await back();
    await waitFor(() => expect(screen.getByRole('radio', { name: 'Board' })).toBeChecked());
    await back();
    await waitFor(() => expect(screen.getByRole('radio', { name: 'Backlog' })).toBeChecked());
  });

  it('peeks at the ticket in /tickets/KEY, and names a peeked row by its key', async () => {
    stubWorkspaces(ticketRoutes);
    at('/tickets/ALP-1');
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('dialog', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(here()).toBe('/tickets'));
    await userEvent.click(await screen.findByRole('row', { name: /Fix login/ }));
    await waitFor(() => expect(here()).toBe('/tickets/ALP-1'));
    expect(await screen.findByRole('dialog', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
  });

  it('shows a ticket as a detail screen on a phone, where ← and Back agree', async () => {
    stubPhone();
    stubWorkspaces(ticketRoutes);
    at('/tickets');
    renderWithQuery(<AppShell />);
    const bar = screen.getByRole('banner', { name: 'Top bar' });
    await userEvent.click(await screen.findByRole('row', { name: /Fix login/ }));
    await waitFor(() => expect(here()).toBe('/tickets/ALP-1'));
    // Under the top bar, in place of the list, not a sheet over it.
    expect(await within(screen.getByRole('main')).findByRole('region', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByRole('row', { name: /Fix login/ })).not.toBeInTheDocument();
    expect(within(bar).getByRole('heading', { name: 'ALP-1' })).toBeInTheDocument();
    // ← goes Back: Forward returns to the ticket.
    await userEvent.click(within(bar).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/tickets'));
    expect(await screen.findByRole('row', { name: /Fix login/ })).toBeInTheDocument();
    await act(() => window.history.forward());
    await waitFor(() => expect(here()).toBe('/tickets/ALP-1'));
    await back();
    await waitFor(() => expect(here()).toBe('/tickets'));
    expect(within(bar).getByRole('button', { name: 'Menu' })).toBeInTheDocument();
  });

  it('opens a ticket key linked in a conversation', async () => {
    stubWorkspaces(ticketRoutes);
    at('/tickets');
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Fix login/ });
    // A link as Markdown renders it (an app path, no target).
    const a = document.createElement('a');
    a.href = '/tickets/ALP-1';
    a.textContent = 'ALP-1';
    document.body.append(a);
    await userEvent.click(a);
    a.remove();
    expect(here()).toBe('/tickets/ALP-1');
    expect(await screen.findByRole('dialog', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
  });

  it('opens Settings pages from their paths, and puts the page chosen in the path', async () => {
    stubWorkspaces({
      'GET /api/connections': [],
      [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null },
      [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null },
    });
    at('/settings/projects/ALP');
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('region', { name: 'Project' })).toBeInTheDocument();
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    await userEvent.click(within(nav).getByRole('row', { name: /Connections/ }));
    expect(here()).toBe('/settings/connections');
    expect(await within(main).findByRole('region', { name: 'Discord webhooks' })).toBeInTheDocument();
    await userEvent.click(within(nav).getByRole('row', { name: /This host/ }));
    expect(here()).toBe('/settings/host');
    await back();
    expect(await within(main).findByRole('region', { name: 'Discord webhooks' })).toBeInTheDocument();
  });

  it('shows the file in /files/<path>', async () => {
    stubWorkspaces({
      [`GET /api/projects/${project.id}/files?path=`]: [{ name: 'docs', path: 'docs', dir: true, status: null }],
      [`GET /api/projects/${project.id}/files?path=docs`]: [{ name: 'notes.txt', path: 'docs/notes.txt', dir: false, status: null }],
      [`GET /api/projects/${project.id}/files/content?path=docs%2Fnotes.txt`]: { path: 'docs/notes.txt', size: 3, status: null, text: 'hi\n', binary: false, tooLarge: false },
    });
    at('/files/docs/notes.txt?project=ALP');
    renderWithQuery(<AppShell />);
    expect(await within(screen.getByRole('main')).findByRole('heading', { name: 'docs/notes.txt' })).toBeInTheDocument();
    // Its folder opens, so the file shows in the tree.
    expect(await screen.findByRole('row', { name: /notes\.txt/ })).toHaveAttribute('aria-selected', 'true');
  });
});
