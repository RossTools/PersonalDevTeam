import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { OpenTicketContext } from './actions';
import { event, page, project, summary, ticket } from './fixtures';
import { NewTicketDialog } from './NewTicketDialog';
import { ticketKeys } from './queries';
import { TicketView } from './TicketView';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const base = ticket(1, {
  title: 'Fix login',
  stage: 'draft',
  description: 'Steps:\n\n- open',
  links: [
    { kind: 'blocks', direction: 'incoming', id: 't2', key: 'ALP-2', title: 'Schema', stage: 'ready' },
    { kind: 'blocks', direction: 'outgoing', id: 't3', key: 'ALP-3', title: 'Deploy', stage: 'draft' },
    { kind: 'relates', direction: 'outgoing', id: 't4', key: 'ALP-4', title: 'Docs', stage: 'done' },
  ],
});

const events = [
  event('e1'),
  event('e2', { kind: 'moved', data: { from: 'draft', to: 'ready' }, createdAt: 2 }),
  event('e3', { kind: 'comment', body: 'Looks **good**', createdAt: 3 }),
];

function stub(extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    'GET /api/tickets/t1': base,
    'PATCH /api/tickets/t1': (c: Call) => ({ ...base, ...(c.body as object) }),
    // Newest first (the timeline shows them oldest first).
    'GET /api/tickets/t1/events': { items: [...events].reverse(), nextCursor: null },
    [`GET /api/projects/${project.id}/labels`]: ['ui'],
    'GET /api/tickets': page([summary(5, { title: 'Cache' }), summary(1)]),
    ...extra,
  });
}

const view = (onOpen = vi.fn()) =>
  renderWithQuery(
    <OpenTicketContext.Provider value={onOpen}>
      <TicketView ticketId="t1" />
    </OpenTicketContext.Provider>,
  );

const patches = (calls: Call[]) => calls.filter((c) => c.method === 'PATCH').map((c) => c.body);

describe('ticket tab', () => {
  it('opens on the section last shown for that ticket, with the diff size on Changes', async () => {
    const attempt = { id: 'a1', branch: 'pdt/ALP-1-fix-login', baseBranch: 'main', worktreeId: 'w1', state: 'active' as const, createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, ci: null, pr: null };
    const active = { ...base, stage: 'in_progress' as const, attempt, attempts: [attempt] };
    const changes = {
      worktreeId: 'w1', branch: attempt.branch, base: 'main', mergeBase: 'abc', head: 'def', dirty: false, attemptId: 'a1', ticketId: 't1',
      attemptActive: true, approved: false,
      files: [{ path: 'a.ts', oldPath: null, status: 'modified', untracked: false, binary: false, added: 7, removed: 2, version: '1', generated: false, uncommitted: false }],
    };
    stub({ 'GET /api/tickets/t1': active, 'POST /api/tickets/t1/check-merge': active, 'GET /api/worktrees/w1/changes': changes, 'GET /api/worktrees/w1/comments': [] });
    const { unmount } = view();
    const tab = await screen.findByRole('tab', { name: /Changes/ });
    await waitFor(() => expect(tab).toHaveTextContent('+7−2'));
    await userEvent.click(tab);
    expect(tab).toHaveAttribute('aria-selected', 'true');
    unmount();
    view();
    expect(await screen.findByRole('tab', { name: /Changes/ })).toHaveAttribute('aria-selected', 'true');
  });

  it('comes back to the Overview where it was scrolled, also after another section', async () => {
    stub();
    const first = view();
    const overview = await screen.findByRole('tabpanel', { name: 'Overview' });
    overview.scrollTop = 420;
    fireEvent.scroll(overview);
    first.unmount();

    view();
    expect((await screen.findByRole('tabpanel', { name: 'Overview' })).scrollTop).toBe(420);
    // Another section and back, without leaving the ticket.
    await userEvent.click(screen.getByRole('tab', { name: 'Conversation' }));
    await userEvent.click(screen.getByRole('tab', { name: 'Overview' }));
    expect(screen.getByRole('tabpanel', { name: 'Overview' }).scrollTop).toBe(420);
  });

  it('shows the header, the Overview with description, activity, details and links, and the later tabs', async () => {
    stub();
    view();
    expect(await screen.findByRole('heading', { name: /Fix login/ })).toBeInTheDocument();
    expect(screen.getByRole('list', { name: 'Stage' })).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tab', { name: 'Conversation' })).toBeEnabled();
    expect(screen.getByRole('tab', { name: 'Changes' })).toBeEnabled();
    expect(screen.getByText('open')).toBeInTheDocument();
    const activity = await screen.findByRole('list', { name: 'Activity' });
    const entries = within(activity).getAllByRole('listitem');
    expect(entries.map((e) => e.textContent)).toEqual([
      expect.stringContaining('Created'), expect.stringContaining('Moved from Draft to Ready'), expect.stringContaining('Commented'),
    ]);
    expect(within(entries[2]).getByText('good').tagName).toBe('STRONG');
    expect(within(screen.getByRole('list', { name: 'Blocked by' })).getByText('Schema')).toBeInTheDocument();
    expect(within(screen.getByRole('list', { name: 'Blocks' })).getByText('Deploy')).toBeInTheDocument();
    expect(within(screen.getByRole('list', { name: 'Related' })).getByText('Docs')).toBeInTheDocument();
  });

  it('says why auto-start passes the ticket over after a failed start, and Try again asks it to try again', async () => {
    const failed = { ...base, stage: 'todo' as const, pickupFailed: 'git failed: no repo' };
    const calls = stub({ 'GET /api/tickets/t1': failed, 'POST /api/tickets/t1/retry-auto-start': { ...failed, pickupFailed: null } });
    view();
    expect(await screen.findByText("Auto-start couldn't start it: git failed: no repo")).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/tickets/t1/retry-auto-start')).toBe(true));
  });

  it('moves between sections with the arrow keys, each shown in its tab panel', async () => {
    stub();
    view();
    const overview = await screen.findByRole('tab', { name: 'Overview' });
    expect(screen.getByRole('tabpanel', { name: 'Overview' })).toBeInTheDocument();
    await userEvent.click(overview);
    await userEvent.keyboard('{ArrowRight}');
    const conversation = screen.getByRole('tab', { name: 'Conversation' });
    expect(conversation).toHaveFocus();
    expect(conversation).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByRole('tabpanel', { name: 'Conversation' })).toBeInTheDocument();
    await userEvent.keyboard('{Home}');
    expect(overview).toHaveAttribute('aria-selected', 'true');
  });

  it('edits the title in place', async () => {
    const calls = stub();
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Edit title' }));
    const input = screen.getByRole('textbox', { name: 'Title' });
    await userEvent.clear(input);
    await userEvent.type(input, 'Fix the login{Enter}');
    await waitFor(() => expect(patches(calls)).toContainEqual({ title: 'Fix the login' }));
  });

  it('edits the description with a preview, saving only when changed', async () => {
    const calls = stub();
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Edit' }));
    const save = screen.getByRole('button', { name: 'Save' });
    expect(save).toBeDisabled();
    const box = screen.getByRole('textbox', { name: 'Description' });
    await userEvent.clear(box);
    await userEvent.type(box, 'New **text**');
    await userEvent.click(screen.getByRole('radio', { name: 'Preview' }));
    expect(screen.getByText('text').tagName).toBe('STRONG');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(patches(calls)).toContainEqual({ description: 'New **text**' }));
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Save' })).not.toBeInTheDocument());
  });

  it('opens the title editor again with what was typed when saving fails', async () => {
    stub({ 'PATCH /api/tickets/t1': new Reply(500, { code: 'internal', message: 'database is locked' }) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Edit title' }));
    const input = screen.getByRole('textbox', { name: 'Title' });
    await userEvent.clear(input);
    await userEvent.type(input, 'Fix the login{Enter}');
    expect(await screen.findByText('database is locked')).toBeInTheDocument();
    expect(await screen.findByRole('textbox', { name: 'Title' })).toHaveValue('Fix the login');
  });

  it('warns when the description changes while it is edited, instead of overwriting it silently', async () => {
    const calls = stub();
    const { qc } = renderWithQuery(<TicketView ticketId="t1" />);
    await userEvent.click(await screen.findByRole('button', { name: 'Edit' }));
    const box = screen.getByRole('textbox', { name: 'Description' });
    // Untouched, the edit follows a change made elsewhere.
    act(() => qc.setQueryData(ticketKeys.one('t1'), { ...base, description: 'Agent v1' }));
    await waitFor(() => expect(box).toHaveValue('Agent v1'));
    expect(screen.queryByText(/changed while you were editing/)).not.toBeInTheDocument();
    await userEvent.type(box, ' and mine');
    act(() => qc.setQueryData(ticketKeys.one('t1'), { ...base, description: 'Agent v2' }));
    expect(await screen.findByText(/changed while you were editing/)).toBeInTheDocument();
    // Escape doesn't drop the typed edit.
    await userEvent.type(box, '{Escape}');
    expect(box).toHaveValue('Agent v1 and mine');
    await userEvent.click(screen.getByRole('button', { name: 'Load the new version' }));
    expect(box).toHaveValue('Agent v2');
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    expect(patches(calls)).toEqual([]);
  });

  it('changes the stage from Details (the header has no stage shortcut)', async () => {
    const calls = stub();
    view();
    const details = await screen.findByRole('region', { name: 'Details' });
    await userEvent.click(within(details).getByRole('button', { name: /Stage/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'In progress' }));
    await waitFor(() => expect(patches(calls)).toContainEqual({ stage: 'in_progress' }));
    expect(screen.queryByRole('button', { name: 'Submit' })).not.toBeInTheDocument();
  });

  it('adds and removes labels from Details', async () => {
    const calls = stub();
    view();
    const details = await screen.findByRole('region', { name: 'Details' });
    await userEvent.type(within(details).getByRole('combobox', { name: 'Add label' }), 'Back End{Enter}');
    await waitFor(() => expect(patches(calls)).toContainEqual({ labels: ['back end'] }));
  });

  it('adds an existing label with one click on the suggestion', async () => {
    const calls = stub();
    view();
    const details = await screen.findByRole('region', { name: 'Details' });
    await userEvent.click(within(details).getByRole('combobox', { name: 'Add label' }));
    await userEvent.click(await screen.findByRole('option', { name: 'ui' }));
    await waitFor(() => expect(patches(calls)).toContainEqual({ labels: ['ui'] }));
  });

  it('comments from the composer', async () => {
    const calls = stub({ 'POST /api/tickets/t1/comments': event('e4', { kind: 'comment', body: 'Hi' }) });
    view();
    const box = await screen.findByRole('textbox', { name: 'Comment' });
    await userEvent.type(box, 'Ship it{Control>}{Enter}{/Control}');
    await waitFor(() => expect(calls.find((c) => c.path === '/api/tickets/t1/comments')?.body).toEqual({ body: 'Ship it', by: 'user' }));
    await waitFor(() => expect(box).toHaveValue(''));
  });

  it('links a ticket with a kind: blocked by is the other ticket blocking this one', async () => {
    const calls = stub({ 'POST /api/tickets/t5/links': ticket(5) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Link ticket…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Link ALP-1 to…' });
    expect(within(dialog).getByRole('radio', { name: 'Blocked by' })).toBeChecked();
    // The ticket itself is not offered.
    await within(dialog).findByRole('menuitem', { name: /ALP-5 · Cache/ });
    expect(within(dialog).queryByRole('menuitem', { name: /ALP-1 ·/ })).not.toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole('menuitem', { name: /ALP-5 · Cache/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')).toMatchObject({ path: '/api/tickets/t5/links', body: { to: 't1', kind: 'blocks' } }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Link ALP-1 to…' })).not.toBeInTheDocument());
  });

  it('shows a refused link (a cycle) in the dialog', async () => {
    stub({ 'POST /api/tickets/t1/links': new Reply(409, { code: 'link_cycle', message: 'That would make a cycle: ALP-1 → ALP-5 → ALP-1' }) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Link ticket…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Link ALP-1 to…' });
    await userEvent.click(within(dialog).getByRole('radio', { name: 'Blocks' }));
    await userEvent.click(await within(dialog).findByRole('menuitem', { name: /ALP-5 · Cache/ }));
    expect(await within(dialog).findByText('That would make a cycle: ALP-1 → ALP-5 → ALP-1')).toBeInTheDocument();
  });

  it('removes a link from its menu (an incoming block is removed from the blocker)', async () => {
    const calls = stub({ 'DELETE /api/tickets/t2/links': ticket(2) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Link to ALP-2' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Remove link' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'DELETE')).toMatchObject({ path: '/api/tickets/t2/links', body: { to: 't1', kind: 'blocks' } }));
  });

  it('opens a linked ticket from its key', async () => {
    const onOpen = vi.fn();
    stub();
    view(onOpen);
    await userEvent.click(await screen.findByRole('button', { name: 'ALP-3' }));
    expect(onOpen).toHaveBeenCalledWith({ id: 't3', projectId: project.id });
  });

  it('deletes from ⋯ after confirming', async () => {
    const calls = stub({ 'DELETE /api/tickets/t1': undefined });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete…' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete ALP-1?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Delete ticket' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/tickets/t1')).toBe(true));
  });

  it('says why a delete from ⋯ failed', async () => {
    stub({ 'DELETE /api/tickets/t1': new Reply(409, { code: 'conflict', message: 'The ticket has an active attempt' }) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete…' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete ALP-1?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Delete ticket' }));
    expect(await screen.findByText('The ticket has an active attempt')).toBeInTheDocument();
  });
});

describe('new ticket form', () => {
  it('creates a ticket with description, stage, priority and labels', async () => {
    const calls = stubApi({
      [`GET /api/projects/${project.id}/labels`]: [],
      [`POST /api/projects/${project.id}/tickets`]: ticket(9),
    });
    const onClose = vi.fn();
    renderWithQuery(<NewTicketDialog projects={[project]} projectId={project.id} title="Half typed" onClose={onClose} />);
    const dialog = screen.getByRole('dialog', { name: 'New ticket' });
    expect(within(dialog).getByRole('textbox', { name: 'Title' })).toHaveValue('Half typed');
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Description' }), 'Why');
    await userEvent.click(within(dialog).getByRole('button', { name: /Priority/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'High' }));
    await userEvent.type(within(dialog).getByRole('combobox', { name: 'Add label' }), 'ui{Enter}');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Create ticket' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      title: 'Half typed', description: 'Why', stage: 'draft', priority: 'high', labels: ['ui'],
    }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('closes an untouched form without asking', async () => {
    stubApi({ [`GET /api/projects/${project.id}/labels`]: [] });
    const onClose = vi.fn();
    renderWithQuery(<NewTicketDialog projects={[project]} projectId={project.id} onClose={onClose} />);
    await userEvent.keyboard('{Escape}');
    expect(onClose).toHaveBeenCalled();
    expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument();
  });

  it('asks before closing a form with something typed in it', async () => {
    stubApi({ [`GET /api/projects/${project.id}/labels`]: [] });
    const onClose = vi.fn();
    renderWithQuery(<NewTicketDialog projects={[project]} projectId={project.id} onClose={onClose} />);
    const dialog = screen.getByRole('dialog', { name: 'New ticket' });
    await userEvent.type(within(dialog).getByRole('textbox', { name: 'Description' }), 'Why');

    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    const ask = await screen.findByRole('alertdialog', { name: 'Discard this ticket?' });
    await userEvent.click(within(ask).getByRole('button', { name: 'Keep editing' }));
    await waitFor(() => expect(screen.queryByRole('alertdialog')).not.toBeInTheDocument());
    expect(onClose).not.toHaveBeenCalled();
    expect(within(dialog).getByRole('textbox', { name: 'Description' })).toHaveValue('Why');
    await waitFor(() => expect(dialog).toContainElement(document.activeElement as HTMLElement));

    await userEvent.keyboard('{Escape}');
    const again = await screen.findByRole('alertdialog', { name: 'Discard this ticket?' });
    await userEvent.click(within(again).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('shows the error from a refused create', async () => {
    stubApi({
      [`GET /api/projects/${project.id}/labels`]: [],
      [`POST /api/projects/${project.id}/tickets`]: new Reply(400, { code: 'invalid_input', message: 'title is too long' }),
    });
    renderWithQuery(<NewTicketDialog projects={[project]} projectId={project.id} title="x" onClose={() => {}} />);
    await userEvent.click(screen.getByRole('button', { name: 'Create ticket' }));
    expect(await screen.findByText('title is too long')).toBeInTheDocument();
  });
});

describe('Create GitHub issue (PDT-221)', () => {
  const forge = { [`GET /api/projects/${project.id}/forge`]: { kind: 'github', repo: 'o/r', issues: true } };
  const filed = { repo: 'o/r', number: 12, issue: 'o/r#12', url: 'https://github.com/o/r/issues/12', ticketKey: 'ALP-1' };

  it('files the ticket as an issue from its menu, pre-filled and editable', async () => {
    const calls = stub({ ...forge, [`POST /api/projects/${project.id}/issues`]: filed });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Create GitHub issue…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Create GitHub issue from ALP-1' });
    expect(within(dialog).getByText(/Opens an issue in o\/r as your GitHub login/)).toBeInTheDocument();
    const title = within(dialog).getByRole('textbox', { name: 'Title' });
    expect(title).toHaveValue('Fix login');
    expect(within(dialog).getByRole('textbox', { name: 'Description' })).toHaveValue('Steps:\n\n- open');
    await userEvent.type(title, ' on Safari');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Create issue' }));
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument());
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({
      title: 'Fix login on Safari', body: 'Steps:\n\n- open', labels: [], ticketId: 't1',
    });
  });

  it('is not offered when the forge has no issues or the ticket already links one', async () => {
    stub({ [`GET /api/projects/${project.id}/forge`]: { kind: 'github', repo: 'o/r', issues: false } });
    const { unmount } = view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await screen.findByRole('menuitem', { name: 'Move to' });
    expect(screen.queryByRole('menuitem', { name: 'Create GitHub issue…' })).not.toBeInTheDocument();
    await userEvent.keyboard('{Escape}');
    unmount();
    const linked = { ...base, external: [{ system: 'github', key: 'o/r#3', url: 'https://github.com/o/r/issues/3', origin: 'created' as const, closeOnDone: true }] };
    stub({ ...forge, 'GET /api/tickets/t1': linked });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await screen.findByRole('menuitem', { name: 'Move to' });
    expect(screen.queryByRole('menuitem', { name: 'Create GitHub issue…' })).not.toBeInTheDocument();
  });

  it('shows why GitHub refused it and keeps the form', async () => {
    stub({ ...forge, [`POST /api/projects/${project.id}/issues`]: new Reply(502, { code: 'forge_error', message: 'GitHub is not connected' }) });
    view();
    await userEvent.click(await screen.findByRole('button', { name: 'Ticket actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Create GitHub issue…' }));
    const dialog = await screen.findByRole('dialog');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Create issue' }));
    expect(await within(dialog).findByText('GitHub is not connected')).toBeInTheDocument();
  });
});
