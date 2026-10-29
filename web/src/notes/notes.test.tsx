import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { EditorView } from '@codemirror/view';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PmNote } from '../api/gen/PmNote';
import { project } from '../tickets/fixtures';
import { editorText, renderWithQuery, setEditorText, stubApi, type Call } from '../test-utils';
import { ALL, previewOf, sectionsOf, ticketDescription, titleFor, wordCount, type Whose } from './model';
import { NoteScreen } from './NotePage';
import { NotesList } from './NotesList';
import { AUTOSAVE_MS } from './useNoteDraft';
import type { NotesFilters } from './useNotes';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const DAY = 24 * 60 * 60 * 1000;
const NOW = Date.now();

const note = (id: string, over: Partial<PmNote> = {}): PmNote => ({
  id, projectId: null, kind: 'user', title: `Note ${id}`, body: '', pinned: false, sessionId: null, createdAt: 1, updatedAt: NOW, ...over,
});

const filters = (over: Partial<NotesFilters> = {}): NotesFilters => ({
  whose: 'user', setWhose: () => {}, scope: ALL, setScope: () => {}, q: '', setQ: () => {}, ...over,
});

const caretIn = (box: HTMLElement) => EditorView.findFromDOM(box.closest<HTMLElement>('.cm-editor')!)!.state.selection.main.head;
const patches = (calls: Call[]) => calls.filter((c) => c.method === 'PATCH').map((c) => c.body);

describe('notes: the rules', () => {
  it('previews a body without its Markdown', () => {
    expect(previewOf('## Layout\n\n- **Swimlanes** per [area](https://x.test)\n- [ ] `compact` cards\n> quoted')).toBe('Layout Swimlanes per area compact cards quoted');
    expect(wordCount('## Two words\n\n```\ncode\n```')).toBe(3);
  });

  it('saves an empty title as the first line of the body, or Untitled', () => {
    expect(titleFor('  Kept  ', 'body')).toBe('Kept');
    expect(titleFor('', '\n\n## Ideas for the *board*\nmore')).toBe('Ideas for the board');
    expect(titleFor(' ', '')).toBe('Untitled');
    expect(titleFor('', 'x'.repeat(300))).toHaveLength(200);
  });

  it('puts notes in sections: Pinned, Last 7 days, Earlier, newest first', () => {
    const s = sectionsOf([
      note('old', { updatedAt: NOW - 30 * DAY }),
      note('pin', { pinned: true, updatedAt: NOW - 40 * DAY }),
      note('new', { updatedAt: NOW - DAY }),
      note('newer', { updatedAt: NOW - 1000 }),
    ], NOW);
    expect(s.map((x) => [x.label, x.notes.map((n) => n.id)])).toEqual([
      ['Pinned', ['pin']], ['Last 7 days', ['newer', 'new']], ['Earlier', ['old']],
    ]);
  });

  it('drafts a ticket description that links back to the note at /notes/<id>', () => {
    expect(ticketDescription(note('n1', { title: 'A [b]' }), 'http://h')).toBe('From the note [A \\[b\\]](http://h/notes/n1).');
    expect(ticketDescription(note('n1', { title: 'T', body: ' Body ' }), 'http://h')).toBe('Body\n\nFrom the note [T](http://h/notes/n1).');
  });
});

describe('notes: the list', () => {
  it('lists my notes in sections, with where each lives, and opens one', async () => {
    const calls = stubApi({
      'GET /api/pm/notes': [
        note('a', { title: 'Ideas for the board', pinned: true, body: '**Swimlanes** per area' }),
        note('b', { title: 'Meeting with Alex', projectId: project.id }),
        note('c', { title: 'Books to read', updatedAt: NOW - 30 * DAY }),
      ],
      'GET /api/projects': [project],
    });
    const onOpen = vi.fn();
    renderWithQuery(<NotesList filters={filters()} selectedId="a" onOpen={onOpen} onNew={() => {}} onWhose={() => {}} />);
    const pinned = await screen.findByRole('region', { name: 'Pinned' });
    expect(calls[0].path).toContain('kind=user');
    expect(within(pinned).getByRole('button', { name: /Ideas for the board/ })).toHaveAttribute('aria-current', 'true');
    expect(within(pinned).getByText('Swimlanes per area')).toBeInTheDocument();
    expect(within(pinned).getByText('Host-wide')).toBeInTheDocument();
    const week = screen.getByRole('region', { name: 'Last 7 days' });
    expect(await within(week).findByText(project.key)).toBeInTheDocument();
    expect(within(screen.getByRole('region', { name: 'Earlier' })).getByText('Books to read')).toBeInTheDocument();
    await userEvent.click(within(week).getByRole('button', { name: /Meeting with Alex/ }));
    expect(onOpen).toHaveBeenCalledWith('b');
  });

  it("switches between Mine and the PM's memory, which says pinned notes are briefed", async () => {
    const calls = stubApi({ 'GET /api/pm/notes': [], 'GET /api/projects': [] });
    const onWhose = vi.fn();
    const { rerender } = renderWithQuery(<NotesList filters={filters()} selectedId={null} onOpen={() => {}} onNew={() => {}} onWhose={onWhose} />);
    expect(await screen.findByText(/No notes yet/)).toBeInTheDocument();
    expect(screen.getByRole('radio', { name: 'Mine' })).toBeChecked();
    await userEvent.click(screen.getByRole('radio', { name: 'PM’s memory' }));
    expect(onWhose).toHaveBeenCalledWith('note');
    rerender(<NotesList filters={filters({ whose: 'note' })} selectedId={null} onOpen={() => {}} onNew={() => {}} onWhose={onWhose} />);
    expect(await screen.findByText(/The PM’s memory is empty/)).toBeInTheDocument();
    expect(screen.getByText(/Pinned notes go in every PM briefing/)).toBeInTheDocument();
    expect(calls.some((c) => c.path.includes('kind=note'))).toBe(true);
  });

  it('filters by scope on the client and searches on the server; / focuses the search', async () => {
    const calls = stubApi({
      'GET /api/pm/notes': [note('a', { title: 'In ALP', projectId: project.id }), note('b', { title: 'Everywhere' })],
      'GET /api/projects': [project],
    });
    const { rerender } = renderWithQuery(<NotesList filters={filters({ scope: project.id })} selectedId={null} onOpen={() => {}} onNew={() => {}} onWhose={() => {}} />);
    expect(await screen.findByText('In ALP')).toBeInTheDocument();
    expect(screen.queryByText('Everywhere')).not.toBeInTheDocument();
    rerender(<NotesList filters={filters({ scope: 'host', q: 'nothing' })} selectedId={null} onOpen={() => {}} onNew={() => {}} onWhose={() => {}} />);
    await waitFor(() => expect(calls.some((c) => c.path.includes('q=nothing'))).toBe(true));
    await userEvent.keyboard('/');
    expect(screen.getByRole('searchbox', { name: 'Search notes' })).toHaveFocus();
  });

  it('says when nothing matches', async () => {
    stubApi({ 'GET /api/pm/notes': [], 'GET /api/projects': [] });
    renderWithQuery(<NotesList filters={filters({ q: 'zzz' })} selectedId={null} onOpen={() => {}} onNew={() => {}} onWhose={() => {}} />);
    expect(await screen.findByText('No note matches.')).toBeInTheDocument();
  });
});

describe('notes: a note', () => {
  const screenProps = (over: Partial<Parameters<typeof NoteScreen>[0]> = {}) => ({
    id: 'a', whose: 'user' as Whose, scope: ALL, phone: false, onCreated: vi.fn(), onGone: vi.fn(), onKind: vi.fn(), ...over,
  });

  it('opens a note with text to read, and Write | Read switch (Esc goes back to Read)', async () => {
    stubApi({ 'GET /api/pm/notes/a': note('a', { title: 'Plan', body: '### Layout\n\n- [x] Ask Sam', createdAt: Date.UTC(2026, 9, 12) }), 'GET /api/projects': [] });
    const props = screenProps();
    renderWithQuery(<NoteScreen {...props} />);
    expect(await screen.findByRole('heading', { name: 'Plan', level: 1 })).toBeInTheDocument();
    expect(screen.getByRole('heading', { name: 'Layout' })).toBeInTheDocument();
    expect(screen.getByRole('checkbox')).toHaveAttribute('aria-checked', 'true');
    expect(screen.getByText(/Created .* · 3 words/)).toBeInTheDocument();
    expect(props.onKind).toHaveBeenCalledWith('user');
    await userEvent.click(screen.getByRole('radio', { name: 'Write' }));
    const body = await screen.findByRole('textbox', { name: 'Note' });
    expect(body).toHaveFocus();
    expect(editorText(body)).toBe('### Layout\n\n- [x] Ask Sam');
    await userEvent.keyboard('{Escape}');
    expect(screen.getByRole('heading', { name: 'Plan', level: 1 })).toBeInTheDocument();
  });

  it('opens to write on a double-click, the caret at the start of the block clicked', async () => {
    const body = 'Intro line\n\n## Layout\n\n- [Ask](https://x.test) Sam';
    stubApi({ 'GET /api/pm/notes/a': note('a', { title: 'Plan', body }), 'GET /api/projects': [] });
    renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.dblClick(await screen.findByRole('heading', { name: 'Layout' }));
    const box = await screen.findByRole('textbox', { name: 'Note' });
    await waitFor(() => expect(box).toHaveFocus());
    expect(caretIn(box)).toBe(body.indexOf('## Layout'));
  });

  it('in Write, a tap below the text puts the caret at its end (PDT-264)', async () => {
    stubApi({ 'GET /api/pm/notes/a': note('a', { title: 'Plan', body: 'Short' }), 'GET /api/projects': [] });
    renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Write' }));
    const box = await screen.findByRole('textbox', { name: 'Note' });
    await userEvent.click(screen.getByRole('textbox', { name: 'Title' }));
    await userEvent.click(screen.getByTestId('end-of-text'));
    await waitFor(() => expect(box).toHaveFocus());
    expect(caretIn(box)).toBe('Short'.length);
  });

  it('opens the title on a double-click on it; not on a link or after a drag', async () => {
    stubApi({ 'GET /api/pm/notes/a': note('a', { title: 'Plan', body: 'See [the spec](https://x.test).' }), 'GET /api/projects': [] });
    renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.dblClick(await screen.findByRole('link', { name: 'the spec' }));
    const read = screen.getByTestId('note-body');
    fireEvent.pointerDown(read, { clientX: 10, clientY: 10 });
    fireEvent.doubleClick(read, { clientX: 80, clientY: 10 });
    expect(screen.queryByRole('textbox', { name: 'Title' })).not.toBeInTheDocument();
    await userEvent.dblClick(screen.getByRole('heading', { name: 'Plan', level: 1 }));
    expect(await screen.findByRole('textbox', { name: 'Title' })).toHaveFocus();
  });

  it('saves AUTOSAVE_MS after typing stops', async () => {
    const calls = stubApi({ 'GET /api/pm/notes/a': note('a', { title: 'Plan', body: 'Hello' }), 'PATCH /api/pm/notes/a': note('a', { title: 'Plan', body: 'Hello world' }), 'GET /api/projects': [] });
    renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Write' }));
    const body = await screen.findByRole('textbox', { name: 'Note' });
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    setEditorText(body, 'Hello wor');
    act(() => vi.advanceTimersByTime(AUTOSAVE_MS - 100));
    setEditorText(body, 'Hello world');
    act(() => vi.advanceTimersByTime(AUTOSAVE_MS - 1));
    expect(patches(calls)).toEqual([]);
    expect(screen.getByRole('status')).toHaveTextContent('Saving…');
    await act(async () => vi.advanceTimersByTime(1));
    vi.useRealTimers();
    await waitFor(() => expect(patches(calls)).toEqual([{ title: 'Plan', body: 'Hello world' }]));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/Saved · edited/));
  });

  it('never lets a poll overwrite an unsaved edit, but takes changes made elsewhere when nothing is unsaved', async () => {
    let theirs = note('a', { title: 'Plan', body: 'Hello' });
    stubApi({ 'GET /api/pm/notes/a': () => theirs, 'PATCH /api/pm/notes/a': (c: Call) => ({ ...theirs, ...(c.body as object) }), 'GET /api/projects': [] });
    const { qc } = renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Write' }));
    const body = await screen.findByRole('textbox', { name: 'Note' });
    setEditorText(body, 'Mine, unsaved');
    theirs = note('a', { title: 'Plan', body: 'Changed elsewhere', updatedAt: NOW + 1 });
    await act(() => qc.refetchQueries({ queryKey: ['pm-notes', 'one', 'a'] }));
    expect(editorText(body)).toBe('Mine, unsaved');
    // Once saved, a change made elsewhere shows.
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/Saved/), { timeout: 2000 });
    theirs = note('a', { title: 'Plan', body: 'The PM changed it', updatedAt: NOW + 2 });
    await act(() => qc.refetchQueries({ queryKey: ['pm-notes', 'one', 'a'] }));
    await waitFor(() => expect(editorText(body)).toBe('The PM changed it'));
  });

  it('saves an empty title as the first line of the text', async () => {
    let theirs = note('a', { title: 'Plan', body: '## First line\nmore' });
    const calls = stubApi({
      'GET /api/pm/notes/a': () => theirs,
      'PATCH /api/pm/notes/a': () => (theirs = note('a', { title: 'First line', body: '## First line\nmore', updatedAt: NOW + 1 })),
      'GET /api/projects': [],
    });
    renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Write' }));
    const title = screen.getByRole('textbox', { name: 'Title' });
    fireEvent.change(title, { target: { value: '' } });
    fireEvent.blur(title);
    await waitFor(() => expect(patches(calls)).toEqual([{ title: 'First line', body: '## First line\nmore' }]));
    // What was typed stays (empty), not the title it was saved with.
    expect(title).toHaveValue('');
  });

  it('creates a new note with its first save, in the side and scope chosen, then keeps editing it', async () => {
    const calls = stubApi({
      'POST /api/pm/notes': note('c', { title: 'Idea', projectId: project.id }),
      'GET /api/pm/notes/c': note('c', { title: 'Idea', projectId: project.id }),
      'PATCH /api/pm/notes/c': note('c', { title: 'Idea', body: 'More' }),
      'GET /api/projects': [project],
    });
    const props = screenProps({ id: 'new', scope: project.id });
    const { rerender } = renderWithQuery(<NoteScreen {...props} />);
    const title = await screen.findByRole('textbox', { name: 'Title' });
    expect(title).toHaveFocus();
    expect(calls.filter((c) => c.method === 'POST')).toEqual([]);
    fireEvent.change(title, { target: { value: 'Idea' } });
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ title: 'Idea', body: '', kind: 'user', project: project.id }), { timeout: 2000 });
    await waitFor(() => expect(props.onCreated).toHaveBeenCalledWith('c'));
    // The path changes to the note's own: the same editor stays.
    rerender(<NoteScreen {...props} id="c" />);
    expect(screen.getByRole('textbox', { name: 'Title' })).toBe(title);
    setEditorText(await screen.findByRole('textbox', { name: 'Note' }), 'More');
    fireEvent.focusOut(await screen.findByRole('textbox', { name: 'Note' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ title: 'Idea', body: 'More' }]));
    expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1);
  });

  it('creates nothing for a new note left empty; a PM note is created without a kind', async () => {
    const calls = stubApi({ 'GET /api/projects': [], 'POST /api/pm/notes': note('d', { kind: 'note' }) });
    const first = renderWithQuery(<NoteScreen {...screenProps({ id: 'new' })} />);
    fireEvent.blur(await screen.findByRole('textbox', { name: 'Title' }));
    first.unmount();
    expect(calls.filter((c) => c.method === 'POST')).toEqual([]);
    renderWithQuery(<NoteScreen {...screenProps({ id: 'new', whose: 'note' })} />);
    const body = await screen.findByRole('textbox', { name: 'Note' });
    setEditorText(body, 'Prefers small PRs');
    fireEvent.focusOut(body);
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ title: 'Prefers small PRs', body: 'Prefers small PRs' }));
    expect(screen.queryByRole('button', { name: 'Make a ticket' })).not.toBeInTheDocument();
  });

  it('shows Not saved with Retry when a save fails', async () => {
    let fail = true;
    const calls = stubApi({
      'GET /api/pm/notes/a': note('a', { title: 'Plan', body: 'Hello' }),
      'PATCH /api/pm/notes/a': () => (fail ? new Response(JSON.stringify({ code: 'boom', message: 'disk full' }), { status: 500 }) : note('a', { title: 'Plan', body: 'Hi' })),
      'GET /api/projects': [],
    });
    renderWithQuery(<NoteScreen {...screenProps()} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Write' }));
    const body = await screen.findByRole('textbox', { name: 'Note' });
    setEditorText(body, 'Hi');
    fireEvent.focusOut(body);
    expect(await screen.findByText(/could not be saved: disk full/)).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Not saved');
    fail = false;
    await userEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent(/Saved/));
    expect(patches(calls)).toHaveLength(2);
  });

  it('makes a ticket from one of my notes, linking back to it', async () => {
    stubApi({ 'GET /api/pm/notes/m': note('m', { title: 'Dark mode', body: 'Follow the OS.' }), 'GET /api/projects': [project], 'GET /api/labels': [] });
    renderWithQuery(<NoteScreen {...screenProps({ id: 'm' })} />);
    await screen.findByRole('heading', { name: 'Dark mode', level: 1 });
    await userEvent.click(screen.getByRole('button', { name: 'Make a ticket' }));
    const form = await screen.findByRole('dialog', { name: 'Make a ticket' });
    expect(within(form).getByRole('textbox', { name: 'Title' })).toHaveValue('Dark mode');
    expect(within(form).getByRole('textbox', { name: 'Description' })).toHaveValue(`Follow the OS.\n\nFrom the note [Dark mode](${window.location.origin}/notes/m).`);
  });

  it('pins, moves to a project and deletes', async () => {
    const mine = note('m', { title: 'Dark mode', body: 'Follow the OS.' });
    const calls = stubApi({
      'GET /api/pm/notes/m': mine,
      'PATCH /api/pm/notes/m': (c: Call) => ({ ...mine, ...(c.body as object) }),
      'DELETE /api/pm/notes/m': undefined,
      'GET /api/projects': [project],
      'GET /api/labels': [],
    });
    const props = screenProps({ id: 'm' });
    renderWithQuery(<NoteScreen {...props} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Pin: listed first' }));
    await waitFor(() => expect(patches(calls)).toEqual([{ pinned: true }]));
    await userEvent.click(screen.getByRole('button', { name: 'Project: Host-wide' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: project.name }));
    await waitFor(() => expect(patches(calls)).toEqual([{ pinned: true }, { project: project.id }]));

    await userEvent.click(screen.getByRole('button', { name: 'Note actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete…' }));
    const sure = await screen.findByRole('alertdialog', { name: /Delete “Dark mode”/ });
    expect(sure).toHaveTextContent('It is gone for good.');
    await userEvent.click(within(sure).getByRole('button', { name: 'Delete note' }));
    await waitFor(() => expect(props.onGone).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/pm/notes/m')).toBe(true);
  });

  it("pins the PM's notes into every briefing, with no Make a ticket", async () => {
    stubApi({ 'GET /api/pm/notes/a': note('a', { kind: 'note', body: 'x' }), 'GET /api/projects': [] });
    const props = screenProps({ whose: 'user' });
    renderWithQuery(<NoteScreen {...props} />);
    expect(await screen.findByRole('button', { name: 'Pin: in every briefing' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Make a ticket' })).not.toBeInTheDocument();
    expect(props.onKind).toHaveBeenCalledWith('note');
  });

  it('shows a deleted note as gone', async () => {
    stubApi({ 'GET /api/projects': [] });
    const props = screenProps({ id: 'x' });
    renderWithQuery(<NoteScreen {...props} />);
    expect(await screen.findByText('This note is gone.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Back to notes' }));
    expect(props.onGone).toHaveBeenCalled();
  });
});
