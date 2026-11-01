import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery } from '../test-utils';
import { page, summary, ticket } from '../tickets/fixtures';
import { plugins } from '../api/gen/plugins';
import { AppShell } from './AppShell';
import { project, stubPhone, stubWorkspaces } from './fixtures';

vi.mock('../terminal/TerminalView', () => ({
  TerminalView: ({ sessionId, sessionStatus, polledAt }: { sessionId: string; sessionStatus?: string; polledAt?: number }) => (
    <div data-testid="terminal-view" data-status={sessionStatus} data-polled={polledAt}>{sessionId}</div>
  ),
}));

afterEach(() => vi.unstubAllGlobals());

describe('routes (PDT-111)', () => {
  const fixLogin = ticket(1, { projectId: project.id, title: 'Fix login' });
  const ticketRoutes = {
    'GET /api/tickets/t1': fixLogin,
    'GET /api/tickets/ALP-1': fixLogin,
    'GET /api/tickets/t1/events': { items: [], nextCursor: null },
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/tickets': page([summary(1, { projectId: project.id, title: 'Fix login' })]),
  };
  const at = (path: string) => window.history.replaceState(null, '', path);
  const here = () => window.location.pathname + window.location.search;
  const back = () => act(() => window.history.back());

  it("opens a note's old path (/pm/notes/<id>) at its new one, and the old notes path at the PM's memory", async () => {
    const pmNote = { id: 'n1', projectId: null, kind: 'note', title: 'Work hours', body: 'Nine to five.', pinned: false, sessionId: null, createdAt: 1, updatedAt: 1 };
    stubWorkspaces({ 'GET /api/pm/notes': [pmNote], 'GET /api/pm/notes/n1': pmNote });
    at('/pm/notes/n1');
    const { unmount } = renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/notes/n1'));
    expect(await within(screen.getByRole('main')).findByRole('heading', { name: 'Work hours', level: 1 })).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Notes' })).toHaveAttribute('aria-current', 'page');
    // The list shows the note's side.
    await waitFor(() => expect(screen.getByRole('radio', { name: 'PM’s memory' })).toBeChecked());
    unmount();
    at('/pm/notes');
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/notes/pm'));
    expect(await screen.findByText(/Pinned notes go in every PM briefing/)).toBeInTheDocument();
  });

  it('creates a new note at /notes/new with its first save and replaces the path with its own', async () => {
    const created = { id: 'c1', projectId: null, kind: 'user', title: 'Idea', body: '', pinned: false, sessionId: null, createdAt: 1, updatedAt: 1 };
    const calls = stubWorkspaces({ 'GET /api/pm/notes': [], 'POST /api/pm/notes': created, 'GET /api/pm/notes/c1': created });
    at('/notes');
    renderWithQuery(<AppShell />);
    await userEvent.click(await within(screen.getByRole('complementary', { name: 'Notes list' })).findByRole('button', { name: 'New note' }));
    expect(here()).toBe('/notes/new');
    const length = window.history.length;
    const title = await screen.findByRole('textbox', { name: 'Title' });
    await waitFor(() => expect(title).toHaveFocus());
    await userEvent.type(title, 'Idea');
    await waitFor(() => expect(here()).toBe('/notes/c1'), { timeout: 2000 });
    expect(window.history.length).toBe(length);
    expect(calls.filter((c) => c.method === 'POST').map((c) => c.body)).toEqual([{ title: 'Idea', body: '', kind: 'user' }]);
    expect(screen.getByRole('textbox', { name: 'Title' })).toHaveValue('Idea');
  });

  it('opens the notes with Ctrl+] o', async () => {
    stubWorkspaces({ 'GET /api/pm/notes': [] });
    at('/sessions');
    renderWithQuery(<AppShell />);
    await userEvent.keyboard('{Control>}]{/Control}o');
    expect(here()).toBe('/notes');
    expect(await screen.findByText('Pick a note, or start a new one.')).toBeInTheDocument();
  });

  it('fills in the workspace and tab at / and /sessions', async () => {
    stubWorkspaces();
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/sessions/wa/ta'));
  });

  it('opens the workspace and tab in the path on reload, and Back/Forward walk the tabs chosen', async () => {
    stubWorkspaces();
    at('/sessions/wa/tb');
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    await userEvent.click(screen.getByRole('tab', { name: /Claude A/ }));
    expect(here()).toBe('/sessions/wa/ta');
    await userEvent.click(within(screen.getByRole('region', { name: 'Workspaces' })).getByRole('row', { name: /Release/ }));
    expect(here()).toBe('/sessions/wr');
    await back();
    await waitFor(() => expect(screen.getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true'));
    await back();
    await waitFor(() => expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true'));
    await act(() => window.history.forward());
    await waitFor(() => expect(screen.getByRole('tab', { name: /Claude A/ })).toHaveAttribute('aria-selected', 'true'));
    // Remembered for when a path doesn't say.
    expect(JSON.parse(localStorage.getItem('pdt.workspaces.current')!).tabs.wa).toBe('ta');
  });

  it('puts the view in the path, and Back returns to the one before, as it was', async () => {
    stubWorkspaces(ticketRoutes);
    at('/sessions/wa/tb');
    renderWithQuery(<AppShell />);
    await screen.findByRole('tab', { name: /Codex B/ });
    await userEvent.click(screen.getByRole('button', { name: 'Tickets' }));
    expect(here()).toBe('/tickets');
    await userEvent.click(await screen.findByRole('radio', { name: 'Board' }));
    expect(here()).toBe('/tickets?view=board');
    await userEvent.click(screen.getByRole('button', { name: 'Sessions' }));
    expect(screen.getByRole('tab', { name: /Codex B/ })).toHaveAttribute('aria-selected', 'true');
    expect(here()).toBe('/sessions/wa/tb');
    // Tickets opens as it was left.
    await userEvent.click(screen.getByRole('button', { name: 'Tickets' }));
    expect(here()).toBe('/tickets?view=board');
    await back();
    await waitFor(() => expect(screen.getByRole('button', { name: 'Sessions' })).toHaveAttribute('aria-current', 'page'));
    await back();
    await waitFor(() => expect(screen.getByRole('radio', { name: 'Board' })).toBeChecked());
    await back();
    await waitFor(() => expect(screen.getByRole('radio', { name: 'Backlog' })).toBeChecked());
  });

  it('peeks at the ticket in /tickets/KEY, and names a peeked row by its key', async () => {
    stubWorkspaces(ticketRoutes);
    at('/tickets/ALP-1');
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('dialog', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    await waitFor(() => expect(here()).toBe('/tickets'));
    await userEvent.click(await screen.findByRole('row', { name: /Fix login/ }));
    await waitFor(() => expect(here()).toBe('/tickets/ALP-1'));
    expect(await screen.findByRole('dialog', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
  });

  it("opens /tickets/KEY by asking pdtd for the key, not by searching (PDT-271)", async () => {
    const old = ticket(7, { projectId: project.id, title: 'Old ticket' });
    // A search for ALP-7 would list tickets mentioning it, and not ALP-7 itself.
    stubWorkspaces({
      ...ticketRoutes,
      'GET /api/tickets/alp-7': old,
      'GET /api/tickets/t7': old,
      'GET /api/tickets/t7/events': { items: [], nextCursor: null },
    });
    at('/tickets/alp-7');
    renderWithQuery(<AppShell />);
    expect(await screen.findByRole('dialog', { name: 'ALP-7 · Old ticket' })).toBeInTheDocument();
  });

  it('says a key no ticket has is not found', async () => {
    stubWorkspaces(ticketRoutes);
    at('/tickets/ALP-99');
    renderWithQuery(<AppShell />);
    expect(await screen.findByText('Ticket not found.')).toBeInTheDocument();
  });

  it('shows a ticket as a detail screen on a phone, where ← and Back agree', async () => {
    stubPhone();
    stubWorkspaces(ticketRoutes);
    at('/tickets');
    renderWithQuery(<AppShell />);
    const bar = screen.getByRole('banner', { name: 'Top bar' });
    await userEvent.click(await screen.findByRole('row', { name: /Fix login/ }));
    await waitFor(() => expect(here()).toBe('/tickets/ALP-1'));
    // Under the top bar, in place of the list, not a sheet over it.
    expect(await within(screen.getByRole('main')).findByRole('region', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByRole('row', { name: /Fix login/ })).not.toBeInTheDocument();
    expect(within(bar).getByRole('heading', { name: 'ALP-1' })).toBeInTheDocument();
    // ← goes Back: Forward returns to the ticket.
    await userEvent.click(within(bar).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/tickets'));
    expect(await screen.findByRole('row', { name: /Fix login/ })).toBeInTheDocument();
    await act(() => window.history.forward());
    await waitFor(() => expect(here()).toBe('/tickets/ALP-1'));
    await back();
    await waitFor(() => expect(here()).toBe('/tickets'));
    expect(within(bar).getByRole('button', { name: 'Menu' })).toBeInTheDocument();
  });

  it("puts Cancel and Save in a phone's top bar while the ticket's title is edited", async () => {
    stubPhone();
    const calls = stubWorkspaces({ ...ticketRoutes, 'PATCH /api/tickets/t1': { ...fixLogin, title: 'Fix login now' }, 'GET /api/inbox': { items: [], total: 0 } });
    at('/tickets/ALP-1');
    renderWithQuery(<AppShell />);
    const bar = screen.getByRole('banner', { name: 'Top bar' });
    await userEvent.click(await screen.findByRole('button', { name: 'Edit title: Fix login' }));
    expect(within(bar).queryByRole('button', { name: 'Back' })).not.toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Title' }), ' now');
    await userEvent.click(within(bar).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ title: 'Fix login now' }));
    // Saved: the bar is the screen's again, and ← goes back to the list.
    await userEvent.click(await within(bar).findByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/tickets'));
  });

  it('drills into Files folder by folder on a phone, with ← going up a folder', async () => {
    stubPhone();
    const p = `/api/projects/${project.id}`;
    stubWorkspaces({
      [`GET ${p}/files?path=`]: [{ name: 'src', path: 'src', dir: true, status: null }],
      [`GET ${p}/files?path=src`]: [{ name: 'lib', path: 'src/lib', dir: true, status: null }],
      [`GET ${p}/files?path=src%2Flib`]: [],
    });
    at('/files');
    renderWithQuery(<AppShell />);
    const bar = screen.getByRole('banner', { name: 'Top bar' });
    await userEvent.click(await within(await screen.findByRole('grid', { name: 'Files' })).findByRole('row', { name: /src/ }));
    await waitFor(() => expect(here()).toBe('/files/src/?project=ALP'));
    expect(within(bar).getByRole('heading', { name: 'src' })).toBeInTheDocument();
    await userEvent.click(await within(await screen.findByRole('grid', { name: 'Files in src' })).findByRole('row', { name: /lib/ }));
    await waitFor(() => expect(here()).toBe('/files/src/lib/?project=ALP'));
    expect(await screen.findByText('This folder is empty.')).toBeInTheDocument();
    await userEvent.click(within(bar).getByRole('button', { name: 'Back' }));
    await waitFor(() => expect(here()).toBe('/files/src/?project=ALP'));
    expect(await screen.findByRole('grid', { name: 'Files in src' })).toBeInTheDocument();
  });

  it('opens a ticket key linked in a conversation', async () => {
    stubWorkspaces(ticketRoutes);
    at('/tickets');
    renderWithQuery(<AppShell />);
    await screen.findByRole('row', { name: /Fix login/ });
    // A link as Markdown renders it (an app path, no target).
    const a = document.createElement('a');
    a.href = '/tickets/ALP-1';
    a.textContent = 'ALP-1';
    document.body.append(a);
    await userEvent.click(a);
    a.remove();
    expect(here()).toBe('/tickets/ALP-1');
    expect(await screen.findByRole('dialog', { name: 'ALP-1 · Fix login' })).toBeInTheDocument();
  });

  it('sends the old Integrations page and This host’s GitHub section to the plugins (PDT-228)', async () => {
    stubWorkspaces({ 'GET /api/connections': [], 'GET /api/integrations': plugins });
    at('/settings/host/github');
    renderWithQuery(<AppShell />);
    await waitFor(() => expect(here()).toBe('/settings/plugins/github'));
    expect(await within(screen.getByRole('main')).findByRole('region', { name: 'GitHub logins' })).toBeInTheDocument();
  });

  it('opens Settings pages from their paths, and puts the page chosen in the path', async () => {
    stubWorkspaces({
      'GET /api/connections': [],
      [`GET /api/projects/${project.id}/settings`]: { runOverrides: null, agentCredentials: null },
      [`GET /api/projects/${project.id}/run-config`]: { file: null, filePath: null, error: null, effective: null },
    });
    at('/settings/projects/ALP');
    renderWithQuery(<AppShell />);
    const main = screen.getByRole('main');
    expect(await within(main).findByRole('region', { name: 'General' })).toBeInTheDocument();
    const nav = screen.getByRole('navigation', { name: 'Settings sections' });
    await userEvent.click(within(nav).getByRole('button', { name: 'All connections' }));
    expect(here()).toBe('/settings/connections');
    expect(await within(main).findByRole('heading', { name: 'All connections' })).toBeInTheDocument();
    // This host's group is closed on a project's page.
    await userEvent.click(within(nav).getByRole('button', { name: 'This host' }));
    await userEvent.click(within(nav).getByRole('button', { name: 'Checks' }));
    expect(here()).toBe('/settings/host/checks');
    await back();
    expect(await within(main).findByRole('heading', { name: 'All connections' })).toBeInTheDocument();
  });

  it('shows the file in /files/<path>', async () => {
    stubWorkspaces({
      [`GET /api/projects/${project.id}/files?path=`]: [{ name: 'docs', path: 'docs', dir: true, status: null }],
      [`GET /api/projects/${project.id}/files?path=docs`]: [{ name: 'notes.txt', path: 'docs/notes.txt', dir: false, status: null }],
      [`GET /api/projects/${project.id}/files/content?path=docs%2Fnotes.txt`]: { path: 'docs/notes.txt', size: 3, status: null, text: 'hi\n', binary: false, tooLarge: false },
    });
    at('/files/docs/notes.txt?project=ALP');
    renderWithQuery(<AppShell />);
    expect(await within(screen.getByRole('main')).findByRole('heading', { name: 'docs/notes.txt' })).toBeInTheDocument();
    // Its folder opens, so the file shows in the tree.
    expect(await screen.findByRole('row', { name: /notes\.txt/ })).toHaveAttribute('aria-selected', 'true');
  });
});
