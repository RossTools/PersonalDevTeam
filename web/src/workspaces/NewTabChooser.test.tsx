import { cleanup, screen, waitFor, within } from '@testing-library/react';
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
    expect(within(panel).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ })).toBeInTheDocument();
    await waitFor(() => expect(panel).toHaveFocus());
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
  });

  it('is a bottom sheet on a phone', async () => {
    setNarrow(true);
    stubApi(routes);
    renderWithQuery(<Anchored onClose={() => {}} />);
    const panel = await screen.findByRole('dialog', { name: 'New tab' });
    expect(panel.closest('[data-slot=popover-content]')).toBeNull();
    expect(within(panel).getByRole('menuitem', { name: /^Codex(?! \(terminal\))/ })).toBeInTheDocument();
  });

  describe('modes', () => {
    const created = { id: 'n', projectId: project.id };
    const post = (calls: { method: string; path: string; body: unknown }[]) =>
      calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body as Record<string, unknown> | undefined;
    const open = async () => {
      setNarrow(false);
      const onClose = vi.fn();
      renderWithQuery(<NewTabChooser ws={fakeWs()} onClose={onClose} />);
      const panel = screen.getByRole('dialog', { name: 'New tab' });
      await waitFor(() => expect(panel).toHaveFocus());
      return panel;
    };

    it('c opens a Claude conversation', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      await open();
      await userEvent.keyboard('c');
      await waitFor(() => expect(post(calls)).toMatchObject({ kind: 'agent_tui', agent: 'claude', mode: 'conversation' }));
      expect(post(calls)).not.toHaveProperty('permissionMode');
      expect(post(calls)).not.toHaveProperty('bypassPermissions');
    });

    it('t opens Claude in its terminal, with the bypass switch', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      const panel = await open();
      await userEvent.click(within(panel).getByRole('switch', { name: /Skip permission prompts/ }));
      await userEvent.click(within(panel).getByRole('menuitem', { name: /^Claude \(terminal\)/ }));
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'claude', mode: 'terminal', bypassPermissions: true }));
    });

    it('sends the picked permission mode with a conversation, never the bypass switch', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      const panel = await open();
      await userEvent.click(within(panel).getByRole('switch', { name: /Skip permission prompts/ }));
      await userEvent.click(within(panel).getByRole('button', { name: /Permission mode for a new conversation/ }));
      await userEvent.click(await screen.findByRole('option', { name: 'Accept edits' }));
      await userEvent.click(within(panel).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ }));
      await waitFor(() => expect(post(calls)).toMatchObject({ mode: 'conversation', permissionMode: 'accept_edits' }));
      expect(post(calls)).not.toHaveProperty('bypassPermissions');
    });

    it('Codex opens a conversation with the picked permission mode', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      const panel = await open();
      await userEvent.click(within(panel).getByRole('button', { name: /Permission mode for a new conversation/ }));
      await userEvent.click(await screen.findByRole('option', { name: 'Plan' }));
      await userEvent.click(within(panel).getByRole('menuitem', { name: /^Codex(?! \(terminal\))/ }));
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'codex', mode: 'conversation', permissionMode: 'plan' }));
      expect(post(calls)).not.toHaveProperty('bypassPermissions');
    });

    it('o opens Codex in its terminal, with the bypass switch', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      const panel = await open();
      await userEvent.click(within(panel).getByRole('switch', { name: /Skip permission prompts/ }));
      await userEvent.click(within(panel).getByRole('menuitem', { name: /^Codex \(terminal\)/ }));
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'codex', mode: 'terminal', bypassPermissions: true }));
      expect(post(calls)).not.toHaveProperty('permissionMode');
    });

    it('x opens a Codex conversation, o its terminal', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      await open();
      await userEvent.keyboard('x');
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'codex', mode: 'conversation' }));
      cleanup();
      calls.length = 0;
      await open();
      await userEvent.keyboard('o');
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'codex', mode: 'terminal' }));
    });

    it('d opens Devin, which is only a terminal, with the bypass switch', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      await open();
      await userEvent.keyboard('d');
      await waitFor(() => expect(post(calls)).toMatchObject({ kind: 'agent_tui', agent: 'devin', mode: 'terminal' }));
      expect(post(calls)).not.toHaveProperty('permissionMode');
      cleanup();
      calls.length = 0;
      const panel = await open();
      await userEvent.click(within(panel).getByRole('switch', { name: /Skip permission prompts/ }));
      await userEvent.click(within(panel).getByRole('menuitem', { name: /^Devin \(terminal\)/ }));
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'devin', mode: 'terminal', bypassPermissions: true }));
    });
  });
});
