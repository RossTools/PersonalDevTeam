import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import type { Notification } from '../api/gen/Notification';
import type { PullRequestStatus } from '../api/gen/PullRequestStatus';
import { ticket } from '../tickets/fixtures';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { merges, triageTitle } from './model';
import { PhoneInboxItem } from './PhoneInbox';
import { PhoneInboxList } from './PhoneInboxList';

afterEach(() => vi.unstubAllGlobals());

const item = (id: string, over: Partial<InboxItem> = {}): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, title: `Item ${id}`,
  body: {}, choices: [], resolution: null, createdAt: Date.now(), resolvedAt: null, ...over,
});

const question = item('q', {
  kind: 'question', group: 'question', sessionId: 'sess', title: 'Which file?',
  body: { questions: [{ question: 'Which file?', choices: ['a.txt', 'b.txt'] }] },
});
const approval = item('approval:sess:r1', {
  kind: 'approval', group: 'approval', sessionId: 'sess', title: 'rm -rf build',
  body: { tool: 'Bash', requestId: 'r1' },
  choices: [{ id: 'allow', title: 'Allow', primary: true }, { id: 'allow_always', title: 'Always', primary: false }, { id: 'deny', title: 'Deny', primary: false }],
});
const stalled = item('st', {
  kind: 'stalled', group: 'stalled', sessionId: 'sess', title: 'Reviewer stalled', body: { why: 'Its turn ended.' },
  choices: [
    { id: 'nudge', title: 'Nudge', primary: true },
    { id: 'outcome:approved', title: 'Take over: approved', primary: false },
    { id: 'outcome:changes_requested', title: 'Take over: changes_requested', primary: false },
  ],
});
const loop = item('l', {
  kind: 'loop_limit', group: 'loop_limit', title: 'Loop limit',
  choices: [
    { id: 'again', title: 'Once more', primary: true },
    { id: 'outcome:approved', title: 'Take over: approved', primary: false },
    { id: 'outcome:changes_requested', title: 'Take over: changes_requested', primary: false },
    { id: 'stop', title: 'Stop', primary: false },
  ],
});
const accept = item('acc', {
  group: 'accept', attemptId: 'a1', title: 'Accept PDT-7 and merge it?',
  choices: [{ id: 'merge', title: 'merge', primary: true }, { id: 'reject', title: 'reject', primary: false }],
});
const manual = item('m', {
  kind: 'manual', title: 'QA is yours', body: { prompt: 'Try it on a phone.' },
  choices: [{ id: 'passed', title: 'passed', primary: true }, { id: 'failed', title: 'failed', primary: false }],
});
const waiting = item('session:sess', { kind: 'session', group: 'session', sessionId: 'sess', ticketId: null, title: 'Claude waits', body: { sessionTitle: 'Coder' } });

const update = (id: string, over: Partial<Notification> = {}): Notification => ({
  id, kind: 'pr.merged', severity: 'info', title: `Update ${id}`, body: null, target: { kind: 'none' }, projectId: null,
  ticketId: null, sessionId: null, inboxItemId: null, readAt: null, clearedAt: null, createdAt: Date.now(), ...over,
} as Notification);

const answered = (calls: Call[]) => calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body;
const bar = () => within(screen.getByRole('group', { name: 'Decision' }));
const barLabels = () => bar().getAllByRole('button').map((b) => b.textContent?.trim());

function triage(items: InboxItem[], itemId: string, routes: Parameters<typeof stubApi>[0] = {}) {
  const calls = stubApi({
    'GET /api/inbox': { items: [], total: 0 },
    'POST /api/notifications/read-all': undefined,
    [`POST /api/inbox/${itemId}/answer`]: {},
    ...routes,
  });
  const onMove = vi.fn();
  const onList = vi.fn();
  renderWithQuery(<PhoneInboxItem items={items} updates={[]} itemId={itemId} onMove={onMove} onList={onList} />);
  return { calls, onMove, onList };
}

describe('phone inbox model', () => {
  it('titles an item by its kind and place in the list', () => {
    expect(triageTitle([stalled, approval, question, accept], 'approval:sess:r1')).toBe('Approval · 2 of 4');
    expect(triageTitle([accept], 'acc')).toBe('Accept · 1 of 1');
    expect(triageTitle([accept], 'gone')).toBeNull();
  });

  it('asks before Merge and before Accept', () => {
    expect(merges(accept, 'merge')).toBe(true);
    expect(merges(accept, 'reject')).toBe(false);
    expect(merges(item('g', { choices: [{ id: 'ship', title: 'ship', primary: true }] }), 'ship')).toBe(false);
    expect(merges(item('g', { choices: [{ id: 'merge', title: 'merge', primary: false }] }), 'merge')).toBe(true);
    expect(merges(manual, 'passed')).toBe(false);
  });
});

describe('phone inbox list', () => {
  it('shows Needs you with its count, and Review all opens the first item in list order', async () => {
    stubApi({});
    const onOpen = vi.fn();
    renderWithQuery(<PhoneInboxList items={[stalled, waiting, question]} updates={[update('u1')]} onOpen={onOpen} onReadAll={() => {}} onSettings={() => {}} />);
    expect(screen.getByRole('radio', { name: 'Needs you (3)' })).toBeChecked();
    expect(screen.getByRole('radio', { name: 'Updates (unread)' })).toBeInTheDocument();
    // Sessions waiting come after Approvals, before Stalled.
    const nav = screen.getByRole('navigation', { name: 'Inbox items' });
    expect(within(nav).getAllByRole('region').map((r) => r.getAttribute('aria-label'))).toEqual(['Questions', 'Sessions waiting', 'Stalled']);
    await userEvent.click(screen.getByRole('button', { name: 'Review all (3)' }));
    expect(onOpen).toHaveBeenCalledWith('q');
    await userEvent.click(screen.getByText('Reviewer stalled'));
    expect(onOpen).toHaveBeenCalledWith('st');
  });

  it('shows the Updates on their own segment, without Review all', async () => {
    stubApi({});
    const onOpen = vi.fn();
    renderWithQuery(<PhoneInboxList items={[question]} updates={[update('u1')]} onOpen={onOpen} onReadAll={() => {}} onSettings={() => {}} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Updates (unread)' }));
    expect(screen.queryByRole('button', { name: /Review all/ })).toBeNull();
    await userEvent.click(screen.getByText('Update u1'));
    expect(onOpen).toHaveBeenCalledWith('update:u1');
    // The list remembers its segment: back to Needs you for the tests after this one.
    await userEvent.click(screen.getByRole('radio', { name: /Needs you/ }));
  });
});

describe('phone triage', () => {
  it('answers a question with a big choice row and Send, then shows the next item', async () => {
    const { calls, onMove } = triage([question, stalled], 'q');
    expect(barLabels()).toEqual(['Send']);
    expect(bar().getByRole('button', { name: 'Send' })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'a.txt' }));
    expect(screen.getByRole('button', { name: 'a.txt' })).toHaveAttribute('aria-pressed', 'true');
    await userEvent.click(bar().getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(answered(calls)).toEqual({ answers: ['a.txt'] }));
    await waitFor(() => expect(onMove).toHaveBeenCalledWith('st'));
  });

  it('has Deny · Always · Allow for an approval, answered in its session', async () => {
    const { calls } = triage([approval], 'approval:sess:r1', { 'POST /api/sessions/sess/requests/r1': undefined });
    expect(barLabels()).toEqual(['Deny', 'Always', 'Allow']);
    await userEvent.click(bar().getByRole('button', { name: 'Always' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/sess/requests/r1')?.body)
      .toEqual({ answer: { type: 'approve', always: true } }));
    expect(await screen.findByText('All clear')).toBeInTheDocument();
  });

  it('has Take over · Nudge for a stalled step; Take over offers its outcomes', async () => {
    const { calls } = triage([stalled], 'st');
    expect(barLabels()).toEqual(['Take over', 'Nudge']);
    await userEvent.click(bar().getByRole('button', { name: 'Take over' }));
    const sheet = await screen.findByRole('dialog', { name: /Take over/ });
    await userEvent.click(within(sheet).getByRole('button', { name: 'changes_requested' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'outcome:changes_requested' }));
  });

  it('offers Resume with this message first for a stalled step whose message never arrived', async () => {
    const undelivered = item('un', {
      ...stalled, id: 'un', body: { why: 'Its message was never delivered: hi', message: 'hi' },
      choices: [{ id: 'resend', title: 'Resume with this message', primary: true }, ...stalled.choices.map((c) => ({ ...c, primary: false }))],
    });
    const { calls } = triage([undelivered], 'un');
    expect(barLabels()).toEqual(['Take over', 'Nudge', 'Resume with this message']);
    await userEvent.click(bar().getByRole('button', { name: 'Resume with this message' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'resend' }));
  });

  it('nudges a stalled step with the words typed', async () => {
    const { calls } = triage([stalled], 'st');
    await userEvent.type(screen.getByRole('textbox', { name: 'Nudge message (optional)' }), 'Carry on');
    await userEvent.click(bar().getByRole('button', { name: 'Nudge' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'nudge', text: 'Carry on' }));
  });

  it('folds more than three choices into More ▾, the primary one on the right', async () => {
    const { calls } = triage([loop], 'l');
    expect(barLabels()).toEqual(['More', 'Once more']);
    await userEvent.click(bar().getByRole('button', { name: 'More' }));
    const sheet = await screen.findByRole('dialog', { name: 'More choices' });
    expect(within(sheet).getAllByRole('button').map((b) => b.textContent).filter((t) => t !== '')).toEqual(
      expect.arrayContaining(['Take over: approved', 'Take over: changes_requested', 'Stop']),
    );
    await userEvent.click(within(sheet).getByRole('button', { name: 'Stop' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'stop' }));
  });

  it("has a manual step's outcomes, with the note", async () => {
    const { calls } = triage([manual], 'm');
    expect(barLabels()).toEqual(['Failed', 'Passed']);
    await userEvent.type(screen.getByRole('textbox', { name: 'What you did (optional)' }), 'Works');
    await userEvent.click(bar().getByRole('button', { name: 'Passed' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'passed', text: 'Works' }));
  });

  it('has Open session for a session waiting', () => {
    triage([waiting], 'session:sess');
    expect(barLabels()).toEqual(['Open session']);
  });

  it('rejects without asking', async () => {
    const { calls } = triage([accept], 'acc');
    expect(barLabels()).toEqual(['Reject', 'Merge']);
    await userEvent.click(bar().getByRole('button', { name: 'Reject' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'reject' }));
  });

  it('merges only after the confirmation, which shows the PR, its CI and the diff size', async () => {
    const pr: PullRequestStatus = {
      number: 42, url: 'https://github.com/o/r/pull/42', title: 'Phone triage', state: 'open', draft: false, baseBranch: 'main',
      headSha: 'abc', mergeable: 'mergeable', behind: false, autoMerge: false, inMergeQueue: false, queuePosition: null,
      ci: { state: 'passed', url: 'https://ci', runs: [] }, queueCi: null, checkedAt: 1, error: null,
    };
    const t = ticket(7, {
      id: 't',
      attempts: [{ id: 'a1', branch: 'b', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, pr, ci: null }],
    });
    const file = (path: string, added: number, removed: number) => ({ path, oldPath: null, status: 'modified', untracked: false, binary: false, added, removed, version: path });
    const { calls } = triage([accept], 'acc', {
      'GET /api/tickets/t': t,
      'GET /api/worktrees/w1/changes': { worktreeId: 'w1', branch: 'b', base: 'main', mergeBase: 'x', head: 'y', dirty: false,
        files: [file('a.ts', 10, 2), file('b.ts', 3, 1)], attemptId: 'a1', ticketId: 't', attemptActive: true, approved: false },
    });
    await userEvent.click(bar().getByRole('button', { name: 'Merge' }));
    const dialog = await screen.findByRole('dialog', { name: 'Merge PDT-7?' });
    expect(answered(calls)).toBeUndefined();
    expect(await within(dialog).findByRole('link', { name: /PR #42/ })).toBeInTheDocument();
    expect(within(dialog).getByText('CI passed')).toBeInTheDocument();
    expect(await within(dialog).findByText('2 files, +13 −3')).toBeInTheDocument();
    expect(within(dialog).getByRole('button', { name: 'View changes' })).toBeInTheDocument();
    // Cancel answers nothing.
    await userEvent.click(within(dialog).getByRole('button', { name: 'Cancel' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Merge PDT-7?' })).toBeNull());
    expect(answered(calls)).toBeUndefined();
    await userEvent.click(bar().getByRole('button', { name: 'Merge' }));
    await userEvent.click(within(await screen.findByRole('dialog', { name: 'Merge PDT-7?' })).getByRole('button', { name: 'Merge' }));
    await waitFor(() => expect(answered(calls)).toEqual({ choice: 'merge' }));
    expect(await screen.findByText('All clear')).toBeInTheDocument();
  });

  it('opens an item that comes in while All clear shows', async () => {
    stubApi({ 'GET /api/inbox': { items: [], total: 0 }, 'POST /api/notifications/read-all': undefined, 'POST /api/inbox/m/answer': {} });
    let setItems: (items: InboxItem[]) => void = () => {};
    function Harness() {
      const [items, set] = useState<InboxItem[]>([manual]);
      const [id, setId] = useState('m');
      setItems = set;
      return <PhoneInboxItem items={items} updates={[]} itemId={id} onMove={setId} onList={() => {}} />;
    }
    renderWithQuery(<Harness />);
    await userEvent.click(bar().getByRole('button', { name: 'Passed' }));
    expect(await screen.findByText('All clear')).toBeInTheDocument();
    // The poll still lists the answered item next to a new one.
    act(() => setItems([manual, stalled]));
    expect(await screen.findByText('Reviewer stalled')).toBeInTheDocument();
    expect(barLabels()).toEqual(['Take over', 'Nudge']);
  });

  it('reaches All clear after the last item, and Back to Inbox goes to the list', async () => {
    const { onList } = triage([manual], 'm');
    await userEvent.click(bar().getByRole('button', { name: 'Passed' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Back to Inbox' }));
    expect(onList).toHaveBeenCalled();
  });
});
