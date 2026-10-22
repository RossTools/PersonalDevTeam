import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { HostSettingsPage } from './HostSettingsPage';
import { UpdateBanner } from './UpdateBanner';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const settingsView = (over: object = {}) => ({
  settings: {
    remoteAccess: false, ownerLogin: null, updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 },
    maxConcurrentRuns: 4, agentCredentials: 'subscription', projectsDir: null, defaultAgent: 'claude',
    defaultSessionMode: 'conversation', ticketPermissionMode: 'bypass', sessionPermissionMode: 'ask', updateCheckMinutes: 60,
  },
  ownerFromEnv: null,
  tokenPresent: false,
  tokenError: null,
  remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: 'me@example.com' },
  ...over,
});
const update = (over: object = {}) => ({
  current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
  error: null, token: { state: 'missing' }, lastResult: null, ...over,
});
const tool = (ok: boolean) => ({ found: true, ok, detail: ok ? 'ok' : 'not logged in' });
const checks = (ok: boolean) => ({ claude: tool(ok), codex: tool(ok), devin: { found: false, ok: false, detail: 'not found' }, git: tool(true), tailscale: tool(true), checkedAt: 1 });
const local = { caller: 'local', login: null };
const release = { version: '0.1.7', tag: 'v0.1.7', notes: 'Fixes', publishedAt: null };

describe('HostSettingsPage', () => {
  it('sets the owner from this device, then turns on remote access', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Use me@example.com' }));
    await userEvent.click(screen.getByRole('switch', { name: 'Remote access' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ ownerLogin: 'me@example.com' }, { remoteAccess: true }]);
  });

  it('offers the tailnet login of the device in use as the owner', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': { caller: 'tailnet', login: 'phone@example.com' },
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByRole('button', { name: 'Use phone@example.com' })).toBeInTheDocument();
  });

  it('shows an owner set by the environment as read-only', async () => {
    stubApi({
      'GET /api/host/settings': settingsView({ ownerFromEnv: 'env@example.com' }),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    const owner = await screen.findByLabelText('Owner login');
    expect(owner).toBeDisabled();
    expect(owner).toHaveValue('env@example.com');
    expect(screen.queryByRole('button', { name: 'Save owner' })).not.toBeInTheDocument();
  });

  it('shows why remote access could not be turned on', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': new Reply(400, { code: 'owner_required', message: 'set the owner first' }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Remote access' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('set the owner first');
  });

  it('stores a token without keeping it on screen', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/token': update({ token: { state: 'valid', expiresAt: null } }),
    });
    renderWithQuery(<HostSettingsPage />);
    const field = await screen.findByLabelText('GitHub token');
    expect(field).toHaveAttribute('type', 'password');
    await userEvent.type(field, 'github_pat_abc');
    await userEvent.click(screen.getByRole('button', { name: 'Save token' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/host/token')?.body).toEqual({ token: 'github_pat_abc' }));
    await waitFor(() => expect(field).toHaveValue(''));
  });

  it('loads with an unreadable keychain and says what to do', async () => {
    stubApi({
      'GET /api/host/settings': settingsView({ tokenError: 'the OS keychain failed: it did not answer in time' }),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText(/Token: cannot be read/)).toBeInTheDocument();
    expect(screen.getByText(/Always Allow/)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Remote access' })).toBeInTheDocument();
  });

  it('saves the update policy at once and says so', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('button', { name: /Update policy/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Manual (no checks)' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ updatePolicy: 'manual' }));
    expect(await screen.findByRole('status')).toHaveTextContent('Saved');
  });

  it('saves the update check interval, which is off while updates are manual', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('button', { name: /Update check interval/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Every 6 hours' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ updateCheckMinutes: 360 }));
    await userEvent.click(screen.getByRole('button', { name: /Update policy/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Manual (no checks)' }));
    expect(await screen.findByText('Off while updates are manual.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: /Update check interval/ })).toBeDisabled();
  });

  it('saves the default agent and view', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('button', { name: /Default agent/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Codex' }));
    await userEvent.click(screen.getByRole('button', { name: /Default view/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Terminal' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ defaultAgent: 'codex' }, { defaultSessionMode: 'terminal' }]));
  });

  it('saves the permission modes for tickets and new sessions', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByRole('button', { name: /Ticket permissions/ })).toHaveTextContent('Bypass (dangerous)');
    expect(screen.getByRole('button', { name: /Session permissions/ })).toHaveTextContent('Ask');
    await userEvent.click(screen.getByRole('button', { name: /Ticket permissions/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Accept edits' }));
    await userEvent.click(screen.getByRole('button', { name: /Session permissions/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Plan' }));
    await waitFor(() =>
      expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ ticketPermissionMode: 'accept_edits' }, { sessionPermissionMode: 'plan' }]),
    );
  });

  it('saves ports and max runs with one Save and shows why it failed', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': new Reply(400, { code: 'invalid_ports', message: 'the port range is empty' }),
    });
    renderWithQuery(<HostSettingsPage />);
    const runs = await screen.findByRole('region', { name: 'Runs' });
    const save = within(runs).getByRole('button', { name: 'Save run settings' });
    expect(save).toBeDisabled();
    const last = within(runs).getByLabelText('Last run port');
    await userEvent.clear(last);
    await userEvent.type(last, '8300');
    const max = within(runs).getByRole('spinbutton', { name: 'Max concurrent runs' });
    await userEvent.clear(max);
    await userEvent.type(max, '6');
    await userEvent.click(save);
    expect(await within(runs).findByRole('alert')).toHaveTextContent('the port range is empty');
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ runPorts: { start: 8400, end: 8300 }, maxConcurrentRuns: 6 });
  });

  it('confirms before removing the token', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView({ tokenPresent: true }),
      'GET /api/host/update': update({ token: { state: 'valid', expiresAt: null } }),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'DELETE /api/host/token': undefined,
    });
    renderWithQuery(<HostSettingsPage />);
    // Only in the Danger zone, never next to Check for updates.
    const updates = await screen.findByRole('region', { name: 'Updates' });
    expect(within(updates).queryByRole('button', { name: /Remove/ })).toBeNull();
    const danger = screen.getByRole('region', { name: 'Danger zone' });
    await userEvent.click(within(danger).getByRole('button', { name: 'Remove token…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);
    await userEvent.click(within(danger).getByRole('button', { name: 'Remove token…' }));
    const ask = await screen.findByRole('alertdialog', { name: 'Remove the GitHub token?' });
    await userEvent.click(within(ask).getByRole('button', { name: 'Remove token' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/host/token')).toBe(true));
  });

  it('checks for updates and shows a newer release; no Danger zone without a token', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'POST /api/host/update/check': update({ state: 'available', latest: release, lastCheckedAt: 1 }),
    });
    renderWithQuery(<HostSettingsPage />);
    const updates = await screen.findByRole('region', { name: 'Updates' });
    await userEvent.click(within(updates).getByRole('button', { name: 'Check for updates' }));
    expect(await within(updates).findByText('v0.1.7 available')).toBeInTheDocument();
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/host/update/check')).toBe(true);
    expect(screen.queryByRole('region', { name: 'Danger zone' })).toBeNull();
  });

  it('shows why the settings failed to load instead of loading forever', async () => {
    stubApi({
      'GET /api/host/settings': new Reply(500, { code: 'internal', message: 'internal error' }),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Host settings could not be loaded: internal error');
  });

  it('reports the token state and why updates are off', async () => {
    stubApi({
      'GET /api/host/settings': settingsView({ tokenPresent: true }),
      'GET /api/host/update': update({ state: 'disabled', disabledReason: 'not supervised', token: { state: 'expired' } }),
      'GET /api/host/checks': { ...checks(true), claude: { found: false, ok: false, detail: 'program not found' } },
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText(/Updates are off: not supervised\./)).toBeInTheDocument();
    expect(screen.getByText('Expired or revoked')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Check for updates' })).not.toBeInTheDocument();
    const claude = screen.getByText('claude').closest('li')!;
    expect(claude).toHaveTextContent('not found');
    expect(within(claude).getByRole('img', { name: 'not found' })).toHaveAttribute('data-status', 'failed');
    expect(within(screen.getByText('git').closest('li')!).getByRole('img', { name: 'OK' })).toHaveAttribute('data-status', 'running');
    // Devin is optional: missing is not a failure.
    const devin = screen.getByText('devin').closest('li')!;
    expect(within(devin).getByRole('img', { name: 'not installed (optional)' })).toHaveAttribute('data-status', 'idle');
  });
});

describe('UpdateBanner', () => {
  it('stays one line and shows the release notes over the page, not in it', async () => {
    stubApi({ 'GET /api/host/update': update({ state: 'available', latest: { ...release, notes: 'Fixes things' } }) });
    renderWithQuery(<UpdateBanner />);
    const toggle = await screen.findByRole('button', { name: 'Release notes' });
    const strip = screen.getByText(/Update to v0.1.7 available/).parentElement!;
    expect(strip).toHaveClass('h-8', 'flex-nowrap');
    expect(screen.queryByText('Fixes things')).not.toBeInTheDocument();
    await userEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByRole('region', { name: 'Release notes' })).toHaveClass('absolute');
    expect(screen.getByText('Fixes things')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByText('Fixes things')).not.toBeInTheDocument();
  });

  it('confirms before updating a busy host', async () => {
    let n = 0;
    const calls = stubApi({
      'GET /api/host/update': update({ state: 'available', latest: release }),
      'POST /api/host/update/install': () =>
        ++n === 1 ? new Reply(409, { code: 'host_busy', message: 'Busy: Claude is working. Update anyway?' }) : update({ state: 'installing' }),
    });
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByText(/Update to v0.1.7 available/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Update' }));
    const ask = await screen.findByRole('alertdialog', { name: 'Update now?' });
    expect(ask).toHaveTextContent('Busy: Claude is working. Update anyway?');
    await userEvent.click(within(ask).getByRole('button', { name: 'Update anyway' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ force: false }, { force: true }]);
  });

  it('does not force the update when the confirmation is declined', async () => {
    const calls = stubApi({
      'GET /api/host/update': update({ state: 'available', latest: release }),
      'POST /api/host/update/install': new Reply(409, { code: 'host_busy', message: 'Busy: Claude is working. Update anyway?' }),
    });
    renderWithQuery(<UpdateBanner />);
    await userEvent.click(await screen.findByRole('button', { name: 'Update' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/host/update').length).toBeGreaterThan(1));
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('shows an install already in progress', async () => {
    stubApi({
      'GET /api/host/update': update({ state: 'available', latest: release }),
      'POST /api/host/update/install': new Reply(409, { code: 'update_in_progress', message: 'an update is already in progress' }),
    });
    renderWithQuery(<UpdateBanner />);
    await userEvent.click(await screen.findByRole('button', { name: 'Update' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('already in progress');
  });

  it.each([['downloading', 'Downloading the update'], ['installing', 'Installing the update']])('reports the %s state', async (state, text) => {
    stubApi({ 'GET /api/host/update': update({ state }) });
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByRole('status')).toHaveTextContent(text);
  });

  it('reloads the page once the host answers with a new version', async () => {
    const reload = vi.fn();
    vi.stubGlobal('location', { ...location, reload });
    let n = 0;
    stubApi({ 'GET /api/host/update': () => (++n === 1 ? update({ state: 'installing' }) : update({ current: '0.1.7' })) });
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByRole('status')).toHaveTextContent('Installing the update');
    await waitFor(() => expect(reload).toHaveBeenCalledTimes(1), { timeout: 4_000 });
  });

  it('does not reload while the version stays the same', async () => {
    const reload = vi.fn();
    vi.stubGlobal('location', { ...location, reload });
    let n = 0;
    const calls = stubApi({ 'GET /api/host/update': () => update({ state: ++n === 1 ? 'installing' : 'idle' }) });
    renderWithQuery(<UpdateBanner />);
    await waitFor(() => expect(calls.length).toBeGreaterThan(1), { timeout: 4_000 });
    expect(reload).not.toHaveBeenCalled();
  });

  it('reports a rollback with its log and can be dismissed', async () => {
    const calls = stubApi({
      'GET /api/host/update': update({
        lastResult: { outcome: 'rolled_back', fromVersion: '0.1.6', toVersion: '0.1.7', message: 'Update to v0.1.7 failed and was rolled back', log: '/d/logs/pdt-host.log', at: 1 },
      }),
      'POST /api/host/update/dismiss': undefined,
    });
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByRole('alert')).toHaveTextContent('failed and was rolled back');
    expect(screen.getByText('/d/logs/pdt-host.log')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/host/update/dismiss')).toBe(true));
  });

  it('reports a failed update as an alert and a successful one as a status', async () => {
    stubApi({
      'GET /api/host/update': update({
        lastResult: { outcome: 'failed', fromVersion: '0.1.6', toVersion: '0.1.7', message: 'Update to v0.1.7 failed', log: null, at: 1 },
      }),
    });
    const { unmount } = renderWithQuery(<UpdateBanner />);
    expect(await screen.findByRole('alert')).toHaveTextContent('Update to v0.1.7 failed');
    unmount();
    stubApi({
      'GET /api/host/update': update({
        lastResult: { outcome: 'installed', fromVersion: '0.1.6', toVersion: '0.1.7', message: 'Updated to v0.1.7', log: null, at: 1 },
      }),
    });
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByRole('status')).toHaveTextContent('Updated to v0.1.7');
  });
});
