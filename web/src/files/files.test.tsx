import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { toast } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, Reply, stubApi, stubClipboard } from '../test-utils';
import { FileViewer, FolderList, FolderTree } from './FolderView';
import { useFolderView } from './useFolderView';

vi.mock('./CodeViewer', () => ({
  CodeViewer: ({ name, text, onChange }: { name: string; text: string; onChange?: (t: string) => void }) =>
    onChange
      ? <textarea aria-label="Editor" data-name={name} defaultValue={text} onChange={(e) => onChange(e.target.value)} />
      : <pre data-testid="code" data-name={name}>{text}</pre>,
}));
vi.mock('../common/MarkdownEditor', () => ({
  MarkdownEditor: ({ label, value, onChange }: { label: string; value: string; onChange: (t: string) => void }) =>
    <textarea aria-label={label} data-markdown defaultValue={value} onChange={(e) => onChange(e.target.value)} />,
}));
vi.mock('../review/CodeDiff', () => ({
  CodeDiff: ({ oldText, newText }: { oldText: string; newText: string }) => <pre data-testid="diff">{`${oldText}|${newText}`}</pre>,
}));
afterEach(() => { vi.unstubAllGlobals(); toast.dismiss(); });

const project: Project = { id: 'p', name: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 1, key: 'PDT', forge: null, kind: 'repo' };
const entry = (path: string, dir = false, status: string | null = null) => ({ name: path.split('/').pop()!, path, dir, status });

function Harness() {
  const state = useFolderView([project], null);
  return <><FolderTree state={state} /><main><FileViewer state={state} /></main></>;
}

describe('folder view', () => {
  it('browses folders lazily and shows a file read-only', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/files?path=': [entry('src', true, 'M'), entry('README.md')],
      'GET /api/projects/p/files?path=src': [entry('src/main.rs', false, '??')],
      'GET /api/projects/p/files/content?path=src%2Fmain.rs': { path: 'src/main.rs', size: 13, status: '??', text: 'fn main() {}\n', binary: false, tooLarge: false },
      'GET /api/projects/p/files/content?path=README.md': { path: 'README.md', size: 4, status: null, text: null, binary: true, tooLarge: false },
    });
    renderWithQuery(<Harness />);
    expect(screen.getByText(/Pick a file/)).toBeInTheDocument();
    expect(await screen.findByRole('button', { name: 'Place: PDT › Repository' })).toBeInTheDocument();
    await userEvent.click(await screen.findByRole('row', { name: /src/ }));
    expect(screen.getByRole('row', { name: /src/ })).toHaveAttribute('aria-expanded', 'true');
    await userEvent.click(await screen.findByRole('row', { name: /main\.rs/ }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveTextContent('fn main() {}'));
    expect(screen.getByTestId('code')).toHaveAttribute('data-name', 'src/main.rs');
    expect(screen.getByRole('row', { name: /main\.rs/ })).toHaveAttribute('aria-selected', 'true');
    const header = screen.getByRole('banner');
    expect(within(header).getByRole('heading', { name: 'src/main.rs' })).toHaveAttribute('title', 'src/main.rs');
    expect(within(header).getByTitle('git: ??')).toHaveTextContent('U');
    expect(within(header).getByText('13 bytes')).toBeInTheDocument();
    const writeText = vi.fn(async () => {});
    stubClipboard({ writeText });
    await userEvent.click(within(header).getByRole('button', { name: 'File actions' }));
    await userEvent.click(within(await screen.findByRole('menu', { name: 'File actions' })).getByRole('menuitem', { name: 'Copy path' }));
    expect(writeText).toHaveBeenCalledWith('src/main.rs');
    expect(await screen.findByText('Path copied')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('row', { name: /README/ }));
    expect(await screen.findByText('This is a binary file.')).toBeInTheDocument();
  });

  it('shows a Markdown file rendered, or as its source', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/files?path=': [entry('README.md')],
      'GET /api/projects/p/files/content?path=README.md': { path: 'README.md', size: 20, status: null, text: '# Hello\n\nSome *text*.\n', binary: false, tooLarge: false },
    });
    renderWithQuery(<Harness />);
    await userEvent.click(await screen.findByRole('row', { name: /README/ }));
    expect(await screen.findByRole('heading', { name: 'Hello', level: 1 })).toBeInTheDocument();
    expect(screen.queryByTestId('code')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Source' }));
    expect(screen.getByTestId('code')).toHaveTextContent('# Hello');
    expect(screen.queryByRole('heading', { name: 'Hello', level: 1 })).not.toBeInTheDocument();
  });

  it('marks git status, strikes deleted files and shows a folder that failed to load', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/files?path=': [entry('gone.txt', false, 'D'), entry('locked', true)],
      'GET /api/projects/p/files?path=locked': new Reply(500, { code: 'internal', message: 'permission denied' }),
    });
    renderWithQuery(<Harness />);
    const gone = await screen.findByRole('row', { name: /gone\.txt/ });
    expect(gone).toHaveTextContent('D');
    expect(within(gone).getByText('gone.txt')).toHaveClass('line-through');
    await userEvent.click(screen.getByRole('row', { name: /locked/ }));
    expect(await screen.findByRole('alert')).toHaveTextContent('permission denied');
  });

  it('shows an error when the checkout cannot be listed', async () => {
    stubApi({ 'GET /api/projects/p/worktrees': [], 'GET /api/projects/p/files?path=': new Reply(404, { code: 'not_found', message: 'checkout is missing' }) });
    renderWithQuery(<Harness />);
    expect(await screen.findByRole('alert')).toHaveTextContent('checkout is missing');
  });
});

/** The phone's screens (PDT-120): a folder path ends in `/`; anything else is a file. */
function PhoneHarness() {
  const state = useFolderView([project], null);
  const file = state.file;
  if (file && !file.endsWith('/')) return <FileViewer state={state} readOnly />;
  return <FolderList state={state} dir={file ? file.slice(0, -1) : ''} />;
}

describe('folder list (phones)', () => {
  it('drills into folders one screen at a time and opens a file', async () => {
    stubApi({
      'GET /api/projects/p/worktrees': [],
      'GET /api/projects/p/files?path=': [entry('src', true, 'M'), entry('README.md')],
      'GET /api/projects/p/files?path=src': [entry('src/main.rs', false, '??')],
      'GET /api/projects/p/files/content?path=src%2Fmain.rs': { path: 'src/main.rs', size: 13, status: '??', text: 'fn main() {}\n', binary: false, tooLarge: false },
    });
    renderWithQuery(<PhoneHarness />);
    expect(await screen.findByRole('button', { name: 'Place: PDT › Repository' })).toBeInTheDocument();
    await userEvent.click(await within(await screen.findByRole('grid', { name: 'Files' })).findByRole('row', { name: /src/ }));
    const folder = await screen.findByRole('grid', { name: 'Files in src' });
    expect(screen.queryByRole('button', { name: 'Place: PDT › Repository' })).not.toBeInTheDocument();
    await userEvent.click(within(folder).getByRole('row', { name: /main\.rs/ }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveTextContent('fn main() {}'));
    // View-only on phones.
    expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
  });
});

const text = (path: string, body: string, hash = `h-${body}`, status: string | null = null) =>
  ({ path, size: body.length, status, text: body, binary: false, tooLarge: false, hash });

const agent = (over: Partial<Session> = {}): Session => ({
  id: 's1', projectId: 'p', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Coder · PDT-7',
  cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'working', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: null, persona: null, background: [], ...over,
});

const editRoutes = (extra: Record<string, unknown> = {}) => ({
  'GET /api/projects/p/worktrees': [],
  'GET /api/projects/p/sessions': [],
  'GET /api/projects/p/files?path=': [entry('a.rs'), entry('b.rs'), entry('blob.bin')],
  'GET /api/projects/p/files/content?path=a.rs': text('a.rs', 'old\n'),
  'GET /api/projects/p/files/content?path=b.rs': text('b.rs', 'b\n'),
  'GET /api/projects/p/files/content?path=blob.bin': { path: 'blob.bin', size: 3, status: null, text: null, binary: true, tooLarge: false, hash: 'x' },
  ...extra,
});

async function startEditing(name: RegExp, editor = 'Editor') {
  await userEvent.click(await screen.findByRole('row', { name }));
  await userEvent.click(await screen.findByRole('button', { name: 'Edit' }));
  return screen.findByRole('textbox', { name: editor });
}

describe('editing files', () => {
  it('edits a file and saves it with the hash it was opened at, by button or ⌘S', async () => {
    const calls = stubApi(editRoutes({
      'PUT /api/projects/p/files/content': ({ body }: { body: { text: string } }) => text('a.rs', body.text, `h-${body.text}`, 'M'),
    }));
    renderWithQuery(<Harness />);
    const editor = await startEditing(/a\.rs/);
    expect(screen.getByRole('button', { name: 'Save' })).toBeDisabled();
    await userEvent.clear(editor);
    await userEvent.type(editor, 'new');
    expect(screen.getByText('Unsaved changes')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument());
    expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ path: 'a.rs', text: 'new', baseHash: 'h-old\n' });
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument();
    // Still editing, from the saved text: ⌘S saves the next change against the new hash.
    await userEvent.type(editor, '!');
    await userEvent.keyboard('{Meta>}s{/Meta}');
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    expect(calls.filter((c) => c.method === 'PUT')[1].body).toEqual({ path: 'a.rs', text: 'new!', baseHash: 'h-new' });
    await waitFor(() => expect(screen.getByRole('button', { name: 'Done' })).toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: 'Done' }));
    expect(screen.queryByRole('textbox', { name: 'Editor' })).not.toBeInTheDocument();
  });

  it('never edits binary files', async () => {
    stubApi(editRoutes());
    renderWithQuery(<Harness />);
    await userEvent.click(await screen.findByRole('row', { name: /blob\.bin/ }));
    expect(await screen.findByText('This is a binary file.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Edit' })).not.toBeInTheDocument();
  });

  it('asks before leaving unsaved changes', async () => {
    stubApi(editRoutes());
    renderWithQuery(<Harness />);
    const editor = await startEditing(/a\.rs/);
    await userEvent.type(editor, 'x');
    await userEvent.click(screen.getByRole('row', { name: /b\.rs/ }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Discard unsaved changes?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    expect(screen.getByRole('textbox', { name: 'Editor' })).toHaveValue('old\nx');
    await userEvent.click(screen.getByRole('row', { name: /b\.rs/ }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Discard changes' }));
    await waitFor(() => expect(screen.getByTestId('code')).toHaveTextContent('b'));
  });

  it('refuses to overwrite a file that changed on disk: look at the diff, then overwrite', async () => {
    let disk = text('a.rs', 'theirs\n');
    const calls = stubApi(editRoutes({
      'GET /api/projects/p/files/content?path=a.rs': () => disk,
      'PUT /api/projects/p/files/content': ({ body }: { body: { text: string; baseHash: string } }) =>
        body.baseHash === disk.hash ? text('a.rs', body.text, 'h-mine') : new Reply(409, { code: 'file_changed', message: 'changed' }),
    }));
    renderWithQuery(<Harness />);
    const editor = await startEditing(/a\.rs/);
    disk = text('a.rs', 'agent wrote\n');
    await userEvent.clear(editor);
    await userEvent.type(editor, 'mine');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('a.rs changed on disk since you opened it')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Show diff' }));
    const dialog = await screen.findByRole('dialog', { name: 'a.rs: on disk → yours' });
    expect((await within(dialog).findByTestId('diff')).textContent).toBe('agent wrote\n|mine');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Overwrite' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT')).toHaveLength(2));
    const puts = calls.filter((c) => c.method === 'PUT');
    expect(puts.map((c) => (c.body as { baseHash: string }).baseHash)).toEqual(['h-theirs\n', 'h-agent wrote\n']);
    await waitFor(() => expect(screen.queryByText(/changed on disk/)).not.toBeInTheDocument());
  });

  it('reloads the file from disk after a conflict', async () => {
    let disk = text('a.rs', 'theirs\n');
    stubApi(editRoutes({
      'GET /api/projects/p/files/content?path=a.rs': () => disk,
      'PUT /api/projects/p/files/content': new Reply(409, { code: 'file_changed', message: 'changed' }),
    }));
    renderWithQuery(<Harness />);
    const editor = await startEditing(/a\.rs/);
    disk = text('a.rs', 'agent wrote\n');
    await userEvent.type(editor, 'mine');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Reload' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Editor' })).toHaveValue('agent wrote\n'));
    expect(screen.queryByText('Unsaved changes')).not.toBeInTheDocument();
  });

  it('warns that an agent works in the checkout and offers to tell it what changed', async () => {
    const calls = stubApi(editRoutes({
      'GET /api/projects/p/sessions': [agent(), agent({ id: 'other', title: 'Elsewhere', worktreeId: 'w' }), agent({ id: 'idle', title: 'Idle', activity: 'idle' })],
      'PUT /api/projects/p/files/content': ({ body }: { body: { text: string } }) => text('a.rs', body.text),
      'POST /api/projects/p/files/tell': undefined,
    }));
    renderWithQuery(<Harness />);
    const editor = await startEditing(/a\.rs/);
    expect(await screen.findByText(/Coder · PDT-7 is working in this checkout/)).toBeInTheDocument();
    expect(screen.queryByText(/Elsewhere|Idle/)).not.toBeInTheDocument();
    await userEvent.type(editor, 'x');
    await userEvent.click(screen.getByRole('button', { name: 'Save' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Tell the agent' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/projects/p/files/tell')?.body).toEqual({ sessionId: 's1', paths: ['a.rs'] }));
  });

  it('holds Rename and Delete of a file with unsaved edits', async () => {
    stubApi(editRoutes());
    renderWithQuery(<Harness />);
    const editor = await startEditing(/a\.rs/);
    await userEvent.type(editor, 'x');
    await userEvent.pointer({ keys: '[MouseRight]', target: screen.getByRole('row', { name: /a\.rs/ }) });
    const menu = await screen.findByRole('menu', { name: 'a.rs actions' });
    expect(within(menu).getByRole('menuitem', { name: 'Rename…' })).toHaveAttribute('aria-disabled', 'true');
    expect(within(menu).getByRole('menuitem', { name: 'Delete…' })).toHaveAttribute('aria-disabled', 'true');
    await userEvent.keyboard('{Escape}');
    // Other files are not held.
    await userEvent.pointer({ keys: '[MouseRight]', target: screen.getByRole('row', { name: /b\.rs/ }) });
    const other = await screen.findByRole('menu', { name: 'b.rs actions' });
    expect(within(other).getByRole('menuitem', { name: 'Delete…' })).not.toHaveAttribute('aria-disabled');
  });

  it('edits Markdown with a preview', async () => {
    stubApi(editRoutes({
      'GET /api/projects/p/files?path=': [entry('README.md')],
      'GET /api/projects/p/files/content?path=README.md': text('README.md', '# Hi\n'),
    }));
    renderWithQuery(<Harness />);
    const editor = await startEditing(/README/, 'README.md');
    await userEvent.type(editor, '\n## More');
    await userEvent.click(screen.getByRole('radio', { name: 'Preview' }));
    expect(await screen.findByRole('heading', { name: 'More', level: 2 })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Edit' }));
    expect(screen.getByRole('textbox', { name: 'README.md' })).toHaveValue('# Hi\n\n## More');
  });

  it('creates, renames and deletes files', async () => {
    const calls = stubApi(editRoutes({
      'PUT /api/projects/p/files/content': text('src/new.rs', ''),
      'GET /api/projects/p/files/content?path=src%2Fnew.rs': text('src/new.rs', ''),
      'GET /api/projects/p/files/content?path=c.rs': text('c.rs', 'b\n'),
      'POST /api/projects/p/files/rename': undefined,
      'DELETE /api/projects/p/files/content': undefined,
    }));
    renderWithQuery(<Harness />);
    await userEvent.click(await screen.findByRole('button', { name: 'New file' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'New file path…' }));
    await userEvent.type(await screen.findByRole('textbox', { name: 'New file path…' }), 'src/new.rs{Enter}');
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ path: 'src/new.rs', text: '' }));
    expect(await screen.findByRole('heading', { name: 'src/new.rs' })).toBeInTheDocument();

    // From the tree: right-click a file.
    await userEvent.pointer({ keys: '[MouseRight]', target: screen.getByRole('row', { name: /b\.rs/ }) });
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Rename…' }));
    const field = await screen.findByRole('textbox', { name: 'Rename…' });
    expect(field).toHaveValue('b.rs');
    await userEvent.clear(field);
    await userEvent.type(field, 'c.rs{Enter}');
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ from: 'b.rs', to: 'c.rs' }));

    await userEvent.pointer({ keys: '[MouseRight]', target: screen.getByRole('row', { name: /a\.rs/ }) });
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Delete…' }));
    const dialog = await screen.findByRole('alertdialog', { name: 'Delete a.rs?' });
    await userEvent.click(within(dialog).getByRole('button', { name: 'Delete file' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'DELETE')?.path).toBe('/api/projects/p/files/content?path=a.rs'));
  });
});
