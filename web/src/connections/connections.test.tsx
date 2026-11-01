import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import type { Connection } from '../api/gen/Connection';
import { ParamsForm } from '../workflows/NewWorkflowDialog';
import { plugins } from '../api/gen/plugins';
import { PluginPage } from '../plugins/PluginPage';

const conn = (name: string, kind: string, hasSecret = true): Connection => ({
  id: `c-${name}`, name, kind, url: '', config: {}, hasSecret, createdAt: 0, updatedAt: 0, lastError: null, lastErrorAt: null,
});

/** A plugin's page (PDT-228), where its connections are added and edited; every plugin of this build is on. */
const page = (id: string) => <PluginPage id={id} onBack={() => {}} />;

/** Stubs the API, with this build's plugins (`GET /api/integrations`) unless a test stubs them. */
const stub = (routes: Parameters<typeof stubApi>[0]) => stubApi({ 'GET /api/integrations': plugins, ...routes });

describe('connections', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('lists a plugin’s connections, with Add for each of its kinds (PDT-171)', async () => {
    stub({ 'GET /api/connections': [conn('d', 'jira'), conn('alerts', 'discord_webhook')] });
    renderWithQuery(page('discord'));
    const section = await screen.findByRole('region', { name: 'Connections' });
    expect(await within(section).findByText('alerts')).toBeTruthy();
    expect(within(section).queryByText('d')).toBeNull();
    expect(within(section).getByRole('button', { name: 'Add Discord webhook' })).toBeTruthy();
    expect(within(section).getByRole('button', { name: 'Add Discord bot' })).toBeTruthy();
  });

  it('shows why a sensor cannot use a connection, and since when', async () => {
    const failing = { ...conn('work', 'jira'), lastError: 'Jira answered 401', lastErrorAt: Date.now() - 5 * 60_000 };
    stub({ 'GET /api/connections': [failing, conn('ci', 'jira')] });
    renderWithQuery(page('jira'));
    expect(await screen.findByText('Failing since 5m ago: Jira answered 401')).toBeTruthy();
    expect(screen.getAllByText(/^Failing since/)).toHaveLength(1);
  });

  it('says how to get a Discord webhook, and saves one with its URL as the secret', async () => {
    const created = conn('alerts', 'discord_webhook', false);
    const calls = stub({
      'GET /api/connections': [],
      'POST /api/connections': created,
      'PUT /api/connections/c-alerts/secret': { ...created, hasSecret: true },
    });
    renderWithQuery(page('discord'));
    expect(await screen.findByText(/Integrations → Webhooks → New Webhook/)).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Add Discord webhook' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).getByText(/Copy Webhook URL/)).toBeTruthy();
    const save = within(dialog).getByRole('button', { name: 'Save' });
    await userEvent.type(within(dialog).getByLabelText('Name'), 'alerts');
    expect(save.hasAttribute('disabled') || save.getAttribute('aria-disabled') === 'true').toBe(true);
    await userEvent.type(within(dialog).getByLabelText('Webhook URL'), 'https://discord.com/api/webhooks/1/abc');
    await userEvent.click(save);
    await waitFor(() => expect(calls.some((c) => c.method === 'PUT')).toBe(true));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ name: 'alerts', kind: 'discord_webhook', url: '', config: {} });
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ secret: 'https://discord.com/api/webhooks/1/abc' });
  });

  it('recovers when the secret could not be saved: the row shows, and Save again updates it', async () => {
    const created = conn('alerts', 'discord_webhook', false);
    let secretFails = true;
    const calls = stub({
      'GET /api/connections': () => (calls.some((c) => c.method === 'POST') ? [created] : []),
      'POST /api/connections': created,
      'PATCH /api/connections/c-alerts': created,
      'PUT /api/connections/c-alerts/secret': () => {
        if (secretFails) { secretFails = false; return new Reply(500, { code: 'keychain', message: 'keychain locked' }); }
        return { ...created, hasSecret: true };
      },
    });
    renderWithQuery(page('discord'));
    await userEvent.click(await screen.findByRole('button', { name: 'Add Discord webhook' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.type(within(dialog).getByLabelText('Name'), 'alerts');
    await userEvent.type(within(dialog).getByLabelText('Webhook URL'), 'https://discord.com/api/webhooks/1/abc');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    expect(await within(dialog).findByText('alerts was saved, but not its secret: keychain locked')).toBeTruthy();
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(calls.filter((c) => c.method === 'POST' && c.path === '/api/connections')).toHaveLength(1);
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2);
  });

  it('sets up a Discord bot: its server, an invite link, and whether the PM acts without asking', async () => {
    const created = conn('home', 'discord_bot', false);
    const calls = stub({
      'GET /api/connections': [],
      'POST /api/connections': created,
      'PUT /api/connections/c-home/secret': { ...created, hasSecret: true },
    });
    renderWithQuery(page('discord'));
    await userEvent.click(await screen.findByRole('button', { name: 'Add Discord bot' }));
    const dialog = await screen.findByRole('dialog');
    const save = within(dialog).getByRole('button', { name: 'Save' });
    await userEvent.type(within(dialog).getByLabelText('Name'), 'home');
    await userEvent.type(within(dialog).getByLabelText('Bot token'), 'tok');
    await userEvent.type(within(dialog).getByLabelText('Server ID'), 'my server');
    expect(save.hasAttribute('disabled') || save.getAttribute('aria-disabled') === 'true').toBe(true);
    await userEvent.clear(within(dialog).getByLabelText('Server ID'));
    await userEvent.type(within(dialog).getByLabelText('Server ID'), '42');
    await userEvent.type(within(dialog).getByLabelText('Application ID'), '7');
    const invite = within(dialog).getByRole('link', { name: /oauth2/ });
    expect(invite.getAttribute('href')).toBe('https://discord.com/oauth2/authorize?client_id=7&scope=bot&permissions=536874000');
    await userEvent.click(within(dialog).getByRole('switch', { name: /without asking/ }));
    await userEvent.click(save);
    await waitFor(() => expect(calls.some((c) => c.method === 'PUT')).toBe(true));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      name: 'home', kind: 'discord_bot', url: '', config: { guildId: '42', applicationId: '7', pmActs: true },
    });
  });

  it('lets a Discord bot pass its user\'s DMs to the PM, and shows whether they are connected', async () => {
    const bot = { ...conn('home', 'discord_bot'), config: { guildId: '42', applicationId: '7', dmUserId: '' } };
    const calls = stub({
      'GET /api/connections': [bot],
      'PATCH /api/connections/c-home': { ...bot, config: { ...bot.config, dmUserId: '77', dmInbox: true } },
      'GET /api/connections/c-home/status': null,
    });
    renderWithQuery(page('discord'));
    expect(await screen.findByText('home')).toBeTruthy();
    expect(screen.queryByText(/DMs with the PM/)).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).queryByRole('switch', { name: /Inbox items/ })).toBeNull();
    await userEvent.type(within(dialog).getByLabelText(/Your Discord user ID/), 'me');
    const save = within(dialog).getByRole('button', { name: 'Save' });
    expect(save.hasAttribute('disabled') || save.getAttribute('aria-disabled') === 'true').toBe(true);
    await userEvent.clear(within(dialog).getByLabelText(/Your Discord user ID/));
    await userEvent.type(within(dialog).getByLabelText(/Your Discord user ID/), '77');
    await userEvent.click(within(dialog).getByRole('switch', { name: /Inbox items/ }));
    await userEvent.click(save);
    await waitFor(() => expect(calls.some((c) => c.method === 'PATCH')).toBe(true));
    expect(calls.find((c) => c.method === 'PATCH')?.body).toMatchObject({
      config: { guildId: '42', applicationId: '7', dmUserId: '77', dmInbox: true },
    });
  });

  it('shows why a Discord bot\'s DMs are not connected', async () => {
    const bot = { ...conn('home', 'discord_bot'), config: { guildId: '42', dmUserId: '77' } };
    stub({
      'GET /api/connections': [bot],
      'GET /api/connections/c-home/status': { label: 'DMs with the PM', state: 'Not connected', tone: 'failed', error: 'Discord refused the Message Content intent' },
    });
    renderWithQuery(page('discord'));
    const line = await screen.findByText(/DMs with the PM/);
    await waitFor(() => expect(line.textContent).toBe('DMs with the PM: Not connected: Discord refused the Message Content intent'));
  });

  it('sets up Scryer: its address, an API key, and whether the PM acts without asking', async () => {
    const created = { ...conn('media', 'scryer', false), url: 'http://nas:8080' };
    const calls = stub({
      'GET /api/connections': [],
      'POST /api/connections': created,
      'PUT /api/connections/c-media/secret': { ...created, hasSecret: true },
    });
    renderWithQuery(page('scryer'));
    await userEvent.click(await screen.findByRole('button', { name: 'Add' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.type(within(dialog).getByLabelText('Name'), 'media');
    await userEvent.type(within(dialog).getByLabelText('Address'), 'http://nas:8080');
    await userEvent.type(within(dialog).getByLabelText('API key'), 'ska_abc');
    await userEvent.click(within(dialog).getByRole('switch', { name: /changes settings without asking/ }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PUT')).toBe(true));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      name: 'media', kind: 'scryer', url: 'http://nas:8080', config: { pmActs: true },
    });
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ secret: 'ska_abc' });
  });

  it('keeps what PDT saved in a connection when it is edited', async () => {
    const made = { ...conn('briefs', 'discord_webhook'), config: { webhookId: '9', channel: 'ai-briefs' } };
    const calls = stub({ 'GET /api/connections': [made], 'PATCH /api/connections/c-briefs': made });
    renderWithQuery(page('discord'));
    expect(await screen.findByText('#ai-briefs')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    await userEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PATCH')).toBe(true));
    expect(calls.find((c) => c.method === 'PATCH')?.body).toMatchObject({ config: { webhookId: '9', channel: 'ai-briefs' } });
  });

  it('shows an incoming webhook its address, and what is missing a secret', async () => {
    stub({ 'GET /api/connections': [conn('deploys', 'webhook', false), conn('alerts', 'discord_webhook')] });
    renderWithQuery(page('webhook'));
    expect(await screen.findByText('deploys')).toBeTruthy();
    expect(screen.getByText(/\/api\/hooks\/deploys$/)).toBeTruthy();
    expect(screen.getAllByText('No secret')).toHaveLength(1);
    // Only its plugin's connections.
    expect(screen.queryByText('alerts')).toBeNull();
  });

  it('adds an MCP server that PDT starts, with its arguments and environment, and needs no secret', async () => {
    const created = conn('notes', 'mcp', false);
    const calls = stub({ 'GET /api/connections': [], 'POST /api/connections': created });
    renderWithQuery(page('mcp'));
    await userEvent.click(await screen.findByRole('button', { name: 'Add' }));
    const dialog = await screen.findByRole('dialog');
    expect(within(dialog).queryByLabelText('Address')).toBeNull();
    await userEvent.type(within(dialog).getByLabelText('Name'), 'notes');
    await userEvent.type(within(dialog).getByLabelText('Program'), 'npx');
    await userEvent.type(within(dialog).getByLabelText('Arguments'), '-y{Enter}notes-server{Enter}/tmp/notes');
    await userEvent.type(within(dialog).getByLabelText('Environment'), 'LOG=warn{Enter}A=b=c');
    await userEvent.type(within(dialog).getByLabelText('Secret’s variable'), 'NOTES_TOKEN');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST')).toBe(true));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      name: 'notes', kind: 'mcp', url: '',
      config: {
        transport: 'stdio', command: 'npx', args: ['-y', 'notes-server', '/tmp/notes'], env: { LOG: 'warn', A: 'b=c' }, secretEnv: 'NOTES_TOKEN',
        // A workflow's server: agents get it once the user ticks Agents in Settings › MCP servers.
        agents: { enabled: false },
      },
    });
    expect(calls.some((c) => c.method === 'PUT')).toBe(false);
  });

  it("shows an MCP server's tools, or why they could not be listed, and lists them again", async () => {
    const tools = (names: string[], error: string | null) => ({
      server: 'notes', error, listedAt: 1, triedAt: 1,
      tools: names.map((name) => ({ name, title: null, description: '', inputSchema: {}, outputSchema: null })),
    });
    const calls = stub({
      'GET /api/connections': [conn('notes', 'mcp', false)],
      'GET /api/connections/c-notes/tools': tools(['search', 'read'], 'notes: listing its tools took too long'),
      'POST /api/connections/c-notes/tools': tools(['search', 'read', 'write'], null),
    });
    renderWithQuery(page('mcp'));
    expect(await screen.findByText('2 tools: search, read')).toBeTruthy();
    expect(screen.getByText(/took too long/)).toBeTruthy();
    expect(screen.queryByText('No secret')).toBeNull();
    await userEvent.click(screen.getByRole('button', { name: 'List again' }));
    expect(await screen.findByText('3 tools: search, read, write')).toBeTruthy();
    expect(calls.some((c) => c.method === 'POST' && c.path.endsWith('/tools'))).toBe(true);
  });

  it("a template's connection param offers the connections of its kind", async () => {
    stubApi({ 'GET /api/connections': [conn('alerts', 'discord_webhook'), conn('deploys', 'webhook')] });
    const onChange = vi.fn();
    renderWithQuery(
      <ParamsForm
        params={[{ id: 'discord', title: 'Discord channel', description: '', type: 'connection', connectionKind: 'discord_webhook', required: true }]}
        values={{ discord: '' }}
        onChange={onChange}
      />,
    );
    await userEvent.click(await screen.findByRole('button', { name: /Discord channel/ }));
    const options = within(screen.getByRole('listbox')).getAllByRole('option').map((o) => o.textContent);
    expect(options).toEqual(['alerts']);
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'alerts' }));
    expect(onChange).toHaveBeenCalledWith({ discord: 'alerts' });
  });
});
