import { screen, waitFor } from '@testing-library/react';
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
    maxConcurrentRuns: 4, agentCredentials: 'subscription',
  },
  ownerFromEnv: null,
  tokenPresent: false,
  remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: 'me@example.com' },
  ...over,
});
const update = (over: object = {}) => ({
  current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
  error: null, token: { state: 'missing' }, lastResult: null, ...over,
});
const tool = (ok: boolean) => ({ found: true, ok, detail: ok ? 'ok' : 'not logged in' });
const checks = (ok: boolean) => ({ claude: tool(ok), codex: tool(ok), git: tool(true), tailscale: tool(true), checkedAt: 1 });
const local = { caller: 'local', login: null };
const release = { version: '0.1.7', tag: 'v0.1.7', notes: 'Fixes', publishedAt: null };

describe('HostSettingsPage', () => {
  it('sets the owner from this device, then turns on remote access', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('button', { name: 'Use me@example.com' }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Remote access' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ ownerLogin: 'me@example.com' }, { remoteAccess: true }]);
  });

  it('offers the tailnet login of the device in use as the owner', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': { caller: 'tailnet', login: 'phone@example.com' },
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
      'PUT /api/host/settings': new Reply(400, { code: 'owner_required', message: 'set the owner first' }),
    });
    renderWithQuery(<HostSettingsPage />);
    await userEvent.click(await screen.findByRole('checkbox', { name: 'Remote access' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('set the owner first');
  });

  it('stores a token without keeping it on screen', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
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

  it('reports the token state and that updates apply to the installed app', async () => {
    stubApi({
      'GET /api/host/settings': settingsView({ tokenPresent: true }),
      'GET /api/host/update': update({ state: 'disabled', disabledReason: 'not supervised', token: { state: 'expired' } }),
      'GET /api/host/checks': { ...checks(true), claude: { found: false, ok: false, detail: 'program not found' } },
      'GET /api/host/whoami': local,
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText(/Updates apply to the installed app/)).toBeInTheDocument();
    expect(screen.getByText(/expired or revoked/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Check now' })).not.toBeInTheDocument();
    expect(screen.getByText(/claude: not found/)).toBeInTheDocument();
  });
});

describe('UpdateBanner', () => {
  it('confirms before updating a busy host', async () => {
    let n = 0;
    const calls = stubApi({
      'GET /api/host/update': update({ state: 'available', latest: release }),
      'POST /api/host/update/install': () =>
        ++n === 1 ? new Reply(409, { code: 'host_busy', message: 'Busy: Claude is working. Update anyway?' }) : update({ state: 'installing' }),
    });
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<UpdateBanner />);
    expect(await screen.findByText(/Update to v0.1.7 available/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Update' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    expect(confirm).toHaveBeenCalledWith('Busy: Claude is working. Update anyway?');
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ force: false }, { force: true }]);
  });

  it('does not force the update when the confirmation is declined', async () => {
    const calls = stubApi({
      'GET /api/host/update': update({ state: 'available', latest: release }),
      'POST /api/host/update/install': new Reply(409, { code: 'host_busy', message: 'Busy: Claude is working. Update anyway?' }),
    });
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderWithQuery(<UpdateBanner />);
    await userEvent.click(await screen.findByRole('button', { name: 'Update' }));
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
