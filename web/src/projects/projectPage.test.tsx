import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { AddProjectForm } from './AddProjectForm';
import { ProjectPage } from './ProjectPage';

afterEach(() => vi.unstubAllGlobals());

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', githubRepo: 'o/pdt' };
const wt = (id: string, branch: string, missing = false) => ({ id, projectId: 'p', branch, path: `/r/.worktrees/${branch}`, baseCommit: 'x', createdAt: 1, missing });
const base = {
  'GET /api/projects/p/worktrees': [wt('w1', 'feature/x'), wt('w2', 'old', true)],
  'GET /api/projects/p/settings': { runOverrides: null, agentCredentials: null },
  'GET /api/projects/p/run-config': { file: null, filePath: null, error: null, effective: null },
};

describe('ProjectPage', () => {
  it('renames and re-keys the project', async () => {
    const calls = stubApi({ ...base, 'PATCH /api/projects/p': { ...project, key: 'CORE' } });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} />);
    const key = screen.getByRole('textbox', { name: 'Project key' });
    await userEvent.clear(key);
    await userEvent.type(key, 'core');
    await userEvent.click(screen.getByRole('button', { name: 'Save name and key' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/projects/p', body: { key: 'core' } }));
    expect(screen.getByRole('link', { name: /o\/pdt on GitHub/ })).toHaveAttribute('href', 'https://github.com/o/pdt');
  });

  it('links worktrees to GitHub and removes one, confirming again when it has uncommitted changes', async () => {
    let first = true;
    const calls = stubApi({
      ...base,
      'DELETE /api/worktrees/w1?force=false&deleteBranch=true': () => (first ? ((first = false), new Reply(409, { code: 'worktree_dirty', message: 'dirty' })) : undefined),
      'DELETE /api/worktrees/w1?force=true&deleteBranch=true': undefined,
    });
    renderWithQuery(<ProjectPage project={project} onRemoved={() => {}} />);
    const list = await screen.findByRole('region', { name: 'Worktrees' });
    const row = (await within(list).findByText('feature/x')).closest('li')!;
    expect(within(row).getByRole('link', { name: /Open PR/ })).toHaveAttribute('href', 'https://github.com/o/pdt/compare/main...feature/x?expand=1');
    await userEvent.click(within(row).getByRole('button', { name: 'Remove…' }));
    await userEvent.click(within(row).getByRole('checkbox', { name: 'Also delete the branch' }));
    await userEvent.click(within(row).getByRole('button', { name: 'Remove' }));
    expect(await within(row).findByText(/uncommitted changes/)).toBeInTheDocument();
    await userEvent.click(within(row).getByRole('button', { name: 'Remove anyway' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/worktrees/w1?force=true&deleteBranch=true')).toBe(true));
  });

  it('forgets a missing worktree and removes the project', async () => {
    const onRemoved = vi.fn();
    const calls = stubApi({ ...base, 'DELETE /api/worktrees/w2?force=false&deleteBranch=false': undefined, 'DELETE /api/projects/p': undefined });
    renderWithQuery(<ProjectPage project={project} onRemoved={onRemoved} />);
    const row = (await screen.findByText('old')).closest('li')!;
    expect(row).toHaveTextContent('folder missing');
    await userEvent.click(within(row).getByRole('button', { name: 'Forget' }));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/worktrees/w2?force=false&deleteBranch=false')).toBe(true));
    await userEvent.click(screen.getByRole('button', { name: 'Remove project…' }));
    expect(screen.getByText(/Files on disk stay/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Remove project' }));
    await waitFor(() => expect(onRemoved).toHaveBeenCalled());
  });
});

describe('AddProjectForm', () => {
  it('clones a URL into the Projects folder by default', async () => {
    const onAdded = vi.fn();
    const calls = stubApi({
      'GET /api/host/settings': { settings: {}, projectsDir: '/home/me/work', ownerFromEnv: null, tokenPresent: false, tokenError: null, remote: {} },
      'POST /api/projects/clone': { ...project, id: 'n' },
    });
    renderWithQuery(<AddProjectForm onAdded={onAdded} />);
    await userEvent.click(screen.getByRole('tab', { name: 'Clone URL' }));
    expect(await screen.findByPlaceholderText('/home/me/work')).toBeInTheDocument();
    await userEvent.type(screen.getByLabelText('Repository URL'), 'https://github.com/o/r');
    await userEvent.type(screen.getByLabelText('Key'), 'rr');
    await userEvent.click(screen.getByRole('button', { name: 'Clone and add' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/projects/clone', body: { url: 'https://github.com/o/r', key: 'rr' } }));
    expect(onAdded).toHaveBeenCalledWith(expect.objectContaining({ id: 'n' }));
  });
});
