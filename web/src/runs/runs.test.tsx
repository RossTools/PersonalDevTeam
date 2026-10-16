import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Run } from '../api/gen/Run';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { ProjectSettingsPanel } from './ProjectSettingsPanel';
import { RunBranchForm } from './RunBranchForm';
import { RunsSection } from './RunsSection';
import { RunView } from './RunView';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ url }: { url?: string }) => <div data-testid="terminal-view">{url}</div>,
}));

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1 };
const run = (over: Partial<Run>): Run => ({
  id: 'r1', projectId: 'p', worktreeId: null, branch: 'main', name: 'web', kind: 'run', state: 'ready',
  port: 8401, url: 'https://box.t.ts.net:8401/', dataDir: '/d', cwd: '/r', exitCode: null, error: null,
  createdAt: 1, updatedAt: 1, ...over,
});
const config = {
  filePath: '/r/.pdt/project.toml', file: '', overrides: null, error: null,
  effective: {
    runs: [{ name: 'web', setup: null, command: 'x', web: true, data: true, env: {} }],
    tests: [{ name: 'unit', setup: null, command: 'y', web: false, data: false, env: {} }],
  },
};

describe('RunsSection', () => {
  it('starts a configured run or test and selects it', async () => {
    const calls = stubApi({
      'GET /api/projects/p/runs': [run({ id: 'old', state: 'stopped' })],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': (c) => run({ id: 'new', ...(c.body as object) }),
    });
    const onSelectRun = vi.fn();
    renderWithQuery(<RunsSection project={project} worktreeId={null} label="Repository" selectedRunId={null} onSelectRun={onSelectRun} />);
    expect(await screen.findByRole('button', { name: /web · stopped/ })).toBeInTheDocument();
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Run in Repository' }), 'test:unit');
    await waitFor(() => expect(onSelectRun).toHaveBeenCalledWith('new'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ name: 'unit', kind: 'test' });
  });

  it('asks before stopping the oldest run when the host is at its limit', async () => {
    let posts = 0;
    const calls = stubApi({
      'GET /api/projects/p/runs': [],
      'GET /api/projects/p/run-config?worktreeId=w': config,
      'GET /api/runs': [run({ id: 'oldest', name: 'web', branch: 'b1' }), run({ id: 'newer', branch: 'b2' })],
      'POST /api/projects/p/runs': (c) =>
        ++posts === 1
          ? new Reply(409, { code: 'too_many_runs', message: 'Stop "web · b1" to start this one?' })
          : run({ id: 'new', ...(c.body as object) }),
    });
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<RunsSection project={project} worktreeId="w" label="feature/x" selectedRunId={null} onSelectRun={() => {}} />);
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Run in feature/x' }), 'run:web');
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    expect(confirm).toHaveBeenCalledWith('Stop "web · b1" to start this one?');
    expect(calls.filter((c) => c.method === 'POST')[1].body).toEqual({ worktreeId: 'w', name: 'web', kind: 'run', stopRunId: 'oldest' });
  });

  it('does nothing more when the user declines', async () => {
    const calls = stubApi({
      'GET /api/projects/p/runs': [],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': new Reply(409, { code: 'too_many_runs', message: 'Stop?' }),
    });
    vi.spyOn(window, 'confirm').mockReturnValue(false);
    renderWithQuery(<RunsSection project={project} worktreeId={null} label="Repository" selectedRunId={null} onSelectRun={() => {}} />);
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Run in Repository' }), 'run:web');
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });

  it('shows a configuration error instead of the menu', async () => {
    stubApi({
      'GET /api/projects/p/runs': [],
      'GET /api/projects/p/run-config': { ...config, effective: null, error: '"x" has no command' },
    });
    renderWithQuery(<RunsSection project={project} worktreeId={null} label="Repository" selectedRunId={null} onSelectRun={() => {}} />);
    expect(await screen.findByText(/has no command/)).toBeInTheDocument();
    expect(screen.queryByRole('combobox')).not.toBeInTheDocument();
  });
});

describe('RunBranchForm', () => {
  it('runs a branch that has no worktree yet', async () => {
    const calls = stubApi({
      'GET /api/projects/p/branches': [{ name: 'main', local: true }, { name: 'feature/y', local: false }],
      'GET /api/projects/p/run-config': config,
      'POST /api/projects/p/runs': (c) => run({ id: 'n', ...(c.body as object) }),
    });
    const onSelectRun = vi.fn();
    renderWithQuery(<RunBranchForm project={project} onSelectRun={onSelectRun} />);
    await userEvent.selectOptions(await screen.findByRole('combobox', { name: 'Branch' }), 'feature/y');
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'What to run' }), 'run:web');
    await userEvent.click(screen.getByRole('button', { name: 'Run branch' }));
    await waitFor(() => expect(onSelectRun).toHaveBeenCalledWith('n'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ branch: 'feature/y', name: 'web', kind: 'run' });
  });
});

describe('RunView', () => {
  it('links a ready web run and stops it', async () => {
    const calls = stubApi({ 'POST /api/runs/r1/stop': undefined, 'GET /api/projects/p/runs': [] });
    renderWithQuery(<RunView run={run({})} onRemoved={() => {}} />);
    expect(screen.getByRole('link', { name: 'https://box.t.ts.net:8401/' })).toHaveAttribute('target', '_blank');
    expect(screen.getByTestId('terminal-view')).toHaveTextContent('/api/runs/r1/pty/command');
    await userEvent.click(screen.getByRole('tab', { name: 'Setup' }));
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
    vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<RunView run={run({ state: 'failed', error: 'setup exited with code 3', exitCode: 3 })} onRemoved={onRemoved} />);
    expect(screen.getByRole('alert')).toHaveTextContent('setup exited with code 3');
    expect(screen.queryByRole('link')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset data' }));
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await userEvent.click(screen.getByRole('button', { name: 'Remove' }));
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
    const confirm = vi.spyOn(window, 'confirm').mockReturnValue(true);
    renderWithQuery(<RunView run={run({ state: 'stopped' })} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Restart' }));
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/runs/r1/restart')).toHaveLength(2));
    expect(confirm).toHaveBeenCalledWith('Stop "api · b1" to start this one?');
    expect(calls.filter((c) => c.path === '/api/runs/r1/restart').map((c) => c.body)).toEqual([{}, { stopRunId: 'oldest' }]);
  });

  it('shows a stop that timed out so the user can retry', async () => {
    stubApi({
      'GET /api/projects/p/runs': [],
      'POST /api/runs/r1/stop': new Reply(409, { code: 'run_stopping', message: 'the run has not stopped yet; try again in a moment' }),
    });
    renderWithQuery(<RunView run={run({})} onRemoved={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('has not stopped yet');
    expect(screen.getByRole('button', { name: 'Stop' })).toBeEnabled();
  });
});

describe('ProjectSettingsPanel', () => {
  it('saves run overrides and the credential policy, and shows validation errors', async () => {
    let attempt = 0;
    const calls = stubApi({
      'GET /api/projects/p/settings': { agentCredentials: null, effectiveCredentials: 'subscription', runOverrides: null },
      'GET /api/projects/p/run-config': { ...config, file: '[[run]]\nname = "web"\n' },
      'PUT /api/projects/p/settings': (c) =>
        ++attempt === 1
          ? new Reply(400, { code: 'invalid_run_config', message: 'the host overrides is not valid: x' })
          : { agentCredentials: 'pass_through', effectiveCredentials: 'pass_through', runOverrides: (c.body as { runOverrides: string }).runOverrides },
    });
    renderWithQuery(<ProjectSettingsPanel project={project} />);
    expect(await screen.findByText(/name = "web"/)).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Host overrides (TOML)' }), 'bad');
    await userEvent.selectOptions(screen.getByRole('combobox', { name: 'Agent credentials' }), 'pass_through');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('not valid');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PUT')[1].body).toEqual({ agentCredentials: 'pass_through', runOverrides: 'bad' });
    const status = await screen.findByRole('status');
    expect(within(status).getByText('Saved')).toBeInTheDocument();
  });
});
