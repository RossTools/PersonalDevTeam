import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Document } from '../api/gen/Document';
import type { DocumentRead } from '../api/gen/DocumentRead';
import type { DocumentsBeside } from '../api/gen/DocumentsBeside';
import type { DocumentProvidersView } from '../api/gen/DocumentProvidersView';
import { renderWithQuery, stubApi } from '../test-utils';
import { OpenSessionContext } from '../tickets/actions';
import { TicketDocuments } from '../tickets/TicketDocuments';
import { ticket } from '../tickets/fixtures';
import { DocumentsBesideLayout, besideKey } from './BesidePanel';
import { DocumentBody, changedCells } from './DocumentBody';
import { documentKeys } from './queries';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
  localStorage.clear();
});

const all = { docs: true, sheets: true, create: true, search: true, edit: true, rename: true, delete: true, comments: true, embed: false };
const providers: DocumentProvidersView = { providers: [{ id: 'fake', label: 'Fake', capabilities: all }], default: 'fake', folder: '/d' };

function doc(id: string, title: string, over: Partial<Document> = {}): Document {
  return {
    id, title, kind: 'doc', provider: 'fake', providerRef: `fake:${id}`, url: null, actor: 'user', agent: null,
    createdAt: 1, updatedAt: 1, projects: [], tickets: [], ...over,
  };
}

const read = (markdown: string): DocumentRead => ({ content: { kind: 'doc', markdown, version: '1' }, embedUrl: null });

function besideOf(docs: Document[], active: string | null): DocumentsBeside {
  return { tabs: docs.map((d, i) => ({ document: d, by: 'user', openedAt: i, focusedAt: d.id === active ? 99 : i })), active };
}

const Chat = () => <p>the conversation</p>;

describe('documents beside a conversation', () => {
  it('shows several documents in tabs: switch, reorder, close, collapse', async () => {
    const one = doc('d1', 'Spec');
    const two = doc('d2', 'Budget', { kind: 'sheet' });
    let now = besideOf([one, two], 'd1');
    const calls = stubApi({
      'GET /api/sessions/s1/documents': () => now,
      'POST /api/sessions/s1/documents': (c) => (now = { ...now, active: (c.body as { documentId: string }).documentId }),
      'PUT /api/sessions/s1/documents': (c) => {
        const ids = (c.body as { documentIds: string[] }).documentIds;
        return (now = { ...now, tabs: ids.map((i) => now.tabs.find((t) => t.document.id === i)!) });
      },
      'DELETE /api/sessions/s1/documents/d1': () => (now = besideOf([two], 'd2')),
      'GET /api/documents/providers': providers,
      'GET /api/documents/d1': one,
      'GET /api/documents/d2': two,
      'GET /api/documents/d1/content': read('# Spec\n\nThe plan.'),
      'GET /api/documents/d2/content': { content: { kind: 'sheet', version: '1', sheets: [{ name: 'Sheet1', rows: [[{ value: 'Rent' }]], charts: [] }] }, embedUrl: null },
    });
    renderWithQuery(<DocumentsBesideLayout sessionId="s1" phone={false}><Chat /></DocumentsBesideLayout>);
    expect(screen.getByText('the conversation')).toBeInTheDocument();
    const tabs = await screen.findByRole('tablist', { name: 'Documents' });
    expect(within(tabs).getAllByRole('tab').map((t) => t.textContent)).toEqual(['Spec', 'Budget']);
    expect(within(tabs).getByRole('tab', { name: /Spec/ })).toHaveAttribute('aria-selected', 'true');
    expect(await screen.findByText('The plan.')).toBeInTheDocument();

    await userEvent.click(within(tabs).getByRole('tab', { name: /Budget/ }));
    expect(await screen.findByText('Rent')).toBeInTheDocument();
    expect(calls.some((c) => c.method === 'POST' && (c.body as { documentId: string }).documentId === 'd2')).toBe(true);

    // Reorder from the tab's menu (a right-click; on touch, a long press).
    await userEvent.pointer({ keys: '[MouseRight]', target: within(tabs).getByRole('tab', { name: /Budget/ }) });
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Move left' }));
    await waitFor(() => expect(within(tabs).getAllByRole('tab').map((t) => t.textContent)).toEqual(['Budget', 'Spec']));
    expect(calls.find((c) => c.method === 'PUT')!.body).toEqual({ documentIds: ['d2', 'd1'] });

    await userEvent.click(screen.getByRole('button', { name: 'Close Spec' }));
    await waitFor(() => expect(within(tabs).getAllByRole('tab').map((t) => t.textContent)).toEqual(['Budget']));

    // Collapsed, it is a strip with the count; the choice is kept for this conversation.
    await userEvent.click(screen.getByRole('button', { name: 'Collapse the documents' }));
    const strip = screen.getByRole('button', { name: 'Show the documents (1)' });
    expect(JSON.parse(localStorage.getItem('pdt.beside.s1')!)).toMatchObject({ collapsed: true });
    await userEvent.click(strip);
    expect(await screen.findByRole('tablist', { name: 'Documents' })).toBeInTheDocument();
  });

  it('a document the agent opens appears beside the conversation, even when the panel was collapsed', async () => {
    const one = doc('d1', 'Spec');
    let now: DocumentsBeside = { tabs: [], active: null };
    stubApi({
      'GET /api/sessions/s1/documents': () => now,
      'GET /api/documents/providers': providers,
      'GET /api/documents/d1': one,
      'GET /api/documents/d1/content': read('Drafted by the PM.'),
    });
    localStorage.setItem('pdt.beside.s1', JSON.stringify({ width: 500, collapsed: true }));
    const { qc } = renderWithQuery(<DocumentsBesideLayout sessionId="s1" phone={false}><Chat /></DocumentsBesideLayout>);
    await waitFor(() => expect(qc.getQueryData(besideKey('s1'))).toBeDefined());
    expect(screen.queryByRole('complementary', { name: 'Documents beside the conversation' })).not.toBeInTheDocument();
    now = { ...besideOf([one], 'd1'), tabs: besideOf([one], 'd1').tabs.map((t) => ({ ...t, by: 'agent' })) };
    await act(() => qc.invalidateQueries({ queryKey: besideKey('s1') }));
    expect(await screen.findByText('Drafted by the PM.')).toBeInTheDocument();
    expect(screen.getByRole('complementary', { name: 'Documents beside the conversation' })).toHaveStyle({ width: '500px' });
  });

  it('on a phone the documents are a pane beside the conversation', async () => {
    stubApi({
      'GET /api/sessions/s1/documents': besideOf([doc('d1', 'Spec')], 'd1'),
      'GET /api/documents/providers': providers,
      'GET /api/documents/d1': doc('d1', 'Spec'),
      'GET /api/documents/d1/content': read('The plan.'),
    });
    renderWithQuery(<DocumentsBesideLayout sessionId="s1" phone><Chat /></DocumentsBesideLayout>);
    await userEvent.click(await screen.findByRole('radio', { name: 'Documents (1)' }));
    expect(await screen.findByText('The plan.')).toBeInTheDocument();
    expect(screen.queryByText('the conversation')).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole('radio', { name: 'Conversation' }));
    expect(screen.getByText('the conversation')).toBeInTheDocument();
  });

  it('opens a ticket’s document beside one of its conversations', async () => {
    const t = ticket(7, { id: 't1', key: 'PDT-7', projectId: 'p1' });
    const spec = doc('d1', 'Spec', { tickets: [{ id: 't1', key: 'PDT-7', title: 'T', projectId: 'p1' }] });
    const calls = stubApi({
      'GET /api/documents': [spec],
      'GET /api/documents/providers': providers,
      'GET /api/projects': [],
      'GET /api/pm': { current: null, sessions: [] },
      'GET /api/tickets/t1': { ...t, sessions: [{ id: 's9', title: 'Coder · PDT-7', status: 'running', agent: 'claude', attemptId: 'a1', persona: 'Coder', mode: 'conversation' }] },
      'POST /api/sessions/s9/documents': besideOf([spec], 'd1'),
    });
    const opened = vi.fn();
    renderWithQuery(
      <OpenSessionContext.Provider value={opened}>
        <TicketDocuments ticket={t} />
      </OpenSessionContext.Provider>,
    );
    await userEvent.click(await screen.findByRole('button', { name: 'Open Spec beside a conversation' }));
    await userEvent.click(await screen.findByRole('menuitem', { name: 'PDT-7 · Coder' }));
    await waitFor(() => expect(opened).toHaveBeenCalledWith({ id: 's9', projectId: 'p1' }));
    expect(calls.find((c) => c.method === 'POST')!.body).toEqual({ documentId: 'd1', by: 'user' });
  });
});

describe('a live document', () => {
  it('reads again when the document changes, lighting what changed', async () => {
    let markdown = '# Plan\n\nFirst.';
    let updatedAt = 1;
    stubApi({
      'GET /api/documents/d1': () => doc('d1', 'Plan', { updatedAt }),
      'GET /api/documents/d1/content': () => read(markdown),
    });
    const { qc } = renderWithQuery(<DocumentBody id="d1" providerLabel="Fake" />);
    expect(await screen.findByText('First.')).toBeInTheDocument();
    markdown = '# Plan\n\nFirst.\n\nAdded by the agent.';
    updatedAt = 2;
    await act(() => qc.invalidateQueries({ queryKey: documentKeys.one('d1') }));
    const added = await screen.findByText('Added by the agent.');
    expect(added).toHaveAttribute('data-changed', 'true');
    expect(screen.getByText('First.')).not.toHaveAttribute('data-changed');
  });

  it('finds the cells and charts a sheet’s edit changed', () => {
    const before = [{ name: 'S', rows: [[{ value: 'a' }, { value: '1' }]], charts: [] }];
    const after = [{
      name: 'S',
      rows: [[{ value: 'a' }, { value: '2', formula: '=1+1' }], [{ value: 'b' }]],
      charts: [{ id: 'c1', spec: { kind: 'column' as const, data: 'A1:B2' } }],
    }];
    expect([...changedCells(before, after)].sort()).toEqual(['S!A2', 'S!B1', 'S#c1']);
  });
});
