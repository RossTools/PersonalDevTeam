import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentKind } from '../api/gen/AgentKind';
import { useAgents } from '../common/agents';
import { renderWithQuery, stubApi } from '../test-utils';
import { switched } from './AgentSwitch';
import { HostSettingsPage } from './HostSettingsPage';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const view = (off: AgentKind[], defaultAgent: AgentKind = 'claude') => ({
  settings: {
    remoteAccess: false, ownerLogin: null, updatePolicy: 'notify', updateChannel: 'stable', runPorts: { start: 8400, end: 8499 },
    maxConcurrentRuns: 4, agentCredentials: 'subscription', projectsDir: null, defaultAgent,
    defaultSessionMode: 'conversation', ticketPermissionMode: 'bypass', sessionPermissionMode: 'ask', updateCheckMinutes: 60,
    releaseRepo: null, acpAgents: [], disabledAgents: off,
  },
  ownerFromEnv: null,
  tokenPresent: false,
  tokenError: null,
  remote: { enabled: false, active: false, url: null, dnsName: null, error: null, tailscaleLogin: null },
  programs: (['claude', 'codex', 'devin'] as const).map((agent) => ({ agent, envVar: `PDT_${agent.toUpperCase()}_PROGRAM`, fromEnv: null, found: `/bin/${agent}` })),
  network: {
    settings: { port: null, allowedHosts: [] }, portFromEnv: null, allowedHostsFromEnv: null, listenPort: 7420, defaultPort: 7420,
    restartNeeded: false, canRestart: true,
  },
  acpAgents: [],
  enabledAgents: (['claude', 'codex', 'devin'] as AgentKind[]).filter((a) => !off.includes(a)),
});

function Offered() {
  const agents = useAgents();
  return <ul>{agents.all.map((a) => <li key={a}>{a}</li>)}</ul>;
}

describe('Settings → Agents: on/off switches (PDT-270)', () => {
  it('works out the agents switched off', () => {
    expect(switched([], 'codex', false)).toEqual(['codex']);
    expect(switched(['codex'], 'codex', false)).toEqual(['codex']);
    expect(switched(['codex', 'devin'], 'codex', true)).toEqual(['devin']);
  });

  it('switches an agent off and on, never the default one, and the pickers follow', async () => {
    let off: AgentKind[] = [];
    const calls = stubApi({
      'GET /api/host/settings': () => view(off),
      'GET /api/host/update': { current: '0.1.0', state: 'idle', token: { state: 'missing' }, releaseRepo: { repo: 'a/b' } },
      'PUT /api/host/settings': (c) => {
        off = (c.body as { disabledAgents: AgentKind[] }).disabledAgents;
        return view(off);
      },
    });
    renderWithQuery(<><HostSettingsPage section="agents" /><Offered /></>);
    const claude = await screen.findByRole('switch', { name: 'Use Claude Code' });
    expect(claude).toBeChecked();
    expect(claude).toBeDisabled();
    expect(screen.getByText(/pick another default before switching it off/)).toBeInTheDocument();

    await userEvent.click(screen.getByRole('switch', { name: 'Use Codex' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ disabledAgents: ['codex'] }));
    await waitFor(() => expect(screen.getByRole('switch', { name: 'Use Codex' })).not.toBeChecked());
    expect(screen.getByText(/Switched off: PDT offers it nowhere/)).toBeInTheDocument();
    expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toEqual(['claude', 'devin']);

    await userEvent.click(screen.getByRole('switch', { name: 'Use Codex' }));
    await waitFor(() => expect(off).toEqual([]));
    await waitFor(() => expect(screen.getAllByRole('listitem').map((l) => l.textContent)).toEqual(['claude', 'codex', 'devin']));
  });
});
