import { screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Project } from '../api/gen/Project';
import type { Worktree } from '../api/gen/Worktree';
import { renderWithQuery, stubApi } from '../test-utils';
import { PlacePicker } from './PlacePicker';

const project = (id: string, name: string, key: string): Project =>
  ({ id, name, key, repoPath: `/r/${id}`, baseBranch: 'main', createdAt: 0, githubRepo: null });
const wt = (id: string, projectId: string, branch: string, missing = false): Worktree =>
  ({ id, projectId, branch, path: `/r/${projectId}/.worktrees/${branch}`, baseCommit: 'abc', createdAt: 0, missing });

const projects = [project('p1', 'PersonalDevTeam', 'PDT'), project('p2', 'Demo', 'DEMO')];

afterEach(() => vi.unstubAllGlobals());

function setup(place = { projectId: 'p1', worktreeId: null as string | null }) {
  const calls = stubApi({
    'GET /api/projects/p1/worktrees': [wt('w1', 'p1', 'phase-3c'), wt('w2', 'p1', 'gone', true)],
    'GET /api/projects/p2/worktrees': [wt('w3', 'p2', 'feature')],
  });
  const onPick = vi.fn();
  renderWithQuery(<PlacePicker projects={projects} place={place} onPick={onPick} />);
  return { onPick, calls };
}

describe('PlacePicker', () => {
  it('shows the place as "Project › place" and fetches only its project until opened', async () => {
    const { calls } = setup({ projectId: 'p1', worktreeId: 'w1' });
    expect(await screen.findByRole('button', { name: 'Place: PersonalDevTeam › phase-3c' })).toBeInTheDocument();
    expect(calls.map((c) => c.path)).toEqual(['/api/projects/p1/worktrees']);
  });

  it('lists every project with its repository and worktrees (not missing ones), and picks one', async () => {
    const user = userEvent.setup();
    const { onPick } = setup();
    await user.click(screen.getByRole('button', { name: /^Place: PersonalDevTeam › Repository/ }));
    const list = screen.getByRole('menu', { name: 'Places' });
    expect(await within(list).findByRole('menuitem', { name: 'feature' })).toBeInTheDocument();
    expect(within(list).getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['Repository', 'phase-3c', 'Repository', 'feature']);
    expect(within(list).queryByText('gone')).not.toBeInTheDocument();
    await user.click(within(list).getByRole('menuitem', { name: 'feature' }));
    expect(onPick).toHaveBeenCalledWith({ projectId: 'p2', worktreeId: 'w3' });
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();
  });

  it('filters by project name or key and picks with the keyboard', async () => {
    const user = userEvent.setup();
    const { onPick } = setup();
    await user.click(screen.getByRole('button', { name: /^Place/ }));
    await screen.findByRole('menuitem', { name: 'feature' });
    await user.keyboard('demo');
    expect(screen.getAllByRole('menuitem').map((e) => e.textContent)).toEqual(['Repository', 'feature']);
    await user.keyboard('{Enter}');
    expect(onPick).toHaveBeenCalledWith({ projectId: 'p2', worktreeId: null });
  });
});
