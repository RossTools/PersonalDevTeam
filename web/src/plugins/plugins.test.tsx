import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import type { Connection } from '../api/gen/Connection';
import { plugins as built } from '../api/gen/plugins';
import { AllConnectionsPage } from './AllConnectionsPage';
import { NO_FILTER, categoryCounts, enabledWith, filterPlugins, groupPlugins, loadLayout, offWith, stuckOn, type Plugin } from './model';
import { PluginPage } from './PluginPage';
import { PluginsPage } from './PluginsPage';

const conn = (name: string, kind: string, over: Partial<Connection> = {}): Connection => ({
  id: `c-${name}`, name, kind, url: '', config: {}, hasSecret: true, createdAt: 0, updatedAt: 0, lastError: null, lastErrorAt: null, ...over,
});

/** This build's plugins (all on, none set up), with some changed. */
function catalog(over: Record<string, Partial<Plugin>> = {}): Plugin[] {
  return built.map((p) => ({ ...p, ...over[p.id] }));
}

const off = { enabled: false, status: 'off' } as const;

/** A catalog like the mockup's: Discord on, Scryer failing, Sonarr and Radarr needing setup, Plex not in this build. */
const mixed = () => catalog({
  discord: { status: 'on' },
  jira: off,
  buildkite: off,
  gitlab: off,
  google: off,
  scryer: { status: 'error', error: 'scryer: connection refused', errorConnection: 'c-scryer', checkable: true },
  plex: { enabled: false, built: false, status: 'not_built' },
});

const labels = (ps: Plugin[]) => ps.map((p) => p.id);

describe('plugins: the catalog’s filters', () => {
  it('searches names and descriptions, and the tabs pick on and needing attention', () => {
    const ps = mixed();
    expect(labels(filterPlugins(ps, { ...NO_FILTER, q: 'DISCORD' }))).toEqual(['discord']);
    expect(labels(filterPlugins(ps, { ...NO_FILTER, q: 'download queues' }))).toEqual(['arr']);
    expect(labels(filterPlugins(ps, { ...NO_FILTER, tab: 'on' }))).toEqual(['webhook', 'github', 'http', 'mcp', 'discord', 'arr', 'scryer']);
    expect(labels(filterPlugins(ps, { ...NO_FILTER, tab: 'attention' }))).toEqual(['arr', 'scryer']);
  });

  it('counts each category after the search and tab, and groups in the catalog’s order', () => {
    const ps = mixed();
    expect(categoryCounts(ps, NO_FILTER)).toEqual({ all: 12, dev: 4, documents: 1, chat: 1, media: 3, general: 3 });
    expect(categoryCounts(ps, { ...NO_FILTER, tab: 'attention' })).toEqual({ all: 2, dev: 0, documents: 0, chat: 0, media: 2, general: 0 });
    expect(labels(filterPlugins(ps, { ...NO_FILTER, category: 'media' }))).toEqual(['arr', 'scryer', 'plex']);
    expect(groupPlugins(filterPlugins(ps, { ...NO_FILTER, q: 'workflows' })).map(([c]) => c)).toEqual(['dev', 'chat', 'media', 'general']);
  });

  it('enables the optional plugins that are on, changed by one; PDT’s own are never sent', () => {
    expect(enabledWith(mixed(), 'jira', true)).toEqual(['discord', 'jira', 'arr', 'scryer']);
    expect(enabledWith(mixed(), 'discord', false)).toEqual(['arr', 'scryer']);
  });

  it('turns off only PDT’s own that can be, and one a project uses as its forge stays on (PDT-251)', () => {
    expect(offWith(mixed(), 'github', false)).toEqual(['github']);
    expect(offWith(mixed(), 'discord', false)).toEqual([]);
    expect(offWith(catalog({ github: off }), 'jira', true)).toEqual(['github']);
    expect(offWith(catalog({ github: off }), 'github', true)).toEqual([]);
    const used = catalog({ github: { forgeProjects: [{ id: 'p1', key: 'PDT', name: 'Personal Dev Team' }] } }).find((p) => p.id === 'github')!;
    expect(stuckOn(used)).toMatch(/^GitHub is the forge of Personal Dev Team\. To turn it off, give it another forge/);
    expect(stuckOn({ ...used, enabled: false })).toBeNull();
  });
});

describe('plugins: Settings → Plugins', () => {
  beforeEach(() => localStorage.clear());
  afterEach(() => vi.unstubAllGlobals());

  it('shows each plugin’s status, groups them, and one not built in can’t be turned on', async () => {
    stubApi({ 'GET /api/integrations': mixed() });
    renderWithQuery(<PluginsPage onOpen={() => {}} />);
    const card = async (name: string) => screen.findByRole('group', { name });
    expect(within(await card('Discord')).getByText('On')).toBeInTheDocument();
    expect(within(await card('Jira')).getByText('Off')).toBeInTheDocument();
    expect(within(await card('Sonarr and Radarr')).getByText('Needs setup')).toBeInTheDocument();
    expect(within(await card('Scryer')).getByText('Error')).toBeInTheDocument();
    expect(within(await card('Plex')).getByText('Not in this build')).toBeInTheDocument();
    expect(within(await card('Plex')).getByRole('switch', { name: 'Use Plex' })).toBeDisabled();
    // PDT's own have no switch, but for GitHub (PDT-251).
    expect(within(await card('MCP servers')).getByText('Built in')).toBeInTheDocument();
    expect(within(await card('MCP servers')).queryByRole('switch')).toBeNull();
    expect(within(await card('GitHub')).getByRole('switch', { name: 'Use GitHub' })).toBeEnabled();
    expect(screen.getByRole('region', { name: 'Media' })).toHaveTextContent('Scryer');
    expect(document.body).toHaveTextContent('Later: an Available section for plugins installed from outside PDT.');
  });

  it('filters by search, tab and category chip', async () => {
    stubApi({ 'GET /api/integrations': mixed() });
    renderWithQuery(<PluginsPage onOpen={() => {}} />);
    await screen.findByRole('group', { name: 'Discord' });
    await userEvent.click(screen.getByRole('radio', { name: 'Needs attention · 2' }));
    expect(screen.queryByRole('group', { name: 'Discord' })).toBeNull();
    expect(screen.getByRole('group', { name: 'Scryer' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'All' }));
    const chips = screen.getByRole('group', { name: 'Categories' });
    expect(within(chips).getByRole('button', { name: 'Chat 1' })).toBeInTheDocument();
    await userEvent.click(within(chips).getByRole('button', { name: 'Dev 4' }));
    expect(screen.getByRole('group', { name: 'Jira' })).toBeInTheDocument();
    expect(screen.queryByRole('group', { name: 'Discord' })).toBeNull();
    await userEvent.click(within(chips).getByRole('button', { name: 'All 12' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Search plugins' }), 'nothing like this');
    expect(screen.getByText('No plugins match.')).toBeInTheDocument();
  });

  it('remembers Cards or List on this device', async () => {
    stubApi({ 'GET /api/integrations': mixed() });
    const { unmount } = renderWithQuery(<PluginsPage onOpen={() => {}} />);
    await screen.findByRole('group', { name: 'Discord' });
    expect(loadLayout()).toBe('cards');
    await userEvent.click(screen.getByRole('radio', { name: 'List' }));
    expect(loadLayout()).toBe('list');
    unmount();
    renderWithQuery(<PluginsPage onOpen={() => {}} />);
    await screen.findByRole('group', { name: 'Discord' });
    expect(screen.getByRole('radio', { name: 'List' })).toBeChecked();
  });

  it('opens a plugin’s page from its card, but the switch only switches', async () => {
    const calls = stubApi({ 'GET /api/integrations': mixed(), 'PUT /api/integrations': mixed() });
    const onOpen = vi.fn();
    renderWithQuery(<PluginsPage onOpen={onOpen} />);
    await userEvent.click(await screen.findByRole('link', { name: 'Discord' }));
    expect(onOpen).toHaveBeenCalledTimes(1);
    expect(onOpen).toHaveBeenCalledWith('discord');
    onOpen.mockClear();
    await userEvent.click(screen.getByRole('switch', { name: 'Use Discord' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ enabled: ['arr', 'scryer'], off: [] }));
    expect(onOpen).not.toHaveBeenCalled();
  });

  it('turning on one that needs a connection asks for one; Later leaves it needing setup', async () => {
    const before = mixed();
    const after = catalog({ discord: { status: 'on' }, buildkite: off, gitlab: off, scryer: { status: 'on' }, plex: { built: false, enabled: false, status: 'not_built' } });
    const calls = stubApi({
      'GET /api/integrations': () => (calls.some((c) => c.method === 'PUT') ? after : before),
      'PUT /api/integrations': after,
      'GET /api/connections': [],
    });
    renderWithQuery(<PluginsPage onOpen={() => {}} />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Use Jira' }));
    const dialog = await screen.findByRole('dialog', { name: 'Add Jira' });
    expect(within(dialog).getByText(/Jira is on\. Add a connection/)).toBeInTheDocument();
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ enabled: ['discord', 'jira', 'arr', 'scryer'], off: [] });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Later' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(within(screen.getByRole('group', { name: 'Jira' })).getByText('Needs setup')).toBeInTheDocument();
  });

  it('turning on one that has its connection already asks for nothing', async () => {
    const after = catalog({ discord: { status: 'on' }, jira: { status: 'on' }, buildkite: off });
    stubApi({ 'GET /api/integrations': mixed(), 'PUT /api/integrations': after });
    renderWithQuery(<PluginsPage onOpen={() => {}} />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Use Jira' }));
    await waitFor(() => expect(screen.getByRole('switch', { name: 'Use Jira' })).toBeChecked());
    expect(screen.queryByRole('dialog')).toBeNull();
  });
});

describe('plugins: a plugin’s page', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('shows what it adds, how to set it up, and the projects linked to it', async () => {
    stubApi({
      'GET /api/integrations': catalog({ jira: { status: 'on', projects: [{ id: 'p1', key: 'PDT', name: 'Personal Dev Team' }] } }),
      'GET /api/connections': [conn('work', 'jira', { url: 'https://work.atlassian.net' })],
    });
    renderWithQuery(<PluginPage id="jira" onBack={() => {}} />);
    expect(await screen.findByRole('heading', { name: /Jira/ })).toBeInTheDocument();
    const adds = screen.getByRole('region', { name: 'What it adds' });
    expect(within(adds).getByText('Comment on a Jira issue')).toBeInTheDocument();
    expect(within(adds).getByRole('link', { name: 'Personal Dev Team' })).toHaveAttribute('href', '/settings/projects/PDT/plugins');
    const connections = screen.getByRole('region', { name: 'Connections' });
    expect(await within(connections).findByText('work')).toBeInTheDocument();
    expect(within(connections).getByText('https://work.atlassian.net')).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Setup' })).toHaveTextContent('Turn Jira on.');
  });

  it('asks for a connection while it needs setup', async () => {
    stubApi({ 'GET /api/integrations': catalog(), 'GET /api/connections': [] });
    renderWithQuery(<PluginPage id="jira" onBack={() => {}} />);
    expect(await screen.findByText(/Jira is on but has no connection yet/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Add a connection' }));
    expect(await screen.findByRole('dialog', { name: 'Add Jira' })).toBeInTheDocument();
  });

  it('shows what is failing, and tests again or edits the connection', async () => {
    const failing = conn('scryer', 'scryer', { url: 'http://nas:8080', lastError: 'connection refused', lastErrorAt: Date.now() });
    const calls = stubApi({
      'GET /api/integrations': mixed(),
      'GET /api/connections': [failing],
      'POST /api/integrations/scryer/check': catalog({ scryer: { status: 'on' } }),
    });
    renderWithQuery(<PluginPage id="scryer" onBack={() => {}} />);
    expect(await screen.findByText('scryer: connection refused')).toBeInTheDocument();
    await userEvent.click(await screen.findByRole('button', { name: 'Edit connection' }));
    expect(await screen.findByRole('dialog', { name: 'Edit scryer' })).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Cancel' }));
    await userEvent.click(screen.getByRole('button', { name: 'Test again' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/integrations/scryer/check')).toBe(true));
    await waitFor(() => expect(screen.queryByText('scryer: connection refused')).toBeNull());
  });

  it('says when this build lacks it, and that a turned-off one keeps its connections', async () => {
    stubApi({ 'GET /api/integrations': mixed(), 'GET /api/connections': [conn('ci', 'buildkite')] });
    const { unmount } = renderWithQuery(<PluginPage id="plex" onBack={() => {}} />);
    expect(await screen.findByText(/This PDT was built without Plex\. It can’t be turned on here\./)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Use Plex' })).toBeDisabled();
    unmount();
    renderWithQuery(<PluginPage id="buildkite" onBack={() => {}} />);
    const connections = await screen.findByRole('region', { name: 'Connections' });
    expect(connections).toHaveTextContent('Turn Buildkite on to add connections');
    expect(within(connections).queryByRole('button', { name: 'Add' })).toBeNull();
    expect(await within(connections).findByText('ci')).toBeInTheDocument();
  });

  it('a plugin that declares a sign-in shows it; with PDT’s own client it adds accounts instead of Add (PDT-378)', async () => {
    const google = built.find((p) => p.id === 'google')!;
    expect(google.signIn?.api).toBe('/api/google');
    const signIn = { ...google.signIn!, builtIn: true };
    stubApi({
      'GET /api/integrations': catalog({ google: { signIn } }),
      'GET /api/connections': [],
      'GET /api/google/accounts': [],
    });
    const { unmount } = renderWithQuery(<PluginPage id="google" onBack={() => {}} />);
    expect(await screen.findByRole('button', { name: 'Sign in with Google' })).toBeInTheDocument();
    expect(within(screen.getByRole('region', { name: 'Connections' })).queryByRole('button', { name: 'Add' })).toBeNull();
    unmount();
    stubApi({
      'GET /api/integrations': catalog({ google: { signIn: { ...signIn, builtIn: false } } }),
      'GET /api/connections': [],
      'GET /api/google/accounts': [],
    });
    renderWithQuery(<PluginPage id="google" onBack={() => {}} />);
    expect(await screen.findByText('Add a connection for your Google account first.')).toBeInTheDocument();
    expect(within(screen.getByRole('region', { name: 'Connections' })).getByRole('button', { name: 'Add' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Sign in with Google' })).toBeNull();
  });

  it('GitHub’s connection is its login', async () => {
    stubApi({ 'GET /api/integrations': catalog({ github: { status: 'needs_setup' } }), 'GET /api/connections': [] });
    renderWithQuery(<PluginPage id="github" onBack={() => {}} />);
    expect(await screen.findByText(/PDT needs your GitHub login/)).toBeInTheDocument();
    expect((await screen.findByRole('region', { name: 'GitHub logins' }))).toBeInTheDocument();
  });

  it('GitLab’s connection is its login too, and it stays on while a project’s forge is GitLab (PDT-246)', async () => {
    const gitlab = catalog({ gitlab: { status: 'needs_setup', forgeProjects: [{ id: 'p2', key: 'APP', name: 'App' }] } });
    stubApi({ 'GET /api/integrations': gitlab, 'GET /api/connections': [] });
    renderWithQuery(<PluginPage id="gitlab" onBack={() => {}} />);
    expect(await screen.findByText(/PDT needs your GitLab login and has none: connect GitLab below/)).toBeInTheDocument();
    expect(await screen.findByRole('region', { name: 'GitLab logins' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Connections' })).toBeNull();
    expect(screen.getByText(/GitLab is the forge of App/)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Use GitLab' })).toBeDisabled();
  });

  it('GitHub stays on while a project uses it as its forge, and turns off when none does (PDT-251)', async () => {
    const used = catalog({ github: { forgeProjects: [{ id: 'p1', key: 'PDT', name: 'Personal Dev Team' }] } });
    stubApi({ 'GET /api/integrations': used, 'GET /api/connections': [] });
    const { unmount } = renderWithQuery(<PluginPage id="github" onBack={() => {}} />);
    expect(await screen.findByText(/GitHub is the forge of Personal Dev Team/)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Use GitHub' })).toBeDisabled();
    unmount();
    vi.unstubAllGlobals();
    const calls = stubApi({ 'GET /api/integrations': catalog(), 'PUT /api/integrations': catalog({ github: off }), 'GET /api/connections': [] });
    renderWithQuery(<PluginPage id="github" onBack={() => {}} />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Use GitHub' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ enabled: enabledWith(catalog(), 'github', false), off: ['github'] }));
    expect(await screen.findByText(/GitHub is off: PDT doesn’t use it for pull requests, CI or issues/)).toBeInTheDocument();
  });
});

describe('plugins: All connections', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('lists every connection with its plugin; search narrows it, and a row opens its plugin', async () => {
    stubApi({
      'GET /api/integrations': mixed(),
      'GET /api/connections': [conn('alerts', 'discord_webhook'), conn('media', 'scryer', { url: 'http://nas:8080', lastError: 'refused' })],
    });
    const onOpen = vi.fn();
    renderWithQuery(<AllConnectionsPage onOpen={onOpen} />);
    const row = (name: string) => screen.getByRole('row', { name: new RegExp(`^${name}`) });
    expect(await screen.findByRole('row', { name: /^alerts Discord/ })).toHaveTextContent('OK');
    expect(row('media')).toHaveTextContent('Scryer');
    expect(row('media')).toHaveTextContent('Failing');
    expect(row('GitHub login')).toHaveTextContent('me');
    await userEvent.type(screen.getByRole('textbox', { name: 'Search connections' }), 'nas');
    expect(screen.queryByRole('row', { name: /^alerts/ })).toBeNull();
    await userEvent.click(within(row('media')).getByRole('button', { name: 'media' }));
    expect(onOpen).toHaveBeenCalledWith('scryer');
  });
});
