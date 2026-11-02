import { act, cleanup, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { onlineManager } from '@tanstack/react-query';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { plugins } from '../api/gen/plugins';
import { AppShell } from './AppShell';
import { project, hostRoutes, health, stubAlpha, stubWorkspaces } from './fixtures';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

describe('AppShell', () => {
  it('shows only the views that exist in the rail, and marks the active one', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    const rail = screen.getByRole('navigation', { name: 'Views' });
    expect(within(rail).getAllByRole('button').map((b) => b.getAttribute('aria-label'))).toEqual(['Project manager', 'Sessions', 'Inbox', 'Tickets', 'Notes', 'Documents', 'Workflows', 'Personas', 'Files', 'Settings', 'Expand side rail']);
    for (const name of ['Board', 'Search']) {
      expect(within(rail).queryByRole('button', { name })).not.toBeInTheDocument();
    }
    await userEvent.click(within(rail).getByRole('button', { name: 'Files' }));
    expect(within(rail).getByRole('button', { name: 'Files' })).toHaveAttribute('aria-current', 'page');
    expect(screen.queryByText(/arrives with/)).not.toBeInTheDocument();
  });

  it('switches the theme in one click from the rail, keeping the choice for the next visit', async () => {
    localStorage.removeItem('pdt.theme');
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    const { unmount } = renderWithQuery(<AppShell />);
    const rail = screen.getByRole('navigation', { name: 'Views' });
    const theme = within(rail).getByRole('radiogroup', { name: 'Theme' });
    expect(within(theme).getByRole('radio', { name: 'System' })).toBeChecked();
    await userEvent.click(within(theme).getByRole('radio', { name: 'Dark' }));
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    // The expanded rail keeps it, in a row.
    await userEvent.click(within(rail).getByRole('button', { name: 'Expand side rail' }));
    expect(within(rail).getByRole('radio', { name: 'Dark' })).toBeChecked();
    unmount();
    renderWithQuery(<AppShell />);
    expect(screen.getByRole('radio', { name: 'Dark' })).toBeChecked();
    await userEvent.click(screen.getByRole('radio', { name: 'System' }));
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    expect(localStorage.getItem('pdt.theme')).toBe('system');
    localStorage.removeItem('pdt.rail.expanded');
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
    // This host's first section, a screen of its own.
    expect(await screen.findByRole('heading', { name: 'Updates' })).toBeInTheDocument();
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    expect(within(nav).getByRole('button', { name: 'This host' })).toHaveAttribute('aria-expanded', 'true');
    expect(within(nav).getByRole('button', { name: 'Updates' })).toHaveAttribute('aria-current', 'page');
    await userEvent.click(within(nav).getByRole('button', { name: 'Remote access' }));
    expect(window.location.pathname).toBe('/settings/host/remote-access');
    expect(await screen.findByRole('switch', { name: 'Remote access' })).toBeChecked();
    expect(screen.queryByRole('heading', { name: 'Updates' })).not.toBeInTheDocument();
  });

  it('finds settings by name from the search box', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    await userEvent.type(within(nav).getByRole('searchbox', { name: 'Search settings' }), 'allowed hosts');
    expect(within(nav).getByRole('button', { name: 'Network' })).toBeInTheDocument();
    expect(within(nav).queryByRole('button', { name: 'Updates' })).toBeNull();
    expect(within(nav).queryByRole('button', { name: 'Secrets' })).toBeNull();
    await userEvent.clear(within(nav).getByRole('searchbox', { name: 'Search settings' }));
    await userEvent.type(within(nav).getByRole('searchbox', { name: 'Search settings' }), 'nothing like it');
    expect(within(nav).getByText(/No settings match/)).toBeInTheDocument();
  });

  it('marks Network with "restart" while a saved network setting waits for a restart', async () => {
    const view = hostRoutes['GET /api/host/settings'];
    stubApi({
      'GET /api/projects': [], ...hostRoutes,
      'GET /api/host/settings': { ...view, network: { settings: { port: 7500, allowedHosts: [] }, portFromEnv: null, allowedHostsFromEnv: null, listenPort: 7420, defaultPort: 7420, restartNeeded: true, canRestart: true } },
    });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    expect(await within(nav).findByRole('button', { name: 'Network restart' })).toBeInTheDocument();
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


  it('has the right panel beside the main area, opened by the strip and closed by its button or the prefix key', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes, 'GET /api/todos': [] });
    renderWithQuery(<AppShell />);
    const panel = screen.getByRole('complementary', { name: 'Right panel' });
    await userEvent.click(within(panel).getByRole('button', { name: 'Open the todo list' }));
    expect(await within(panel).findByRole('textbox', { name: 'Add a todo' })).toBeInTheDocument();
    await userEvent.click(within(panel).getByRole('button', { name: 'Collapse the right panel' }));
    expect(within(panel).queryByRole('textbox')).not.toBeInTheDocument();
    await userEvent.keyboard('{Control>}]{/Control}t');
    expect(await within(screen.getByRole('complementary', { name: 'Right panel' })).findByRole('textbox', { name: 'Add a todo' })).toBeInTheDocument();
  });

  it('shows the empty state when the project workspace has no tabs', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    expect(await within(list).findByRole('row', { name: /Alpha/ })).toHaveAttribute('data-current', 'true');
    expect(screen.getByRole('main')).toHaveTextContent('No tabs here.');
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
    expect(within(nav).getByRole('button', { name: /This host/ })).toBeInTheDocument();
    expect(screen.queryByRole('radio', { name: 'Clone URL' })).not.toBeInTheDocument();
    await userEvent.click(within(nav).getByRole('button', { name: 'Add project' }));
    const dialog = screen.getByRole('dialog', { name: 'Add project' });
    expect(within(dialog).getByRole('radio', { name: 'Clone URL' })).toBeInTheDocument();
  });

  it('has Plugins and All connections beside This host, and a plugin opens its page (PDT-228)', async () => {
    const here = () => window.location.pathname;
    stubWorkspaces({ 'GET /api/connections': [], 'GET /api/integrations': plugins });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    expect(within(nav).queryByRole('button', { name: 'Integrations' })).toBeNull();
    await userEvent.click(within(nav).getByRole('button', { name: 'Plugins' }));
    const main = screen.getByRole('main');
    expect(here()).toBe('/settings/plugins');
    await userEvent.click(await within(main).findByRole('button', { name: 'Discord' }));
    expect(here()).toBe('/settings/plugins/discord');
    expect(await within(main).findByRole('region', { name: 'Connections' })).toBeInTheDocument();
    await userEvent.click(within(nav).getByRole('button', { name: 'All connections' }));
    expect(here()).toBe('/settings/connections');
    expect(await within(main).findByRole('heading', { name: 'All connections' })).toBeInTheDocument();
  });


  it('lays a project page out in sections', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    await userEvent.click(await within(nav).findByRole('button', { name: /Alpha/ }));
    const group = within(nav).getByRole('group', { name: 'Alpha' });
    expect(within(group).getAllByRole('button').map((b) => b.textContent))
      .toEqual(['General', 'Worktrees', 'Runs on this host', 'Agent instructions', 'Agents', 'Review', 'Pipeline and policies', 'Plugins', 'Documents', 'Danger zone']);
    // One section per screen, under a breadcrumb.
    await userEvent.click(within(group).getByRole('button', { name: 'Review' }));
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('heading', { name: 'Review' })).toBeInTheDocument();
    expect(within(main).getAllByRole('region')).toHaveLength(1);
    expect(within(main).getByRole('navigation', { name: 'Breadcrumb' })).toHaveTextContent('Projects › Alpha');
  });
});

describe('Get started (PDT-230)', () => {
  const here = () => window.location.pathname + window.location.search;
  const setupView = (opensOnLaunch: boolean) => ({
    state: { needed: opensOnLaunch, dismissed: false, finished: false, githubSkipped: false, defaultAgentConfirmed: false },
    steps: { agents: 'done', github: 'todo', project: 'todo', defaultAgent: 'todo' },
    done: 1, opensOnLaunch, defaultAgent: 'claude',
  });
  afterEach(() => cleanup());

  it('a new install opens on Get started, beside the projects', async () => {
    window.history.replaceState(null, '', '/');
    stubApi({ 'GET /api/projects': [], ...hostRoutes, 'GET /api/host/setup': setupView(true) });
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/setup'));
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('heading', { name: 'Get started' })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page');
    // The sidebar's Add a project opens the add-project dialog.
    expect(screen.getByText('No projects yet.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Add a project' }));
    expect(await screen.findByRole('dialog', { name: 'Add project' })).toBeInTheDocument();
  });

  it('an install already set up opens where it always did, and its empty pane links to Get started', async () => {
    window.history.replaceState(null, '', '/');
    stubApi({ 'GET /api/projects': [], ...hostRoutes, 'GET /api/host/setup': setupView(false) });
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/sessions'));
    await userEvent.click(await screen.findByRole('link', { name: 'Add a project to get started.' }));
    expect(here()).toBe('/setup');
  });

  it('Settings › This host › Setup reopens it', async () => {
    window.history.replaceState(null, '', '/settings/host/setup');
    stubApi({ 'GET /api/projects': [], ...hostRoutes, 'GET /api/host/setup': setupView(false) });
    renderWithQuery(<AppShell />);
    const setup = await within(screen.getByRole('main')).findByRole('region', { name: 'Setup' });
    expect(await within(setup).findByText('1 of 4 steps done.')).toBeInTheDocument();
    await userEvent.click(within(setup).getByRole('button', { name: 'Open Get started' }));
    await waitFor(() => expect(here()).toBe('/setup'));
  });

  it("shows that the host can't be reached on desktop, and the Inbox says it can't load", async () => {
    window.history.replaceState(null, '', '/inbox');
    stubWorkspaces({
      'GET /api/workspaces': new Reply(502, { code: 'unreachable', message: 'down' }),
      'GET /api/inbox': new Reply(502, { code: 'unreachable', message: 'down' }),
    });
    renderWithQuery(<AppShell />);
    expect((await screen.findByText(/Can't reach the host/)).closest('[role=alert]')).toBeInTheDocument();
    expect(await screen.findAllByText(/Can't load the Inbox/)).not.toHaveLength(0);
    expect(screen.queryByText(/Nothing waits on you/)).toBeNull();
  });

  it("keeps saying the Inbox can't load while it refetches, when only the Inbox fails", async () => {
    window.history.replaceState(null, '', '/inbox');
    let inboxCalls = 0;
    const calls = stubWorkspaces({
      // The first round fails; the refetch after it hangs, so the query sits pending with no data.
      'GET /api/inbox': () => (++inboxCalls === 1 ? new Reply(500, { code: 'internal', message: 'boom' }) : new Promise(() => {})),
      'GET /api/notifications': () => new Reply(500, { code: 'internal', message: 'boom' }),
    });
    const { qc } = renderWithQuery(<AppShell />);
    // The list column and the work area.
    await waitFor(() => expect(screen.getAllByText(/Can't load the Inbox/)).toHaveLength(2));
    expect(await screen.findByText("Can't load updates.")).toBeInTheDocument();
    await act(() => { void qc.invalidateQueries({ queryKey: ['inbox'] }); });
    await waitFor(() => expect(calls.filter((c) => c.path.startsWith('/api/inbox')).length).toBeGreaterThan(1));
    expect(screen.getAllByText(/Can't load the Inbox/)).toHaveLength(2);
    expect(screen.queryByText(/Nothing waits on you/)).toBeNull();
    expect(screen.queryByText(/Can't reach the host/)).toBeNull();
  });

  it('shows no connection banner while the host answers, and one while the browser is offline', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    expect(screen.queryByText(/Can't reach the host/)).toBeNull();
    // Offline, queries pause rather than fail: the browser's own state shows it.
    act(() => onlineManager.setOnline(false));
    try {
      expect(await screen.findByText(/Can't reach the host/)).toBeInTheDocument();
    } finally {
      act(() => onlineManager.setOnline(true));
    }
    await waitFor(() => expect(screen.queryByText(/Can't reach the host/)).toBeNull());
  });
});
