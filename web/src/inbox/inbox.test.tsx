import { cleanup, fireEvent, screen, waitFor, within } from '@testing-library/react';
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
import { after, dismissable, grouped, step } from './model';

afterEach(() => vi.unstubAllGlobals());

const item = (id: string, over: Partial<InboxItem> = {}): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, changeRequest: null, title: `Item ${id}`,
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

  it('Enter on a focused non-primary button presses that button, not the primary decision', async () => {
    const calls = stubApi({ 'POST /api/inbox/g/answer': (c: Call) => c.body, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[gate]} selectedId="g" onSelect={() => {}} />);
    const hold = screen.getByRole('button', { name: /hold/ });
    hold.focus();
    // The Inbox's window listener sees the keydown before the button's press (React Aria presses on keyup).
    fireEvent.keyDown(hold, { key: 'Enter' }); await new Promise((r) => setTimeout(r, 50)); expect(calls.filter((c) => c.path.endsWith('/answer'))).toEqual([]);
    await userEvent.click(hold);
    const answers = () => calls.filter((c) => c.method === 'POST' && c.path.endsWith('/answer'));
    await waitFor(() => expect(answers()).toHaveLength(1));
    expect(answers()[0].body).toEqual({ choice: 'hold' });
  });

  it('Enter on a focused link follows it and does not take the primary decision', async () => {
    const manual = item('m', {
      kind: 'manual', group: 'gate', title: 'Your turn', body: { prompt: 'Read [the spec](https://example.com/spec) first.' },
      choices: [{ id: 'done', title: 'done', primary: true }, { id: 'failed', title: 'failed', primary: false }],
    });
    const calls = stubApi({ 'POST /api/inbox/m/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[manual]} selectedId="m" onSelect={() => {}} />);
    const link = screen.getByRole('link', { name: 'the spec' });
    link.focus();
    fireEvent.keyDown(link, { key: 'Enter' });
    await new Promise((r) => setTimeout(r, 50));
    expect(calls.filter((c) => c.path.endsWith('/answer'))).toEqual([]);
  });

  it('takes a requested secret in a masked field and sends it only as the secret', async () => {
    const request = item('sr', {
      kind: 'secret', group: 'question', sessionId: 'sess', title: 'Secret NPM_TOKEN: Publish',
      body: { name: 'NPM_TOKEN', purpose: 'Publish the package', sessionTitle: 'Coder · PDT-9' },
    });
    const calls = stubApi({ 'POST /api/inbox/sr/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[request]} selectedId="sr" onSelect={() => {}} />);
    expect(screen.getByText(/Never paste it into the chat/)).toBeInTheDocument();
    const field = screen.getByLabelText('Value of NPM_TOKEN');
    expect(field).toHaveAttribute('type', 'password');
    const store = screen.getByRole('button', { name: 'Store secret' });
    await userEvent.type(field, 'abc');
    expect(store.hasAttribute('disabled') || store.getAttribute('aria-disabled') === 'true').toBe(true);
    await userEvent.type(field, '-def-123{Enter}');
    await waitFor(() => expect(calls.find((c) => c.path.endsWith('/answer'))?.body).toEqual({ secret: 'abc-def-123' }));
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

  it('resumes a stalled step whose message never arrived with that message, Enter too', async () => {
    const undelivered = item('un', {
      kind: 'stalled', group: 'stalled', sessionId: 'sess', personaName: 'Coder', title: 'Coder stalled',
      body: { why: 'Its message was never delivered: [PDT] Your step is back for another pass.', message: '[PDT] Your step is back for another pass.' },
      choices: [
        { id: 'resend', title: 'Resume with this message', primary: true },
        { id: 'nudge', title: 'Nudge', primary: false },
        { id: 'outcome:submitted', title: 'Take over: submitted', primary: false },
      ],
    });
    const calls = stubApi({ 'POST /api/inbox/un/answer': (c: Call) => c.body, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[undelivered]} selectedId="un" onSelect={() => {}} />);
    expect(screen.getByText(/Its message was never delivered/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Resume with this message/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path.endsWith('/answer'))?.body).toEqual({ choice: 'resend' }));
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(calls.filter((c) => c.method === 'POST' && c.path.endsWith('/answer'))[1]?.body).toEqual({ choice: 'resend' }));
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

  it('shows an unheard notice: what it said, Enter opens the ticket, e dismisses it', async () => {
    const notice = item('n', {
      kind: 'notice', group: 'notice', stepId: null, title: 'PR #12 conflicts with its base and no agent session is open',
      body: { notice: 'conflict', key: 'conflict:abc', text: '[PDT] PR #12 (b) conflicts with main.', workflow: 'PR notices' },
    });
    expect(grouped([gate, notice]).map((g) => g.title)).toEqual(['Unheard notices', 'Gates']);
    const calls = stubApi({ 'POST /api/inbox/n/dismiss': undefined, 'GET /api/inbox': { items: [], total: 0 } });
    const openTicket = vi.fn();
    renderWithQuery(
      <OpenTicketContext.Provider value={openTicket}>
        <InboxView items={[notice]} selectedId="n" onSelect={() => {}} />
      </OpenTicketContext.Provider>,
    );
    expect(screen.getByRole('region', { name: 'Unheard notice' })).toHaveTextContent('conflicts with main');
    expect(screen.getByText(/Unheard notice ·/)).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Enter' });
    expect(openTicket).toHaveBeenCalledWith({ id: 't', projectId: 'p' });
    fireEvent.keyDown(window, { key: 'e' });
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/inbox/n/dismiss')).toBe(true));
  });

  it('shows a triaged ticket with its notes and issue: Enter approves it, Reject rejects it (PDT-225)', async () => {
    const triaged = item('a', {
      kind: 'admit', group: 'admit', runId: 'r', stepId: null, title: 'Crash on start',
      body: { summary: 'Panics when the config is empty.', issue: 'https://github.com/o/r/issues/2', external: true, workflow: 'Bug triage' },
      choices: [{ id: 'approve', title: 'Approve', primary: true }, { id: 'reject', title: 'Reject', primary: false }],
    });
    expect(grouped([gate, triaged]).map((g) => g.title)).toEqual(['Triaged tickets', 'Gates']);
    expect(dismissable(triaged)).toBe(false);
    const calls = stubApi({ 'POST /api/inbox/a/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[triaged]} selectedId="a" onSelect={() => {}} />);
    const region = screen.getByRole('region', { name: 'Triaged ticket' });
    expect(region).toHaveTextContent('Panics when the config is empty.');
    expect(region).toHaveTextContent('only you can');
    expect(screen.getByRole('link', { name: 'https://github.com/o/r/issues/2' })).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/inbox/a/answer')?.body).toEqual({ choice: 'approve' }));
    await userEvent.click(screen.getByRole('button', { name: 'Reject' }));
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/inbox/a/answer').map((c) => c.body)).toContainEqual({ choice: 'reject' }));
  });

  it("shows an agent's workflow proposal with its note and what changed: Enter accepts it, Discard discards it, Open opens it (PDT-320)", async () => {
    const proposal = item('proposal:w1:2', {
      kind: 'proposal', group: 'proposal', projectId: null, ticketId: null, ticketKey: null, ticketTitle: null, runId: null, stepId: null,
      title: 'PM proposed version 2 of Scryer import sweeper',
      body: { workflowId: 'w1', workflow: 'Scryer import sweeper', version: 2, current: 1, by: 'PM', note: 'Sweep every hour', summary: 'changed sweep', enabled: true },
      choices: [{ id: 'discard', title: 'Discard', primary: false }, { id: 'accept', title: 'Accept', primary: true }],
    });
    expect(grouped([gate, proposal]).map((g) => g.title)).toEqual(['Workflow proposals', 'Gates']);
    expect(dismissable(proposal)).toBe(false);
    const calls = stubApi({ 'POST /api/inbox/proposal%3Aw1%3A2/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[proposal]} selectedId={proposal.id} onSelect={() => {}} />);
    const region = screen.getByRole('region', { name: 'Workflow proposal' });
    expect(region).toHaveTextContent('Sweep every hour');
    expect(region).toHaveTextContent('changed sweep');
    expect(region).toHaveTextContent('which runs from then on');
    const answers = () => calls.filter((c) => c.method === 'POST' && c.path.endsWith('/answer')).map((c) => c.body);
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(answers()).toEqual([{ choice: 'accept' }]));
    await userEvent.click(within(region).getByRole('button', { name: 'Discard' }));
    await waitFor(() => expect(answers()).toContainEqual({ choice: 'discard' }));
    await userEvent.click(within(region).getByRole('button', { name: 'Open' }));
    expect(window.location.pathname).toBe('/workflows/w1/versions');
  });

  it('shows a Codex PM\'s call with its arguments: Enter allows it, Deny denies it (PDT-256)', async () => {
    const call = item('c', {
      kind: 'confirm', group: 'approval', runId: null, stepId: null, title: 'Report a bug in PDT',
      body: { tool: 'report_bug', toolTitle: 'Report a bug in PDT', agent: 'Codex', sessionTitle: 'PM', input: { title: 'The board flickers' } },
      choices: [{ id: 'allow', title: 'Allow', primary: true }, { id: 'deny', title: 'Deny', primary: false }],
    });
    expect(dismissable(call)).toBe(false);
    const calls = stubApi({ 'POST /api/inbox/c/answer': {}, 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[call]} selectedId="c" onSelect={() => {}} />);
    const region = screen.getByRole('region', { name: 'Agent call' });
    expect(region).toHaveTextContent('PM (Codex) wants to call report_bug');
    expect(screen.getByLabelText("The call's arguments")).toHaveTextContent('"title": "The board flickers"');
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/inbox/c/answer')?.body).toEqual({ choice: 'allow' }));
    await userEvent.click(screen.getByRole('button', { name: 'Deny' }));
    await waitFor(() => expect(calls.filter((c) => c.path === '/api/inbox/c/answer').map((c) => c.body)).toContainEqual({ choice: 'deny' }));
  });

  it('asks to connect GitHub: Enter opens GitHub’s plugin page, and it is not dismissed', async () => {
    const connect = item('connect:github', {
      kind: 'connect', group: 'notice', projectId: null, ticketId: null, runId: null, stepId: null, title: 'Connect GitHub',
      body: { forge: 'github', text: 'PDT reads pull requests with your GitHub login, and it has none.' },
    });
    stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[connect]} selectedId="connect:github" onSelect={() => {}} />);
    expect(screen.getByRole('region', { name: 'Connect GitHub' })).toHaveTextContent('it has none');
    expect(dismissable(connect)).toBe(false);
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(window.location.pathname).toBe('/settings/plugins/github'));
  });

  it('says PDT can’t save changes, with what to do, and it is not dismissed', () => {
    const storage = item('storage', {
      kind: 'storage', group: 'notice', projectId: null, ticketId: null, runId: null, stepId: null,
      title: "PDT can't save changes: the disk is full",
      body: { text: "The disk with PDT's data is full (12 MB free). Free up space on it.", diskFull: true, freeBytes: 12582912 },
    });
    stubApi({ 'GET /api/inbox': { items: [], total: 0 } });
    renderWithQuery(<InboxView items={[storage]} selectedId="storage" onSelect={() => {}} />);
    expect(screen.getByRole('region', { name: "Can't save changes" })).toHaveTextContent('(12 MB free)');
    expect(dismissable(storage)).toBe(false);
  });

  it('warns of low disk space, cleans finished worktrees, and opens Settings › Storage', async () => {
    const low = item('low_disk', {
      kind: 'low_disk', group: 'notice', projectId: null, ticketId: null, runId: null, stepId: null,
      title: 'Disk space is low (12 GB free)',
      body: { text: 'The disk (with `/work`) has 12 GB free.', freeBytes: 12884901888, folders: ['/work'], critical: false },
    });
    const calls = stubApi({
      'GET /api/inbox': { items: [], total: 0 },
      'POST /api/storage/clean-finished': { cleaned: 2, freed: 3221225472, skipped: [] },
    });
    renderWithQuery(<InboxView items={[low]} selectedId="low_disk" onSelect={() => {}} />);
    expect(screen.getByRole('region', { name: 'Low disk space' })).toHaveTextContent('12 GB free');
    expect(dismissable(low)).toBe(false);
    await userEvent.click(screen.getByRole('button', { name: 'Clean all finished' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/storage/clean-finished')).toBe(true));
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(window.location.pathname).toBe('/settings/host/storage'));
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
      mergeable: 'conflicting', behind: false, autoMerge: false, inMergeQueue: false, queuePosition: null, ci: null, queueCi: null, checkedAt: 1, error: null, local: false, mergeHeld: false,
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

  it('shows what a gate says beyond its question, links too (PDT-288)', () => {
    stubApi({});
    const nightly = item('n', {
      ticketId: null, ticketKey: null, ticketTitle: null, title: "Tonight's nightly didn't run",
      body: { workflow: 'Missing nightly', text: 'See [the release workflow\'s runs](https://github.com/o/r/actions/workflows/release.yml).' },
      choices: [{ id: 'start', title: 'Start it', primary: true }, { id: 'ignore', title: 'Ignore', primary: false }],
    });
    renderWithQuery(<InboxView items={[nightly]} selectedId="n" onSelect={() => {}} />);
    expect(screen.getByRole('link', { name: "the release workflow's runs" })).toHaveAttribute('href', 'https://github.com/o/r/actions/workflows/release.yml');
    expect(screen.getByRole('button', { name: /Start it/ })).toBeInTheDocument();
  });

  it('shows a gate\'s question as Markdown under a short title, with its pull request (PDT-306)', async () => {
    const review = 'Your review:\n\n**[acme/app#314](https://github.com/acme/app/pull/314)** by bot\n\n**Risks:** none.\n\n' + 'More of the review. '.repeat(400);
    const pr = item('pr', {
      ticketId: 't', title: 'Ask me', changeRequest: { url: 'https://github.com/acme/app/pull/314', name: 'pull request' },
      body: { workflow: 'PDT pull request review', question: review },
      choices: [{ id: 'approve_merge', title: 'Approve merge', primary: true }, { id: 'close', title: 'Close', primary: false }],
    });
    stubApi({ 'GET /api/tickets/t': ticket(1) });
    const open = vi.spyOn(window, 'open').mockReturnValue(null);
    renderWithQuery(<InboxView items={[pr]} selectedId="pr" onSelect={() => {}} />);
    expect(screen.getByRole('heading', { name: 'Ask me' })).toBeInTheDocument();
    const question = screen.getByLabelText('Question');
    const link = within(question).getByRole('link', { name: 'acme/app#314' });
    expect(link).toHaveAttribute('href', 'https://github.com/acme/app/pull/314');
    expect(link).toHaveAttribute('target', '_blank');
    expect(within(question).getByText('Risks:').tagName).toBe('STRONG');
    // However long the question, the choices sit outside the part that scrolls.
    const choices = screen.getByRole('group', { name: 'Your choice' });
    expect(within(choices).getByRole('button', { name: /Approve merge/ })).toBeInTheDocument();
    expect(question.closest('.overflow-auto')).not.toContainElement(choices);
    await userEvent.click(screen.getByRole('button', { name: 'Open pull request' }));
    expect(open).toHaveBeenCalledWith('https://github.com/acme/app/pull/314', '_blank', 'noopener');
    open.mockRestore();
  });

  it('offers no pull request for a gate that has none, and repeats no one-line question', () => {
    stubApi({ 'GET /api/tickets/t': ticket(1) });
    renderWithQuery(<InboxView items={[item('g1', { title: 'Ship it?', body: { question: 'Ship it?' }, choices: gate.choices })]} selectedId="g1" onSelect={() => {}} />);
    expect(screen.queryByRole('button', { name: /Open pull request|Open merge request/ })).toBeNull();
    expect(screen.queryByLabelText('Question')).toBeNull();
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

  it('clears one Update from its row or its view, and Clear all empties Updates (PDT-224)', async () => {
    const { notification } = await import('../notifications/fixtures');
    const onClear = vi.fn();
    const onClearAll = vi.fn();
    const { unmount } = renderWithQuery(
      <InboxList items={[]} updates={[notification('u1', { title: 'PDT-7 merged' }), notification('u2', { title: 'Old' })]}
        selectedId={null} onSelect={() => {}} onClear={onClear} onClearAll={onClearAll} />,
    );
    await userEvent.click(screen.getAllByRole('button', { name: 'Clear' })[0]);
    expect(onClear).toHaveBeenCalledWith('u1');
    await userEvent.click(screen.getByRole('button', { name: 'Clear all' }));
    expect(onClearAll).toHaveBeenCalled();
    unmount();

    renderWithQuery(<InboxList items={[]} updates={[]} selectedId={null} onSelect={() => {}} onClearAll={onClearAll} />);
    expect(screen.getByRole('button', { name: 'Clear all' })).toBeDisabled();
    cleanup();

    const calls = stubApi({ 'POST /api/notifications/u1/read': null, 'POST /api/notifications/u1/clear': null, 'GET /api/notifications': { items: [], unreadUpdates: 0 }, 'GET /api/inbox': { items: [], total: 0 } });
    const onSelect = vi.fn();
    renderWithQuery(<InboxView items={[]} updates={[notification('u1')]} selectedId="update:u1" onSelect={onSelect} />);
    await userEvent.click(screen.getByRole('button', { name: 'Clear' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/notifications/u1/clear')).toBe(true));
    expect(onSelect).toHaveBeenCalledWith(null);
  });
});
