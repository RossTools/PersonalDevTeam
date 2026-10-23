import { fireEvent, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import type { PullRequestStatus } from '../api/gen/PullRequestStatus';
import { OpenTicketContext } from '../tickets/actions';
import { OpenChangesContext } from '../review/open';
import { ticket } from '../tickets/fixtures';
import { TicketTabsContext } from '../workspaces/ticketTabs';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { InboxList } from './InboxList';
import { InboxView } from './InboxView';
import { after, grouped, step } from './model';

afterEach(() => vi.unstubAllGlobals());

const item = (id: string, over: Partial<InboxItem> = {}): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, title: `Item ${id}`,
  body: {}, choices: [], resolution: null, createdAt: Date.now(), resolvedAt: null, ...over,
});

const gate = item('g', { title: 'Ship it?', choices: [{ id: 'ship', title: 'ship', primary: true }, { id: 'hold', title: 'hold', primary: false }] });
const question = item('q', {
  kind: 'question', group: 'question', sessionId: 'sess', personaName: 'Refiner', title: 'Which file?',
  body: { questions: [{ question: 'Which file?', choices: ['a.txt', 'b.txt'] }, { question: 'Keep it?', choices: [] }] },
});
const stalled = item('st', {
  kind: 'stalled', group: 'stalled', sessionId: 'sess', personaName: 'Reviewer', title: 'Reviewer stalled',
  body: { why: 'Its turn ended without an outcome.' },
  choices: [
    { id: 'nudge', title: 'Nudge', primary: true },
    { id: 'outcome:approved', title: 'Take over: approved', primary: false },
    { id: 'outcome:changes_requested', title: 'Take over: changes_requested', primary: false },
  ],
});

describe('inbox model', () => {
  it('groups most urgent first and steps through the items in that order', () => {
    const items = [gate, stalled, question];
    expect(grouped(items).map((g) => g.title)).toEqual(['Questions', 'Stalled', 'Gates']);
    expect(step(items, null, 1)).toBe('q');
    expect(step(items, 'q', 1)).toBe('st');
    expect(step(items, 'g', 1)).toBe('g');
    expect(step(items, 'st', -1)).toBe('q');
    expect(after(items, 'st')).toBe('g');
    expect(after(items, 'g')).toBe('st');
    expect(after([gate], 'g')).toBeNull();
  });
});

describe('inbox', () => {
  it('lists items under their kind with the ticket key', () => {
    renderWithQuery(<InboxList items={[gate, question]} selectedId="g" onSelect={() => {}} />);
    expect(screen.getByRole('region', { name: 'Questions' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Gates' })).toBeInTheDocument();
    expect(screen.getAllByText('PDT-7')).toHaveLength(2);
    expect(screen.getByText(/Question · Refiner/)).toBeInTheDocument();
  });

  it('answers a gate with Enter and moves on', async () => {
    const calls = stubApi({ 'POST /api/inbox/g/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    const onSelect = vi.fn();
    renderWithQuery(<InboxView items={[gate, stalled]} selectedId="g" onSelect={onSelect} />);
    expect(screen.getByRole('button', { name: /ship/ })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body).toEqual({ choice: 'ship' }));
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('st'));
  });

  it('sends the answers to an agent\'s questions', async () => {
    const calls = stubApi({ 'POST /api/inbox/q/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[question]} selectedId="q" onSelect={() => {}} />);
    const send = screen.getByRole('button', { name: 'Send answers' });
    expect(send).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'a.txt' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Answer 2' }), 'Yes');
    await userEvent.click(send);
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body).toEqual({ answers: ['a.txt', 'Yes'] }));
  });

  it('nudges a stalled step, or takes it over with an outcome', async () => {
    const calls = stubApi({ 'POST /api/inbox/st/answer': (c: Call) => c.body, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[stalled]} selectedId="st" onSelect={() => {}} />);
    expect(screen.getByText(/Its turn ended without an outcome/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Take over' }));
    await userEvent.click(screen.getByRole('button', { name: 'changes_requested' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body).toEqual({ choice: 'outcome:changes_requested' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Nudge message (optional)' }), 'Finish up.');
    await userEvent.click(screen.getByRole('button', { name: /Nudge/ }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST' && c.path.endsWith('/answer'))[1]?.body).toEqual({ choice: 'nudge', text: 'Finish up.' }));
  });

  it('resumes a timed-out step with the user\'s note, or stops it', async () => {
    const timedOut = item('to', {
      kind: 'timed_out', group: 'stalled', sessionId: 'sess', personaName: 'Coder', title: 'Coder timed out at code',
      body: { why: 'it had no outcome within its time limit', node: 'code', workflow: 'Pipeline' },
      choices: [{ id: 'resume', title: 'Resume', primary: true }, { id: 'stop', title: 'Stop', primary: false }],
    });
    const calls = stubApi({ 'POST /api/inbox/to/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[timedOut]} selectedId="to" onSelect={() => {}} />);
    expect(screen.getByText(/Step code of workflow Pipeline ended without an outcome/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open session' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Stop' }));
    const answers = () => calls.filter((c) => c.method === 'POST' && c.path === '/api/inbox/to/answer');
    await waitFor(() => expect(answers()[0]?.body).toEqual({ choice: 'stop' }));
    await userEvent.type(screen.getByRole('textbox', { name: 'Note (optional)' }), 'Push what you have.');
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(answers()[1]?.body).toEqual({ choice: 'resume', text: 'Push what you have.' }));
  });

  it('moves with j and k, opens the ticket with o and dismisses with e', async () => {
    const calls = stubApi({ 'POST /api/inbox/q/dismiss': undefined, 'GET /api/inbox': { items: [], total: 0 } });
    const onSelect = vi.fn();
    const openTicket = vi.fn();
    renderWithQuery(
      <OpenTicketContext.Provider value={openTicket}>
        <InboxView items={[question, gate]} selectedId="q" onSelect={onSelect} />
      </OpenTicketContext.Provider>,
    );
    fireEvent.keyDown(window, { key: 'j' });
    expect(onSelect).toHaveBeenLastCalledWith('g');
    fireEvent.keyDown(window, { key: 'k' });
    expect(onSelect).toHaveBeenLastCalledWith('q');
    fireEvent.keyDown(window, { key: 'o' });
    expect(openTicket).toHaveBeenCalledWith({ id: 't', projectId: 'p' });
    fireEvent.keyDown(window, { key: 'e' });
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/inbox/q/dismiss')).toBe(true));
  });

  it('asks the user to do a manual stage and pick its outcome, with their note as the summary', async () => {
    const manual = item('m', {
      kind: 'manual', title: 'Your turn: Review', personaName: 'Reviewer',
      body: { prompt: 'Review the **diff**.', workflow: 'Pipeline' },
      choices: [{ id: 'outcome:approved', title: 'approved', primary: true }, { id: 'outcome:changes_requested', title: 'changes_requested', primary: false }],
    });
    const calls = stubApi({ 'POST /api/inbox/m/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[manual]} selectedId="m" onSelect={() => {}} />);
    expect(screen.getByText(/This stage is yours in workflow Pipeline/)).toBeInTheDocument();
    expect(screen.getByText('diff')).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'What you did (optional)' }), 'Looks right');
    await userEvent.click(screen.getByRole('button', { name: 'changes_requested' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body).toEqual({ choice: 'outcome:changes_requested', text: 'Looks right' }));
  });

  it('opens the ticket\'s changes and runs its branch next to its tab in its own workspace', async () => {
    const accept = item('a', { group: 'accept', ticketId: 't1', attemptId: 'a1', title: 'Accept PDT-7?', choices: [{ id: 'merge', title: 'merge', primary: true }] });
    const attempt = { id: 'a1', branch: 'pdt-PDT-7-x', baseBranch: 'main', worktreeId: 'w1', state: 'active' as const, createdAt: 1, endedAt: null, lastHead: null, workspaceId: 'ws7', ci: null, pr: null };
    const calls = stubApi({
      'GET /api/tickets/t1': ticket(1, { attempts: [attempt], sessions: [
        { id: 'old', title: 'Coder · PDT-7', status: 'finished', agent: 'claude', attemptId: 'a1', persona: 'Coder', mode: 'conversation' },
        { id: 's5', title: 'Coder · PDT-7', status: 'running', agent: 'claude', attemptId: 'a1', persona: 'Coder', mode: 'conversation' },
      ] }),
      'GET /api/projects/p/run-config': { filePath: null, file: null, overrides: null, error: null,
        effective: { runs: [{ name: 'web', setup: null, command: 'x', web: true, data: true, env: {} }], tests: [] } },
      'POST /api/projects/p/runs': { id: 'r9', projectId: 'p' },
      'GET /api/inbox': { items: [], total: 0 },
    });
    const openChanges = vi.fn();
    const openRun = vi.fn();
    const placementNear = vi.fn(() => ({ workspaceId: 'ws7', position: 3 }));
    const openNear = vi.fn();
    renderWithQuery(
      <OpenChangesContext.Provider value={openChanges}>
        <TicketTabsContext.Provider value={{ placementNear, openRun, openNear }}>
          <InboxView items={[accept]} selectedId="a" onSelect={() => {}} />
        </TicketTabsContext.Provider>
      </OpenChangesContext.Provider>,
    );
    await userEvent.click(await screen.findByRole('button', { name: 'Open changes' }));
    expect(openChanges).toHaveBeenCalledWith({ id: 'w1', projectId: 'p' }, { ticketId: 't1' });
    // The running coder session, to talk to it.
    await userEvent.click(screen.getByRole('button', { name: 'Open coder session' }));
    expect(openNear).toHaveBeenCalledWith({ kind: 'session', id: 's5' }, 'p', 't1');
    await userEvent.click(await screen.findByRole('button', { name: /Run the ticket's branch/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'web' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/projects/p/runs')?.body)
      .toEqual({ worktreeId: 'w1', name: 'web', kind: 'run', tab: { workspaceId: 'ws7', position: 3 } }));
    await waitFor(() => expect(openRun).toHaveBeenCalledWith({ id: 'r9', projectId: 'p' }));
  });

  it('offers the attempt\'s pull request on a blocked merge', async () => {
    const blocked = item('b', { group: 'gate', ticketId: 't1', attemptId: 'a1', title: 'Merge blocked on PDT-7: a draft', choices: [{ id: 'merge_again', title: 'Merge again', primary: true }, { id: 'stop', title: 'Stop', primary: false }] });
    const pr: PullRequestStatus = {
      number: 12, url: 'https://github.com/o/r/pull/12', title: 'Ship it', state: 'open', draft: false, baseBranch: 'main', headSha: 'abc',
      mergeable: 'conflicting', behind: false, autoMerge: false, inMergeQueue: false, queuePosition: null, ci: null, queueCi: null, checkedAt: 1,
    };
    const attempt = { id: 'a1', branch: 'pdt-PDT-7-x', baseBranch: 'main', worktreeId: null, state: 'active' as const, createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, ci: null, pr };
    stubApi({ 'GET /api/tickets/t1': ticket(1, { attempts: [attempt], sessions: [] }), 'GET /api/inbox': { items: [], total: 0 } });
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    renderWithQuery(<InboxView items={[blocked]} selectedId="b" onSelect={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Open PR' }));
    expect(open).toHaveBeenCalledWith('https://github.com/o/r/pull/12', '_blank', 'noopener');
    expect(screen.getByRole('button', { name: /Merge again/ })).toBeInTheDocument();
    open.mockRestore();
  });

  it('sends a note with a gate\'s choice', async () => {
    const reject = item('r', { group: 'accept', title: 'Accept PDT-7?', choices: [{ id: 'merge', title: 'merge', primary: true }, { id: 'reject', title: 'reject', primary: false }] });
    const calls = stubApi({ 'POST /api/inbox/r/answer': {}, 'GET /api/inbox': { items: [], total: 0 }, 'GET /api/tickets/t': ticket(1) });
    renderWithQuery(<InboxView items={[reject]} selectedId="r" onSelect={() => {}} />);
    expect(screen.getByText(/goes on the ticket as your comment/)).toBeInTheDocument();
    await userEvent.type(screen.getByRole('textbox', { name: 'Note (optional)' }), 'The button is the wrong colour');
    await userEvent.click(screen.getByRole('button', { name: 'reject' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body).toEqual({ choice: 'reject', text: 'The button is the wrong colour' }));
  });

  it('says when nothing waits', () => {
    renderWithQuery(<InboxView items={[]} selectedId={null} onSelect={() => {}} />);
    expect(screen.getByText(/Nothing waits on you/)).toBeInTheDocument();
  });
});

describe('inbox sections (PDT-108)', () => {
  it('lists Needs you, then Updates with unread in bold, and marks them all read', async () => {
    const { notification } = await import('../notifications/fixtures');
    const waiting = item('session:s1', { kind: 'session', group: 'session', sessionId: 's1', title: 'Fix the build', ticketKey: null });
    const onReadAll = vi.fn();
    const onSelect = vi.fn();
    renderWithQuery(
      <InboxList items={[waiting]} updates={[notification('u1', { title: 'PDT-7 merged' }), notification('u2', { title: 'Old', readAt: 1 })]}
        selectedId={null} onSelect={onSelect} onReadAll={onReadAll} onSettings={() => {}} />,
    );
    expect(screen.getByRole('heading', { name: 'Needs you' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Sessions waiting' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Updates' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Mark all read' }));
    expect(onReadAll).toHaveBeenCalled();
    await userEvent.click(screen.getByText('PDT-7 merged'));
    expect(onSelect).toHaveBeenCalledWith('update:u1');
  });

  it('shows an Update, reads it and opens what it is about', async () => {
    const { notification } = await import('../notifications/fixtures');
    const calls = stubApi({ 'POST /api/notifications/u1/read': null, 'GET /api/notifications': { items: [], unreadUpdates: 0 }, 'GET /api/inbox': { items: [], total: 0 } });
    const onOpenTarget = vi.fn();
    const u = notification('u1', { title: 'CI failed on PDT-7 #12', body: 'Failed: test (linux)', target: { kind: 'ticket', id: 't', projectId: 'p' } });
    renderWithQuery(<InboxView items={[]} updates={[u]} selectedId="update:u1" onSelect={() => {}} onOpenTarget={onOpenTarget} />);
    expect(screen.getByText('Failed: test (linux)')).toBeInTheDocument();
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/notifications/u1/read')).toBe(true));
    await userEvent.click(screen.getAllByRole('button', { name: 'Open' })[0]);
    expect(onOpenTarget).toHaveBeenCalledWith({ kind: 'ticket', id: 't', projectId: 'p' });
  });
});
