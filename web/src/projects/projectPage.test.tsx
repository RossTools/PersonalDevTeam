import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import { githubLinks, renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { AddProjectForm } from './AddProjectForm';
import { ProjectPage } from './ProjectPage';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', forge: githubLinks('o/pdt'), kind: 'repo' };
const wt = (id: string, branch: string, missing = false) => ({ id, projectId: 'p', branch, path: `/r/.worktrees/${branch}`, baseCommit: 'x', createdAt: 1, missing });
const base = {
  'GET /api/projects/p/worktrees': [wt('w1', 'feature/x'), wt('w2', 'old', true)],
  'GET /api/projects/p/settings': { runOverrides: null, agentCredentials: null },
  'GET /api/projects/p/run-config': { file: null, filePath: null, error: null, effective: null },
  'GET /api/projects/p/branches': [{ name: 'main', local: true }, { name: 'release', local: false }],
};

describe('ProjectPage', () => {
  it('saves the name and key as they are left, and follows the new key', async () => {
    const onKeyChange = vi.fn();
    const calls = stubApi({ ...base, 'PATCH /api/projects/p': (c: Call) => ({ ...project, ...(c.body as object), key: 'CORE' }) });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} onKeyChange={onKeyChange} />);
    const form = screen.getByRole('region', { name: 'General' });
    expect(within(form).queryByRole('button', { name: 'Save' })).toBeNull();
    const key = within(form).getByRole('textbox', { name: 'Project key' });
    await userEvent.clear(key);
    await userEvent.type(key, 'core{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { key: 'core' } }));
    expect(await within(form).findByRole('status')).toHaveTextContent('Saved');
    await waitFor(() => expect(onKeyChange).toHaveBeenCalledWith('CORE'));
    expect(screen.getByRole('link', { name: /o\/pdt on GitHub/ })).toHaveAttribute('href', 'https://github.com/o/pdt');
  });

  it('saves the name on blur, and nothing when it did not change', async () => {
    const calls = stubApi({ ...base, 'PATCH /api/projects/p': (c: Call) => ({ ...project, ...(c.body as object) }) });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} />);
    const name = screen.getByRole('textbox', { name: 'Project name' });
    await userEvent.click(name);
    await userEvent.tab();
    expect(calls.some((c) => c.method === 'PATCH')).toBe(false);
    await userEvent.type(name, ' Core');
    await userEvent.tab();
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { name: 'PDT Core' } }));
  });

  it('changes the base branch to a local or origin branch at once', async () => {
    const calls = stubApi({ ...base, 'PATCH /api/projects/p': { ...project, baseBranch: 'release' } });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} />);
    const form = screen.getByRole('region', { name: 'General' });
    await userEvent.click(within(form).getByRole('button', { name: /Base branch/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'release (origin)' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { baseBranch: 'release' } }));
    expect(await within(form).findByRole('status')).toHaveTextContent('Saved');
  });

  it('shows why saving the key failed, and keeps what was typed', async () => {
    stubApi({ ...base, 'PATCH /api/projects/p': new Reply(409, { code: 'key_taken', message: 'the key CORE is taken' }) });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} />);
    const form = screen.getByRole('region', { name: 'General' });
    const key = within(form).getByRole('textbox', { name: 'Project key' });
    await userEvent.clear(key);
    await userEvent.type(key, 'core{Enter}');
    expect(await within(form).findByRole('alert')).toHaveTextContent('the key CORE is taken');
    expect(key).toHaveValue('core');
  });

  it('links worktrees to GitHub and removes one, confirming again when it has uncommitted changes', async () => {
    let first = true;
    const calls = stubApi({
      ...base,
      'DELETE /api/worktrees/w1?force=false&deleteBranch=true': () => (first ? ((first = false), new Reply(409, { code: 'worktree_dirty', message: 'dirty' })) : undefined),
      'DELETE /api/worktrees/w1?force=true&deleteBranch=true': undefined,
    });
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} section="worktrees" />);
    const list = await screen.findByRole('region', { name: 'Worktrees' });
    const row = (await within(list).findByText('feature/x')).closest('li')!;
    await userEvent.click(within(row).getByRole('button', { name: 'Branch feature/x' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Open PR' }));
    expect(open).toHaveBeenCalledWith('https://github.com/o/pdt/compare/main...feature/x?expand=1', '_blank', 'noopener,noreferrer');
    await userEvent.click(within(row).getByRole('button', { name: 'Remove…' }));
    const ask = await screen.findByRole('alertdialog', { name: 'Remove worktree feature/x?' });
    await userEvent.click(within(ask).getByRole('checkbox', { name: 'Also delete the branch' }));
    await userEvent.click(within(ask).getByRole('button', { name: 'Remove worktree' }));
    const again = await screen.findByRole('alertdialog', { name: 'feature/x has uncommitted changes' });
    expect(again).toHaveTextContent(/Removing it discards them/);
    expect(within(again).getByRole('checkbox', { name: 'Also delete the branch' })).toBeChecked();
    await userEvent.click(within(again).getByRole('button', { name: 'Remove anyway' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/worktrees/w1?force=true&deleteBranch=true')).toBe(true));
  });

  it('removes nothing when either worktree confirmation is cancelled', async () => {
    const calls = stubApi({
      ...base,
      'DELETE /api/worktrees/w1?force=false&deleteBranch=false': new Reply(409, { code: 'worktree_dirty', message: 'dirty' }),
    });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} section="worktrees" />);
    const row = (await screen.findByText('feature/x')).closest('li')!;
    await userEvent.click(within(row).getByRole('button', { name: 'Remove…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);
    await userEvent.click(within(row).getByRole('button', { name: 'Remove…' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Remove worktree' }));
    const again = await screen.findByRole('alertdialog', { name: 'feature/x has uncommitted changes' });
    await userEvent.click(within(again).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(calls.filter((c) => c.method === 'DELETE').map((c) => c.path)).toEqual(['/api/worktrees/w1?force=false&deleteBranch=false']);
  });

  it('forgets a missing worktree and removes the project after confirming', async () => {
    const onRemoved = vi.fn();
    const calls = stubApi({ ...base, 'DELETE /api/worktrees/w2?force=false&deleteBranch=false': undefined, 'DELETE /api/projects/p': undefined });
    const { unmount } = renderWithQuery(<ProjectPage project={project} onRemoved={onRemoved} section="worktrees" />);
    const row = (await screen.findByText('old')).closest('li')!;
    expect(row).toHaveTextContent('folder missing');
    await userEvent.click(within(row).getByRole('button', { name: 'Forget' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/worktrees/w2?force=false&deleteBranch=false')).toBe(true));
    unmount();
    renderWithQuery(<ProjectPage project={project} onRemoved={onRemoved} section="danger-zone" />);
    // Cancelling keeps the project.
    await userEvent.click(screen.getByRole('button', { name: 'Remove project…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog', { name: 'Remove PDT?' })).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).toBeNull());
    expect(calls.some((c) => c.path === '/api/projects/p')).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: 'Remove project…' }));
    const ask = await screen.findByRole('alertdialog', { name: 'Remove PDT?' });
    expect(ask).toHaveTextContent(/Files on disk stay/);
    // Removing waits for the project's name to be typed.
    expect(within(ask).getByRole('button', { name: 'Remove project' })).toBeDisabled();
    await userEvent.type(within(ask).getByRole('textbox', { name: 'Type PDT to confirm' }), 'PDT');
    await userEvent.click(within(ask).getByRole('button', { name: 'Remove project' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
    expect(calls).toContainEqual({ method: 'DELETE', path: '/api/projects/p', body: undefined });
  });

  it('sets and clears the permission mode for workflow steps, sending the other settings back', async () => {
    const calls = stubApi({
      ...base,
      'GET /api/projects/p/settings': { runOverrides: 'x = 1', agentCredentials: 'subscription', agentPermissionMode: null },
      'PUT /api/projects/p/settings': (c: Call) => ({ runOverrides: 'x = 1', agentCredentials: 'subscription', ...(c.body as object) }),
    });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} section="agents" />);
    const trigger = await screen.findByRole('button', { name: /Agent permission mode for workflow steps/ });
    await waitFor(() => expect(trigger).toBeEnabled());
    await userEvent.click(trigger);
    await userEvent.click(await screen.findByRole('option', { name: 'Plan' }));
    await waitFor(() => expect(calls).toContainEqual({
      method: 'PUT', path: '/api/projects/p/settings', body: { agentCredentials: 'subscription', runOverrides: 'x = 1', agentPermissionMode: 'plan' },
    }));
    await userEvent.click(screen.getByRole('button', { name: /Agent permission mode for workflow steps/ }));
    await userEvent.click(await screen.findByRole('option', { name: /^None/ }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').at(-1)?.body).toMatchObject({ agentPermissionMode: '' }));
  });

  it('shows a Personal project as a folder, with no base branch, worktrees, runs or merging', async () => {
    const personal: Project = { ...project, kind: 'personal', baseBranch: '', forge: null, repoPath: '/data/personal/p' };
    const calls = stubApi(base);
    const { unmount } = renderWithQuery(<ProjectPage project={personal} onRemoved={() => {}} />);
    const form = screen.getByRole('region', { name: 'General' });
    expect(within(form).getByText(/Personal project \(no repository\)/)).toBeInTheDocument();
    expect(within(form).getByText('/data/personal/p')).toBeInTheDocument();
    expect(within(form).queryByRole('button', { name: /Base branch/ })).toBeNull();
    unmount();
    // On a phone its list of sections has no worktrees, runs or review.
    renderWithQuery(<ProjectPage project={personal} onRemoved={() => {}} drill />);
    const sections = screen.getByRole('grid', { name: 'Sections' });
    expect(within(sections).queryByText('Worktrees')).toBeNull();
    expect(within(sections).queryByText('Runs on this host')).toBeNull();
    expect(within(sections).queryByText('Review')).toBeNull();
    expect(within(sections).getByText('Agents')).toBeInTheDocument();
    expect(calls.some((c) => c.path.includes('/branches') || c.path.includes('/worktrees') || c.path.includes('/run-config'))).toBe(false);
  });
});

describe('AddProjectForm', () => {
  it('adds a Personal project with a name and key only', async () => {
    const calls = stubApi({
      'GET /api/host/settings': { settings: {}, projectsDir: '/home/me/work', ownerFromEnv: null, tokenPresent: false, tokenError: null, remote: {} },
      'POST /api/projects': { ...project, id: 'n', kind: 'personal' },
    });
    renderWithQuery(<AddProjectForm />);
    await userEvent.click(screen.getByRole('radio', { name: 'Personal (no repo)' }));
    expect(screen.queryByText('Repository path')).toBeNull();
    await userEvent.type(screen.getByLabelText('Name'), 'Errands');
    await userEvent.type(screen.getByLabelText('Key'), 'err');
    await userEvent.click(screen.getByRole('button', { name: 'Add project' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/projects', body: { name: 'Errands', repoPath: '', kind: 'personal', key: 'err' } }));
  });

  it('clones a URL into the Projects folder by default', async () => {
    const onAdded = vi.fn();
    const calls = stubApi({
      'GET /api/host/settings': { settings: {}, projectsDir: '/home/me/work', ownerFromEnv: null, tokenPresent: false, tokenError: null, remote: {} },
      'POST /api/projects/clone': { ...project, id: 'n' },
    });
    renderWithQuery(<AddProjectForm onAdded={onAdded} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Clone URL' }));
    expect(await screen.findByPlaceholderText('/home/me/work')).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText('Repository URL'), 'https://github.com/o/r');
    await userEvent.type(screen.getByLabelText('Key'), 'rr');
    await userEvent.click(screen.getByRole('button', { name: 'Clone and add' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/projects/clone', body: { url: 'https://github.com/o/r', key: 'rr' } }));
    expect(onAdded).toHaveBeenCalledWith(expect.objectContaining({ id: 'n' }));
  });
});
