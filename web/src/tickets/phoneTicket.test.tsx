import { useState } from 'react';
import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Ticket } from '../api/gen/Ticket';
import { PhoneTopBar, TopBarActionsContext, TopBarEditContext, type TopBarActions, type TopBarEdit } from '../shell/PhoneTopBar';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { OpenSessionContext, OpenTicketContext, StartTicketContext } from './actions';
import { event, project, ticket } from './fixtures';
import { phoneTicketActions, PhoneTicket } from './PhoneTicket';

beforeEach(() => localStorage.clear());
afterEach(() => vi.unstubAllGlobals());

const attempt: AttemptSummary = {
  id: 'a1', branch: 'pdt/ALP-1-fix-login', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: { state: 'passed', url: 'u', runs: [] }, pr: null,
};
const inProgress = ticket(1, {
  title: 'Fix login', stage: 'in_progress', priority: 'medium', attempt, attempts: [attempt],
  sessions: [{ id: 's1', title: 'Coder · ALP-1', status: 'running', agent: 'claude', attemptId: 'a1', persona: 'Coder', mode: 'conversation' }],
  agent: { persona: 'Coder', state: 'working', sessionId: 's1', busy: true, runId: 'r1', background: [] },
});
const gate: InboxItem = {
  id: 'g1', kind: 'gate', group: 'accept', status: 'open', projectId: project.id, ticketId: 't1', ticketKey: 'ALP-1', ticketTitle: 'Fix login',
  attemptId: 'a1', sessionId: null, runId: 'r1', stepId: null, personaId: null, personaName: null, title: 'Accept ALP-1?', body: {},
  choices: [{ id: 'reject', title: 'reject', primary: false }, { id: 'merge', title: 'merge', primary: true }], resolution: null, createdAt: 1, resolvedAt: null,
};

/** The shell's part: the top bar with the screen's ⋯ and its edit (Cancel and Save). */
function Harness({ t, onStart = vi.fn(), onSession = vi.fn(), onDeleted = vi.fn() }: {
  t: Ticket; onStart?: () => void; onSession?: (s: { id: string }) => void; onDeleted?: () => void;
}) {
  const [actions, setActions] = useState<{ entries: TopBarActions } | null>(null);
  const [edit, setEdit] = useState<TopBarEdit | null>(null);
  return (
    <OpenTicketContext.Provider value={vi.fn()}>
      <OpenSessionContext.Provider value={onSession}>
        <StartTicketContext.Provider value={onStart}>
          <TopBarActionsContext.Provider value={setActions}>
            <TopBarEditContext.Provider value={setEdit}>
              <PhoneTopBar detail onMenu={() => {}} menuOpen={false} onBack={() => {}} title={t.key} needsYou={0} onInbox={() => {}}
                onSearch={() => {}} actions={actions?.entries ?? null} edit={edit} />
              <PhoneTicket ticket={t} onDeleted={onDeleted} />
            </TopBarEditContext.Provider>
          </TopBarActionsContext.Provider>
        </StartTicketContext.Provider>
      </OpenSessionContext.Provider>
    </OpenTicketContext.Provider>
  );
}

function stub(t: Ticket, extra: Parameters<typeof stubApi>[0] = {}): Call[] {
  return stubApi({
    'GET /api/tickets/t1': t,
    'PATCH /api/tickets/t1': (c: Call) => ({ ...t, ...(c.body as object) }),
    'GET /api/tickets/t1/events': { items: [event('e1')], nextCursor: null },
    'POST /api/tickets/t1/comments': event('e2', { kind: 'comment', body: 'hi' }),
    'POST /api/tickets/t1/refine': { ...t, pipelineStage: 'refine' },
    [`GET /api/projects/${project.id}/labels`]: [],
    'GET /api/inbox': { items: [], total: 0 },
    'GET /api/worktrees/w1/changes': { files: [] },
    ...extra,
  });
}

const bar = () => screen.getByRole('group', { name: 'Ticket actions' });
const topBar = () => screen.getByRole('banner', { name: 'Top bar' });
const barButtons = () => within(bar()).getAllByRole('button').map((b) => b.textContent);

describe('phone ticket: the primary action by stage', () => {
  it.each([
    ['draft', ['refine', 'start']],
    ['ready', ['refine', 'start']],
    ['todo', ['start']],
    ['in_progress', ['open']],
    ['done', []],
    ['wont_do', []],
  ] as const)('%s → %j', (stage, want) => {
    const t = stage === 'in_progress' ? inProgress : ticket(1, { stage });
    expect(phoneTicketActions(t, false)).toEqual(want);
  });

  it('offers Accept… for an In progress ticket whose Accept gate waits, and nothing to start on a parent', () => {
    expect(phoneTicketActions(inProgress, true)).toEqual(['accept']);
    expect(phoneTicketActions(ticket(1, { stage: 'in_progress' }), false)).toEqual([]);
    const parent = ticket(1, { stage: 'ready', parts: { total: 2, done: 0, merged: 0, review: 0, progress: 0, todo: 2, backlog: 0, segments: [] } });
    expect(phoneTicketActions(parent, false)).toEqual(['refine']);
    expect(phoneTicketActions(ticket(1, { stage: 'draft', pipelineStage: 'refine' }), false)).toEqual(['start']);
  });

  it('shows Comment… with Refine · Start for a Draft ticket; Start opens the Start dialog, Refine asks the pipeline', async () => {
    const onStart = vi.fn();
    const calls = stub(ticket(1, { title: 'Fix login' }));
    renderWithQuery(<Harness t={ticket(1, { title: 'Fix login' })} onStart={onStart} />);
    expect(barButtons()).toEqual(['Comment…', 'Refine', 'Start']);
    await userEvent.click(within(bar()).getByRole('button', { name: 'Start' }));
    expect(onStart).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }));
    await userEvent.click(within(bar()).getByRole('button', { name: 'Refine' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/tickets/t1/refine')).toBe(true));
  });

  it('shows Open work for an In progress ticket, opening the session working on it', async () => {
    const onSession = vi.fn();
    stub(inProgress);
    renderWithQuery(<Harness t={inProgress} onSession={onSession} />);
    expect(barButtons()).toEqual(['Comment…', 'Open work']);
    await userEvent.click(within(bar()).getByRole('button', { name: 'Open work' }));
    expect(onSession).toHaveBeenCalledWith({ id: 's1', projectId: project.id });
  });

  it('shows Accept… once the Accept gate waits, and merges only after the confirmation', async () => {
    const calls = stub(inProgress, { 'GET /api/inbox': { items: [gate], total: 1 }, 'POST /api/inbox/g1/answer': {} });
    renderWithQuery(<Harness t={inProgress} />);
    await userEvent.click(await within(bar()).findByRole('button', { name: 'Accept…' }));
    const dialog = await screen.findByRole('dialog', { name: 'Merge ALP-1?' });
    expect(calls.some((c) => c.path.startsWith('/api/inbox/g1/answer'))).toBe(false);
    await userEvent.click(within(dialog).getByRole('button', { name: 'Merge' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/inbox/g1/answer')?.body).toEqual({ choice: 'merge' }));
  });

  it('shows only Comment… on a Done ticket', () => {
    stub(ticket(1, { stage: 'done' }));
    renderWithQuery(<Harness t={ticket(1, { stage: 'done' })} />);
    expect(barButtons()).toEqual(['Comment…']);
  });
});

describe('phone ticket screen', () => {
  it("reads the state at the top: title, stage · priority · persona, the stepper, and folded summaries", async () => {
    stub(inProgress);
    renderWithQuery(<Harness t={inProgress} />);
    const screenEl = screen.getByRole('region', { name: 'ALP-1 · Fix login' });
    expect(within(screenEl).getByRole('button', { name: 'Edit title: Fix login' })).toBeInTheDocument();
    expect(screenEl).toHaveTextContent('In progress·Medium·Coder · working');
    expect(within(screenEl).getByRole('list', { name: 'Stage' })).toBeInTheDocument();
    expect(within(screenEl).getByRole('tab', { name: 'Overview' })).toHaveAttribute('aria-selected', 'true');
    const details = within(screenEl).getByRole('button', { name: /Details/ });
    expect(details).toHaveAttribute('aria-expanded', 'false');
    expect(details).toHaveTextContent('In progress · Medium');
    expect(within(screenEl).getByRole('button', { name: /Attempt/ })).toHaveTextContent('pdt/ALP-1-fix-login · CI passed');
    expect(within(screenEl).getByRole('button', { name: /Links/ })).toHaveTextContent('None');
    expect(within(screenEl).queryByRole('combobox', { name: 'Priority' })).not.toBeInTheDocument();
    await userEvent.click(details);
    expect(details).toHaveAttribute('aria-expanded', 'true');
    expect(within(screenEl).getByRole('region', { name: 'Details' })).toBeInTheDocument();
  });

  it('edits the title in place, with Cancel and Save in the top bar', async () => {
    const calls = stub(ticket(1, { title: 'Fix login' }));
    renderWithQuery(<Harness t={ticket(1, { title: 'Fix login' })} />);
    await userEvent.click(screen.getByRole('button', { name: 'Edit title: Fix login' }));
    expect(within(topBar()).queryByRole('button', { name: 'Back' })).not.toBeInTheDocument();
    expect(within(topBar()).getByRole('button', { name: 'Save' })).toBeDisabled();
    // Cancel leaves it as it was.
    await userEvent.type(screen.getByRole('textbox', { name: 'Title' }), ' now');
    await userEvent.click(within(topBar()).getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('textbox', { name: 'Title' })).not.toBeInTheDocument();
    expect(within(topBar()).getByRole('button', { name: 'Back' })).toBeInTheDocument();
    expect(calls.some((c) => c.method === 'PATCH')).toBe(false);

    await userEvent.click(screen.getByRole('button', { name: 'Edit title: Fix login' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Title' }), ' now');
    await userEvent.click(within(topBar()).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ title: 'Fix login now' }));
    await waitFor(() => expect(screen.queryByRole('textbox', { name: 'Title' })).not.toBeInTheDocument());
  });

  it('edits the description with ✎, saved from the top bar', async () => {
    const t = ticket(1, { description: 'Old' });
    const calls = stub(t);
    renderWithQuery(<Harness t={t} />);
    await userEvent.click(screen.getByRole('button', { name: 'Edit description' }));
    const box = screen.getByRole('textbox', { name: 'Description' });
    await userEvent.clear(box);
    await userEvent.type(box, 'New');
    await userEvent.click(within(topBar()).getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ description: 'New' }));
  });

  it('comments from a bottom sheet', async () => {
    const calls = stub(ticket(1));
    renderWithQuery(<Harness t={ticket(1)} />);
    await userEvent.click(within(bar()).getByRole('button', { name: 'Comment…' }));
    const sheet = await screen.findByRole('dialog', { name: 'Comment' });
    await userEvent.type(within(sheet).getByRole('textbox', { name: 'Comment' }), 'Looks good');
    await userEvent.click(within(sheet).getByRole('button', { name: 'Comment' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/tickets/t1/comments')?.body).toEqual({ body: 'Looks good', by: 'user' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Comment' })).not.toBeInTheDocument());
  });

  it('keeps the rest under ⋯: Move to, Make into parts, Merge, Abandon and Delete', async () => {
    stub(inProgress);
    renderWithQuery(<Harness t={inProgress} />);
    await userEvent.click(within(topBar()).getByRole('button', { name: 'Screen actions' }));
    const menu = await screen.findByRole('menu');
    const items = within(menu).getAllByRole('menuitem').map((m) => m.textContent);
    expect(items).toEqual(expect.arrayContaining(['Move to', 'Priority', 'Merge…', 'Abandon attempt…', 'Delete…']));
  });
});
