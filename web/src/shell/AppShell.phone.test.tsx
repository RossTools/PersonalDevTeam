import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { applyThemePref } from '@ui';
import { AppShell } from './AppShell';
import { project, shell, hostRoutes, health, stubPhone, stubAlpha, sess, tabOf, stubWorkspaces } from './fixtures';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

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
    expect((await within(bar).findByText('DEV')).closest('[data-slot=badge]')).toHaveAttribute('title', 'Development instance on port 8512');
    await userEvent.click(within(bar).getByRole('button', { name: 'Search' }));
    expect(await screen.findByRole('dialog', { name: 'Command palette' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    await userEvent.click(await within(bar).findByRole('button', { name: 'Needs you (3)' }));
    expect(here()).toBe('/inbox');
  });

  it('opens the todo list in a sheet from the top bar, with no right panel', async () => {
    stubPhone();
    stubWorkspaces({ 'GET /api/todos': [{ id: 'a', title: 'Call Sam', position: 0, done: false, projectId: null, ticketId: null,
      createdAt: 1, updatedAt: 1, doneAt: null, projectKey: null, ticketKey: null, ticketTitle: null, body: '' }] });
    renderWithQuery(<AppShell />);
    expect(screen.queryByRole('complementary', { name: 'Right panel' })).not.toBeInTheDocument();
    await userEvent.click(within(screen.getByRole('banner', { name: 'Top bar' })).getByRole('button', { name: 'Todos' }));
    const sheet = await screen.findByRole('dialog', { name: 'Todos' });
    expect(await within(sheet).findByText('Call Sam')).toBeInTheDocument();
    expect(within(sheet).getByRole('textbox', { name: 'Add a todo' })).toBeInTheDocument();
  });

  it('lists the notes as the Notes screen; a tap opens one with ←, its actions under ⋯', async () => {
    stubPhone();
    const darkMode = { id: 'n1', projectId: null, kind: 'user', title: 'Dark mode', body: 'Follow the OS.', pinned: false, sessionId: null, createdAt: 1, updatedAt: 1 };
    const calls = stubApi({
      ...hostRoutes,
      'GET /api/projects': [project],
      [`GET /api/projects/${project.id}/worktrees`]: [],
      [`GET /api/projects/${project.id}/sessions`]: [shell],
      [`GET /api/projects/${project.id}/runs`]: [],
      'GET /api/workspaces': [{ id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] }],
      'GET /api/pm/notes': [darkMode],
      'GET /api/pm/notes/n1': darkMode,
      'PATCH /api/pm/notes/n1': { ...darkMode, pinned: true },
    });
    window.history.replaceState(null, '', '/notes');
    renderWithQuery(<AppShell />);
    const bar = () => screen.getByRole('banner', { name: 'Top bar' });
    expect(within(bar()).getByRole('heading', { name: 'Notes' })).toBeInTheDocument();
    await userEvent.click(await screen.findByRole('row', { name: /Dark mode/ }));
    expect(here()).toBe('/notes/n1');
    expect(await screen.findByRole('heading', { name: 'Dark mode', level: 1 })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Make a ticket' })).not.toBeInTheDocument();
    await userEvent.click(within(bar()).getByRole('button', { name: 'Screen actions' }));
    expect(screen.getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['Project: Host-wide', 'Pin (listed first)', 'Make a ticket', 'Delete…']);
    await userEvent.click(screen.getByRole('menuitem', { name: 'Pin (listed first)' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ pinned: true }));
    await userEvent.click(within(bar()).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/notes'));
  });

  it("lists the views in the drawer in the rail's order, with the Inbox count, Settings and the host's state", async () => {
    stubPhone();
    stubWorkspaces({ 'GET /api/inbox': { items: [], total: 3 } });
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    const d = await openDrawer();
    const nav = within(d).getByRole('navigation', { name: 'Views' });
    await waitFor(() => expect(within(nav).getAllByRole('button').map((b) => b.getAttribute('aria-label')))
      .toEqual(['Project manager', 'Sessions', 'Inbox (3)', 'Tickets', 'Notes', 'Documents', 'Workflows', 'Personas', 'Files']));
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

  it("switches the theme from the drawer's footer, keeping the choice", async () => {
    localStorage.removeItem('pdt.theme');
    stubPhone();
    stubWorkspaces();
    const { unmount } = renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    let d = await openDrawer();
    await userEvent.click(within(within(d).getByRole('radiogroup', { name: 'Theme' })).getByRole('radio', { name: 'Light' }));
    expect(document.documentElement).toHaveAttribute('data-theme', 'light');
    // The drawer stays open: the theme is not a place to go.
    expect(screen.getByRole('dialog', { name: 'Menu' })).toBeInTheDocument();
    unmount();
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    d = await openDrawer();
    expect(within(d).getByRole('radio', { name: 'Light' })).toBeChecked();
    applyThemePref('system');
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
    // 56px on touch screens too: the row's 44px coarse-pointer minimum is replaced, not left to win in its media query.
    const row = await within(main).findByRole('row', { name: /Alpha/ });
    expect(row).toHaveClass('min-h-14', 'pointer-coarse:min-h-14');
    expect(row).not.toHaveClass('pointer-coarse:min-h-control-touch');
    expect(within(within(main).getByRole('region', { name: 'Agents' })).getAllByRole('row')[0]).toHaveClass('pointer-coarse:min-h-14');
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
    window.history.replaceState(null, '', '/settings/projects/ALP/general');
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    const topBar = screen.getByRole('banner', { name: 'Top bar' });
    // A section is a screen of its own, titled by it.
    expect(await within(main).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
    expect(within(main).queryByRole('region', { name: 'Danger zone' })).not.toBeInTheDocument();
    expect(topBar).toHaveTextContent('General');
    // ← walks out: the project's sections, then the Settings list.
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(here()).toBe('/settings/projects/ALP');
    const sections = await within(main).findByRole('grid', { name: 'Sections' });
    expect(within(sections).getAllByRole('row').map((r) => r.textContent)).toEqual([
      'General', 'Worktrees', 'Runs on this host', 'Branch names', 'Agent instructions', 'Agents', 'Review', 'Pipeline and policies', 'Plugins', 'Documents', 'Danger zone',
    ]);
    expect(topBar).toHaveTextContent('Alpha');
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    expect(here()).toBe('/settings');
    const list = await within(main).findByRole('navigation', { name: 'Settings sections' });
    expect(within(list).getAllByRole('button').map((r) => r.textContent)).toEqual(expect.arrayContaining(['This host', 'Plugins', 'All connections', 'Notifications', 'Add project']));
    // A section in the list is a screen of its own, and Back comes out of it.
    await userEvent.click(within(list).getByRole('button', { name: /Alpha/ }));
    await userEvent.click(within(within(list).getByRole('group', { name: 'Alpha' })).getByRole('button', { name: 'Danger zone' }));
    expect(here()).toBe('/settings/projects/ALP/danger-zone');
    expect(await within(main).findByRole('button', { name: 'Remove project…' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/settings'));
  });

  it('shows each of this host\'s sections on its own screen on a phone', async () => {
    stubPhone();
    stubWorkspaces({});
    window.history.replaceState(null, '', '/settings/host');
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    const sections = await within(main).findByRole('grid', { name: 'Sections' });
    expect(within(sections).getAllByRole('row').map((r) => r.textContent)).toEqual([
      'Updates', 'Agents', 'PM', 'Network', 'Startup', 'Remote access', 'Runs', 'Storage', 'Projects folder', 'Documents', 'Appearance', 'Bug reports', 'Checks', 'Setup', 'Danger zone',
    ]);
    await userEvent.click(within(sections).getByRole('row', { name: /Runs/ }));
    expect(here()).toBe('/settings/host/runs');
    expect(screen.getByRole('banner', { name: 'Top bar' })).toHaveTextContent('Runs');
    const runs = await within(main).findByRole('region', { name: 'Runs' });
    expect(within(main).queryByRole('region', { name: 'Updates' })).not.toBeInTheDocument();
    // It saves as it changes: no Save bar.
    expect(runs.querySelector('[data-slot="section-footer"]')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/settings/host'));
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

  it('triages from Review all: the item screen counts its place, ⋯ has its actions, and Back returns to the list', async () => {
    const q = {
      id: 'q', kind: 'question', group: 'question', status: 'open', projectId: project.id, ticketId: null, ticketKey: null,
      ticketTitle: null, attemptId: null, sessionId: 'sess', runId: null, stepId: null, personaId: null, personaName: null,
      title: 'Which file?', body: { questions: [{ question: 'Which file?', choices: ['a.txt'] }] }, choices: [], resolution: null,
      createdAt: 1, resolvedAt: null,
    };
    stubPhone();
    stubWorkspaces({ 'GET /api/inbox': { items: [q], total: 1 }, 'POST /api/inbox/q/answer': {}, 'POST /api/notifications/read-all': undefined });
    window.history.replaceState(null, '', '/inbox');
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Review all (1)' }));
    expect(here()).toBe('/inbox/q');
    const top = screen.getByRole('banner', { name: 'Top bar' });
    expect(await within(top).findByText('Question · 1 of 1')).toBeInTheDocument();
    await userEvent.click(within(top).getByRole('button', { name: 'Screen actions' }));
    expect(screen.getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['Open session', 'Dismiss']);
    await userEvent.keyboard('{Escape}');
    await userEvent.click(await screen.findByRole('radio', { name: 'a.txt' }));
    await userEvent.click(within(screen.getByRole('group', { name: 'Decision' })).getByRole('button', { name: 'Send' }));
    expect(await screen.findByText('All clear')).toBeInTheDocument();
    await userEvent.click(within(top).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/inbox'));
  });

  it("has the PM's New conversation and ended conversations under ⋯ (the notes have a view of their own)", async () => {
    stubPhone();
    stubAlpha();
    window.history.replaceState(null, '', '/pm');
    renderWithQuery(<AppShell />);
    await userEvent.click(within(screen.getByRole('banner', { name: 'Top bar' })).getByRole('button', { name: 'Screen actions' }));
    expect(screen.getAllByRole('menuitem').map((m) => m.textContent)).toEqual(['New conversation', 'Ended conversations']);
  });

  it('lists the PM conversations at /pm and opens each as a screen of its own (PDT-149)', async () => {
    stubPhone();
    const pmSession = (id: string, title: string, more: Record<string, unknown>) => ({ ...shell, id, projectId: 'host', kind: 'agent_tui', agent: 'claude', title, mode: 'conversation', createdAt: Date.UTC(2026, 9, 20, 9), persona: { id: 'pm', name: 'PM', version: 1, stepId: null }, ...more });
    const old = pmSession('pm1', 'Phone plans', { status: 'finished' });
    const cb = pmSession('pm2', 'CB cutover', { status: 'running' });
    stubApi({
      ...hostRoutes,
      'GET /api/projects': [project],
      [`GET /api/projects/${project.id}/worktrees`]: [],
      [`GET /api/projects/${project.id}/sessions`]: [shell],
      [`GET /api/projects/${project.id}/runs`]: [],
      'GET /api/workspaces': [{ id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] }],
      'GET /api/pm': { current: cb, sessions: [
        { session: cb, open: true, title: 'CB cutover', unseen: false, lastActivity: 1, rotation: 'none', reason: null, journal: null },
        { session: old, open: false, title: 'Phone plans', unseen: false, lastActivity: 1, rotation: 'done', reason: 'age', journal: '## Planned the phone' },
      ] },
      'GET /api/agents/claude/capabilities': { conversation: true, permissionModes: [], models: [], commands: [] },
    });
    vi.stubGlobal('WebSocket', class { readyState = 0; onopen = null; onmessage = null; onclose = null; close() {} });
    window.history.replaceState(null, '', '/pm');
    renderWithQuery(<AppShell />);
    const bar = () => screen.getByRole('banner', { name: 'Top bar' });
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    await userEvent.click(await within(open).findByText('CB cutover'));
    expect(here()).toBe('/pm/pm2');
    expect(within(bar()).getByRole('heading', { name: 'CB cutover' })).toBeInTheDocument();
    // The conversation adds Rename, Reset… and Close… once it shows, which can come after the title: look again until it has.
    await waitFor(async () => {
      await userEvent.click(within(bar()).getByRole('button', { name: 'Screen actions' }));
      const items = screen.getAllByRole('menuitem').map((m) => m.textContent);
      await userEvent.keyboard('{Escape}');
      expect(items).toEqual(['New conversation', 'Ended conversations', 'Rename', 'Open a document beside…', 'Reset…', 'Close…']);
    });
    await userEvent.click(within(bar()).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/pm'));
    const ended = await screen.findByRole('grid', { name: 'Ended PM conversations' });
    expect(ended).toHaveTextContent('Planned the phone');
    await userEvent.click(within(ended).getByText('Phone plans'));
    expect(here()).toBe('/pm/pm1');
    expect(await screen.findByText(/This conversation ended because it was a day old\./)).toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: 'Message' })).not.toBeInTheDocument();
    await userEvent.click(within(bar()).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/pm'));
    // A new conversation: the box, not focused (the keyboard would cover the screen).
    await userEvent.click(within(bar()).getByRole('button', { name: 'Screen actions' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'New conversation' }));
    expect(here()).toBe('/pm/new');
    expect(within(bar()).getByRole('heading', { name: 'New conversation' })).toBeInTheDocument();
    expect(await screen.findByRole('textbox', { name: 'Ask the PM' })).not.toHaveFocus();
  });

  describe("the PM's unread reply (PDT-145)", () => {
    const pmRoutes = (session: Record<string, unknown>) => {
      const pmSession: Record<string, unknown> = { ...shell, id: 'pm1', projectId: 'host', kind: 'agent_tui', agent: 'claude', title: 'PM', mode: 'conversation', persona: { id: 'pm', name: 'PM', version: 1, stepId: null }, ...session };
      const state = { current: pmSession, sessions: [{ session: pmSession, open: true, title: 'PM', unseen: true, lastActivity: 1, rotation: 'none', reason: null, journal: null }] };
      return {
        ...hostRoutes,
        'GET /api/projects': [],
        'GET /api/pm': () => state,
        'POST /api/sessions/pm1/view': () => { pmSession.unseen = false; pmSession.lastViewedAt = 9; },
        'GET /api/agents/claude/capabilities': { conversation: true, permissionModes: [], models: [], commands: [] },
      };
    };
    const rail = () => screen.getByRole('navigation', { name: 'Views' });

    it('dots the PM item when its turn ended unseen, and opening the PM clears it', async () => {
      const calls = stubApi(pmRoutes({ activity: 'idle', attentionAt: 5, unseen: true }));
      vi.stubGlobal('WebSocket', class { readyState = 0; onopen = null; onmessage = null; onclose = null; close() {} });
      renderWithQuery(<AppShell />);
      const item = await within(rail()).findByRole('button', { name: 'Project manager (new reply)' });
      await userEvent.click(item);
      expect(within(rail()).getByRole('button', { name: 'Project manager' })).toHaveAttribute('aria-current', 'page');
      await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/sessions/pm1/view', body: undefined }));
      // Seen now: no dot after leaving the PM either.
      await userEvent.click(within(rail()).getByRole('button', { name: 'Sessions' }));
      await waitFor(() => expect(within(rail()).getByRole('button', { name: 'Project manager' })).toBeInTheDocument());
    });

    it('shows no dot while the PM runs a turn', async () => {
      const calls = stubApi(pmRoutes({ activity: 'working', attentionAt: 5, unseen: true }));
      renderWithQuery(<AppShell />);
      await waitFor(() => expect(calls.some((c) => c.path === '/api/pm')).toBe(true));
      await act(() => new Promise((r) => setTimeout(r, 50)));
      expect(within(rail()).getByRole('button', { name: 'Project manager' })).toBeInTheDocument();
    });
  });
});
