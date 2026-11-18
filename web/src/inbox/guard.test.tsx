// The Inbox's guards on answers (PDT-623): a double-click answers one item, a click on the item that slid into place
// does nothing, answers that act outside PDT ask for a second press, Undo cancels an answer before it goes out, and
// Recently answered says what was pressed and how.
import { useState } from 'react';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { ApprovalCard, approvalTiming, toast } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import { answerTiming, guardedAnswer } from '../common/answerGuard';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { holdAnswer, holdTiming, useHeldIds } from './held';
import { InboxList } from './InboxList';
import { InboxView } from './InboxView';
import { needsConfirm } from './model';
import { useInbox } from './useInbox';

const item = (id: string, over: Partial<InboxItem> = {}): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, changeRequest: null, title: `Item ${id}`,
  body: {}, choices: [{ id: 'ship', title: 'Ship', primary: true }, { id: 'hold', title: 'Hold', primary: false }],
  resolution: null, createdAt: 1, resolvedAt: null, ...over,
});

/** A pull request's review gate, as the outside-PR workflow asks it. */
const review = item('pr', {
  title: 'Review #613', createdAt: 0,
  choices: [
    { id: 'approve_merge', title: 'Approve merge', primary: true },
    { id: 'request_changes', title: 'Request changes', primary: false },
    { id: 'close', title: 'Close', primary: false },
  ],
});

/** The Inbox as the app shows it: answered (held) items leave the list, and the next one is selected. */
function Inbox({ items }: { items: InboxItem[] }) {
  const [selected, setSelected] = useState<string | null>(items[0].id);
  const held = useHeldIds();
  return <InboxView items={items.filter((i) => !held.has(i.id))} selectedId={selected} onSelect={setSelected} />;
}

const answers = (calls: Call[]) => calls.filter((c) => c.method === 'POST' && c.path.endsWith('/answer'));

describe('the guard on answers', () => {
  it('runs one answer at a time, and none for a moment after one', async () => {
    answerTiming.cooldownMs = 200;
    let release = () => {};
    const first = guardedAnswer(() => new Promise<string>((r) => { release = () => r('first'); }));
    expect(await guardedAnswer(async () => 'double')).toBeUndefined();
    release();
    expect(await first).toBe('first');
    expect(await guardedAnswer(async () => 'too soon')).toBeUndefined();
    await new Promise((r) => setTimeout(r, 250));
    expect(await guardedAnswer(async () => 'later')).toBe('later');
  });

  it('answers only the first item on a fast double-click; the item that slid into place waits a moment', async () => {
    answerTiming.cooldownMs = 400;
    const calls = stubApi({ 'POST /api/inbox/a/answer': {}, 'POST /api/inbox/b/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[item('a', { title: 'First' }), item('b', { title: 'Second' })]} />);
    await userEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    // The second click lands on the next item, which slid into place: its actions are off, and Enter does nothing.
    expect(screen.getByRole('heading', { name: 'Second' })).toBeInTheDocument();
    const ship = screen.getByRole('button', { name: /^Ship/ });
    expect(ship).toBeDisabled();
    await userEvent.click(ship);
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(answers(calls).map((c) => c.path)).toEqual(['/api/inbox/a/answer']));
    // A moment later it is answerable.
    await waitFor(() => expect(screen.getByRole('button', { name: /^Ship/ })).toBeEnabled());
    await userEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    await waitFor(() => expect(answers(calls).map((c) => c.path)).toEqual(['/api/inbox/a/answer', '/api/inbox/b/answer']));
  });

  it('answers one request when a conversation card is double-clicked', async () => {
    let release = () => {};
    const onAnswer = vi.fn(() => new Promise<void>((r) => { release = r; }));
    renderWithQuery(<ApprovalCard title="Run this command?" subject="ls" onAnswer={onAnswer} />);
    const allow = screen.getByRole('button', { name: 'Allow' });
    fireEvent.click(allow);
    fireEvent.click(allow);
    expect(onAnswer).toHaveBeenCalledTimes(1);
    await act(async () => release());
  });
});

describe('answers that act outside PDT ask for a second press', () => {
  it('knows which: Request changes and Close on a gate, Always allow, Allow on a tool that posts outside', () => {
    expect(needsConfirm(review, 'request_changes')).toBe(true);
    expect(needsConfirm(review, 'close')).toBe(true);
    expect(needsConfirm(review, 'approve_merge')).toBe(false);
    const approval = item('approval:s:r', { kind: 'approval', group: 'approval', body: { tool: 'mcp__pdt__issue_create' } });
    expect(needsConfirm(approval, 'allow')).toBe(true);
    expect(needsConfirm(approval, 'deny')).toBe(false);
    expect(needsConfirm(approval, 'always')).toBe(true);
    const local = item('approval:s:q', { kind: 'approval', group: 'approval', body: { tool: 'Bash' } });
    expect(needsConfirm(local, 'allow')).toBe(false);
    expect(needsConfirm(item('c', { kind: 'confirm', body: { tool: 'change_request_comment' } }), 'allow')).toBe(true);
  });

  it('arms Request changes on the first press and sends it on the second, not on a double-click', async () => {
    answerTiming.cooldownMs = 300;
    const calls = stubApi({ 'POST /api/inbox/pr/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[review]} />);
    const request = screen.getByRole('button', { name: 'Request changes' });
    fireEvent.click(request);
    fireEvent.click(screen.getByRole('button', { name: 'Confirm request changes' }));
    expect(screen.getByText(/Press again to confirm/)).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 350));
    expect(answers(calls)).toEqual([]);
    await userEvent.click(screen.getByRole('button', { name: 'Confirm request changes' }));
    await waitFor(() => expect(answers(calls).map((c) => c.body)).toEqual([{ choice: 'request_changes' }]));
  });

  it('asks again for y on an Allow that posts outside PDT', async () => {
    const calls = stubApi({ 'POST /api/sessions/s/requests/r': undefined, 'GET /api/inbox': { items: [], total: 0 } });
    const approval = item('approval:s:r', {
      kind: 'approval', group: 'approval', sessionId: 's', title: 'Create an issue', body: { requestId: 'r', tool: 'mcp__pdt__issue_create', input: { title: 'Bug' } },
      choices: [{ id: 'allow', title: 'Allow', primary: true }, { id: 'deny', title: 'Deny', primary: false }],
    });
    renderWithQuery(<Inbox items={[approval]} />);
    fireEvent.keyDown(window, { key: 'y' });
    expect(screen.getByRole('button', { name: /^Confirm allow once/ })).toBeInTheDocument();
    expect(calls.filter((c) => c.path.startsWith('/api/sessions'))).toEqual([]);
    fireEvent.keyDown(window, { key: 'y' });
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s/requests/r')?.body).toEqual({ answer: { type: 'approve', always: false } }));
  });

  it('asks again for Close and for an approval’s Always allow', async () => {
    const calls = stubApi({ 'POST /api/inbox/pr/answer': {}, 'POST /api/sessions/s/requests/r': undefined, 'GET /api/inbox': { items: [], total: 0 } });
    const approval = item('approval:s:r', {
      kind: 'approval', group: 'approval', sessionId: 's', title: 'ls', body: { requestId: 'r', tool: 'Bash', input: { command: 'ls' } },
      choices: [{ id: 'allow', title: 'Allow', primary: true }, { id: 'allow_always', title: 'Always allow in this session', primary: false }, { id: 'deny', title: 'Deny', primary: false }],
    });
    renderWithQuery(<Inbox items={[approval, review]} />);
    await userEvent.click(screen.getByRole('button', { name: 'Always allow' }));
    expect(calls.filter((c) => c.path.startsWith('/api/sessions'))).toEqual([]);
    await userEvent.click(screen.getByRole('button', { name: 'Confirm always allow' }));
    await waitFor(() => expect(calls.find((c) => c.path === '/api/sessions/s/requests/r')?.body).toEqual({ answer: { type: 'approve', always: true } }));
    await userEvent.click(await screen.findByRole('button', { name: 'Close' }));
    expect(answers(calls)).toEqual([]);
    await userEvent.click(screen.getByRole('button', { name: 'Confirm close' }));
    await waitFor(() => expect(answers(calls).map((c) => c.body)).toEqual([{ choice: 'close' }]));
  });

  it('asks again for Always allow on a conversation’s card, not within a double-click', async () => {
    approvalTiming.confirmAfterMs = 300;
    const onAnswer = vi.fn();
    renderWithQuery(<ApprovalCard title="Edit this file?" subject="a.ts" canAlways onAnswer={onAnswer} />);
    fireEvent.click(screen.getByRole('button', { name: 'Always allow in this session' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm: always allow in this session' }));
    expect(onAnswer).not.toHaveBeenCalled();
    await new Promise((r) => setTimeout(r, 350));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm: always allow in this session' }));
    expect(onAnswer).toHaveBeenCalledWith({ type: 'approve', always: true });
  });
});

describe('Undo', () => {
  it('cancels a review before it is posted: the answer never goes out and the item comes back', async () => {
    holdTiming.undoMs = 1500;
    // Earlier tests' toasts (Sonner keeps them for the page) are not this one's.
    toast.dismiss();
    const calls = stubApi({ 'POST /api/inbox/pr/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[review, item('b', { title: 'Second' })]} />);
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument());
    await userEvent.click(screen.getByRole('button', { name: 'Request changes' }));
    await userEvent.click(screen.getByRole('button', { name: 'Confirm request changes' }));
    expect(await screen.findByRole('heading', { name: 'Second' })).toBeInTheDocument();
    const shown = (await screen.findByText('Request changes')).closest('li')!;
    await userEvent.click(within(shown).getByRole('button', { name: 'Undo' }));
    expect(await screen.findByRole('heading', { name: 'Review #613' })).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 1600));
    expect(answers(calls)).toEqual([]);
  });

  it('sends the answer once the window passes, saying how it was given', async () => {
    holdTiming.undoMs = 200;
    const calls = stubApi({ 'POST /api/inbox/a/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[item('a')]} />);
    fireEvent.keyDown(window, { key: 'Enter' });
    await new Promise((r) => setTimeout(r, 50));
    expect(answers(calls)).toEqual([]);
    await waitFor(() => expect(answers(calls).map((c) => c.body)).toEqual([{ choice: 'ship' }]));
    const init = vi.mocked(fetch).mock.calls.find(([path]) => String(path) === '/api/inbox/a/answer')![1]!;
    expect((init.headers as Record<string, string>)['x-pdt-answer-source']).toBe('keyboard/desktop');
  });

  it('leaves held answers out of the Inbox until they have gone, and brings back one that failed', async () => {
    const a = item('a');
    stubApi({ 'GET /api/inbox': { items: [a, item('b')], total: 2 } });
    let fail = () => {};
    function Count() {
      const { data } = useInbox();
      return <p>{data ? `${data.items.map((i) => i.id).join(',')} of ${data.total}` : 'loading'}</p>;
    }
    renderWithQuery(<Count />);
    expect(await screen.findByText('a,b of 2')).toBeInTheDocument();
    holdTiming.undoMs = 0;
    act(() => holdAnswer({ itemId: 'a', title: a.title, label: 'Ship', send: () => new Promise<void>((_, reject) => { fail = () => reject(new Error('no')); }) }));
    expect(await screen.findByText('b of 1')).toBeInTheDocument();
    await act(async () => fail());
    expect(await screen.findByText('a,b of 2')).toBeInTheDocument();
  });
});

describe('Recently answered', () => {
  const answered = (over: object) => ({
    seq: 1, id: 'e1', type: 'inbox.answered', key: 'k', projectId: 'p', ticketId: 't', attemptId: null, sessionId: null, source: 'inbox',
    createdAt: Date.now(), processedAt: null,
    payload: {
      itemId: 'pr', kind: 'gate', group: 'gate', title: 'Review #613', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', sessionId: null,
      persona: null, runId: 'r', workflowId: 'w', createdAt: 0, ageMs: 1, choice: 'request_changes', answer: null, by: 'user',
      source: { via: 'click', device: 'phone' }, ...over,
    },
  });

  it('lists the last answers with what they did and how, and held ones with Undo', async () => {
    stubApi({
      'GET /api/events': [answered({}), { ...answered({ itemId: 'q', title: 'Which file?', choice: null, answer: 'a.txt', source: { via: 'keyboard', device: 'desktop' } }), id: 'e2' }],
      'GET /api/inbox': { items: [], total: 0 },
    });
    renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
    const list = await screen.findByRole('grid', { name: 'Recently answered' });
    expect(await within(list).findByText('Request changes · click · phone')).toBeInTheDocument();
    expect(within(list).getByText('Answered: “a.txt” · keyboard · desktop')).toBeInTheDocument();
    holdTiming.undoMs = 60_000;
    act(() => holdAnswer({ itemId: 'x', title: 'Ship it?', label: 'Ship', send: async () => {} }));
    expect(await within(list).findByText('Ship · waiting to send')).toBeInTheDocument();
    await userEvent.click(within(list).getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(within(list).queryByText('Ship · waiting to send')).not.toBeInTheDocument());
  });

  it('says when an agent answered', async () => {
    stubApi({ 'GET /api/events': [answered({ by: 'agent', source: null, choice: 'allow' })], 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
    expect(await screen.findByText('Allow · by an agent')).toBeInTheDocument();
  });

  it('shows nothing until something was answered', async () => {
    stubApi({ 'GET /api/events': [], 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
    await waitFor(() => expect(vi.mocked(fetch)).toHaveBeenCalled());
    expect(screen.queryByRole('heading', { name: 'Recently answered' })).not.toBeInTheDocument();
  });
});
