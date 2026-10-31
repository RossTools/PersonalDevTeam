import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Document } from '../api/gen/Document';
import type { DocumentProvidersView } from '../api/gen/DocumentProvidersView';
import { renderWithQuery, stubApi } from '../test-utils';
import { TicketDocuments } from '../tickets/TicketDocuments';
import { ticket } from '../tickets/fixtures';
import { DocumentPage } from './DocumentPage';
import { DocumentsList, useDocumentsFilters } from './DocumentsList';
import { ImportDocumentDialog, NewDocumentDialog } from './DocumentDialogs';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const all = { docs: true, sheets: true, create: true, search: true, edit: true, rename: true, delete: true, comments: true, embed: false };

const providers: DocumentProvidersView = {
  providers: [
    { id: 'fake', label: 'Fake', capabilities: all },
    { id: 'reader', label: 'Reader', capabilities: { ...all, create: false, search: false, delete: false } },
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
    const list = await screen.findByRole('list', { name: 'Documents' });
    expect(within(list).getByText('Plan')).toBeInTheDocument();
    expect(within(list).getByText('Budget')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Kind/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Sheets' }));
    await waitFor(() => expect(within(list).queryByText('Plan')).not.toBeInTheDocument());
    expect(calls.some((c) => c.path.includes('kind=sheet'))).toBe(true);
    await userEvent.type(screen.getByRole('searchbox', { name: 'Search documents' }), 'zzz');
    expect(await screen.findByText('No document matches.')).toBeInTheDocument();
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
    await userEvent.click(screen.getByRole('button', { name: 'Rename' }));
    const field = screen.getByRole('textbox', { name: 'Document title' });
    await userEvent.clear(field);
    await userEvent.type(field, 'Q4 plan{Enter}');
    expect(await screen.findByRole('heading', { name: 'Q4 plan', level: 1 })).toBeInTheDocument();
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
        content: { kind: 'sheet', version: '1', sheets: [{ name: 'Sheet1', rows: [[{ value: 'Rent' }, { value: '12', formula: '=6*2' }]] }] },
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

  it('PDT’s view reads a document again when its version changes', async () => {
    let version = '1';
    stubApi({
      'GET /api/documents/d1': doc('d1', 'Plan'),
      'GET /api/documents/d1/content': () => ({ content: { kind: 'doc', markdown: `Version ${version}`, version }, embedUrl: null }),
      'GET /api/documents/d1/version': () => ({ version }),
      'GET /api/documents/providers': providers,
      'GET /api/projects': [project],
    });
    const { qc } = renderWithQuery(<DocumentPage id="d1" phone={false} onGone={() => {}} />);
    expect(await screen.findByText('Version 1')).toBeInTheDocument();
    version = '2';
    await qc.refetchQueries({ queryKey: ['documents', 'version', 'd1'] });
    expect(await screen.findByText('Version 2')).toBeInTheDocument();
  });

  it('Google’s import offers its Drive picker for each signed-in account', async () => {
    const google = { id: 'google', label: 'Google', capabilities: { ...all, embed: true } };
    stubApi({
      'GET /api/documents/providers': { ...providers, providers: [google], default: 'google' },
      'GET /api/documents/providers/google/files': [],
      'GET /api/google/accounts': [
        { connectionId: 'c1', name: 'Work', signedIn: true, email: 'me@work.com', hasClient: true, picker: true },
        { connectionId: 'c2', name: 'Home', signedIn: false, email: null, hasClient: true, picker: true },
      ],
    });
    renderWithQuery(<ImportDocumentDialog onClose={() => {}} onImported={() => {}} />);
    expect(await screen.findByRole('button', { name: 'Pick from Google Drive' })).toBeInTheDocument();
  });
});
