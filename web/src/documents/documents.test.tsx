import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import { focusManager } from '@tanstack/react-query';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Document } from '../api/gen/Document';
import type { DocumentProvidersView } from '../api/gen/DocumentProvidersView';
import { editorText, renderWithQuery, Reply, setEditorText, stubApi } from '../test-utils';
import { TicketDocuments } from '../tickets/TicketDocuments';
import { ticket } from '../tickets/fixtures';
import { DocumentPage } from './DocumentPage';
import { DocumentsList, useDocumentsFilters } from './DocumentsList';
import { anchorLabel, headingsOf } from './DocumentComments';
import { ImportDocumentDialog, NewDocumentDialog } from './DocumentDialogs';
import { pickerModules } from './picker';
import { api } from '../api/client';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const all = { docs: true, sheets: true, create: true, search: true, edit: true, rename: true, delete: true, comments: true, embed: false, charts: false, live: false };

const providers: DocumentProvidersView = {
  providers: [
    { id: 'fake', label: 'Fake', capabilities: all, picker: null },
    { id: 'reader', label: 'Reader', capabilities: { ...all, create: false, search: false, delete: false }, picker: null },
  ],
  default: 'fake',
  folder: '/data/documents',
};

const project = { id: 'p1', name: 'PDT', key: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: 0, kind: 'repo' };

function doc(id: string, title: string, over: Partial<Document> = {}): Document {
  return {
    id, title, kind: 'doc', provider: 'fake', providerRef: `fake:${id}`, url: null, actor: 'user', agent: null,
    createdAt: Date.now(), updatedAt: Date.now(), projects: [], tickets: [], ...over,
  };
}

function List() {
  const filters = useDocumentsFilters();
  return <DocumentsList filters={filters} selectedId={null} onOpen={() => {}} onNew={() => {}} onImport={() => {}} />;
}

describe('documents', () => {
  it('the list shows documents and filters them by kind and search', async () => {
    const plan = doc('d1', 'Plan', { projects: [{ id: 'p1', key: 'PDT', name: 'PDT' }] });
    const budget = doc('d2', 'Budget', { kind: 'sheet' });
    const calls = stubApi({
      'GET /api/documents': (c) => {
        const q = new URLSearchParams(c.path.split('?')[1] ?? '');
        return [plan, budget].filter((d) => (!q.get('kind') || d.kind === q.get('kind')) && (!q.get('q') || d.title.toLowerCase().includes(q.get('q')!)));
      },
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
    });
    renderWithQuery(<List />);
    const list = await screen.findByRole('grid', { name: 'Documents' });
    expect(within(list).getByText('Plan')).toBeInTheDocument();
    expect(within(list).getByText('Budget')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Kind/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Sheets' }));
    await waitFor(() => expect(within(list).queryByText('Plan')).not.toBeInTheDocument());
    expect(calls.some((c) => c.path.includes('kind=sheet'))).toBe(true);
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search documents' }), 'zzz');
    expect(await screen.findByText('No document matches.')).toBeInTheDocument();
  });

  it('the list loads 100 documents at a time, Show more the next', async () => {
    const many = Array.from({ length: 150 }, (_, i) => doc(`d${i}`, `Doc ${i}`));
    const calls = stubApi({
      'GET /api/documents': (c) => many.slice(0, Number(new URLSearchParams(c.path.split('?')[1]).get('limit'))),
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
    });
    renderWithQuery(<List />);
    const list = await screen.findByRole('grid', { name: 'Documents' });
    expect(within(list).getAllByRole('row')).toHaveLength(100);
    await userEvent.click(screen.getByRole('button', { name: 'Show more' }));
    await waitFor(() => expect(within(list).getAllByRole('row')).toHaveLength(150));
    expect(screen.queryByRole('button', { name: 'Show more' })).not.toBeInTheDocument();
    expect(calls.some((c) => c.path.includes('limit=200'))).toBe(true);
  });

  it('New document makes one in the default provider, linked to the project', async () => {
    const calls = stubApi({
      'GET /api/documents/providers': providers,
      'POST /api/documents': (c) => doc('d9', (c.body as { title: string }).title),
    });
    const created = vi.fn();
    renderWithQuery(<NewDocumentDialog projects={[project as never]} projectId="p1" onClose={() => {}} onCreated={created} />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Title' }), 'Q4 plan');
    await waitFor(() => expect(screen.getByRole('button', { name: 'Create' })).toBeEnabled());
    await userEvent.click(screen.getByRole('button', { name: 'Create' }));
    await waitFor(() => expect(created).toHaveBeenCalled());
    const post = calls.find((c) => c.method === 'POST')!;
    expect(post.body).toEqual({ title: 'Q4 plan', kind: 'doc', provider: 'fake', projectId: 'p1', by: 'user' });
  });

  it('a ticket shows its documents, makes its design doc and unlinks one', async () => {
    let linked = [doc('d1', 'Old notes')];
    const calls = stubApi({
      'GET /api/documents': () => linked,
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
      'POST /api/tickets/t1/design-doc': () => {
        const d = doc('d2', 'PDT-1 design: Faster');
        linked = [...linked, d];
        return d;
      },
      'DELETE /api/documents/d1/links': () => {
        linked = linked.filter((d) => d.id !== 'd1');
        return doc('d1', 'Old notes');
      },
    });
    renderWithQuery(<TicketDocuments ticket={ticket(1, { id: 't1', key: 'PDT-1', projectId: 'p1' })} />);
    const section = await screen.findByRole('region', { name: 'Documents' });
    expect(await within(section).findByRole('button', { name: 'Old notes' })).toBeInTheDocument();
    await userEvent.click(within(section).getByRole('button', { name: 'Create design doc' }));
    expect(await within(section).findByRole('button', { name: 'PDT-1 design: Faster' })).toBeInTheDocument();
    expect(calls.find((c) => c.path === '/api/tickets/t1/design-doc')?.body).toEqual({ by: 'user' });
    await userEvent.click(within(section).getByRole('button', { name: 'Unlink Old notes' }));
    await waitFor(() => expect(within(section).queryByRole('button', { name: 'Old notes' })).not.toBeInTheDocument());
    expect(calls.find((c) => c.method === 'DELETE')?.body).toEqual({ ticketId: 't1' });
  });

  it('a document page shows its content and links, renames it and removes it from PDT', async () => {
    let current = doc('d1', 'Plan', { tickets: [{ id: 't1', key: 'PDT-1', title: 'Faster', projectId: 'p1' }] });
    const gone = vi.fn();
    const calls = stubApi({
      'GET /api/documents/d1': () => current,
      'GET /api/documents/d1/content': { content: { kind: 'doc', markdown: '## Goal\n\nShip it.', version: '1' }, embedUrl: null },
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
      'PATCH /api/documents/d1': (c) => {
        current = { ...current, title: (c.body as { title: string }).title };
        return current;
      },
      'DELETE /api/documents/d1': undefined,
    });
    renderWithQuery(<DocumentPage id="d1" phone={false} onGone={gone} />);
    expect(await screen.findByRole('heading', { name: 'Plan', level: 1 })).toBeInTheDocument();
    expect(await screen.findByText('Ship it.')).toBeInTheDocument();
    expect(screen.getByText('PDT-1')).toBeInTheDocument();
    const contentReads = calls.filter((c) => c.path === '/api/documents/d1/content').length;
    await userEvent.click(screen.getByRole('button', { name: 'Rename' }));
    const field = screen.getByRole('textbox', { name: 'Document title' });
    await userEvent.clear(field);
    await userEvent.type(field, 'Q4 plan{Enter}');
    expect(await screen.findByRole('heading', { name: 'Q4 plan', level: 1 })).toBeInTheDocument();
    // A rename reads the document again, not its content from the provider.
    expect(calls.filter((c) => c.path === '/api/documents/d1/content')).toHaveLength(contentReads);
    await userEvent.click(screen.getByRole('button', { name: 'Document actions' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Remove from PDT…' }));
    const confirm = await screen.findByRole('alertdialog');
    await userEvent.click(within(confirm).getByRole('button', { name: 'Remove from PDT' }));
    await waitFor(() => expect(gone).toHaveBeenCalled());
    expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/documents/d1')).toBe(true);
  });

  it('a sheet shows as a table with its formulas', async () => {
    stubApi({
      'GET /api/documents/d2': doc('d2', 'Budget', { kind: 'sheet' }),
      'GET /api/documents/d2/content': {
        content: { kind: 'sheet', version: '1', sheets: [{ name: 'Sheet1', rows: [[{ value: 'Rent' }, { value: '12', formula: '=6*2' }]], charts: [] }] },
        embedUrl: null,
      },
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
    });
    renderWithQuery(<DocumentPage id="d2" phone={false} onGone={() => {}} />);
    const table = await screen.findByRole('table');
    expect(within(table).getByRole('columnheader', { name: 'B' })).toBeInTheDocument();
    expect(within(table).getByText('12')).toHaveAttribute('title', '=6*2');
  });

  it('a local doc is written in Write, saved as typed on top of its version, and a clash asks', async () => {
    const local = { id: 'local', label: 'Local', capabilities: { ...all, rename: false, charts: true, live: true } };
    let saves = 0;
    const calls = stubApi({
      'GET /api/documents/d3': doc('d3', 'Notes', { provider: 'local' }),
      'GET /api/documents/d3/content': { content: { kind: 'doc', markdown: '# Notes\n\n## Goal\n', version: 'v1' }, embedUrl: null },
      'GET /api/documents/d3/comments': [],
      'GET /api/documents/providers': { ...providers, providers: [local] },
      'GET /api/projects': [project],
      'POST /api/documents/d3/edits': () => (++saves === 2 ? new Reply(409, { code: 'conflict', message: 'changed' }) : { version: `v${saves + 1}` }),
    });
    renderWithQuery(<DocumentPage id="d3" phone={false} onGone={() => {}} />);
    expect(await screen.findByRole('heading', { name: 'Goal', level: 2 })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Write' }));
    const box = await screen.findByRole('textbox', { name: 'Document text' });
    expect(editorText(box)).toBe('# Notes\n\n## Goal\n');
    setEditorText(box, '# Notes\n\n## Goal\n\nShip it.\n');
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(1));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ edits: [{ op: 'replace_text', markdown: '# Notes\n\n## Goal\n\nShip it.\n' }], baseVersion: 'v1' });
    expect(await screen.findByText('Saved')).toBeInTheDocument();

    // Someone else saved in between: nothing is overwritten until the user says.
    setEditorText(box, 'Mine\n');
    expect(await screen.findByText('This document changed while you were writing', {})).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Keep mine' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(3));
    const last = calls.filter((c) => c.method === 'POST').at(-1)!;
    expect(last.body).toEqual({ edits: [{ op: 'replace_text', markdown: 'Mine\n' }] });
    await waitFor(() => expect(screen.queryByText('This document changed while you were writing')).not.toBeInTheDocument());
  });

  it('typing while a save is on its way is saved next, not lost', async () => {
    const local = { id: 'local', label: 'Local', capabilities: { ...all, rename: false, charts: true, live: true } };
    let release: (() => void) | null = null;
    let saves = 0;
    const calls = stubApi({
      'GET /api/documents/d4': doc('d4', 'Draft', { provider: 'local' }),
      'GET /api/documents/d4/content': { content: { kind: 'doc', markdown: 'A\n', version: 'v1' }, embedUrl: null },
      'GET /api/documents/d4/comments': [],
      'GET /api/documents/providers': { ...providers, providers: [local] },
      'GET /api/projects': [project],
      'POST /api/documents/d4/edits': () => {
        saves += 1;
        if (saves > 1) return { version: `v${saves + 1}` };
        return new Promise((resolve) => { release = () => resolve({ version: 'v2' }); });
      },
    });
    renderWithQuery(<DocumentPage id="d4" phone={false} onGone={() => {}} />);
    await userEvent.click(await screen.findByRole('radio', { name: 'Write' }));
    const box = await screen.findByRole('textbox', { name: 'Document text' });
    setEditorText(box, 'AB\n');
    await waitFor(() => expect(release).not.toBeNull());
    // More typing while the first save is on its way.
    setEditorText(box, 'ABC\n');
    release!();
    expect(await screen.findByText('Unsaved changes')).toBeInTheDocument();
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST')).toHaveLength(2));
    const posts = calls.filter((c) => c.method === 'POST');
    expect(posts[0]!.body).toEqual({ edits: [{ op: 'replace_text', markdown: 'AB\n' }], baseVersion: 'v1' });
    expect(posts[1]!.body).toEqual({ edits: [{ op: 'replace_text', markdown: 'ABC\n' }], baseVersion: 'v2' });
    expect(await screen.findByText('Saved')).toBeInTheDocument();
    expect(editorText(box)).toBe('ABC\n');
  });

  it('comments go on a heading of a doc, and show where they are', async () => {
    const calls = stubApi({
      'GET /api/documents/d1': doc('d1', 'Plan'),
      'GET /api/documents/d1/content': { content: { kind: 'doc', markdown: '# Plan\n\n## Goal\n\nShip it.\n', version: '1' }, embedUrl: null },
      'GET /api/documents/d1/comments': () => calls.some((c) => c.method === 'POST')
        ? [{ id: 'c1', author: 'You', body: 'Which goal?', quote: 'Goal', anchor: { on: 'heading', heading: 'Goal' }, resolved: false, createdAt: Date.now(), replies: [] }]
        : [],
      'POST /api/documents/d1/comments': { id: 'c1' },
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
    });
    renderWithQuery(<DocumentPage id="d1" phone={false} onGone={() => {}} />);
    const section = await screen.findByRole('region', { name: 'Comments' });
    expect(await within(section).findByText('No comments yet.')).toBeInTheDocument();
    await userEvent.click(within(section).getByRole('button', { name: 'Comment' }));
    await userEvent.click(within(section).getByRole('button', { name: /Comment on/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Heading: Goal' }));
    await userEvent.type(within(section).getByRole('textbox', { name: 'New comment' }), 'Which goal?');
    await userEvent.click(within(section).getByRole('button', { name: 'Comment' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ body: 'Which goal?', anchor: { on: 'heading', heading: 'Goal' }, by: 'user' }));
    expect(await within(section).findByText('Which goal?')).toBeInTheDocument();
    expect(within(section).getByText('Goal')).toBeInTheDocument();
  });

  it('headings and anchors read as people see them', () => {
    expect(headingsOf('# A\n\n```\n# not one\n```\n## B ##\n#nope\n')).toEqual(['A', 'B']);
    expect(anchorLabel({ on: 'cell', sheet: 'Sheet1', range: 'B3:C4' })).toBe('Sheet1!B3:C4');
    expect(anchorLabel({ on: 'text', quote: 'ship' })).toBe('“ship”');
  });

  it('a document with an embedded editor shows it, or PDT’s view, and remembers which', async () => {
    localStorage.removeItem('pdt.documents.view');
    const google = { id: 'google', label: 'Google', capabilities: { ...all, embed: true } };
    stubApi({
      'GET /api/documents/g1': doc('g1', 'Plan', { provider: 'google', providerRef: 'c1/d1', url: 'https://docs.google.com/document/d/d1/edit' }),
      'GET /api/documents/g1/content': { content: { kind: 'doc', markdown: 'Ship it.', version: '3' }, embedUrl: 'https://docs.google.com/document/d/d1/edit?rm=minimal' },
      'GET /api/documents/g1/version': { version: '3' },
      'GET /api/documents/providers': { ...providers, providers: [...providers.providers, google] },
      'GET /api/projects': [project],
    });
    renderWithQuery(<DocumentPage id="g1" phone={false} onGone={() => {}} />);
    const frame = await screen.findByTitle('Plan in Google');
    expect(frame).toHaveAttribute('src', 'https://docs.google.com/document/d/d1/edit?rm=minimal');
    expect(screen.queryByText('Ship it.')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Read in PDT' }));
    expect(await screen.findByText('Ship it.')).toBeInTheDocument();
    expect(screen.queryByTitle('Plan in Google')).not.toBeInTheDocument();
    expect(localStorage.getItem('pdt.documents.view')).toBe('pdt');
  });

  it('Read in PDT reads an embedded document again when its version changes; one without an embed never polls', async () => {
    localStorage.setItem('pdt.documents.view', 'pdt');
    let version = '1';
    const calls = stubApi({
      'GET /api/documents/d1': doc('d1', 'Plan', { provider: 'google' }),
      'GET /api/documents/d1/content': () => ({ content: { kind: 'doc', markdown: `Version ${version}`, version }, embedUrl: 'https://docs.google.com/document/d/x/edit' }),
      'GET /api/documents/d1/version': () => ({ version }),
      'GET /api/documents/d2': doc('d2', 'Notes'),
      'GET /api/documents/d2/content': { content: { kind: 'doc', markdown: 'Local', version: '1' }, embedUrl: null },
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
    });
    const { qc, unmount } = renderWithQuery(<DocumentPage id="d1" phone={false} onGone={() => {}} />);
    expect(await screen.findByText('Version 1')).toBeInTheDocument();
    version = '2';
    await qc.refetchQueries({ queryKey: ['documents', 'version', 'd1'] });
    expect(await screen.findByText('Version 2')).toBeInTheDocument();
    unmount();
    localStorage.removeItem('pdt.documents.view');
    renderWithQuery(<DocumentPage id="d2" phone={false} onGone={() => {}} />);
    expect(await screen.findByText('Local')).toBeInTheDocument();
    expect(calls.some((c) => c.path === '/api/documents/d2/version')).toBe(false);
  });

  it('a provider’s own picker opens for each signed-in account and imports what it picks', async () => {
    const picker = { label: 'Browse the drive', module: '/api/drive/picker.js', accounts: '/api/drive/accounts', help: 'Sign in to pick.' };
    const drive = { id: 'drive', label: 'Drive', capabilities: { ...all, embed: true }, picker };
    const pick = vi.fn().mockResolvedValue(['c1/f1']);
    const load = vi.spyOn(pickerModules, 'load').mockResolvedValue({ pick });
    const onImported = vi.fn();
    const calls = stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/documents/providers': { ...providers, providers: [drive], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [
        { connectionId: 'c1', name: 'Work', signedIn: true, email: 'me@work.com', hasClient: true, builtIn: true, agents: false, picker: true, note: null },
        { connectionId: 'c2', name: 'Home', signedIn: false, email: null, hasClient: true, builtIn: true, agents: false, picker: true, note: null },
      ],
      'POST /api/documents/import': doc('d9', 'Picked'),
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={onImported} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Browse the drive…' }));
    await waitFor(() => expect(onImported).toHaveBeenCalled());
    expect(load).toHaveBeenCalledWith('/api/drive/picker.js');
    expect(pick.mock.calls[0][0].account).toBe('c1');
    expect(calls.find((c) => c.path === '/api/documents/import')?.body).toEqual({ provider: 'drive', reference: 'c1/f1', by: 'user' });
  });

  it('closing a provider’s picker without a file brings focus back to its button', async () => {
    const picker = { label: 'Browse the drive', module: '/api/drive/picker.js', accounts: '/api/drive/accounts', help: 'Sign in to pick.' };
    vi.spyOn(pickerModules, 'load').mockResolvedValue({ pick: vi.fn().mockResolvedValue([]) });
    stubApi({
      'GET /api/ui/token': { token: 'ui' },
      'GET /api/documents/providers': { ...providers, providers: [{ id: 'drive', label: 'Drive', capabilities: all, picker }], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [
        { connectionId: 'c1', name: 'Work', signedIn: true, email: 'me@work.com', hasClient: true, builtIn: true, agents: false, picker: true, note: null },
      ],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    (await screen.findByRole('button', { name: 'Browse the drive…' })).focus();
    await userEvent.keyboard('{Enter}');
    await waitFor(() => expect(document.activeElement).toBe(screen.getByRole('button', { name: 'Browse the drive…' })));
  });

  it('a provider’s picker says how to get one when no account opens it', async () => {
    const picker = { label: 'Browse the drive', module: '/api/drive/picker.js', accounts: '/api/drive/accounts', help: 'Sign in to pick.' };
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [{ id: 'drive', label: 'Drive', capabilities: all, picker }], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    expect(await screen.findByText('Sign in to pick.')).toBeInTheDocument();
  });

  // Google with PDT's own sign-in, in a build without the picker's key: nothing to set, so nothing asked (PDT-493).
  const texts = {
    help: 'Sign in to pick.',
    unavailable: 'This build has no picker.',
    setup: 'Give the account a picker key.',
    searchNote: 'Only files PDT opened appear here.',
  };
  const textPicker = { label: 'Browse the drive', module: '/api/drive/picker.js', accounts: '/api/drive/accounts', ...texts };
  const account = (over: object) => ({
    connectionId: 'c1', name: 'Work', signedIn: true, email: 'me@work.com', hasClient: true, builtIn: true, agents: false, picker: false, note: null, ...over,
  });

  it('a built-in account in a build without the picker hears so, and is never asked for a key', async () => {
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [{ id: 'drive', label: 'Drive', capabilities: all, picker: textPicker }], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [account({})],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    expect(await screen.findByText('This build has no picker.')).toBeInTheDocument();
    expect(screen.queryByText('Sign in to pick.')).toBeNull();
    expect(screen.queryByText('Give the account a picker key.')).toBeNull();
    // The empty search says what it can find, and offers no picker there is none of.
    expect(await screen.findByText('No files PDT can open yet')).toBeInTheDocument();
    expect(screen.getByText('Only files PDT opened appear here.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Browse the drive/ })).toBeNull();
  });

  it('an own-client account without the picker is asked for its key, and edits its connection from the dialog', async () => {
    const conn = { id: 'c1', name: 'Work', kind: 'drive', url: '', config: { clientId: 'cid' }, hasSecret: true, createdAt: 0, updatedAt: 0 };
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [{ id: 'drive', label: 'Drive', capabilities: all, picker: textPicker }], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [account({ builtIn: false })],
      'GET /api/connections': [conn],
      'GET /api/connections/kinds': [{ id: 'drive', label: 'Drive account', plural: 'Drive accounts', setup: '', description: '', fields: [{ key: 'apiKey', label: 'Picker API key', optional: true }] }],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    expect(await screen.findByText('Give the account a picker key.')).toBeInTheDocument();
    expect(screen.queryByText('This build has no picker.')).toBeNull();
    await userEvent.click(await screen.findByRole('button', { name: 'Add to Work' }));
    expect(await screen.findByRole('dialog', { name: 'Edit Work' })).toBeInTheDocument();
    expect(screen.getByLabelText(/Picker API key/)).toBeInTheDocument();
  });

  it('with a picker, a search that finds nothing says why and opens the picker for it', async () => {
    const pick = vi.fn().mockResolvedValue([]);
    vi.spyOn(pickerModules, 'load').mockResolvedValue({ pick });
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [{ id: 'drive', label: 'Drive', capabilities: all, picker: textPicker }], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [account({ picker: true })],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    await userEvent.type(await screen.findByRole('searchbox'), 'roadmap');
    expect(await screen.findByText('No files named “roadmap” that PDT can open')).toBeInTheDocument();
    expect(screen.getByText('Only files PDT opened appear here.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Browse the drive for it' }));
    await waitFor(() => expect(pick).toHaveBeenCalled());
    expect(pick.mock.calls[0][0].account).toBe('c1');
  });

  it('the provider block names the account, and with several the browse button is a menu of them', async () => {
    const pick = vi.fn().mockResolvedValue([]);
    vi.spyOn(pickerModules, 'load').mockResolvedValue({ pick });
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [{ id: 'drive', label: 'Drive', capabilities: all, picker: textPicker }], default: 'drive' },
      'GET /api/documents/providers/drive/files': [],
      'GET /api/drive/accounts': [account({ picker: true }), account({ connectionId: 'c2', name: 'Home', email: 'me@home.com', picker: true })],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    const block = await screen.findByRole('group', { name: 'Provider' });
    expect(await within(block).findByText('2 accounts')).toBeInTheDocument();
    await userEvent.click(within(block).getByRole('button', { name: /Browse the drive…/ }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'me@home.com' }));
    await waitFor(() => expect(pick).toHaveBeenCalled());
    expect(pick.mock.calls[0][0].account).toBe('c2');
  });

  it('several providers are a segmented control', async () => {
    const calls = stubApi({
      'GET /api/documents/providers': { ...providers, providers: [providers.providers[0], { id: 'more', label: 'More', capabilities: all, picker: null }] },
      'GET /api/documents/providers/fake/files': [],
      'GET /api/documents/providers/more/files': [],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    const group = await screen.findByRole('radiogroup', { name: 'Provider' });
    await userEvent.click(within(group).getByRole('radio', { name: 'More' }));
    await waitFor(() => expect(calls.some((c) => c.path.startsWith('/api/documents/providers/more/files'))).toBe(true));
  });

  it('rows show the kind, when they were edited, whether PDT has them and a link; the one importing spins', async () => {
    const found = (reference: string, title: string, over: object = {}) => ({
      reference, title, kind: 'doc', url: null, version: null, modified: null, documentId: null, ticketIds: [], ...over,
    });
    let finish: (d: Document) => void = () => {};
    const onImported = vi.fn();
    const calls = stubApi({
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
      'GET /api/documents/providers/fake/files': [
        found('r1', 'Release plan', { modified: Date.now() - 2 * 3_600_000 }),
        found('r2', 'Launch checklist', { documentId: 'd2', ticketIds: ['t9'], url: 'https://example.com/r2' }),
        found('r3', 'Runner costs', { kind: 'sheet', documentId: 'd3', ticketIds: ['other'] }),
      ],
      'POST /api/documents/import': () => new Promise<Document>((resolve) => { finish = resolve; }),
    });
    renderWithQuery(<ImportDocumentDialog ticket={{ id: 't9', key: 'PDT-9', projectId: 'p1' }} onClose={() => {}} onImported={onImported} />);
    const dialog = await screen.findByRole('dialog', { name: 'Import a document' });
    await waitFor(() => expect(within(dialog).getByText(/^Into/)).toHaveTextContent('Into PDT-9 · PDT'));
    const row = (name: string) => screen.getByRole('menuitem', { name: new RegExp(name) });
    await screen.findByText('Release plan');
    expect(row('Release plan')).toHaveTextContent('Doc · edited 2h ago');
    expect(within(row('Launch checklist')).getByText('On this ticket')).toBeInTheDocument();
    expect(within(row('Launch checklist')).getByRole('link', { name: /Open Launch checklist/ })).toHaveAttribute('href', 'https://example.com/r2');
    expect(within(row('Runner costs')).getByText('In PDT')).toBeInTheDocument();
    expect(row('Runner costs')).toHaveTextContent('Sheet');
    expect(within(row('Release plan')).queryByText(/In PDT|On this ticket/)).toBeNull();
    // Clicking a row's link opens the file; it doesn't import it.
    await userEvent.click(within(row('Launch checklist')).getByRole('link', { name: /Open Launch checklist/ }));
    expect(calls.some((c) => c.method === 'POST')).toBe(false);
    await userEvent.click(screen.getByText('Release plan'));
    expect(await within(row('Release plan')).findByLabelText('Importing')).toBeInTheDocument();
    expect(within(row('Launch checklist')).queryByLabelText('Importing')).toBeNull();
    finish(doc('d1', 'Release plan'));
    await waitFor(() => expect(onImported).toHaveBeenCalled());
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ provider: 'fake', reference: 'r1', ticketId: 't9', by: 'user' });
  });

  it('a provider’s search is not asked again when the window is focused again', async () => {
    const calls = stubApi({
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
      'GET /api/documents/providers/fake/files': [],
    });
    renderWithQuery(<ImportDocumentDialog projectId="p1" onClose={() => {}} onImported={() => {}} />);
    await screen.findByRole('dialog', { name: 'Import a document' });
    const searches = () => calls.filter((c) => c.path.startsWith('/api/documents/providers/fake/files')).length;
    await waitFor(() => expect(searches()).toBe(1));
    act(() => {
      focusManager.setFocused(false);
      focusManager.setFocused(true);
    });
    await new Promise((r) => setTimeout(r, 50));
    expect(searches()).toBe(1);
    focusManager.setFocused(undefined);
  });

  it('a file dropped anywhere on the dialog is imported, and the zone says what it becomes while it is dragged', async () => {
    const onImported = vi.fn();
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [...providers.providers, { id: 'local', label: 'Local', capabilities: all, picker: null }] },
      'GET /api/documents/providers/fake/files': [],
    });
    const upload = vi.spyOn(api, 'uploadDocument').mockResolvedValue({ document: doc('d5', 'Q4 plan', { kind: 'sheet' }), dropped: [] });
    renderWithQuery(<ImportDocumentDialog projectId="p1" onClose={() => {}} onImported={onImported} />);
    await screen.findByText(/becomes a doc/);
    const file = new File(['a,b'], 'Q4 plan.csv', { type: 'text/csv' });
    const drag = (files: File[]) => ({ dataTransfer: { types: ['Files'], items: [{ kind: 'file', type: 'text/csv' }], files, dropEffect: 'none' } });
    const elsewhere = screen.getByText('Files PDT can open', { selector: 'h3' });
    fireEvent.dragEnter(elsewhere, drag([]));
    expect(await screen.findByText(/Drop to import it as a sheet/)).toBeInTheDocument();
    expect(screen.queryByText(/becomes a doc/)).toBeNull();
    fireEvent.drop(elsewhere, drag([file]));
    await waitFor(() => expect(onImported).toHaveBeenCalled());
    expect(upload).toHaveBeenCalledWith(file, { projectId: 'p1' });
  });

  it('without the local provider there is no drop zone', async () => {
    stubApi({ 'GET /api/documents/providers': providers, 'GET /api/documents/providers/fake/files': [] });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    await screen.findByText('No files PDT can open yet');
    expect(screen.queryByText('From your computer')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Choose file…' })).toBeNull();
  });
});
