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
  ticketId: null, sessionId: null, inboxItemId: null, readAt: null, clearedAt: null, dismissedAt: null, createdAt: Date.now(), ...over,
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

// Real-shaped items, as `GET /api/inbox` returns them (PDT-209): an `ask_user` question from a workflow step, and an
// approval wrapping Claude's AskUserQuestion request, with the session's request event in its body.
const asked = item('01a182c0-9fea-719f-b79e-2d6418ebc334', {
  kind: 'question', group: 'question', attemptId: 'a1', sessionId: 'sess', personaId: 'qa', personaName: 'QA',
  title: 'PDT-7 QA: one point needs a fork with the signing secrets. How should I treat it?',
  body: { questions: [{
    choices: ['Pass now: I will do the dry run myself', 'Hold: I will do the dry run first', 'Fail until the dry run has been done'],
    question: 'PDT-7 QA: everything I can check locally passes. One point needs a fork with the signing secrets. How should I treat it?',
  }] },
});
const askInput = { questions: [{ question: 'Which library?', header: 'Library', multiSelect: false,
  options: [{ label: 'zod', description: 'Schemas' }, { label: 'valibot', description: 'Smaller' }] }] };
const asking = item('approval:sess:req-9', {
  kind: 'approval', group: 'approval', attemptId: 'a1', sessionId: 'sess', ticketKey: null, ticketTitle: null, title: 'AskUserQuestion',
  body: { requestId: 'req-9', sessionTitle: 'Coder · PDT-7', tool: 'AskUserQuestion', description: null, input: askInput, diff: null,
    request: { type: 'request', id: 'req-9', requestKind: 'question', tool: 'AskUserQuestion', title: '', input: askInput, canAlways: false, state: 'open' } },
  choices: [{ id: 'allow', title: 'Allow', primary: true }, { id: 'deny', title: 'Deny', primary: false }],
});

/** The phone's Inbox as the shell shows it: the list, and an item once one is tapped. */
function PhoneInbox({ items }: { items: InboxItem[] }) {
  const [id, setId] = useState<string | null>(null);
  return id
    ? <PhoneInboxItem items={items} updates={[]} itemId={id} onMove={setId} onList={() => setId(null)} />
    : <PhoneInboxList items={items} updates={[]} onOpen={setId} onReadAll={() => {}} onClear={() => {}} onClearAll={() => {}} onSettings={() => {}} />;
}

describe('phone inbox: opening a question (PDT-209)', () => {
  it("opens an agent's ask_user question from the list and answers it", async () => {
    const calls = stubApi({ 'POST /api/notifications/read-all': undefined, [`POST /api/inbox/${asked.id}/answer`]: {} });
    renderWithQuery(<PhoneInbox items={[asked, asking]} />);
    await userEvent.click(screen.getByText(asked.title));
    expect(screen.getByRole('heading', { name: asked.title })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Hold: I will do the dry run first' }));
    await userEvent.click(bar().getByRole('button', { name: 'Send' }));
    await waitFor(() => expect(answered(calls)).toEqual({ answers: ['Hold: I will do the dry run first'] }));
  });

  it("opens an approval wrapping the agent's AskUserQuestion from the list and answers it in its session", async () => {
    const calls = stubApi({ 'POST /api/notifications/read-all': undefined, 'POST /api/sessions/sess/requests/req-9': undefined });
    renderWithQuery(<PhoneInbox items={[asked, asking]} />);
    await userEvent.click(screen.getByText('AskUserQuestion'));
    expect(screen.getByText('Which library?')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /valibot/ }));
    await userEvent.click(screen.getByRole('button', { name: 'Submit' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/sess/requests/req-9')?.body)
      .toEqual({ answer: { type: 'question', answers: { 'Which library?': ['valibot'] } } }));
  });

  it('shows Something went wrong, not a blank page, when an item fails to render', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    // A malformed item: `choices` missing throws while it renders.
    const broken = item('bad', { title: 'Broken gate', choices: null as unknown as InboxItem['choices'] });
    const { onList } = triage([broken], 'bad');
    expect(screen.getByRole('alert')).toHaveTextContent('Something went wrong');
    expect(screen.getByRole('button', { name: 'Reload' })).toBeInTheDocument();
    expect(error).toHaveBeenCalledWith('A view failed to render:', expect.any(TypeError), expect.anything());
    await userEvent.click(screen.getByRole('button', { name: 'Back to Inbox' }));
    expect(onList).toHaveBeenCalled();
    error.mockRestore();
  });
});

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
    renderWithQuery(<PhoneInboxList items={[stalled, waiting, question]} updates={[update('u1')]} onOpen={onOpen} onReadAll={() => {}} onClear={() => {}} onClearAll={() => {}} onSettings={() => {}} />);
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
    renderWithQuery(<PhoneInboxList items={[question]} updates={[update('u1')]} onOpen={onOpen} onReadAll={() => {}} onClear={() => {}} onClearAll={() => {}} onSettings={() => {}} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Updates (unread)' }));
    expect(screen.queryByRole('button', { name: /Review all/ })).toBeNull();
    await userEvent.click(screen.getByText('Update u1'));
    expect(onOpen).toHaveBeenCalledWith('update:u1');
    // The list remembers its segment: back to Needs you for the tests after this one.
    await userEvent.click(screen.getByRole('radio', { name: /Needs you/ }));
  });

  it('clears one Update, or all of them (PDT-224)', async () => {
    stubApi({});
    const onClear = vi.fn();
    const onClearAll = vi.fn();
    renderWithQuery(<PhoneInboxList items={[]} updates={[update('u1'), update('u2')]} onOpen={() => {}} onReadAll={() => {}} onClear={onClear} onClearAll={onClearAll} onSettings={() => {}} />);
    await userEvent.click(screen.getByRole('radio', { name: 'Updates (unread)' }));
    await userEvent.click(screen.getAllByRole('button', { name: 'Clear' })[1]);
    expect(onClear).toHaveBeenCalledWith('u2');
    await userEvent.click(screen.getByRole('button', { name: 'Clear all' }));
    expect(onClearAll).toHaveBeenCalled();
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
      ci: { state: 'passed', url: 'https://ci', runs: [] }, queueCi: null, checkedAt: 1, error: null, local: false, mergeHeld: false,
    };
    const t = ticket(7, {
      id: 't',
      attempts: [{ id: 'a1', branch: 'b', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null, lastHead: null, workspaceId: null, pr, ci: null }],
    });
    const file = (path: string, added: number, removed: number) => ({ path, oldPath: null, status: 'modified', untracked: false, binary: false, added, removed, version: path, generated: false, uncommitted: false });
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
