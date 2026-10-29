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
    remoteAccess: false, ownerLogin: null, updatePolicy: 'notify', updateChannel: 'stable', runPorts: { start: 8400, end: 8499 },
    maxConcurrentRuns: 4, agentCredentials: 'subscription', projectsDir: null, defaultAgent: 'claude',
    defaultSessionMode: 'conversation', ticketPermissionMode: 'bypass', sessionPermissionMode: 'ask', updateCheckMinutes: 60,
    releaseRepo: null,
  },
  ownerFromEnv: null,
  tokenPresent: false,
  tokenError: null,
  remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: 'me@example.com' },
  ...over,
});
const update = (over: object = {}) => ({
  current: '0.1.6', currentExperimental: false, channel: 'stable', state: 'idle', disabledReason: null, policy: 'notify',
  latest: null, lastCheckedAt: null, error: null, token: { state: 'missing' }, lastResult: null,
  releaseRepo: { repo: 'RossTools/PersonalDevTeam', id: 1, movedTo: null, overriddenBy: null }, ...over,
});
const tool = (ok: boolean) => ({ found: true, ok, detail: ok ? 'ok' : 'not logged in' });
const checks = (ok: boolean) => ({ claude: tool(ok), codex: tool(ok), devin: { found: false, ok: false, detail: 'not found' }, git: tool(true), tailscale: tool(true), checkedAt: 1 });
const local = { caller: 'local', login: null };
const release = { version: '0.1.7', tag: 'v0.1.7', experimental: false, notes: 'Fixes', publishedAt: null, changes: null, releases: null };

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

  it('switches the update channel and checks again at once', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
      'POST /api/host/update/check': update({ channel: 'experimental', state: 'available', latest: { ...release, version: '0.1.7-exp.2', experimental: true } }),
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText('Full releases only.')).toBeInTheDocument();
    await userEvent.click(await screen.findByRole('button', { name: /Update channel/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Experimental builds too' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ updateChannel: 'experimental' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/host/update/check')).toBe(true));
    expect(await screen.findByText('v0.1.7-exp.2 available (experimental)')).toBeInTheDocument();
    expect(screen.getByText(/can carry database migrations no stable release has yet/)).toBeInTheDocument();
  });

  it('says an experimental build stays until a newer stable release when back on stable', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update({ current: '0.2.1-exp.3', currentExperimental: true }),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText('This host stays on the experimental build v0.2.1-exp.3 until a newer stable release exists.')).toBeInTheDocument();
    expect(screen.getByText('Experimental')).toBeInTheDocument();
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

  it('sets another release repository, and goes back to the built-in one', async () => {
    const calls = stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'PUT /api/host/settings': (c) => settingsView({ settings: { ...settingsView().settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage />);
    const input = await screen.findByRole('textbox', { name: 'Release repository' });
    await waitFor(() => expect(input).toHaveAttribute('placeholder', 'RossTools/PersonalDevTeam'));
    await userEvent.type(input, ' Acme/pdt ');
    await userEvent.click(screen.getByRole('button', { name: 'Save release repository' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ releaseRepo: 'Acme/pdt' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Use the built-in one' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toContainEqual({ releaseRepo: '' }));
  });

  it('shows a release repository set by the environment, or a move GitHub reported', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update({ releaseRepo: { repo: 'Acme/pdt', id: null, movedTo: null, overriddenBy: 'environment' } }),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    const { unmount } = renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText('Set by PDT_RELEASE_REPO: Acme/pdt.')).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Release repository' })).toBeDisabled();
    expect(screen.queryByRole('button', { name: 'Save release repository' })).not.toBeInTheDocument();
    unmount();
    vi.unstubAllGlobals();
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update({ releaseRepo: { repo: 'RossTools/PersonalDevTeam', id: 1, movedTo: 'repositories/1', overriddenBy: null } }),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
    });
    renderWithQuery(<HostSettingsPage />);
    expect(await screen.findByText(/GitHub says it moved \(to repositories\/1\)/)).toBeInTheDocument();
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
      'POST /api/host/update/check': { ...update({ state: 'available', latest: release, lastCheckedAt: 1 }), result: { kind: 'available', version: '0.1.7' } },
    });
    renderWithQuery(<HostSettingsPage />);
    const updates = await screen.findByRole('region', { name: 'Updates' });
    await userEvent.click(within(updates).getByRole('button', { name: 'Check for updates' }));
    expect(await within(updates).findByText('v0.1.7 available')).toBeInTheDocument();
    expect(within(updates).getByRole('status')).toHaveTextContent('v0.1.7 is available: install it from the banner');
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/host/update/check')).toBe(true);
    expect(screen.queryByRole('region', { name: 'Danger zone' })).toBeNull();
  });

  it('says when the host is already up to date', async () => {
    stubApi({
      'GET /api/host/settings': settingsView({ tokenPresent: true }),
      'GET /api/host/update': update({ token: { state: 'valid', expiresAt: null } }),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'POST /api/host/update/check': { ...update({ lastCheckedAt: 5, token: { state: 'valid', expiresAt: null } }), result: { kind: 'upToDate', version: '0.1.6' } },
    });
    renderWithQuery(<HostSettingsPage />);
    const updates = await screen.findByRole('region', { name: 'Updates' });
    await userEvent.click(within(updates).getByRole('button', { name: 'Check for updates' }));
    expect(await within(updates).findByRole('status')).toHaveTextContent("You're up to date (v0.1.6). Last checked");
  });

  it('says why it could not check, and points at the token field', async () => {
    stubApi({
      'GET /api/host/settings': settingsView(),
      'GET /api/host/update': update(),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'POST /api/host/update/check': { ...update(), result: { kind: 'cannotCheck', reason: 'noToken', message: "no GitHub token is saved, so PDT can't read its releases" } },
    });
    renderWithQuery(<HostSettingsPage />);
    const updates = await screen.findByRole('region', { name: 'Updates' });
    await userEvent.click(within(updates).getByRole('button', { name: 'Check for updates' }));
    expect(await within(updates).findByRole('status')).toHaveTextContent("Couldn't check: no GitHub token is saved");
    await userEvent.click(within(updates).getByRole('link', { name: 'Add a token' }));
    expect(screen.getByLabelText('GitHub token')).toHaveFocus();
  });

  it('says when GitHub could not be reached, with no token link', async () => {
    stubApi({
      'GET /api/host/settings': settingsView({ tokenPresent: true }),
      'GET /api/host/update': update({ token: { state: 'valid', expiresAt: null } }),
      'GET /api/host/checks': checks(true),
      'GET /api/host/whoami': local,
      'GET /api/connections': [],
      'POST /api/host/update/check': {
        ...update({ lastCheckedAt: 5, error: 'could not reach GitHub: timed out', token: { state: 'valid', expiresAt: null } }),
        result: { kind: 'cannotCheck', reason: 'network', message: 'could not reach GitHub: timed out' },
      },
    });
    renderWithQuery(<HostSettingsPage />);
    const updates = await screen.findByRole('region', { name: 'Updates' });
    await userEvent.click(within(updates).getByRole('button', { name: 'Check for updates' }));
    expect(await within(updates).findByRole('status')).toHaveTextContent("Couldn't check: could not reach GitHub: timed out.");
    expect(within(updates).queryByRole('link', { name: /token/ })).toBeNull();
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

  it('shows the notes of every release since the installed version, newest first', async () => {
    const rel = (version: string, notes: string) => ({ version, tag: `v${version}`, notes, publishedAt: null });
    const changes = { since: '0.1.6', total: 1, moreUrl: null, entries: [{ sha: 'a', subject: 'A commit', title: 'A commit', ticket: null, ticketHere: null, pr: null, prUrl: null }] };
    const releases = [rel('0.2.1', '### Fixed\n\n- The banner lists every release.'), rel('0.2.0', '')];
    stubApi({ 'GET /api/host/update': update({ current: '0.1.840', state: 'available', latest: { ...release, version: '0.2.1', changes, releases } }) });
    renderWithQuery(<UpdateBanner />);
    await userEvent.click(await screen.findByRole('button', { name: 'Release notes' }));
    const notes = screen.getByRole('region', { name: 'Release notes' });
    expect(within(notes).getByText('2 releases since v0.1.840')).toBeInTheDocument();
    const sections = within(notes).getAllByRole('region');
    expect(sections.map((r) => r.getAttribute('aria-label'))).toEqual(['v0.2.1', 'v0.2.0']);
    expect(within(sections[0]).getByRole('heading', { name: 'Fixed' })).toBeInTheDocument();
    expect(within(sections[0]).getByText('The banner lists every release.')).toBeInTheDocument();
    expect(within(sections[1]).getByText('No notes.')).toBeInTheDocument();
    // The release notes take the place of the commit list.
    expect(within(notes).queryByText('A commit')).toBeNull();
  });

  it('marks an experimental update and its notes', async () => {
    const releases = [{ version: '0.2.1-exp.2', tag: 'v0.2.1-exp.2', experimental: true, notes: 'Not yet released:\n\n- Things.', publishedAt: null }];
    stubApi({ 'GET /api/host/update': update({ current: '0.2.0', state: 'available', latest: { ...release, version: '0.2.1-exp.2', experimental: true, releases } }) });
    renderWithQuery(<UpdateBanner />);
    const strip = (await screen.findByText(/Update to v0.2.1-exp.2 available/)).parentElement!;
    expect(within(strip).getByText('Experimental')).toBeInTheDocument();
    expect(strip).toHaveClass('h-8', 'flex-nowrap');
    await userEvent.click(screen.getByRole('button', { name: 'Release notes' }));
    const section = within(screen.getByRole('region', { name: 'Release notes' })).getByRole('region', { name: 'v0.2.1-exp.2' });
    expect(within(section).getByText('Experimental')).toBeInTheDocument();
    expect(within(section).getByText('Things.')).toBeInTheDocument();
  });

  it('lists every change since the installed version, linking tickets here and pull requests', async () => {
    const entry = (sha: string, title: string, ticket: string | null, pr: number | null, ticketHere: object | null = null) => ({
      sha, subject: title, title, ticket, ticketHere, pr, prUrl: pr ? `https://github.com/o/r/pull/${pr}` : null,
    });
    const changes = {
      since: '0.1.741', total: 2, moreUrl: null,
      entries: [entry('a2', 'Second change', 'PDT-9', 11), entry('a1', 'First change', 'PDT-5', 10, { id: 't5', projectId: 'p' })],
    };
    stubApi({ 'GET /api/host/update': update({ state: 'available', latest: { ...release, changes } }) });
    renderWithQuery(<UpdateBanner />);
    await userEvent.click(await screen.findByRole('button', { name: 'Release notes' }));
    const notes = screen.getByRole('region', { name: 'Release notes' });
    expect(within(notes).getByText('2 changes since v0.1.741')).toBeInTheDocument();
    const items = within(notes).getAllByRole('listitem');
    expect(items.map((li) => li.textContent)).toEqual(['PDT-9 Second change #11', 'PDT-5 First change #10']);
    expect(within(items[0]).getByRole('link', { name: '#11' })).toHaveAttribute('href', 'https://github.com/o/r/pull/11');
    // Only a ticket this instance has is a link.
    expect(within(items[0]).queryByRole('link', { name: 'PDT-9' })).toBeNull();
    expect(within(items[1]).getByRole('link', { name: 'PDT-5' })).toBeInTheDocument();
    expect(within(notes).queryByText('Fixes')).toBeNull();
  });

  it('links to the rest of a range too long to list', async () => {
    const entries = [{ sha: 's1', subject: 'One', title: 'One', ticket: null, ticketHere: null, pr: null, prUrl: null }];
    const changes = { since: '0.1.1', total: 300, moreUrl: 'https://github.com/o/r/compare/v0.1.1...v0.1.7', entries };
    stubApi({ 'GET /api/host/update': update({ state: 'available', latest: { ...release, changes } }) });
    renderWithQuery(<UpdateBanner />);
    await userEvent.click(await screen.findByRole('button', { name: 'Release notes' }));
    expect(screen.getByText('300 changes since v0.1.1')).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /and 299 more/ })).toHaveAttribute('href', changes.moreUrl);
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
