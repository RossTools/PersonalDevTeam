import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { BugReportDialog } from './BugReportDialog';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const preview = (over: object = {}) => ({
  setting: 'auto', route: 'github', repo: 'o/r', project: null, tokenPresent: true,
  diagnostics: 'PDT version: 0.1.42\nLast 1 lines of the pdtd log:\nINFO ~/work started\n', ...over,
});

describe('BugReportDialog', () => {
  it('says it goes to GitHub, previews the diagnostics and links to the issue', async () => {
    const calls = stubApi({
      'GET /api/bug-reports/preview': preview(),
      'POST /api/bug-reports': { kind: 'issue', number: 7, url: 'https://github.com/o/r/issues/7' },
    });
    renderWithQuery(<BugReportDialog onClose={() => {}} onOpenSettings={() => {}} />);
    expect(await screen.findByText(/public issue on/)).toHaveTextContent('o/r');
    await userEvent.click(screen.getByRole('button', { name: 'Show what is sent' }));
    expect(screen.getByLabelText('Diagnostics')).toHaveTextContent('INFO ~/work started');
    const send = screen.getByRole('button', { name: 'Send report' });
    expect(send).toBeDisabled();
    await userEvent.type(screen.getByLabelText('Title'), 'Board freezes');
    await userEvent.type(screen.getByLabelText('What happened'), 'It stops.');
    await userEvent.type(screen.getByLabelText('What you expected'), 'It moves.');
    await userEvent.type(screen.getByLabelText('How to reproduce'), 'Drag a card.');
    await userEvent.click(send);
    expect(await screen.findByRole('link', { name: '#7' })).toHaveAttribute('href', 'https://github.com/o/r/issues/7');
    const body = calls.find((c) => c.method === 'POST')!.body as Record<string, unknown>;
    expect(body).toMatchObject({
      title: 'Board freezes', description: 'It stops.', expected: 'It moves.', steps: 'Drag a card.', includeDiagnostics: true,
    });
    expect(typeof body.nonce).toBe('string');
  });

  it('files a local ticket on the instance with the PDT project', async () => {
    const calls = stubApi({
      'GET /api/bug-reports/preview': preview({ route: 'local', project: { id: 'p', key: 'PDT', name: 'Personal Dev Team' } }),
      'POST /api/bug-reports': { kind: 'ticket', key: 'PDT-9', id: 't', projectId: 'p' },
    });
    renderWithQuery(<BugReportDialog onClose={() => {}} onOpenSettings={() => {}} />);
    expect(await screen.findByText(/Nothing is sent to GitHub/)).toHaveTextContent('PDT · Personal Dev Team');
    await userEvent.type(screen.getByLabelText('Title'), 'Crash');
    await userEvent.click(screen.getByRole('checkbox', { name: 'Include diagnostics' }));
    await userEvent.click(screen.getByRole('button', { name: 'File ticket' }));
    expect(await screen.findByRole('link', { name: 'PDT-9' })).toBeInTheDocument();
    expect(calls.find((c) => c.method === 'POST')!.body).toMatchObject({ includeDiagnostics: false });
  });

  it('points a token error at Host settings and keeps the nonce for the retry', async () => {
    const onOpenSettings = vi.fn();
    const calls = stubApi({
      'GET /api/bug-reports/preview': preview(),
      'POST /api/bug-reports': new Reply(409, { code: 'token_no_issue_access', message: 'The update token can’t create issues on o/r.' }),
    });
    renderWithQuery(<BugReportDialog onClose={() => {}} onOpenSettings={onOpenSettings} />);
    await userEvent.type(await screen.findByLabelText('Title'), 'x');
    await userEvent.click(screen.getByRole('button', { name: 'Send report' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('can’t create issues');
    await userEvent.click(screen.getByRole('button', { name: 'Host settings' }));
    expect(onOpenSettings).toHaveBeenCalled();
    await userEvent.click(screen.getByRole('button', { name: 'Send report' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    const [a, b] = calls.filter((c) => c.method === 'POST').map((c) => (c.body as { nonce: string }).nonce);
    expect(a).toBe(b);
  });

  it('cannot send while reports are off', async () => {
    stubApi({ 'GET /api/bug-reports/preview': preview({ route: 'off', setting: 'off' }) });
    renderWithQuery(<BugReportDialog onClose={() => {}} onOpenSettings={() => {}} />);
    expect(await screen.findByText(/turned off/)).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText('Title'), 'x');
    expect(screen.getByRole('button', { name: 'Send report' })).toBeDisabled();
  });
});

describe('Host settings: Bug reports', () => {
  it('says where reports go and saves the choice', async () => {
    const { HostSettingsPage } = await import('./HostSettingsPage');
    const settings = { remoteAccess: false, ownerLogin: null, updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 },
      maxConcurrentRuns: 4, agentCredentials: 'subscription', projectsDir: null, defaultAgent: 'claude',
      defaultSessionMode: 'conversation', ticketPermissionMode: 'bypass', sessionPermissionMode: 'ask', updateCheckMinutes: 60,
      maxAgentTurns: 4, triagePerAuthorHour: 3, triagePerRepoHour: 20, bugReports: 'auto' };
    const view = { settings, ownerFromEnv: null, tokenPresent: true, tokenError: null, projectsDir: null,
      remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: null } };
    const calls = stubApi({
      'GET /api/host/settings': view,
      'GET /api/host/update': { current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null, error: null, token: { state: 'missing' }, lastResult: null },
      'GET /api/host/checks': { claude: { found: true, ok: true, detail: '' }, codex: { found: true, ok: true, detail: '' }, devin: { found: false, ok: false, detail: '' }, git: { found: true, ok: true, detail: '' }, gitIdentity: { found: true, ok: true, detail: '' }, tailscale: { found: true, ok: true, detail: '' }, checkedAt: 1 },
      'GET /api/host/whoami': { caller: 'local', login: null },
      'GET /api/connections': [],
      'GET /api/bug-reports/preview': preview({ route: 'local', project: { id: 'p', key: 'PDT', name: 'Personal Dev Team' } }),
      'PUT /api/host/settings': (c) => ({ ...view, settings: { ...settings, ...(c.body as object) } }),
    });
    renderWithQuery(<HostSettingsPage section="bug-reports" />);
    expect(await screen.findByText(/Draft bug tickets in PDT · Personal Dev Team/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Send bug reports to/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'GitHub issue' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ bugReports: 'github' }]));
    await userEvent.click(screen.getByRole('button', { name: 'Report a bug…' }));
    expect(await screen.findByRole('dialog', { name: 'Report a bug' })).toBeInTheDocument();
  });
});
