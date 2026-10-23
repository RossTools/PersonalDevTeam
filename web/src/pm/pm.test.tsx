import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PmNote } from '../api/gen/PmNote';
import type { PmState } from '../api/gen/PmState';
import type { Session } from '../api/gen/Session';
import { project } from '../tickets/fixtures';
import { renderWithQuery, stubApi } from '../test-utils';
import { NotePage, NotesPage, ticketDescription } from './NotesPage';
import { NotesPanel } from './NotesPanel';
import { PmList } from './PmList';
import { PmView } from './PmView';
import { firstLine } from './usePm';

afterEach(() => vi.unstubAllGlobals());

const session = (id: string, createdAt: number): Session => ({
  id, projectId: 'host', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PM',
  cwd: '/data/pm', status: 'finished', exitCode: null, createdAt, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: { id: 'pm', name: 'PM', version: 1, stepId: null },
});

const ended: PmState = {
  current: null,
  sessions: [{ session: session('s1', Date.UTC(2026, 9, 20, 9)), rotation: 'done', reason: 'age', journal: '## Today\nPlanned the PM.' }],
};

const note = (id: string, over: Partial<PmNote> = {}): PmNote => ({
  id, projectId: null, kind: 'note', title: `Note ${id}`, body: '', pinned: false, sessionId: null, createdAt: 1, updatedAt: 1, ...over,
});

describe('the PM', () => {
  it('reads a journal’s first line without its heading marks', () => {
    expect(firstLine('\n## Today\nPlanned.')).toBe('Today');
    expect(firstLine(null)).toBe('');
  });

  it('lists the current session, the notes and earlier sessions with their journal', async () => {
    stubApi({ 'GET /api/pm': ended });
    const onSelect = vi.fn();
    renderWithQuery(<PmList selection="current" onSelect={onSelect} onNew={() => {}} />);
    expect(await screen.findByText('Ask something to start one')).toBeInTheDocument();
    expect(await screen.findByText('Today')).toBeInTheDocument();
    await userEvent.click(screen.getByText('Notes'));
    expect(onSelect).toHaveBeenCalledWith('notes');
  });

  it('starts a session with the first message when there is none', async () => {
    const calls = stubApi({ 'GET /api/pm': ended, 'POST /api/pm/sessions': session('s2', 2) });
    const onSelect = vi.fn();
    renderWithQuery(<PmView selection="current" onSelect={onSelect} onNew={() => {}} />);
    const box = await screen.findByRole('textbox', { name: 'Ask the PM' });
    await userEvent.type(box, "What's in flight?{Enter}");
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('current'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ text: "What's in flight?" });
  });

  it('keeps the start box’s unsent text when the page goes and comes back; starting clears it', async () => {
    stubApi({ 'GET /api/pm': ended, 'POST /api/pm/sessions': session('s2', 2) });
    const first = renderWithQuery(<PmView selection="current" onSelect={() => {}} onNew={() => {}} />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Ask the PM' }), 'half a thought');
    first.unmount();
    const onSelect = vi.fn();
    const second = renderWithQuery(<PmView selection="current" onSelect={onSelect} onNew={() => {}} />);
    const box = await screen.findByRole('textbox', { name: 'Ask the PM' });
    expect(box).toHaveValue('half a thought');
    await userEvent.type(box, '{Enter}');
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('current'));
    second.unmount();
    renderWithQuery(<PmView selection="current" onSelect={() => {}} onNew={() => {}} />);
    expect(await screen.findByRole('textbox', { name: 'Ask the PM' })).toHaveValue('');
  });

  it("lists the PM's notes and adds one", async () => {
    const calls = stubApi({
      'GET /api/pm/notes': [note('a', { title: 'Work hours', pinned: true, body: 'Nine to five.' }), note('b', { projectId: project.id })],
      'GET /api/projects': [project],
      'POST /api/pm/notes': note('c'),
    });
    renderWithQuery(<NotesPage />);
    await userEvent.click(await screen.findByRole('radio', { name: 'PM’s' }));
    await waitFor(() => expect(calls.some((c) => c.path.includes('kind=note'))).toBe(true));
    expect(await screen.findByText('Work hours')).toBeInTheDocument();
    expect(screen.getByLabelText('Pinned')).toBeInTheDocument();
    expect(await screen.findByText(project.key)).toBeInTheDocument();
    expect(screen.getByText('Host-wide')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'New note' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Title' }), 'Deploys');
    await userEvent.type(screen.getByRole('textbox', { name: 'Note' }), 'Keys in the vault.');
    await userEvent.click(screen.getByRole('switch', { name: /Pinned/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Add note' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ title: 'Deploys', body: 'Keys in the vault.', pinned: true }));
  });

  it('opens a note from the list when it can (phones), and shows all of it with Edit', async () => {
    const calls = stubApi({
      'GET /api/pm/notes': [note('a', { title: 'Work hours', body: 'Nine to five.\n\nNot on Fridays.' })],
      'GET /api/projects': [project],
      'GET /api/pm/notes/a': note('a', { title: 'Work hours', body: 'Nine to five.\n\nNot on Fridays.' }),
      'PATCH /api/pm/notes/a': note('a'),
    });
    const onOpen = vi.fn();
    const list = renderWithQuery(<NotesPage onOpen={onOpen} />);
    await userEvent.click(await screen.findByRole('button', { name: /Work hours/ }));
    expect(onOpen).toHaveBeenCalledWith('a');
    expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
    list.unmount();
    renderWithQuery(<NotePage id="a" onGone={() => {}} />);
    expect(await screen.findByText('Not on Fridays.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Edit' }));
    await userEvent.clear(screen.getByRole('textbox', { name: 'Title' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Title' }), 'Hours');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toMatchObject({ title: 'Hours' }));
  });

  it('shows my notes by default, adds one of mine, and makes a ticket from it', async () => {
    const mine = note('m', { kind: 'user', title: 'Dark mode', body: 'Follow the OS.', projectId: project.id });
    const calls = stubApi({
      'GET /api/pm/notes': [mine],
      'GET /api/projects': [project],
      'POST /api/pm/notes': note('c', { kind: 'user' }),
      'GET /api/labels': [],
    });
    renderWithQuery(<NotesPage />);
    expect(await screen.findByText('Dark mode')).toBeInTheDocument();
    expect(calls.find((c) => c.path.startsWith('/api/pm/notes'))?.path).toContain('kind=user');
    expect(screen.getByRole('radio', { name: 'Mine' })).toBeChecked();
    await userEvent.click(screen.getByRole('button', { name: 'New note' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Title' }), 'Idea');
    await userEvent.click(screen.getByRole('button', { name: 'Add note' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ title: 'Idea', body: '', pinned: false, kind: 'user' }));

    await userEvent.click(screen.getByRole('button', { name: 'Make a ticket' }));
    const form = await screen.findByRole('dialog', { name: 'Make a ticket' });
    expect(within(form).getByRole('textbox', { name: 'Title' })).toHaveValue('Dark mode');
    const description = within(form).getByRole('textbox', { name: 'Description' });
    expect(description).toHaveValue(`Follow the OS.\n\nFrom the note [Dark mode](${window.location.origin}/pm/notes/m).`);
  });

  it('drafts a ticket description that links back to the note', () => {
    expect(ticketDescription(note('n1', { kind: 'user', title: 'A [b]', body: '' }), 'http://h')).toBe('From the note [A \\[b\\]](http://h/pm/notes/n1).');
  });

  it('shows a deleted note as gone', async () => {
    stubApi({ 'GET /api/projects': [project] });
    renderWithQuery(<NotePage id="x" onGone={() => {}} />);
    expect(await screen.findByText('This note is gone.')).toBeInTheDocument();
  });

  it('captures a note in the panel with ⌘Enter, in the chosen project, and opens one', async () => {
    const other = { ...project, id: 'p2', key: 'OTH', name: 'Other' };
    const calls = stubApi({
      'GET /api/pm/notes': [
        note('a', { kind: 'user', title: 'In PX', projectId: project.id }),
        note('b', { kind: 'user', title: 'Everywhere' }),
        note('c', { kind: 'user', title: 'In OTH', projectId: other.id }),
      ],
      'GET /api/projects': [project, other],
      'POST /api/pm/notes': note('d', { kind: 'user' }),
    });
    const onOpen = vi.fn();
    renderWithQuery(<NotesPanel onOpen={onOpen} />);
    expect(await screen.findByText('In OTH')).toBeInTheDocument();
    expect(calls[0].path).toContain('kind=user');
    await userEvent.click(screen.getByRole('button', { name: /Scope/ }));
    await userEvent.click(await screen.findByRole('option', { name: `${project.name} (${project.key})` }));
    const list = screen.getByRole('list', { name: 'My notes' });
    await waitFor(() => expect(within(list).queryByText('In OTH')).toBeNull());
    expect(within(list).queryByText('Everywhere')).toBeNull();
    await userEvent.type(screen.getByRole('textbox', { name: 'Note title' }), 'Quick idea');
    await userEvent.type(screen.getByRole('textbox', { name: 'Note text' }), 'More *words*{Meta>}{Enter}{/Meta}');
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ title: 'Quick idea', body: 'More *words*', kind: 'user', project: project.id }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Note title' })).toHaveValue(''));
    await userEvent.click(within(list).getByRole('button', { name: /In PX/ }));
    expect(onOpen).toHaveBeenCalledWith('a');
  });
});
