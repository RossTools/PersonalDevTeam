import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { NewTabChooser } from './NewTabChooser';
import type { WorkspacesApi } from './useWorkspaces';

const project = { id: 'p1', name: 'Alpha', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'ALP', githubRepo: null };

/** Just the parts of the workspaces API the chooser reads. */
function fakeWs(): WorkspacesApi {
  return {
    currentInfo: null,
    workspace: { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] },
    tab: null,
    projects: [project],
    allInfos: [],
    workspaces: [],
    refresh: vi.fn(),
    focusItem: vi.fn(),
    openIn: vi.fn(),
  } as unknown as WorkspacesApi;
}

function Anchored({ onClose }: { onClose: () => void }) {
  const plus = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={plus} type="button">+</button>
      <NewTabChooser ws={fakeWs()} onClose={onClose} anchor={plus} />
    </>
  );
}

const setNarrow = (narrow: boolean) =>
  vi.stubGlobal('matchMedia', (query: string) => ({
    matches: narrow && query.includes('max-width'),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));

afterEach(() => vi.unstubAllGlobals());

describe('NewTabChooser', () => {
  const routes = { [`GET /api/projects/${project.id}/worktrees`]: [], [`GET /api/projects/${project.id}/run-config`]: { effective: null, error: null } };

  it('is a popover at its anchor that closes on Escape', async () => {
    setNarrow(false);
    stubApi(routes);
    const onClose = vi.fn();
    renderWithQuery(<Anchored onClose={onClose} />);
    const panel = screen.getByRole('dialog', { name: 'New tab' });
    expect(panel.closest('[data-slot=popover-content]')).not.toBeNull();
    expect(within(panel).getByRole('menuitem', { name: /^Claude/ })).toBeInTheDocument();
    await waitFor(() => expect(panel).toHaveFocus());
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
  });

  it('is a bottom sheet on a phone', () => {
    setNarrow(true);
    stubApi(routes);
    renderWithQuery(<Anchored onClose={() => {}} />);
    const panel = screen.getByRole('dialog', { name: 'New tab' });
    expect(panel.closest('[data-slot=popover-content]')).toBeNull();
    expect(within(panel).getByRole('menuitem', { name: /^Codex/ })).toBeInTheDocument();
  });
});
