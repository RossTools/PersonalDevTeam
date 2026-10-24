import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import type { Connection } from '../api/gen/Connection';
import { ParamsForm } from '../workflows/NewWorkflowDialog';
import { ConnectionsPage } from './ConnectionsPage';

const conn = (name: string, kind: string, hasSecret = true): Connection => ({
  id: `c-${name}`, name, kind, url: '', config: {}, hasSecret, createdAt: 0, updatedAt: 0, lastError: null, lastErrorAt: null,
});

describe('connections', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('shows why a sensor cannot use a connection, and since when', async () => {
    const failing = { ...conn('work', 'jira'), lastError: 'Jira answered 401', lastErrorAt: Date.now() - 5 * 60_000 };
    stubApi({ 'GET /api/connections': [failing, conn('ci', 'buildkite')] });
    renderWithQuery(<ConnectionsPage />);
    expect(await screen.findByText('Failing since 5m ago: Jira answered 401')).toBeTruthy();
    expect(screen.getAllByText(/^Failing since/)).toHaveLength(1);
  });

  it('says how to get a Discord webhook, and saves one with its URL as the secret', async () => {
    const created = conn('alerts', 'discord_webhook', false);
    const calls = stubApi({
      'GET /api/connections': [],
      'POST /api/connections': created,
      'PUT /api/connections/c-alerts/secret': { ...created, hasSecret: true },
    });
    renderWithQuery(<ConnectionsPage />);
    expect(await screen.findByText(/Integrations → Webhooks → New Webhook/)).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Add discord webhook' }));
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
    const calls = stubApi({
      'GET /api/connections': () => (calls.some((c) => c.method === 'POST') ? [created] : []),
      'POST /api/connections': created,
      'PATCH /api/connections/c-alerts': created,
      'PUT /api/connections/c-alerts/secret': () => {
        if (secretFails) { secretFails = false; return new Reply(500, { code: 'keychain', message: 'keychain locked' }); }
        return { ...created, hasSecret: true };
      },
    });
    renderWithQuery(<ConnectionsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add discord webhook' }));
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
    const calls = stubApi({
      'GET /api/connections': [],
      'POST /api/connections': created,
      'PUT /api/connections/c-home/secret': { ...created, hasSecret: true },
    });
    renderWithQuery(<ConnectionsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add discord bot' }));
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

  it('sets up Scryer: its address, an API key, and whether the PM adds without asking', async () => {
    const created = { ...conn('media', 'scryer', false), url: 'http://nas:8080' };
    const calls = stubApi({
      'GET /api/connections': [],
      'POST /api/connections': created,
      'PUT /api/connections/c-media/secret': { ...created, hasSecret: true },
    });
    renderWithQuery(<ConnectionsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add scryer' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.type(within(dialog).getByLabelText('Name'), 'media');
    await userEvent.type(within(dialog).getByLabelText('Address'), 'http://nas:8080');
    await userEvent.type(within(dialog).getByLabelText('API key'), 'ska_abc');
    await userEvent.click(within(dialog).getByRole('switch', { name: /adds to this library without asking/ }));
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PUT')).toBe(true));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      name: 'media', kind: 'scryer', url: 'http://nas:8080', config: { pmActs: true },
    });
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ secret: 'ska_abc' });
  });

  it('keeps what PDT saved in a connection when it is edited', async () => {
    const made = { ...conn('briefs', 'discord_webhook'), config: { webhookId: '9', channel: 'ai-briefs' } };
    const calls = stubApi({ 'GET /api/connections': [made], 'PATCH /api/connections/c-briefs': made });
    renderWithQuery(<ConnectionsPage />);
    expect(await screen.findByText('#ai-briefs')).toBeTruthy();
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    await userEvent.click(within(await screen.findByRole('dialog')).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'PATCH')).toBe(true));
    expect(calls.find((c) => c.method === 'PATCH')?.body).toMatchObject({ config: { webhookId: '9', channel: 'ai-briefs' } });
  });

  it('shows an incoming webhook its address, and lists what is missing a secret', async () => {
    stubApi({ 'GET /api/connections': [conn('deploys', 'webhook', false), conn('alerts', 'discord_webhook')] });
    renderWithQuery(<ConnectionsPage />);
    expect(await screen.findByText('deploys')).toBeTruthy();
    expect(screen.getByText(/\/api\/hooks\/deploys$/)).toBeTruthy();
    expect(screen.getAllByText('No secret')).toHaveLength(1);
    // Each in its kind's section.
    const section = (name: string) => screen.getByRole('region', { name });
    expect(within(section('Incoming webhooks')).getByText('deploys')).toBeTruthy();
    expect(within(section('Discord webhooks')).getByText('alerts')).toBeTruthy();
    expect(within(section('HTTP services')).getByText('None yet.')).toBeTruthy();
    expect(within(section('Discord bots')).getByText('None yet.')).toBeTruthy();
  });

  it('adds an MCP server that PDT starts, with its arguments and environment, and needs no secret', async () => {
    const created = conn('notes', 'mcp', false);
    const calls = stubApi({ 'GET /api/connections': [], 'POST /api/connections': created });
    renderWithQuery(<ConnectionsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add mcp server' }));
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
      config: { transport: 'stdio', command: 'npx', args: ['-y', 'notes-server', '/tmp/notes'], env: { LOG: 'warn', A: 'b=c' }, secretEnv: 'NOTES_TOKEN' },
    });
    expect(calls.some((c) => c.method === 'PUT')).toBe(false);
  });

  it("shows an MCP server's tools, or why they could not be listed, and lists them again", async () => {
    const tools = (names: string[], error: string | null) => ({
      server: 'notes', error, listedAt: 1, triedAt: 1,
      tools: names.map((name) => ({ name, title: null, description: '', inputSchema: {}, outputSchema: null })),
    });
    const calls = stubApi({
      'GET /api/connections': [conn('notes', 'mcp', false)],
      'GET /api/connections/c-notes/tools': tools(['search', 'read'], 'notes: listing its tools took too long'),
      'POST /api/connections/c-notes/tools': tools(['search', 'read', 'write'], null),
    });
    renderWithQuery(<ConnectionsPage />);
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
