import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { SetupView } from '../api/gen/SetupView';
import { forgeAccount, githubLogin, renderWithQuery, stubApi } from '../test-utils';
import { firstOpenStep, foundAgents, SetupPage, type SetupPageProps } from './SetupPage';

afterEach(() => vi.unstubAllGlobals());

const tool = (found: boolean, path: string | null = null) => ({ found, ok: found, detail: '', path });
const checks = (o: { claude?: boolean; codex?: boolean; devin?: boolean; git?: boolean } = {}) => ({
  git: tool(o.git ?? true, '/usr/bin/git'),
  claude: tool(o.claude ?? true, '/u/.local/bin/claude'),
  codex: tool(o.codex ?? false),
  devin: tool(o.devin ?? false),
  tailscale: tool(false),
  acp: {},
  checkedAt: 1,
});

function view(o: Partial<Omit<SetupView, 'steps'>> & { steps?: Partial<SetupView['steps']> } = {}): SetupView {
  const steps = { agents: 'todo', github: 'todo', project: 'todo', defaultAgent: 'todo', ...o.steps } as SetupView['steps'];
  return {
    state: { needed: true, dismissed: false, finished: false, githubSkipped: false, defaultAgentConfirmed: false },
    done: Object.values(steps).filter((s) => s !== 'todo').length,
    opensOnLaunch: true,
    defaultAgent: 'claude',
    ...o,
    steps,
  };
}

const notConnected = githubLogin();

function props(): SetupPageProps {
  return { onAddProject: vi.fn(), onBoard: vi.fn(), onPm: vi.fn(), onHostSettings: vi.fn(), onDismissed: vi.fn() };
}

function stub(v: SetupView, extra: Record<string, unknown> = {}) {
  return stubApi({
    'GET /api/host/setup': v,
    'GET /api/host/checks': checks(),
    'GET /api/forges/github/login': notConnected,
    'GET /api/projects': [],
    'GET /api/host/settings': { settings: { defaultAgent: 'claude' }, projectsDir: '/u/work', programs: [] },
    ...extra,
  });
}

const step = (name: string) => screen.findByRole('region', { name });

describe('Get started', () => {
  it('opens the first step not done', () => {
    expect(firstOpenStep(view())).toBe('agents');
    expect(firstOpenStep(view({ steps: { agents: 'done', github: 'skipped' } }))).toBe('project');
    expect(firstOpenStep(view({ steps: { agents: 'done', github: 'done', project: 'done', defaultAgent: 'done' } }))).toBeNull();
    expect(foundAgents(checks({ codex: true }))).toEqual(['claude', 'codex']);
    expect(foundAgents(checks({ codex: true }), ['codex'])).toEqual(['claude']);
  });

  it('shows the progress and each step’s state, and lists git and the agents with their paths', async () => {
    stub(view({ steps: { agents: 'done', github: 'skipped' } }));
    renderWithQuery(<SetupPage {...props()} />);
    expect(await screen.findByRole('progressbar', { name: 'Setup progress' })).toHaveAttribute('aria-valuenow', '2');
    expect(screen.getByText('2 of 4')).toBeInTheDocument();
    expect(within(await step('Agents on this machine')).getByRole('img', { name: 'done' })).toBeInTheDocument();
    expect(within(await step('Connect GitHub')).getByRole('img', { name: 'skipped' })).toBeInTheDocument();
    expect(within(await step('Add a project')).getByRole('img', { name: 'to do' })).toBeInTheDocument();
    // The project step is open (the first not done); open the agents.
    const agents = await step('Agents on this machine');
    await userEvent.click(within(agents).getByRole('button', { expanded: false }));
    // Each path cuts in the middle (the folders shrink, the program's name stays), with the whole path in a tooltip.
    const claude = await within(agents).findByTitle('/u/.local/bin/claude');
    expect(claude).toHaveTextContent('/u/.local/bin/claude');
    expect(within(claude).getByText('/claude')).toHaveClass('shrink-0');
    expect(within(claude).getByText('/u/.local/bin')).toHaveClass('truncate');
    expect(claude.closest('td')).toHaveClass('w-full', 'max-w-0');
    expect(within(agents).getByTitle('/usr/bin/git')).toBeInTheDocument();
    const devin = within(agents).getByRole('row', { name: /Devin/ });
    expect(within(devin).getByRole('link', { name: /How to install/ })).toHaveAttribute('href', 'https://cli.devin.ai');
    expect(screen.queryByText(/You’re set up/)).not.toBeInTheDocument();
  });

  it('saves an agent path set by hand, then checks again', async () => {
    const calls = stub(view(), {
      'PUT /api/host/settings': { settings: {}, projectsDir: null, programs: [] },
      'POST /api/host/checks/refresh': checks({ codex: true }),
    });
    renderWithQuery(<SetupPage {...props()} />);
    const agents = await step('Agents on this machine');
    await userEvent.click(within(agents).getByRole('button', { name: 'Set a path by hand…' }));
    await userEvent.click(within(within(agents).getByRole('form', { name: 'Agent program path' })).getByRole('button', { name: /Agent/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Codex' }));
    await userEvent.type(within(agents).getByRole('textbox', { name: 'Program path' }), '/opt/codex/bin/codex');
    await userEvent.click(within(agents).getByRole('button', { name: 'Save path' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/host/checks/refresh')).toBe(true));
    expect(calls.find((c) => c.method === 'PUT' && c.path === '/api/host/settings')?.body).toEqual({ codexProgram: '/opt/codex/bin/codex' });
  });

  it('skips GitHub, or shows the code to enter while connecting', async () => {
    const skipped = view({ steps: { agents: 'done', github: 'skipped' } });
    const calls = stub(view({ steps: { agents: 'done' } }), {
      'PUT /api/host/setup': skipped,
      'POST /api/forges/github/login/device': { ...notConnected, pending: { userCode: '4F2A-9C1D', verificationUri: 'https://github.com/login/device', expiresAt: 0 } },
    });
    renderWithQuery(<SetupPage {...props()} />);
    const gh = await step('Connect GitHub');
    expect(within(gh).getByText('Optional')).toBeInTheDocument();
    await userEvent.click(await within(gh).findByRole('button', { name: 'Connect GitHub' }));
    expect(await within(gh).findByText('4F2A-9C1D')).toBeInTheDocument();
    expect(within(gh).getByRole('link', { name: /github\.com\/login\/device/ })).toHaveAttribute('href', 'https://github.com/login/device');
    await userEvent.click(within(gh).getByRole('button', { name: 'Skip, my projects aren’t on GitHub' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT' && c.path === '/api/host/setup')?.body).toEqual({ githubSkipped: true }));
    expect(await within(await step('Connect GitHub')).findByText(/Skipped: projects without GitHub merge locally/)).toBeInTheDocument();
  });

  it('adds a project through the dialog, offering Clone from GitHub only when connected', async () => {
    const p = props();
    stub(view({ steps: { agents: 'done', github: 'done' } }), { 'GET /api/forges/github/login': githubLogin({ accounts: [forgeAccount('me')] }) });
    renderWithQuery(<SetupPage {...p} />);
    const proj = await step('Add a project');
    await userEvent.click(within(proj).getByRole('button', { name: 'Choose a folder…' }));
    expect(p.onAddProject).toHaveBeenCalledWith();
    await userEvent.click(await within(proj).findByRole('button', { name: 'Clone from GitHub…' }));
    expect(p.onAddProject).toHaveBeenCalledWith('clone');
    expect(await within(proj).findByText('/u/work')).toBeInTheDocument();
    await userEvent.click(within(proj).getByRole('link', { name: 'change' }));
    expect(p.onHostSettings).toHaveBeenCalledWith('projects-folder');
  });

  it('wraps a long project path in the step’s summary instead of running off the card', async () => {
    const repoPath = '/private/var/folders/xc/1dctjgbs0wzc9g83j2hjy3qc0000gn/T/scratch/7600/data/website/acme-shop';
    stub(view({ steps: { agents: 'done', github: 'done', project: 'done' } }), { 'GET /api/projects': [{ id: 'p1', name: 'Acme Shop', repoPath }] });
    renderWithQuery(<SetupPage {...props()} />);
    const summary = await within(await step('Add a project')).findByText(`Acme Shop · ${repoPath}`);
    expect(summary).toHaveClass('wrap-anywhere');
  });

  it('hides Clone from GitHub while GitHub is not connected', async () => {
    stub(view({ steps: { agents: 'done', github: 'skipped' } }));
    renderWithQuery(<SetupPage {...props()} />);
    const proj = await step('Add a project');
    expect(within(proj).getByRole('button', { name: 'Choose a folder…' })).toBeInTheDocument();
    expect(within(proj).queryByRole('button', { name: 'Clone from GitHub…' })).not.toBeInTheDocument();
  });

  it('picks a default agent among the ones found', async () => {
    const calls = stub(view({ steps: { agents: 'done', github: 'done', project: 'done' } }), {
      'GET /api/host/checks': checks({ codex: true }),
      'PUT /api/host/settings': { settings: { defaultAgent: 'codex' }, projectsDir: null, programs: [] },
    });
    renderWithQuery(<SetupPage {...props()} />);
    const def = await step('Pick a default agent');
    const group = within(def).getByRole('radiogroup', { name: 'Default agent' });
    expect(await within(group).findByRole('radio', { name: /Devin/ })).toBeDisabled();
    expect(within(group).getByRole('radio', { name: /Claude Code/ })).toHaveAttribute('aria-checked', 'true');
    await userEvent.click(within(group).getByRole('radio', { name: /Codex/ }));
    await userEvent.click(within(def).getByRole('button', { name: 'Use Codex' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT' && c.path === '/api/host/settings')?.body).toEqual({ defaultAgent: 'codex' }));
  });

  it('says so when every step is done, with the board and the PM', async () => {
    const p = props();
    stub(view({ steps: { agents: 'done', github: 'done', project: 'done', defaultAgent: 'done' } }));
    renderWithQuery(<SetupPage {...p} />);
    expect(await screen.findByText(/You’re set up/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Go to the board' }));
    await userEvent.click(screen.getByRole('button', { name: 'Open the PM' }));
    expect(p.onBoard).toHaveBeenCalled();
    expect(p.onPm).toHaveBeenCalled();
  });

  it('dismisses setup on the host', async () => {
    const p = props();
    const calls = stub(view(), { 'PUT /api/host/setup': view({ opensOnLaunch: false }) });
    renderWithQuery(<SetupPage {...p} />);
    expect(await screen.findByText(/Reopen this any time from Settings › This host › Setup/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Dismiss setup' }));
    await waitFor(() => expect(p.onDismissed).toHaveBeenCalled());
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ dismissed: true });
  });
});
