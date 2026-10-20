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

    it('d opens a Devin conversation, e its terminal (with the bypass switch)', async () => {
      const calls = stubApi({ ...routes, 'POST /api/sessions': created });
      await open();
      await userEvent.keyboard('d');
      await waitFor(() => expect(post(calls)).toMatchObject({ kind: 'agent_tui', agent: 'devin', mode: 'conversation' }));
      expect(post(calls)).not.toHaveProperty('bypassPermissions');
      cleanup();
      calls.length = 0;
      await open();
      await userEvent.keyboard('e');
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'devin', mode: 'terminal' }));
      expect(post(calls)).not.toHaveProperty('permissionMode');
      cleanup();
      calls.length = 0;
      const panel = await open();
      await userEvent.click(within(panel).getByRole('switch', { name: /Skip permission prompts/ }));
      await userEvent.click(within(panel).getByRole('menuitem', { name: /^Devin \(terminal\)/ }));
      await waitFor(() => expect(post(calls)).toMatchObject({ agent: 'devin', mode: 'terminal', bypassPermissions: true }));
    });
  });

  describe('agents the host has', () => {
    const tool = (found: boolean, ok: boolean) => ({ found, ok, detail: '' });
    const checks = (devin: { found: boolean; ok: boolean; detail: string }, codex = tool(true, true)) =>
      ({ claude: tool(true, true), codex, devin, git: tool(true, true), tailscale: tool(true, true), checkedAt: 1 });
    const open = () => {
      setNarrow(false);
      renderWithQuery(<NewTabChooser ws={fakeWs()} onClose={() => {}} />);
      return screen.getByRole('dialog', { name: 'New tab' });
    };

    it('offers every agent while the checks load', async () => {
      stubApi({ ...routes, 'GET /api/host/checks': () => new Promise(() => {}) });
      const panel = open();
      expect(within(panel).getByRole('menuitem', { name: /^Devin(?! \(terminal\))/ })).toBeInTheDocument();
      expect(within(panel).getByRole('menuitem', { name: /^Devin \(terminal\)/ })).toBeInTheDocument();
    });

    it('hides an agent that is not installed, and its keys do nothing', async () => {
      const calls = stubApi({ ...routes, 'GET /api/host/checks': checks(tool(false, false)), 'POST /api/sessions': { id: 'n', projectId: project.id } });
      const panel = open();
      await waitFor(() => expect(within(panel).queryByRole('menuitem', { name: /^Devin/ })).toBeNull());
      expect(within(panel).getByRole('menuitem', { name: /^Codex(?! \(terminal\))/ })).toBeInTheDocument();
      await waitFor(() => expect(panel).toHaveFocus());
      await userEvent.keyboard('d');
      await userEvent.keyboard('e');
      expect(calls.some((c) => c.method === 'POST')).toBe(false);
    });

    it('keeps a logged-out agent with a hint', async () => {
      stubApi({ ...routes, 'GET /api/host/checks': checks(tool(true, false)) });
      const panel = open();
      await waitFor(() => expect(within(panel).getByRole('menuitem', { name: /^Devin(?! \(terminal\)).*not logged in/ })).toBeInTheDocument());
      expect(within(panel).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ }).textContent).not.toMatch(/not logged in/);
    });
  });

  it("lists the host's default agent and view first", async () => {
    setNarrow(false);
    stubApi({ ...routes, 'GET /api/host/settings': { settings: { defaultAgent: 'codex', defaultSessionMode: 'terminal' } } });
    renderWithQuery(<NewTabChooser ws={fakeWs()} onClose={() => {}} />);
    const panel = screen.getByRole('dialog', { name: 'New tab' });
    const names = () => within(panel).getAllByRole('menuitem').map((m) => m.textContent ?? '').filter((t) => /^(Claude|Codex|Devin|Shell)/.test(t)).map((t) => t.replace(/[a-z]$/, ''));
    await waitFor(() => expect(names()[0]).toMatch(/^Codex \(terminal\)/));
    expect(names().map((n) => n.replace(/\s*$/, ''))).toEqual(['Codex (terminal)', 'Codex', 'Claude (terminal)', 'Claude', 'Devin (terminal)', 'Devin', 'Shell']);
  });
});
