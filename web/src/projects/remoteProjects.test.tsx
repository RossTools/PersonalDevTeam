import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { RemoteInstanceView } from '../api/gen/RemoteInstanceView';
import type { RemoteProjectView } from '../api/gen/RemoteProjectView';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { AddProjectForm } from './AddProjectForm';
import { ago, RemoteInstances } from './RemoteInstances';
import { RemoteProjectsPage } from './RemoteProjectsPage';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const project: Project = { id: 'p', name: 'Valley', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'VAL', forge: null, kind: 'repo' };

const winBox: RemoteInstanceView = {
  linkId: 'l1', projectId: 'p', instanceId: 'i1', name: 'win-box', url: 'https://win-box.example.ts.net', publicKey: 'k',
  version: '0.7.0', protocol: 1, paused: false, lastSeenAt: Date.now() - 1000, lastError: null, createdAt: 1, state: 'connected', problem: null,
  facts: {
    os: 'windows', arch: 'x86_64', cpus: 8, diskFreeBytes: 50 * 1024 ** 3,
    agents: [{ agent: 'claude', installed: true, loggedIn: true }, { agent: 'codex', installed: true, loggedIn: false }, { agent: 'devin', installed: false, loggedIn: false }],
    tools: [{ name: 'git', path: 'C:/git.exe' }, { name: 'godot', path: 'C:/godot.exe' }],
  },
};

const view = (o: Partial<{ instances: RemoteInstanceView[]; pairing: { code: string; expiresAt: number } | null; address: string | null }> = {}) => ({
  name: 'studio', address: 'https://studio.example.ts.net', instances: [], pairing: null, ...o,
});

describe('Remote instances (the owner)', () => {
  it('lists each instance with its state, version, OS, agents and disk, and removes one after asking', async () => {
    const calls = stubApi({
      'GET /api/projects/p/remote-instances': view({ instances: [winBox, { ...winBox, linkId: 'l2', instanceId: 'i2', name: 'old-box', state: 'incompatible', problem: 'update old-box to v0.7.0 to link it with studio' }] }),
      'DELETE /api/projects/p/remote-instances/l1': undefined,
      'GET /api/projects/p/placement': { rules: [] },
    });
    renderWithQuery(<RemoteInstances project={project} />);
    const section = await screen.findByRole('region', { name: 'Remote instances' });
    const row = (await within(section).findAllByText('win-box')).map((e) => e.closest('li')).find((li) => li?.textContent?.includes('connected'))!;
    expect(within(row).getByText('connected')).toBeInTheDocument();
    expect(within(row).getByText('v0.7.0')).toBeInTheDocument();
    expect(row).toHaveTextContent('Windows · 8 CPUs');
    expect(row).toHaveTextContent('claude, codex (not logged in)');
    expect(row).toHaveTextContent('git, godot');
    expect(row).toHaveTextContent('free');
    expect(within(section).getByText('update old-box to v0.7.0 to link it with studio')).toBeInTheDocument();
    expect(within(section).getByText('needs an update')).toBeInTheDocument();

    await userEvent.click(within(row).getByRole('button', { name: /Remove/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'DELETE', path: '/api/projects/p/remote-instances/l1', body: undefined }));
  });

  it('maps a label to an instance in Placement', async () => {
    const calls = stubApi({
      'GET /api/projects/p/remote-instances': view({ instances: [winBox] }),
      'GET /api/projects/p/placement': { rules: [] },
      'PUT /api/projects/p/placement': { rules: [{ label: 'windows', instance: winBox.instanceId }] },
    });
    renderWithQuery(<RemoteInstances project={project} />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Label' }), 'windows');
    await userEvent.click(screen.getByRole('button', { name: /Add rule/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PUT', path: '/api/projects/p/placement', body: { rules: [{ label: 'windows', instance: winBox.instanceId }] } }));
    expect(await screen.findByText('→ win-box')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Remove the rule for windows' })).toBeInTheDocument();
  });

  it('shows a pairing code with this instance\'s address on Add', async () => {
    let shown = view();
    const calls = stubApi({
      'GET /api/projects/p/remote-instances': () => shown,
      'POST /api/projects/p/remote-instances/pairing': () => (shown = view({ pairing: { code: 'ABCD-EFGH', expiresAt: Date.now() + 600_000 } })),
    });
    renderWithQuery(<RemoteInstances project={project} />);
    expect(await screen.findByText('No remote instances yet.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Add' }));
    expect(await screen.findByText('ABCD-EFGH')).toBeInTheDocument();
    expect(screen.getByText('https://studio.example.ts.net')).toBeInTheDocument();
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/projects/p/remote-instances/pairing')).toBe(true);
  });
});

const remote: RemoteProjectView = {
  project: { ...project, id: 'rp', kind: 'remote', repoPath: 'remote:l1' },
  link: {
    projectId: 'rp', linkId: 'l1', ownerProjectId: 'p', ownerInstanceId: 'o', ownerName: 'studio', ownerUrl: 'https://studio.example.ts.net',
    ownerVersion: '0.7.0', ownerPublicKey: 'k', repoUrl: null, checkoutPath: null, paused: false, lastContactAt: Date.now(), createdAt: 1,
  },
  state: 'connected',
  openUrl: 'https://studio.example.ts.net/tickets?view=board',
  checkoutPath: '/data/checkouts/l1',
  running: [],
};

describe('Remote projects (the remote instance)', () => {
  it('shows each as a remote project of its owner, with Pause and Open on the owner', async () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    const calls = stubApi({
      'GET /api/remote-projects': [remote],
      'PATCH /api/remote-projects/rp': { ...remote, link: { ...remote.link, paused: true }, state: 'paused' },
      'GET /api/host/settings': { settings: {}, remote: { active: true } },
    });
    renderWithQuery(<RemoteProjectsPage />);
    const row = (await screen.findByText('Valley')).closest('li')!;
    expect(row).toHaveTextContent('· remote project of studio');
    expect(within(row).getByText('connected')).toBeInTheDocument();
    expect(row).toHaveTextContent('Nothing runs here for studio now.');
    await userEvent.click(within(row).getByRole('button', { name: 'Open on studio' }));
    expect(open).toHaveBeenCalledWith('https://studio.example.ts.net/tickets?view=board', '_blank', 'noopener,noreferrer');
    await userEvent.click(within(row).getByRole('button', { name: 'Pause' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/remote-projects/rp', body: { paused: true } }));
  });

  it('lists what runs here for the owner, read-only, and moves the checkout', async () => {
    const running = { ...remote, running: [{ sessionId: 's', title: 'Coder · VAL-4', agent: 'claude', ticketKey: 'VAL-4', startedAt: Date.now(), live: true }] };
    const calls = stubApi({
      'GET /api/remote-projects': [running],
      'PATCH /api/remote-projects/rp': { ...running, checkoutPath: '/work/valley' },
      'GET /api/host/settings': { settings: {}, remote: { active: true } },
    });
    renderWithQuery(<RemoteProjectsPage />);
    const list = await screen.findByRole('list', { name: 'Running for studio' });
    expect(list).toHaveTextContent('VAL-4');
    expect(list).toHaveTextContent('Coder · VAL-4');
    expect(within(list).getByText('running')).toBeInTheDocument();
    const folder = screen.getByRole('textbox', { name: 'Checkout folder of Valley' });
    expect(folder).toHaveValue('/data/checkouts/l1');
    await userEvent.clear(folder);
    await userEvent.type(folder, '/work/valley');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/remote-projects/rp', body: { checkoutPath: '/work/valley' } }));
  });

  it('joins with the owner\'s address and code from Add project, and shows why it was refused', async () => {
    const onJoined = vi.fn();
    let n = 0;
    const calls = stubApi({
      'GET /api/host/settings': { settings: {}, remote: { active: false } },
      'POST /api/remote-projects': () => (n++ === 0 ? new Reply(403, { code: 'pairing_code', message: 'that pairing code is wrong or has expired: show a new one' }) : remote),
    });
    renderWithQuery(<AddProjectForm initialMode="remote" onJoined={onJoined} />);
    expect(await screen.findByText(/turn it on in Settings → This host → Remote access/)).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: "Owner's address" }), 'studio.example.ts.net');
    await userEvent.type(screen.getByRole('textbox', { name: 'Pairing code' }), 'abcd-efgh');
    await userEvent.click(screen.getByRole('button', { name: 'Join project' }));
    expect(await screen.findByText(/that pairing code is wrong/)).toBeInTheDocument();
    expect(calls).toContainEqual({ method: 'POST', path: '/api/remote-projects', body: { ownerAddress: 'studio.example.ts.net', code: 'abcd-efgh' } });
    await userEvent.click(screen.getByRole('button', { name: 'Join project' }));
    await waitFor(() => expect(onJoined).toHaveBeenCalled());
  });

  it('says how long ago', () => {
    const now = 10_000_000;
    expect(ago(now - 5_000, now)).toBe('just now');
    expect(ago(now - 3 * 60_000, now)).toBe('3 min ago');
    expect(ago(now - 5 * 3_600_000, now)).toBe('5 h ago');
  });
});
