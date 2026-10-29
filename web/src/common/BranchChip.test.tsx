import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { githubLinks } from '../test-utils';
import { BranchChip, ProjectKey } from './BranchChip';

afterEach(() => vi.restoreAllMocks());

describe('BranchChip', () => {
  it('is a plain chip without a forge', () => {
    render(<BranchChip branch="feature/x" />);
    expect(screen.getByText('feature/x')).toBeInTheDocument();
    expect(screen.queryByRole('button')).not.toBeInTheDocument();
  });

  it('opens the branch and its PR on GitHub from its menu', async () => {
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    render(<BranchChip branch="feature/x" forge={githubLinks('me/pdt')} base="main" />);
    await userEvent.click(screen.getByRole('button', { name: 'Branch feature/x' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Open on GitHub' }));
    expect(open).toHaveBeenLastCalledWith('https://github.com/me/pdt/tree/feature/x', '_blank', 'noopener,noreferrer');
    await userEvent.click(screen.getByRole('button', { name: 'Branch feature/x' }));
    await userEvent.click(screen.getByRole('menuitem', { name: 'Open PR' }));
    expect(open).toHaveBeenLastCalledWith('https://github.com/me/pdt/compare/main...feature/x?expand=1', '_blank', 'noopener,noreferrer');
  });

  it('has no "Open PR" on the base branch', async () => {
    render(<BranchChip branch="main" forge={githubLinks('me/pdt')} base="main" />);
    await userEvent.click(screen.getByRole('button', { name: 'Branch main' }));
    expect(screen.getByRole('menuitem', { name: 'Open on GitHub' })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: 'Open PR' })).not.toBeInTheDocument();
  });

  it('names the forge, and leaves out the pages it has none of', async () => {
    const forge = { ...githubLinks('me/pdt'), kind: 'gitea', name: 'Gitea', compareUrl: null };
    render(<BranchChip branch="feature/x" forge={forge} base="main" />);
    await userEvent.click(screen.getByRole('button', { name: 'Branch feature/x' }));
    expect(screen.getByRole('menuitem', { name: 'Open on Gitea' })).toBeInTheDocument();
    expect(screen.queryByRole('menuitem', { name: 'Open PR' })).not.toBeInTheDocument();
  });
});

describe('ProjectKey', () => {
  it('shows the key', () => {
    render(<ProjectKey projectKey="PDT" />);
    expect(screen.getByText('PDT')).toBeInTheDocument();
  });
});
