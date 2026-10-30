import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AcpAgentConfig } from '../api/gen/AcpAgentConfig';
import { renderWithQuery, stubApi } from '../test-utils';
import { idFor, installedText, parseArgs, parseEnv } from './AcpAgents';
import { HostSettingsPage } from './HostSettingsPage';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const view = (acp: AcpAgentConfig[]) => ({
  settings: {
    remoteAccess: false, ownerLogin: null, updatePolicy: 'notify', updateChannel: 'stable', runPorts: { start: 8400, end: 8499 },
    maxConcurrentRuns: 4, agentCredentials: 'subscription', projectsDir: null, defaultAgent: 'claude',
    defaultSessionMode: 'conversation', ticketPermissionMode: 'bypass', sessionPermissionMode: 'ask', updateCheckMinutes: 60,
    releaseRepo: null, acpAgents: acp,
  },
  ownerFromEnv: null,
  tokenPresent: false,
  tokenError: null,
  remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: null },
  programs: [],
  network: {
    settings: { port: null, allowedHosts: [] }, portFromEnv: null, allowedHostsFromEnv: null, listenPort: 7420, defaultPort: 7420,
    restartNeeded: false, canRestart: true,
  },
  acpAgents: acp.map((a) => ({ ...a, agent: `acp:${a.id}`, icon: null, builtin: false, found: a.command === 'opencode' ? '/bin/opencode' : null })),
});

describe('ACP agent fields', () => {
  it('makes an id from the label that no other agent has', () => {
    expect(idFor('Open Code!', [])).toBe('open-code');
    expect(idFor('Open Code', ['open-code', 'open-code-2'])).toBe('open-code-3');
    expect(idFor('***', [])).toBe('agent');
  });

  it('reads arguments one per line and the environment as NAME=value lines', () => {
    expect(parseArgs(' acp \n\n--verbose\n')).toEqual(['acp', '--verbose']);
    expect(parseEnv('A=1\n\nB = x=y')).toEqual([{ name: 'A', value: '1' }, { name: 'B', value: ' x=y' }]);
    expect(parseEnv('A=1\nnope')).toEqual({ badLine: 2 });
  });
});

describe('Settings → Agents → ACP agents', () => {
  it('adds an agent by name, command, arguments and environment, and offers it as the default agent', async () => {
    let saved: AcpAgentConfig[] = [];
    const calls = stubApi({
      'GET /api/host/settings': () => view(saved),
      'GET /api/host/update': { current: '0.1.0', state: 'idle', token: { state: 'missing' }, releaseRepo: { repo: 'a/b' } },
      'GET /api/host/checks': { claude: { found: true, ok: true, detail: '' }, codex: { found: true, ok: true, detail: '' }, devin: { found: false, ok: false, detail: '' }, git: { found: true, ok: true, detail: '' }, tailscale: { found: true, ok: true, detail: '' }, acp: {}, checkedAt: 1 },
      'PUT /api/host/settings': (c) => {
        saved = (c.body as { acpAgents: AcpAgentConfig[] }).acpAgents;
        return view(saved);
      },
    });
    renderWithQuery(<HostSettingsPage section="agents" />);
    await userEvent.click(await screen.findByRole('button', { name: 'Add ACP agent' }));
    const dialog = await screen.findByRole('dialog', { name: 'Add ACP agent' });
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Name' }), 'OpenCode');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /Command/ }), 'opencode');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /Arguments/ }), 'acp');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /Environment/ }), 'OPENCODE_X=1');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Add agent' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([
      { acpAgents: [{ id: 'opencode', label: 'OpenCode', command: 'opencode', args: ['acp'], env: [{ name: 'OPENCODE_X', value: '1' }] }] },
    ]);
    expect(await screen.findByText('Found')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Default agent/ }));
    expect(await screen.findByRole('option', { name: 'OpenCode' })).toBeInTheDocument();
  });

  it('removes an agent after asking', async () => {
    let saved: AcpAgentConfig[] = [{ id: 'gone', label: 'Gone', command: 'gone', args: [], env: [] }];
    const calls = stubApi({
      'GET /api/host/settings': () => view(saved),
      'GET /api/host/update': { current: '0.1.0', state: 'idle', token: { state: 'missing' }, releaseRepo: { repo: 'a/b' } },
      'PUT /api/host/settings': (c) => {
        saved = (c.body as { acpAgents: AcpAgentConfig[] }).acpAgents;
        return view(saved);
      },
    });
    renderWithQuery(<HostSettingsPage section="agents" />);
    expect(await screen.findByText('Missing')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove agent' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ acpAgents: [] }]));
    expect(await screen.findByText('None yet.')).toBeInTheDocument();
  });

  it('installs Antigravity, signs it in and sets its path', async () => {
    const agy = (installedVersion: string | null, signedIn: string | null, program: string | null = null) => ({
      ...view([]),
      acpAgents: [{
        id: 'antigravity', label: 'Antigravity', agent: 'acp:antigravity', icon: null, command: 'agy_acp_server', args: [], env: [],
        builtin: true, installer: 'antigravity-acp', installedVersion, signedIn, program,
        found: installedVersion ? '/data/toolchains/antigravity-acp/1.4.5/agy_acp_server.par' : null,
      }],
    });
    let current = agy(null, null);
    const calls = stubApi({
      'GET /api/host/settings': () => current,
      'GET /api/host/update': { current: '0.1.0', state: 'idle', token: { state: 'missing' }, releaseRepo: { repo: 'a/b' } },
      'POST /api/host/acp-agents/antigravity/install': () => (current = agy('1.4.5', null)),
      'POST /api/host/acp-agents/antigravity/sign-in': () => (current = agy('1.4.5', 'oauth-personal')),
      'PUT /api/host/settings': () => (current = agy('1.4.5', 'oauth-personal', '~/agy/server')),
    });
    renderWithQuery(<HostSettingsPage section="agents" />);
    expect(await screen.findByText('Not installed')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Sign in' })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'Install' }));
    expect(await screen.findByText("Google's ACP server 1.4.5, installed by PDT.")).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Update' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Sign in' }));
    expect(await screen.findByText('Signed in (oauth-personal).')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Path' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.type(within(dialog).getByRole('textbox', { name: /Path/ }), '~/agy/server');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ acpPrograms: { antigravity: '~/agy/server' } }]);
    expect(await screen.findByText('Runs ~/agy/server, set by hand.')).toBeInTheDocument();
  });
});

describe('installedText', () => {
  it('says what runs and whether it is signed in', () => {
    const a = { installedVersion: null, program: null, signedIn: null } as Parameters<typeof installedText>[0];
    expect(installedText(a)[0]).toMatch(/^Not installed/);
    expect(installedText(a)[1]).toMatch(/^Not signed in/);
  });
});
