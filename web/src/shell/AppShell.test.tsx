import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { AppShell } from './AppShell';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

const project = { id: '0190aaaa-bbbb-cccc-dddd-eeeeffff0000', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const shell = { id: 'sh', projectId: project.id, worktreeId: null, kind: 'shell', agent: null, title: 'Shell', cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: 1, activity: null, attentionAt: null, lastViewedAt: null, unseen: false };

// The shell always renders the update banner and the first-run checklist; a fully set-up host shows neither.
const hostTool = { found: true, ok: true, detail: 'ok' };
const hostRoutes = {
  'GET /api/host/settings': {
    settings: { remoteAccess: true, ownerLogin: 'me@example.com', updatePolicy: 'notify', runPorts: { start: 8400, end: 8499 }, maxConcurrentRuns: 4, agentCredentials: 'subscription' },
    ownerFromEnv: null,
    tokenPresent: true,
    remote: { enabled: true, active: true, url: 'https://box.ts.net', dnsName: 'box.ts.net', error: null, tailscaleLogin: 'me@example.com' },
  },
  'GET /api/host/update': {
    current: '0.1.6', state: 'idle', disabledReason: null, policy: 'notify', latest: null, lastCheckedAt: null,
    error: null, token: { state: 'valid', expiresAt: null }, lastResult: null,
  },
  'GET /api/host/checks': { claude: hostTool, codex: hostTool, git: hostTool, tailscale: hostTool, checkedAt: 1 },
  'GET /api/host/whoami': { caller: 'local', login: null },
};

function stubAlpha() {
  stubApi({
    ...hostRoutes,
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/worktrees`]: [],
    [`GET /api/projects/${project.id}/sessions`]: [shell],
  });
}

describe('AppShell', () => {
  it('renders the six rail views and marks the active one', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    for (const name of ['Inbox', 'Board', 'Sessions', 'Personas', 'Files', 'Settings']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole('button', { name: 'Board' }));
    expect(screen.getByRole('button', { name: 'Board' })).toHaveAttribute('aria-current', 'page');
  });

  it('shows the host settings in the Settings view', async () => {
    stubApi({ 'GET /api/projects': [], ...hostRoutes });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    expect(await screen.findByRole('heading', { name: 'This host' })).toBeInTheDocument();
    expect(await screen.findByRole('checkbox', { name: 'Remote access' })).toBeChecked();
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

  it('shows the selected project name, not its id', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    const main = screen.getByRole('main');
    expect(main).toHaveTextContent('Project Alpha');
    expect(main).not.toHaveTextContent(project.id);
    expect(screen.getByRole('button', { name: 'Alpha' })).toHaveAttribute('aria-current', 'true');
  });

  it('opens a selected session in the work area and shells as panel tabs', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    // The session row's name includes its StatusDot label ("running" + "Shell", no space in jsdom), unlike "New Shell in Repository".
    await userEvent.click(await screen.findByRole('button', { name: /^running\s*Shell$/ }));
    expect(screen.getByRole('main')).toContainElement(screen.getByTestId('terminal-view'));
    await userEvent.click(screen.getByRole('button', { name: 'Toggle terminal panel' }));
    expect(await screen.findByRole('tab', { name: /Shell/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('button', { name: 'Maximize terminal panel' }));
    expect(screen.getByRole('button', { name: 'Restore terminal panel' })).toBeInTheDocument();
    expect(screen.getAllByTestId('terminal-view')).toHaveLength(1);
  });

  it('tells both terminals the session status and when the list was last polled', async () => {
    stubAlpha();
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Alpha' }));
    await userEvent.click(await screen.findByRole('button', { name: /^running\s*Shell$/ }));
    const inWork = screen.getByRole('main').querySelector('[data-testid="terminal-view"]')!;
    expect(inWork).toHaveAttribute('data-status', 'running');
    expect(Number(inWork.getAttribute('data-polled'))).toBeGreaterThan(0);
    await userEvent.click(screen.getByRole('button', { name: 'Toggle terminal panel' }));
    const panel = (await screen.findByRole('region', { name: 'Terminal panel' })).querySelector('[data-testid="terminal-view"]')!;
    expect(panel).toHaveAttribute('data-status', 'running');
    expect(Number(panel.getAttribute('data-polled'))).toBeGreaterThan(0);
  });
});
