import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, type Call } from '../test-utils';
import { AppShell } from './AppShell';
import { project, stubPhone, sess, tabOf, stubWorkspaces, runConfig, runOf } from './fixtures';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

describe('workspaces', () => {
  beforeEach(() => { document.title = 'PDT'; });
  afterEach(() => { document.title = ''; });

  it('lists ticket workspaces apart, with the ticket, its PR and its agent state', async () => {
    const ticketWs = {
      id: 'wt', name: 'ALP-7', projectId: null, position: 1, createdAt: 3, tabs: [tabOf('td', 'wt', 'd', 0)],
      ticket: {
        id: 't7', projectId: project.id, key: 'ALP-7', title: 'Fix login', stage: 'in_progress', attemptId: 'a7', attemptState: 'active',
        pr: {
          number: 48, url: 'https://gh/pull/48', title: 'Fix login', state: 'open', draft: false, baseBranch: 'main', headSha: 'h',
          mergeable: 'mergeable', behind: false, autoMerge: true, inMergeQueue: false, queuePosition: null, queueCi: null, checkedAt: 1, error: null,
          ci: { state: 'failed', url: 'https://gh/runs/1', runs: [] },
        },
      },
    };
    stubWorkspaces({
      'GET /api/workspaces': [
        { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [tabOf('ta', 'wa', 'a', 0)] },
        ticketWs,
        { id: 'wr', name: 'Release', projectId: null, position: 2, createdAt: 2, tabs: [] },
      ],
    });
    renderWithQuery(<AppShell />);
    const tickets = await screen.findByRole('region', { name: 'Ticket workspaces' });
    const row = await within(tickets).findByRole('row', { name: /Fix login/ });
    expect(within(row).getByRole('button', { name: 'ALP-7' })).toBeInTheDocument();
    expect(within(row).getByText(/#48 · CI failed/)).toBeInTheDocument();
    expect(within(row).getByText('Done')).toBeInTheDocument();
    const general = screen.getByRole('region', { name: 'Workspaces' });
    expect(within(general).queryByRole('row', { name: /Fix login|ALP-7/ })).toBeNull();
    expect(within(general).getByRole('row', { name: /Release/ })).toBeInTheDocument();
  });

  it('shows the workspace tabs with their status and the current tab', async () => {
    // The title counts the Inbox's items only (PDT-207): one question; B needing the user and D's unseen reply don't add.
    const question = {
      id: 'q', kind: 'question', group: 'question', status: 'open', projectId: project.id, ticketId: null, ticketKey: null,
      ticketTitle: null, attemptId: null, sessionId: null, runId: null, stepId: null, personaId: null, personaName: null,
      title: 'Which file?', body: {}, choices: [], resolution: null, createdAt: 1, resolvedAt: null,
    };
    stubWorkspaces({ 'GET /api/inbox': { items: [question], total: 1 } });
    renderWithQuery(<AppShell />);
    const tabs = await screen.findByRole('tablist', { name: 'Tabs' });
    await within(tabs).findByRole('tab', { name: /Claude A/ });
    const b = within(tabs).getByRole('tab', { name: /Codex B/ });
    expect(within(b).getByRole('img', { name: 'unseen' })).toBeInTheDocument();
    expect(within(tabs).getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true');
    expect(await within(screen.getByRole('main')).findByTestId('terminal-view')).toHaveTextContent('a');
    await userEvent.click(b);
    expect(b).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('main').querySelector('[data-testid="terminal-view"]')).toHaveTextContent('b');
    await waitFor(() => expect(document.title).toBe('(1) PDT'));
  });

  it('scrolls the tab strip to the current tab when it changes from the sidebar', async () => {
    const scroll = vi.fn();
    const original = Element.prototype.scrollIntoView;
    Element.prototype.scrollIntoView = scroll;
    try {
      stubWorkspaces({});
      renderWithQuery(<AppShell />);
      const agents = await screen.findByRole('region', { name: 'Agents' });
      await userEvent.click(await within(agents).findByRole('row', { name: /Codex B/ }));
      await waitFor(() => expect(scroll.mock.contexts.some((el) => (el as HTMLElement).getAttribute('aria-selected') === 'true'
        && (el as HTMLElement).textContent?.includes('Codex B'))).toBe(true));
    } finally {
      Element.prototype.scrollIntoView = original;
    }
  });
  it('lists agents in the sidebar; clicking one focuses its tab, or opens one when it has none', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wa/tabs': tabOf('td', 'wa', 'd', 2) });
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const b = await within(agents).findByRole('row', { name: /Codex B/ });
    expect(b).toHaveTextContent('Needs you');
    expect(within(agents).queryByRole('row', { name: /Shell C/ })).not.toBeInTheDocument();
    await userEvent.click(b);
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    const d = within(agents).getByRole('row', { name: /Claude D/ });
    expect(d).toHaveTextContent('Done');
    expect(d).toHaveTextContent('no tab');
    await userEvent.click(d);
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wa/tabs', body: { sessionId: 'd' } }));
  });
  // A ticket's running Coder in a tab and an ended one, an ended Triage in another workspace's tab, an ended one with no
  // tab and a running one, and the user's own conversation on the same ticket.
  const stepSessions = () => {
    const forTicket = { attemptId: 'at1', ticket: { id: 't1', key: 'PDT-1' } };
    const step = (id: string, title: string, name: string, status: string, more: Record<string, unknown> = {}) =>
      sess(id, title, { kind: 'agent_tui', agent: 'claude', status, ...more, persona: { id: name, name, version: 1, stepId: `st-${id}` } });
    return {
      [`GET /api/projects/${project.id}/sessions`]: [
        sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude', activity: 'working' }),
        step('c1', 'Coder in a tab', 'Coder', 'running', forTicket),
        step('c2', 'Coder done', 'Coder', 'finished', forTicket),
        step('t1', 'Triage in a tab', 'Triage', 'finished'),
        step('t2', 'Triage running', 'Triage', 'running'),
        step('t3', 'Triage done', 'Triage', 'finished'),
        sess('u1', 'Started by hand', { kind: 'agent_tui', agent: 'claude', status: 'running', ...forTicket, persona: { id: 'co', name: 'Coder', version: 1, stepId: null } }),
      ],
      'GET /api/workspaces': [
        { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [tabOf('ta', 'wa', 'a', 0), tabOf('tc1', 'wa', 'c1', 1)] },
        { id: 'wr', name: 'Release', projectId: null, position: 1, createdAt: 2, tabs: [tabOf('tt1', 'wr', 't1', 0)] },
      ],
    };
  };
  const expectOnlyOwn = async (agents: HTMLElement) => {
    await within(agents).findByRole('row', { name: /Started by hand/ });
    expect(within(agents).getByRole('row', { name: /Claude A/ })).toBeInTheDocument();
    for (const name of [/Coder in a tab/, /Coder done/, /Triage in a tab/, /Triage running/, /Triage done/]) {
      expect(within(agents).queryByRole('row', { name })).not.toBeInTheDocument();
    }
  };
  it('lists only the user\'s own conversations in Agents: workflow step sessions never, even in a tab', async () => {
    stubWorkspaces(stepSessions());
    renderWithQuery(<AppShell />);
    await expectOnlyOwn(await screen.findByRole('region', { name: 'Agents' }));
    // Their tabs stay.
    expect(screen.getByRole('tab', { name: /Coder in a tab/ })).toBeInTheDocument();
  });
  it('finds a step session in the palette and Bring here only while a tab holds it (or it runs with no ticket)', async () => {
    stubWorkspaces(stepSessions());
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Coder in a tab/ });
    await userEvent.keyboard('{Control>}]{/Control}k');
    const palette = await screen.findByRole('dialog', { name: 'Command palette' });
    await userEvent.keyboard('Triage');
    expect(await within(palette).findByRole('menuitem', { name: /Triage in a tab/ })).toBeInTheDocument();
    expect(within(palette).getByRole('menuitem', { name: /Triage running/ })).toBeInTheDocument();
    expect(within(palette).queryByRole('menuitem', { name: /Triage done/ })).not.toBeInTheDocument();
    await userEvent.clear(within(palette).getByRole('searchbox'));
    await userEvent.keyboard('Coder');
    expect(await within(palette).findByRole('menuitem', { name: /Coder in a tab/ })).toBeInTheDocument();
    expect(within(palette).queryByRole('menuitem', { name: /Coder done/ })).not.toBeInTheDocument();
    // The first Escape clears the query, the second closes.
    await userEvent.keyboard('{Escape}{Escape}');
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Command palette' })).not.toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const list = within(screen.getByRole('dialog', { name: 'New tab' })).getByRole('menu', { name: 'New tab choices' });
    expect(await within(list).findByRole('menuitem', { name: /Triage in a tab/ })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: /Triage running/ })).toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: /Triage done/ })).not.toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: /Coder done/ })).not.toBeInTheDocument();
  });
  it('lists only the user\'s own conversations in the phone drawer\'s Agents', async () => {
    window.history.replaceState(null, '', '/sessions');
    stubPhone();
    stubWorkspaces(stepSessions());
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Alpha/ });
    await userEvent.click(screen.getByRole('button', { name: 'Menu' }));
    const d = await screen.findByRole('dialog', { name: 'Menu' });
    await expectOnlyOwn(within(d).getByRole('region', { name: 'Agents' }));
  });
  it('shows each agent\'s branch, and its workspace only when it is not the current one', async () => {
    stubWorkspaces({
      [`GET /api/projects/${project.id}/worktrees`]: [{ id: 'wt1', projectId: project.id, branch: 'feat/login', path: '/r/.worktrees/feat-login', baseCommit: 'x', createdAt: 2 }],
      [`GET /api/projects/${project.id}/sessions`]: [
        sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude', activity: 'working' }),
        sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', worktreeId: 'wt1', cwd: '/r/.worktrees/feat-login' }),
        sess('d', 'Claude D', { kind: 'agent_tui', agent: 'claude' }),
      ],
    });
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const secondary = (name: RegExp) => within(agents).getByRole('row', { name }).querySelector('[data-slot=list-secondary]')?.textContent ?? null;
    await within(agents).findByRole('row', { name: /Codex B/ });
    // In the current workspace (Alpha): the main checkout says nothing, a worktree its branch.
    await waitFor(() => expect(secondary(/Codex B/)).toBe('feat/login'));
    expect(secondary(/Claude A/)).toBeNull();
    expect(secondary(/Claude D/)).toBe('no tab');
    // From another workspace, the agents' workspace follows.
    await userEvent.click(within(screen.getByRole('region', { name: 'Workspaces' })).getByRole('row', { name: /Release/ }));
    await waitFor(() => expect(secondary(/Codex B/)).toBe('feat/login · Alpha'));
    expect(secondary(/Claude A/)).toBe('Alpha');
  });

  it('opens a session\'s changes in the workspace on screen, after its tab', async () => {
    const calls = stubWorkspaces({
      [`GET /api/projects/${project.id}/worktrees`]: [{ id: 'wt1', projectId: project.id, branch: 'feat-login', path: '/r/.worktrees/feat-login', baseCommit: 'x', createdAt: 2 }],
      [`GET /api/projects/${project.id}/sessions`]: [sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', worktreeId: 'wt1', cwd: '/r/.worktrees/feat-login' })],
      'GET /api/workspaces': [
        { id: 'wa', name: 'Alpha', projectId: project.id, position: 0, createdAt: 1, tabs: [] },
        { id: 'wr', name: 'Release', projectId: null, position: 1, createdAt: 2, tabs: [tabOf('tb', 'wr', 'b', 0)] },
      ],
      'POST /api/workspaces/wr/tabs': { id: 'tc', workspaceId: 'wr', sessionId: null, runId: null, worktreeId: 'wt1', position: 1 },
    });
    renderWithQuery(<AppShell />);
    await userEvent.click(await within(await screen.findByRole('region', { name: 'Workspaces' })).findByRole('row', { name: /Release/ }));
    await userEvent.click(await screen.findByRole('button', { name: /^Changes/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wr/tabs', body: { worktreeId: 'wt1', position: 1 } }));
    expect(calls.some((c) => c.path === '/api/workspaces/wa/tabs')).toBe(false);
  });

  it('keeps the agents list a plain view: no drag handles, and a menu without tab movement', async () => {
    stubWorkspaces({});
    renderWithQuery(<AppShell />);
    const agents = await screen.findByRole('region', { name: 'Agents' });
    const b = await within(agents).findByRole('row', { name: /Codex B/ });
    expect(agents.querySelector('[data-drag-handle]')).toBeNull();
    // The workspaces list above still has them (workspaces reorder by dragging).
    expect(screen.getByRole('region', { name: 'Workspaces' }).querySelector('[data-drag-handle]')).not.toBeNull();
    fireEvent.contextMenu(b);
    const menu = await screen.findByRole('menu', { name: 'Agent Codex B' });
    expect(within(menu).getAllByRole('menuitem').map((i) => i.textContent)).toEqual(['Open', 'Rename']);
  });
  it('asks before closing a running session\'s tab, which stops and removes it', async () => {
    const calls = stubWorkspaces({ 'DELETE /api/tabs/tb': undefined });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'Close Codex B' }));
    let dialog = await screen.findByRole('alertdialog', { name: 'Close Codex B?' });
    expect(dialog).toHaveTextContent(/stops the agent and removes it/);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(calls.some((c) => c.method === 'DELETE')).toBe(false);

    await userEvent.click(screen.getByRole('button', { name: 'Close Codex B' }));
    dialog = await screen.findByRole('alertdialog', { name: 'Close Codex B?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Stop and close' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tb')).toBe(true));
  });

  it('closes an ended session\'s tab without asking', async () => {
    const calls = stubWorkspaces({
      [`GET /api/projects/${project.id}/sessions`]: [
        sess('a', 'Claude A', { kind: 'agent_tui', agent: 'claude' }),
        sess('b', 'Codex B', { kind: 'agent_tui', agent: 'codex', status: 'finished' }),
      ],
      'DELETE /api/tabs/tb': undefined,
    });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await waitFor(() => expect(within(screen.getByRole('tab', { name: /Codex B/ })).queryByRole('img', { name: 'Ended' })).toBeTruthy());
    await userEvent.click(screen.getByRole('button', { name: 'Close Codex B' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tabs/tb')).toBe(true));
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('switches tabs and opens the key help with the prefix keys', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await userEvent.keyboard('{Control>}]{/Control}2');
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.keyboard('{Control>}]{/Control}p');
    expect(screen.getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.keyboard('{Control>}]{/Control}?');
    expect(screen.getByRole('dialog', { name: 'Keyboard shortcuts' })).toBeInTheDocument();
  });

  it('jumps to the next session that needs you', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await userEvent.keyboard('{Control>}]{/Control}a');
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
  });

  it('launches an agent from the new-tab chooser right after the current tab', async () => {
    const created = sess('d', 'Claude D', { kind: 'agent_tui', agent: 'claude' });
    const calls = stubWorkspaces({ 'POST /api/sessions': created });
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.keyboard('{Control>}]{/Control}c');
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await waitFor(() => expect(chooser).toHaveFocus());
    await userEvent.keyboard('c');
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body).toMatchObject({
      projectId: project.id, kind: 'agent_tui', agent: 'claude', tab: { workspaceId: 'wa', position: 1 },
    }));
  });

  it('keeps focus in the chooser\'s menus when the screen refreshes', async () => {
    stubWorkspaces();
    const { rerender } = renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await waitFor(() => expect(chooser).toHaveFocus());
    const place = within(chooser).getByRole('button', { name: /^Place:/ });
    place.focus();
    rerender(<AppShell />);
    expect(place).toHaveFocus();
  });

  it('brings an existing session into the workspace from the chooser', async () => {
    const calls = stubWorkspaces({ 'POST /api/workspaces/wr/tabs': tabOf('ta', 'wr', 'a', 0) });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    await userEvent.click(await within(list).findByRole('row', { name: /Release/ }));
    expect(within(list).getByRole('row', { name: /Release/ })).toHaveAttribute('data-current', 'true');
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const choices = within(screen.getByRole('dialog', { name: 'New tab' })).getByRole('menu', { name: 'New tab choices' });
    // Its workspace follows the title.
    await userEvent.click(within(choices).getByRole('menuitem', { name: /Claude A Alpha/ }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces/wr/tabs', body: { sessionId: 'a', position: 0 } }));
  });
  it('creates, renames and deletes workspaces from the sidebar', async () => {
    const calls = stubWorkspaces({
      'POST /api/workspaces': { id: 'wn', name: 'Spike', projectId: null, position: 2, createdAt: 3, tabs: [] },
      'PATCH /api/workspaces/wr': { id: 'wr', name: 'Ship', projectId: null, position: 1, createdAt: 2, tabs: [] },
      'DELETE /api/workspaces/wr': undefined,
    });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    await userEvent.click(within(list).getByRole('button', { name: 'New workspace' }));
    await userEvent.type(within(list).getByRole('textbox', { name: 'New workspace name' }), 'Spike{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/workspaces', body: { name: 'Spike' } }));

    fireEvent.contextMenu(within(list).getByRole('row', { name: /Release/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Release' })).getByRole('menuitem', { name: 'Rename' }));
    const name = within(list).getByRole('textbox', { name: 'Workspace name' });
    await userEvent.clear(name);
    await userEvent.type(name, 'Ship{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/workspaces/wr', body: { name: 'Ship' } }));

    fireEvent.contextMenu(within(list).getByRole('row', { name: /Release/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Release' })).getByRole('menuitem', { name: /Delete/ }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/workspaces/wr')).toBe(true));

    // A project's own workspace can be neither renamed nor deleted.
    fireEvent.contextMenu(within(list).getByRole('row', { name: /Alpha/ }));
    const menu = screen.getByRole('menu', { name: 'Workspace Alpha' });
    expect(within(menu).queryByRole('menuitem', { name: 'Rename' })).not.toBeInTheDocument();
    expect(within(menu).queryByRole('menuitem', { name: /Delete/ })).not.toBeInTheDocument();
  });
  it('moves tabs from the right-click menu and by dragging', async () => {
    const calls = stubWorkspaces({ 'PATCH /api/tabs/tb': tabOf('tb', 'wa', 'b', 0), 'PATCH /api/tabs/ta': tabOf('ta', 'wr', 'a', 0) });
    renderWithQuery(<AppShell />);
    fireEvent.contextMenu(await screen.findByRole('tab', { name: /Claude A/ }));
    const menu = screen.getByRole('menu', { name: 'Tab Claude A' });
    for (const item of ['Rename', 'Move left', 'Move right', 'Release', 'Close tab']) {
      expect(within(menu).getByRole('menuitem', { name: item })).toBeInTheDocument();
    }
    expect(within(menu).getByRole('menuitem', { name: 'New workspace…' })).toBeInTheDocument();
    await userEvent.click(within(menu).getByRole('menuitem', { name: 'Release' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/tabs/ta', body: { workspaceId: 'wr' } }));
    expect(screen.queryByRole('menu')).not.toBeInTheDocument();

    const data: Record<string, string> = {};
    const dataTransfer = {
      setData: (k: string, v: string) => { data[k] = v; },
      getData: (k: string) => data[k] ?? '',
      get types() { return Object.keys(data); },
      effectAllowed: 'move',
    };
    fireEvent.dragStart(screen.getByRole('tab', { name: /Codex B/ }), { dataTransfer });
    const target = screen.getByRole('tab', { name: /Claude A/ }).parentElement!;
    fireEvent.dragOver(target, { dataTransfer });
    fireEvent.drop(target, { dataTransfer });
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/tabs/tb', body: { workspaceId: 'wa', position: 0 } }));
  });
  it('opens project settings from the sidebar, which has no Add project link (that is in Settings)', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    expect(screen.queryByRole('button', { name: 'Add project' })).not.toBeInTheDocument();
    fireEvent.contextMenu(await within(list).findByRole('row', { name: /Alpha/ }));
    await userEvent.click(within(screen.getByRole('menu', { name: 'Workspace Alpha' })).getByRole('menuitem', { name: 'Project settings…' }));
    expect(await within(screen.getByRole('main')).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
  });

  it('rolls up each workspace to its most urgent agent', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    const alpha = await within(list).findByRole('row', { name: /Alpha/ });
    await waitFor(() => expect(alpha).toHaveTextContent('Needs you'));
    expect(within(alpha).getByRole('img', { name: 'Needs you' })).toBeInTheDocument();
    expect(within(list).getByRole('row', { name: /Release/ })).not.toHaveTextContent('Needs you');
  });

  it('focuses the workspace list with Ctrl+] w', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const list = await screen.findByRole('region', { name: 'Workspaces' });
    await within(list).findByRole('row', { name: /Alpha/ });
    await userEvent.keyboard('{Control>}]{/Control}w');
    await waitFor(() => expect(within(list).getByRole('row', { name: /Alpha/ })).toHaveFocus());
    await userEvent.keyboard('{ArrowDown}');
    expect(within(list).getByRole('row', { name: /Release/ })).toHaveFocus();
  });

  it('starts an agent with skip-permissions in another place from the chooser', async () => {
    const calls = stubWorkspaces({
      [`GET /api/projects/${project.id}/worktrees`]: [{ id: 'wt1', projectId: project.id, branch: 'feat', path: '/r/.worktrees/feat', baseCommit: 'x', createdAt: 2 }],
      'POST /api/sessions': sess('e', 'Codex E', { kind: 'agent_tui', agent: 'codex' }),
    });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await userEvent.click(within(chooser).getByRole('button', { name: /^Place:/ }));
    await userEvent.click(await within(screen.getByRole('menu', { name: 'Places' })).findByRole('menuitem', { name: 'feat' }));
    await within(chooser).findByRole('button', { name: 'Place: Alpha › feat' });
    await userEvent.click(within(chooser).getByRole('switch', { name: /Skip permission prompts/ }));
    await userEvent.click(within(chooser).getByRole('menuitem', { name: /^Codex \(terminal\)/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body).toMatchObject({
      worktreeId: 'wt1', agent: 'codex', mode: 'terminal', bypassPermissions: true,
    }));
  });

  it('manages projects in the Settings view', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/settings`]: { overrides: null, agentCredentials: null, effective: null, error: null }, [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null } });
    renderWithQuery(<AppShell />);
    await userEvent.click(screen.getByRole('button', { name: 'Settings' }));
    const nav = await screen.findByRole('navigation', { name: 'Settings sections' });
    const alpha = await within(nav).findByRole('button', { name: /Alpha/ });
    expect(alpha).toHaveTextContent('ALP');
    expect(alpha).toHaveAttribute('aria-expanded', 'false');
    await userEvent.click(alpha);
    await userEvent.click(within(within(nav).getByRole('group', { name: 'Alpha' })).getByRole('button', { name: 'General' }));
    expect(window.location.pathname).toBe('/settings/projects/ALP/general');
    expect(within(within(nav).getByRole('group', { name: 'Alpha' })).getByRole('button', { name: 'General' })).toHaveAttribute('aria-current', 'page');
    expect(await within(screen.getByRole('main')).findByRole('textbox', { name: /^(Project )?key$/i })).toHaveValue('ALP');
    expect(screen.getByRole('button', { name: 'Add project' })).toBeInTheDocument();
  });

  it('shows one state dot per tab, plus the unseen dot, then what the session runs', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    const a = await screen.findByRole('tab', { name: /Claude A/ });
    expect(within(a).getAllByRole('img').map((e) => e.getAttribute('aria-label'))).toEqual(['Working', 'Claude']);
    const b = screen.getByRole('tab', { name: /Codex B/ });
    expect(within(b).getAllByRole('img').map((e) => e.getAttribute('aria-label'))).toEqual(['Needs you', 'unseen', 'Codex']);
  });

  it('toasts when a tab action fails', async () => {
    stubWorkspaces({ 'DELETE /api/tabs/tb': new Reply(500, { code: 'internal', message: 'Tab store is locked' }) });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('tab', { name: /Codex B/ }));
    await waitFor(() => expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true'));
    await userEvent.click(await screen.findByRole('button', { name: 'Close Codex B' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Stop and close' }));
    expect(await screen.findByText('Tab store is locked')).toBeInTheDocument();
    // The tab that failed to close is still the one shown.
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    fireEvent.contextMenu(screen.getByRole('tab', { name: /Claude A/ }));
    stubWorkspaces({ 'PATCH /api/tabs/ta': new Reply(409, { code: 'conflict', message: 'Workspace is gone' }) });
    await userEvent.click(within(screen.getByRole('menu', { name: 'Tab Claude A' })).getByRole('menuitem', { name: 'Release' }));
    expect(await screen.findByText('Workspace is gone')).toBeInTheDocument();
  });

  it('does not launch an agent on type-ahead in the chooser\'s run-another-branch pickers', async () => {
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig, [`GET /api/projects/${project.id}/branches`]: [] });
    renderWithQuery(<AppShell />);
    await userEvent.click(await screen.findByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: 'Run another branch…' }));
    await userEvent.click(await within(chooser).findByRole('button', { name: /What to run/ }));
    const listbox = await screen.findByRole('listbox');
    await waitFor(() => expect(listbox).toContainElement(document.activeElement as HTMLElement));
    await userEvent.keyboard('c');
    await userEvent.keyboard('{Escape}');
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/sessions')).toBe(false);
  });

});

describe('new-tab chooser', () => {
  const sessionPost = (calls: Call[]) => calls.find((c) => c.method === 'POST' && c.path === '/api/sessions')?.body;
  async function open() {
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Claude A/ });
    await userEvent.click(screen.getByRole('button', { name: 'New tab' }));
    const chooser = screen.getByRole('dialog', { name: 'New tab' });
    await waitFor(() => expect(chooser).toHaveFocus());
    return chooser;
  }

  it('groups Start here, Run and Bring here, with the keys and the footer', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig });
    const chooser = await open();
    const list = within(chooser).getByRole('menu', { name: 'New tab choices' });
    for (const g of ['Start here', 'Run', 'Bring here']) expect(within(list).getByText(g)).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ })).toHaveTextContent('c');
    expect(within(list).getByRole('menuitem', { name: /^Claude \(terminal\)/ })).toHaveTextContent('t');
    expect(within(list).getByRole('menuitem', { name: /^Codex(?! \(terminal\))/ })).toHaveTextContent('x');
    expect(within(list).getByRole('menuitem', { name: /^Codex \(terminal\)/ })).toHaveTextContent('o');
    expect(await within(list).findByRole('menuitem', { name: 'Run web' })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: 'Test check' })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: 'Run another branch…' })).toBeInTheDocument();
    // Sessions without a tab here: c and d (no workspace).
    expect(within(list).getByRole('menuitem', { name: /Shell C/ })).toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: /Codex B/ })).not.toBeInTheDocument();
    expect(within(chooser).getByRole('switch', { name: /Skip permission prompts/ })).toBeInTheDocument();
    expect(within(chooser).getByRole('button', { name: /Permission mode for a new conversation/ })).toBeInTheDocument();
    expect(within(chooser).getByText('Esc to close')).toBeInTheDocument();
  });

  it.each([
    ['Shell', /^Shell/, 's', 'shell'],
    ['Codex', /^Codex(?! \(terminal\))/, 'x', 'agent_tui'],
    ['Codex (terminal)', /^Codex \(terminal\)/, 'o', 'agent_tui'],
  ] as const)('launches %s by click and by its key', async (_label, name, key, kind) => {
    const calls = stubWorkspaces({ 'POST /api/sessions': sess('e', 'New', { kind }) });
    const chooser = await open();
    await userEvent.click(within(chooser).getByRole('menuitem', { name }));
    await waitFor(() => expect(sessionPost(calls)).toMatchObject({ kind, tab: { workspaceId: 'wa', position: 1 } }));
    cleanup();
    calls.length = 0;
    await open();
    await userEvent.keyboard(key);
    await waitFor(() => expect(sessionPost(calls)).toMatchObject({ kind }));
  });

  it('types other keys into the filter, which filters every group and turns the shortcuts off', async () => {
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig });
    const chooser = await open();
    const list = within(chooser).getByRole('menu', { name: 'New tab choices' });
    await within(list).findByRole('menuitem', { name: 'Run web' });
    await userEvent.keyboard('hell');
    const filter = within(chooser).getByRole('searchbox');
    expect(filter).toHaveFocus();
    expect(filter).toHaveValue('hell');
    expect(within(list).getByRole('menuitem', { name: /^Shell(?! C)/ })).toBeInTheDocument();
    expect(within(list).getByRole('menuitem', { name: /Shell C/ })).toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: /^Claude/ })).not.toBeInTheDocument();
    expect(within(list).queryByRole('menuitem', { name: 'Run web' })).not.toBeInTheDocument();
    expect(within(list).queryByText('Run')).not.toBeInTheDocument();
    // With text in the filter, c / x / s are just letters.
    await userEvent.keyboard('{Backspace}{Backspace}{Backspace}{Backspace}x');
    expect(filter).toHaveValue('x');
    expect(sessionPost(calls)).toBeUndefined();
  });

  it('starts a configured run after the current tab', async () => {
    const calls = stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig, [`POST /api/projects/${project.id}/runs`]: runOf('r9'),
      'POST /api/workspaces/wa/tabs': { id: 'tr', workspaceId: 'wa', sessionId: null, runId: 'r9', position: 1 },
    });
    const chooser = await open();
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: 'Test check' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === `/api/projects/${project.id}/runs`)?.body).toEqual({
      name: 'check', kind: 'test', tab: { workspaceId: 'wa', position: 1 },
    }));
  });

  it('swaps the list for the branch form and back', async () => {
    stubWorkspaces({ [`GET /api/projects/${project.id}/run-config`]: runConfig, [`GET /api/projects/${project.id}/branches`]: [] });
    const chooser = await open();
    await userEvent.click(await within(chooser).findByRole('menuitem', { name: 'Run another branch…' }));
    expect(within(chooser).queryByRole('menu', { name: 'New tab choices' })).not.toBeInTheDocument();
    expect(await within(chooser).findByRole('button', { name: /^Branch/ })).toBeInTheDocument();
    await userEvent.click(within(chooser).getByRole('button', { name: 'Back' }));
    expect(within(chooser).getByRole('menu', { name: 'New tab choices' })).toBeInTheDocument();
  });

  it('shows the error from a failed launch', async () => {
    stubWorkspaces({ 'POST /api/sessions': new Reply(500, { code: 'spawn', message: 'claude not found' }) });
    const chooser = await open();
    await userEvent.click(within(chooser).getByRole('menuitem', { name: /^Claude(?! \(terminal\))/ }));
    expect(await within(chooser).findByText('claude not found')).toBeInTheDocument();
  });
});
