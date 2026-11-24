import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { RemoteInstanceView } from '../api/gen/RemoteInstanceView';
import type { RemoteProjectView } from '../api/gen/RemoteProjectView';
import { toast } from '@ui';
import { renderWithQuery, Reply, stubApi, stubClipboard } from '../test-utils';
import { AddProjectForm } from './AddProjectForm';
import { minutesLeft, RemoteInstances } from './RemoteInstances';
import { RemoteProjectsPage } from './RemoteProjectsPage';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const project: Project = { id: 'p', name: 'Valley', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'VAL', forge: null, kind: 'repo' };

const winBox: RemoteInstanceView = {
  linkId: 'l1', projectId: 'p', instanceId: 'i1', name: 'win-box', url: 'https://win-box.example.ts.net', publicKey: 'k',
  version: '0.7.0', protocol: 1, paused: false, lastSeenAt: Date.now() - 1000, lastError: null, secrets: [], createdAt: 1, state: 'connected', problem: null,
  cap: null, remoteCap: null, poolCap: 1,
  facts: {
    os: 'windows', arch: 'x86_64', cpus: 8, diskFreeBytes: 50 * 1024 ** 3,
    agents: [{ agent: 'claude', installed: true, loggedIn: true }, { agent: 'codex', installed: true, loggedIn: false }, { agent: 'devin', installed: false, loggedIn: false }],
    agentsCheckedAt: 1,
    tools: [{ name: 'git', path: 'C:/git.exe' }, { name: 'godot', path: 'C:/godot.exe' }],
  },
};

const view = (o: Partial<{ instances: RemoteInstanceView[]; pairing: { code: string; expiresAt: number } | null; address: string | null }> = {}) => ({
  name: 'studio', address: 'https://studio.example.ts.net', instances: [], pairing: null, ...o,
});

describe('Remote instances (the owner)', () => {
  it('lists each instance with its state, version, OS, agents and disk, and removes one after asking', async () => {
    const calls = stubApi({
      'GET /api/projects/p/remote-instances': view({ instances: [winBox, { ...winBox, linkId: 'l2', instanceId: 'i2', name: 'old-box', state: 'incompatible', problem: 'update old-box to v0.7.0 to link it with studio', facts: { ...winBox.facts!, agents: [], agentsCheckedAt: null } }] }),
      'DELETE /api/projects/p/remote-instances/l1': { told: true },
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
    const oldBox = within(section).getByText('needs an update').closest('li')!;
    expect(oldBox).toHaveTextContent('checking agents…');

    await userEvent.click(within(row).getByRole('button', { name: /Remove/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'DELETE', path: '/api/projects/p/remote-instances/l1', body: undefined }));
  });

  it('says when an instance that was removed couldn\'t be told', async () => {
    const info = vi.spyOn(toast, 'info');
    stubApi({
      'GET /api/projects/p/remote-instances': view({ instances: [{ ...winBox, state: 'offline' }] }),
      'DELETE /api/projects/p/remote-instances/l1': { told: false },
      'GET /api/projects/p/placement': { rules: [] },
    });
    renderWithQuery(<RemoteInstances project={project} />);
    const section = await screen.findByRole('region', { name: 'Remote instances' });
    const row = (await within(section).findByText('offline')).closest('li')!;
    await userEvent.click(within(row).getByRole('button', { name: /Remove/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(info).toHaveBeenCalledWith("win-box couldn't be told; leave Valley there too.", expect.anything()));
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

  it('sets how many tickets an instance takes at once, beside its own cap', async () => {
    const capped = { ...winBox, cap: 3, remoteCap: 2, poolCap: 2 };
    const calls = stubApi({
      'GET /api/projects/p/remote-instances': view({ instances: [{ ...winBox, remoteCap: 2, poolCap: 2 }] }),
      'GET /api/projects/p/placement': { rules: [] },
      'PATCH /api/projects/p/remote-instances/l1': view({ instances: [capped] }),
    });
    renderWithQuery(<RemoteInstances project={project} />);
    const cap = await screen.findByRole('textbox', { name: 'Cap for win-box' });
    expect(cap.closest('span')?.parentElement).toHaveTextContent("(win-box's own cap: 2). Now 2.");
    await userEvent.type(cap, '3{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/projects/p/remote-instances/l1', body: { cap: 3 } }));
    expect(await screen.findByDisplayValue('3')).toBeInTheDocument();
  });

  it('shows a pairing code and this instance\'s address on Add instance, each with Copy, and cancels it', async () => {
    let shown = view();
    const writeText = vi.fn().mockResolvedValue(undefined);
    stubClipboard({ writeText });
    const calls = stubApi({
      'GET /api/projects/p/remote-instances': () => shown,
      'POST /api/projects/p/remote-instances/pairing': () => (shown = view({ pairing: { code: 'ABCD-EFGH', expiresAt: Date.now() + 9 * 60_000 } })),
      'DELETE /api/projects/p/remote-instances/pairing': () => { shown = view(); return undefined; },
    });
    renderWithQuery(<RemoteInstances project={project} />);
    expect(await screen.findByText('No remote instances yet.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Add instance' }));
    expect(await screen.findByText('ABCD-EFGH')).toBeInTheDocument();
    expect(screen.getByText('https://studio.example.ts.net')).toBeInTheDocument();
    expect(screen.getByText('Expires in 9 min')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Copy the code' }));
    expect(writeText).toHaveBeenCalledWith('ABCD-EFGH');
    await userEvent.click(screen.getByRole('button', { name: 'Copy the address' }));
    expect(writeText).toHaveBeenCalledWith('https://studio.example.ts.net');
    expect(screen.getByRole('button', { name: /New code/ })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Cancel code' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'DELETE', path: '/api/projects/p/remote-instances/pairing', body: undefined }));
    expect(await screen.findByText('No remote instances yet.')).toBeInTheDocument();
    // Cancel code went with its row: focus is on Add instance (PDT-668).
    await waitFor(() => expect(screen.getByRole('button', { name: 'Add instance' })).toHaveFocus());
  });

  it('moves focus to the next instance\'s Remove when one is removed, then to Add instance (PDT-668)', async () => {
    let instances = [winBox, { ...winBox, linkId: 'l2', instanceId: 'i2', name: 'mac-box' }];
    const gone = (id: string) => () => { instances = instances.filter((i) => i.linkId !== id); return { told: true }; };
    stubApi({
      'GET /api/projects/p/remote-instances': () => view({ instances }),
      'DELETE /api/projects/p/remote-instances/l1': gone('l1'),
      'DELETE /api/projects/p/remote-instances/l2': gone('l2'),
      'GET /api/projects/p/placement': { rules: [] },
    });
    renderWithQuery(<RemoteInstances project={project} />);
    const remove = async (name: string) => {
      const row = (await screen.findAllByText(name)).map((e) => e.closest('li')).find((li) => li?.textContent?.includes('connected'))!;
      await userEvent.click(within(row).getByRole('button', { name: /Remove/ }));
      await userEvent.click(await screen.findByRole('button', { name: 'Remove' }));
    };
    await remove('win-box');
    await waitFor(() => expect(screen.queryAllByText('win-box')).toHaveLength(0));
    const macRow = screen.getAllByText('mac-box').map((e) => e.closest('li')).find((li) => li?.textContent?.includes('connected'))!;
    await waitFor(() => expect(within(macRow).getByRole('button', { name: /Remove/ })).toHaveFocus());
    await remove('mac-box');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Add instance' })).toHaveFocus());
  });

  it('offers no pairing while remote access is off, only a way to turn it on', async () => {
    window.history.replaceState(null, '', '/projects/p');
    stubApi({ 'GET /api/projects/p/remote-instances': view({ address: null, pairing: { code: 'ABCD-EFGH', expiresAt: Date.now() + 600_000 } }) });
    renderWithQuery(<RemoteInstances project={project} />);
    expect(await screen.findByText("Remote access is off, so other instances can't reach this one.")).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Add instance|New code/ })).not.toBeInTheDocument();
    expect(screen.queryByText('ABCD-EFGH')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Turn on remote access/ }));
    await waitFor(() => expect(window.location.pathname).toBe('/settings/host/remote-access'));
  });

  it('says how long a pairing code has left', () => {
    expect(minutesLeft(10 * 60_000, 0)).toBe('10 min');
    expect(minutesLeft(30_000, 0)).toBe('1 min');
    expect(minutesLeft(0, 5_000)).toBe('1 min');
  });
});

const remote: RemoteProjectView = {
  project: { ...project, id: 'rp', kind: 'remote', repoPath: 'remote:l1' },
  link: {
    projectId: 'rp', linkId: 'l1', ownerProjectId: 'p', ownerInstanceId: 'o', ownerName: 'studio', ownerUrl: 'https://studio.example.ts.net',
    ownerVersion: '0.7.0', ownerPublicKey: 'k', repoUrl: null, checkoutPath: null, paused: false, ownUpdateChannel: false, cap: null,
    lastContactAt: Date.now(), removedAt: null, createdAt: 1,
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
    expect(row).not.toHaveTextContent('Nothing runs here');
    await userEvent.click(within(row).getByRole('button', { name: 'Open on studio' }));
    expect(open).toHaveBeenCalledWith('https://studio.example.ts.net/tickets?view=board', '_blank', 'noopener,noreferrer');
    await userEvent.click(within(row).getByRole('button', { name: 'Pause' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/remote-projects/rp', body: { paused: true } }));
  });

  it('says when the owner removed this instance, and when leaving couldn\'t tell the owner', async () => {
    const info = vi.spyOn(toast, 'info');
    stubApi({
      'GET /api/remote-projects': [{ ...remote, state: 'removed', link: { ...remote.link, removedAt: 1 } }, { ...remote, project: { ...remote.project, id: 'rp2', name: 'Hills' }, state: 'offline' }],
      'DELETE /api/remote-projects/rp2': { told: false },
      'GET /api/host/settings': { settings: {}, remote: { active: true } },
    });
    renderWithQuery(<RemoteProjectsPage />);
    const row = (await screen.findByText('Valley')).closest('li')!;
    expect(within(row).getByText('removed by its owner')).toBeInTheDocument();
    expect(row).toHaveTextContent('studio removed this instance from Valley, or removed the project. Leave it to finish here.');
    const hills = screen.getByText('Hills').closest('li')!;
    await userEvent.click(within(hills).getByRole('button', { name: /Leave/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Leave project' }));
    await waitFor(() => expect(info).toHaveBeenCalledWith("studio couldn't be told; remove this instance from Hills there too."));
  });

  it('moves focus to the join form when the last project is left (PDT-668)', async () => {
    let projects = [remote];
    stubApi({
      'GET /api/remote-projects': () => projects,
      'DELETE /api/remote-projects/rp': () => { projects = []; return { told: true }; },
      'GET /api/host/settings': { settings: {}, remote: { active: true } },
      'GET /api/secrets': [],
    });
    renderWithQuery(<RemoteProjectsPage />);
    const row = (await screen.findByText('Valley')).closest('li')!;
    await userEvent.click(within(row).getByRole('button', { name: /Leave/ }));
    await userEvent.click(await screen.findByRole('button', { name: 'Leave project' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: "Owner's address" })).toHaveFocus());
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

  it('keeps secrets here for the project, never for the owner to read, and picks whose update channel to use', async () => {
    const cert = { id: 's1', name: 'CERT', description: 'For Valley', projects: ['rp'], personas: [], createdAt: 1, updatedAt: 1, lastUsedAt: null, lastUsedBy: null };
    const other = { ...cert, id: 's2', name: 'OTHER', projects: [] };
    const calls = stubApi({
      'GET /api/remote-projects': [remote],
      'GET /api/secrets': [cert, other],
      'POST /api/secrets': { ...cert, id: 's3', name: 'SIGN_KEY' },
      'DELETE /api/secrets/s1': undefined,
      'PATCH /api/remote-projects/rp': { ...remote, link: { ...remote.link, ownUpdateChannel: true } },
    });
    renderWithQuery(<RemoteProjectsPage />);
    const list = await screen.findByRole('list', { name: 'Secrets for Valley' });
    expect(list).toHaveTextContent('CERT');
    expect(list).not.toHaveTextContent('OTHER');
    await userEvent.type(screen.getByRole('textbox', { name: "New secret's name for Valley" }), 'SIGN_KEY');
    await userEvent.type(screen.getByLabelText("New secret's value for Valley"), 'sign-key-value');
    await userEvent.click(screen.getByRole('button', { name: 'Add secret' }));
    await waitFor(() => expect(calls).toContainEqual({
      method: 'POST', path: '/api/secrets', body: { name: 'SIGN_KEY', description: 'For Valley', projects: ['rp'], value: 'sign-key-value' },
    }));
    await userEvent.click(within(list).getByRole('button', { name: 'Remove…' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'DELETE', path: '/api/secrets/s1', body: undefined }));

    await userEvent.click(screen.getByRole('switch', { name: /use this host's own update channel/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/remote-projects/rp', body: { ownUpdateChannel: true } }));
  });

  it('sets this instance\'s own cap for the project; blank leaves it to the owner', async () => {
    const calls = stubApi({
      'GET /api/remote-projects': [remote],
      'PATCH /api/remote-projects/rp': { ...remote, link: { ...remote.link, cap: 2 } },
      'GET /api/host/settings': { settings: {}, remote: { active: true } },
    });
    renderWithQuery(<RemoteProjectsPage />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Cap for Valley' }), '2{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/remote-projects/rp', body: { cap: 2 } }));
  });

  it('joins with the owner\'s address and code from Add project, and shows why it was refused', async () => {
    const onJoined = vi.fn();
    let n = 0;
    const calls = stubApi({
      'GET /api/host/settings': { settings: {}, remote: { active: true } },
      'POST /api/remote-projects': () => (n++ === 0 ? new Reply(403, { code: 'pairing_code', message: 'that pairing code is wrong or has expired: show a new one' }) : remote),
    });
    renderWithQuery(<AddProjectForm initialMode="remote" onJoined={onJoined} />);
    await userEvent.type(screen.getByRole('textbox', { name: "Owner's address" }), 'studio.example.ts.net');
    await userEvent.type(screen.getByRole('textbox', { name: 'Pairing code' }), 'abcd-efgh');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Join project' })).toBeEnabled());
    await userEvent.click(screen.getByRole('button', { name: 'Join project' }));
    expect(await screen.findByText(/that pairing code is wrong/)).toBeInTheDocument();
    expect(calls).toContainEqual({ method: 'POST', path: '/api/remote-projects', body: { ownerAddress: 'studio.example.ts.net', code: 'abcd-efgh' } });
    await userEvent.click(screen.getByRole('button', { name: 'Join project' }));
    await waitFor(() => expect(onJoined).toHaveBeenCalled());
  });

  it('offers no Join while remote access is off, only a way to turn it on, which closes the dialog', async () => {
    window.history.replaceState(null, '', '/projects');
    const onLeave = vi.fn();
    stubApi({ 'GET /api/host/settings': { settings: {}, remote: { active: false } } });
    renderWithQuery(<AddProjectForm initialMode="remote" onLeave={onLeave} />);
    expect(await screen.findByText("Remote access is off, so other instances can't reach this one.")).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Join project' })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Turn on remote access/ }));
    expect(onLeave).toHaveBeenCalled();
    await waitFor(() => expect(window.location.pathname).toBe('/settings/host/remote-access'));
  });

});
