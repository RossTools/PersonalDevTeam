import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { plugins } from '../api/gen/plugins';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { PmState } from '../api/gen/PmState';
import type { Session } from '../api/gen/Session';
import type { WatchInfo } from '../api/gen/WatchInfo';
import { renderWithQuery, stubApi } from '../test-utils';
import { PmList } from './PmList';
import { PmView } from './PmView';
import { firstLine } from './usePm';

afterEach(() => vi.unstubAllGlobals());

const session = (id: string, createdAt: number): Session => ({
  id, projectId: 'host', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PM',
  cwd: '/data/pm', status: 'finished', exitCode: null, createdAt, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: { id: 'pm', name: 'PM', version: 1, stepId: null }, background: [],
});

const info = (s: Session, more: Partial<PmSessionInfo> = {}): PmSessionInfo => ({
  session: s, open: true, title: s.title, unseen: s.unseen, lastActivity: s.createdAt, rotation: 'none', reason: null, journal: null, via: null, resetFrom: null, ...more,
});

const endedOne = info({ ...session('s1', Date.UTC(2026, 9, 20, 9)), title: 'Old plans' }, { open: false, rotation: 'done', reason: 'age', journal: '## Today\nPlanned the PM.' });
const ended: PmState = { current: null, sessions: [endedOne] };
const live = (id: string, title: string, more: Partial<Session> = {}) => ({ ...session(id, 1), status: 'running' as const, title, ...more });
const two: PmState = {
  current: live('c2', 'A PDT bug'),
  sessions: [info(live('c2', 'A PDT bug')), info(live('c1', 'CB cutover', { unseen: true, activity: 'idle', attentionAt: 5 })), endedOne],
};
const conversationRoutes = {
  'GET /api/agents/claude/capabilities': { conversation: true, permissionModes: [], models: [], commands: [] },
};

describe('the PM', () => {
  beforeEach(() => vi.stubGlobal('WebSocket', class { readyState = 0; onopen = null; onmessage = null; onclose = null; close() {} }));

  it('reads a journal’s first line without its heading marks', () => {
    expect(firstLine('\n## Today\nPlanned.')).toBe('Today');
    expect(firstLine(null)).toBe('');
  });

  it('lists the open conversations (an unread one in bold), then the ended ones with their journal, and no notes', async () => {
    stubApi({ 'GET /api/pm': two });
    const onSelect = vi.fn();
    renderWithQuery(<PmList selection="current" onSelect={onSelect} onNew={() => {}} />);
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    await within(open).findByText('A PDT bug');
    expect(within(open).getAllByRole('row').map((r) => r.getAttribute('aria-label') ?? r.textContent)).toEqual(['A PDT bug', 'CB cutover']);
    // The newest open one is on show; the other has an unseen reply.
    expect(within(open).getByRole('row', { name: /A PDT bug/ })).toHaveAttribute('data-current');
    expect(within(open).getByText('CB cutover')).toHaveClass('font-semibold');
    expect(within(open).getByText('A PDT bug')).not.toHaveClass('font-semibold');
    const old = screen.getByRole('grid', { name: 'Ended PM conversations' });
    expect(old).toHaveTextContent('Old plans');
    expect(old).toHaveTextContent('Today');
    expect(screen.queryByText('Notes')).not.toBeInTheDocument();
    await userEvent.click(within(open).getByText('CB cutover'));
    expect(onSelect).toHaveBeenCalledWith('c1');
    await userEvent.click(within(old).getByText('Today'));
    expect(onSelect).toHaveBeenCalledWith('s1');
  });

  it('marks a conversation held in the Discord bot\'s DMs', async () => {
    stubApi({ 'GET /api/pm': { ...two, sessions: [info(live('c2', 'A PDT bug'), { via: 'discord' }), ...two.sessions.slice(1)] }, 'GET /api/integrations': plugins });
    renderWithQuery(<PmList selection="current" onSelect={() => {}} onNew={() => {}} />);
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    await waitFor(() => expect(within(open).getByRole('row', { name: /A PDT bug/ })).toHaveTextContent('via Discord'));
    expect(within(open).getByRole('row', { name: /CB cutover/ })).not.toHaveTextContent('via Discord');
  });

  it('dots a conversation that finished a turn in the background, and the dot goes once it is opened', async () => {
    stubApi({ 'GET /api/pm': two });
    const first = renderWithQuery(<PmList selection="current" onSelect={() => {}} onNew={() => {}} />);
    let open = await screen.findByRole('grid', { name: 'PM conversations' });
    await within(open).findByText('A PDT bug');
    expect(within(within(open).getByRole('row', { name: /CB cutover/ })).getByRole('img', { name: 'New reply' })).toBeInTheDocument();
    expect(within(within(open).getByRole('row', { name: /A PDT bug/ })).queryByRole('img', { name: 'New reply' })).not.toBeInTheDocument();
    first.unmount();
    // Opened: it is on show, so neither bold nor dotted.
    renderWithQuery(<PmList selection="c1" onSelect={() => {}} onNew={() => {}} />);
    open = await screen.findByRole('grid', { name: 'PM conversations' });
    await within(open).findByText('CB cutover');
    expect(within(open).getByText('CB cutover')).not.toHaveClass('font-semibold');
    expect(within(open).queryByRole('img', { name: 'New reply' })).not.toBeInTheDocument();
  });

  it('shows a spinner, not a dot, while a conversation is working', async () => {
    stubApi({ 'GET /api/pm': { current: live('c2', 'A PDT bug'), sessions: [
      info(live('c2', 'A PDT bug')), info(live('c1', 'CB cutover', { unseen: true, activity: 'working' })),
    ] } satisfies PmState });
    renderWithQuery(<PmList selection="current" onSelect={() => {}} onNew={() => {}} />);
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    const row = await within(open).findByRole('row', { name: /CB cutover/ });
    expect(row).toHaveTextContent('Working…');
    expect(row.querySelector('[data-slot="spinner"]')).not.toBeNull();
    expect(within(row).queryByRole('img', { name: 'New reply' })).not.toBeInTheDocument();
  });

  it('offers a new conversation when none is open', async () => {
    stubApi({ 'GET /api/pm': ended });
    const onNew = vi.fn();
    renderWithQuery(<PmList selection="current" onSelect={() => {}} onNew={onNew} />);
    expect(await screen.findByText('Ask something to start one')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'New conversation' }));
    expect(onNew).toHaveBeenCalled();
  });

  it('starts a conversation with the first message, beside the open ones', async () => {
    const calls = stubApi({ 'GET /api/pm': two, 'POST /api/pm/sessions': session('s2', 2) });
    const onSelect = vi.fn();
    renderWithQuery(<PmView selection="new" onSelect={onSelect} onNew={() => {}} />);
    const box = await screen.findByRole('textbox', { name: 'Ask the PM' });
    await userEvent.type(box, "What's in flight?{Enter}");
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('s2'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ text: "What's in flight?" });
  });

  it('keeps the start box’s unsent text when the page goes and comes back; starting clears it', async () => {
    stubApi({ 'GET /api/pm': ended, 'POST /api/pm/sessions': session('s2', 2) });
    const first = renderWithQuery(<PmView selection="current" onSelect={() => {}} onNew={() => {}} />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Ask the PM' }), 'half a thought');
    first.unmount();
    const onSelect = vi.fn();
    const second = renderWithQuery(<PmView selection="current" onSelect={onSelect} onNew={() => {}} />);
    const box = await screen.findByRole('textbox', { name: 'Ask the PM' });
    expect(box).toHaveValue('half a thought');
    await userEvent.type(box, '{Enter}');
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('s2'));
    second.unmount();
    renderWithQuery(<PmView selection="current" onSelect={() => {}} onNew={() => {}} />);
    expect(await screen.findByRole('textbox', { name: 'Ask the PM' })).toHaveValue('');
  });

  it('renames the conversation on show, and closes it after asking', async () => {
    const calls = stubApi({
      ...conversationRoutes,
      'GET /api/pm': two,
      'PATCH /api/pm/sessions/c1': live('c1', 'Cutover'),
      'DELETE /api/pm/sessions/c1': undefined,
    });
    const onSelect = vi.fn();
    renderWithQuery(<PmView selection="c1" onSelect={onSelect} onNew={() => {}} />);
    expect(await screen.findByRole('heading', { name: 'CB cutover' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Rename' }));
    const field = screen.getByRole('textbox', { name: 'Conversation name' });
    await userEvent.clear(field);
    await userEvent.type(field, 'Cutover{Enter}');
    await waitFor(() => expect(calls).toContainEqual({ method: 'PATCH', path: '/api/pm/sessions/c1', body: { title: 'Cutover' } }));
    await userEvent.click(screen.getByRole('button', { name: 'Close…' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent('writes its journal');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/pm/sessions/c1')).toBe(true));
    expect(onSelect).toHaveBeenCalledWith('current');
  });

  it('shows what the conversation watches in its header, and stops a watch after asking', async () => {
    const watch = (over: Partial<WatchInfo>): WatchInfo => ({
      id: 'w1', target: 'ticket', targetId: 't12', targetLabel: 'ALP-12', targetTitle: 'Cache the board', targetState: 'in_progress',
      workflowId: null, sessionId: 'c1', watcher: 'PM “CB cutover”', events: ['finished', 'failed', 'needs_you'], once: true,
      createdAt: Date.now() - 60_000, ...over,
    });
    let watches = [
      watch({}),
      watch({ id: 'w2', target: 'run', targetId: 'r1', targetLabel: 'Ship', targetTitle: null, targetState: 'running', workflowId: 'wf1', once: false, events: ['comment'] }),
      watch({ id: 'w3', sessionId: 'c2', watcher: 'PM “A PDT bug”' }),
    ];
    const calls = stubApi({
      ...conversationRoutes,
      'GET /api/pm': two,
      'GET /api/watches': () => watches,
      'DELETE /api/watches/w1': () => { watches = watches.filter((w) => w.id !== 'w1'); return undefined; },
    });
    renderWithQuery(<PmView selection="c1" onSelect={() => {}} onNew={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Watching 2 things' }));
    const list = await screen.findByRole('list', { name: 'Watches' });
    const [ticketRow, runRow] = within(list).getAllByRole('listitem');
    expect(within(ticketRow).getByRole('link', { name: 'ALP-12' })).toHaveAttribute('href', '/tickets/ALP-12');
    expect(ticketRow).toHaveTextContent('Cache the board');
    expect(ticketRow).toHaveTextContent('In progress');
    expect(ticketRow).toHaveTextContent('Hears when it finishes, fails or needs you · until it finishes · since');
    expect(within(runRow).getByRole('link', { name: 'Ship' })).toHaveAttribute('href', '/workflows/wf1/runs/r1');
    expect(runRow).toHaveTextContent('Running');
    expect(runRow).toHaveTextContent('Hears when it someone comments · ongoing');
    expect(list).not.toHaveTextContent('A PDT bug');
    // One request for every watch, shared with the board and the PM list.
    expect(calls.filter((c) => c.path.startsWith('/api/watches')).every((c) => c.path === '/api/watches')).toBe(true);

    await userEvent.click(within(ticketRow).getByRole('button', { name: 'Stop watching' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent('Stop watching ALP-12?');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Stop watching' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'DELETE', path: '/api/watches/w1', body: undefined }));
    await waitFor(() => expect(within(screen.getByRole('list', { name: 'Watches' })).getAllByRole('listitem')).toHaveLength(1));
    expect(screen.getByRole('list', { name: 'Watches' })).toHaveTextContent('Ship');
    // The chip behind the open popover counts one now.
    expect(await screen.findByRole('button', { name: 'Watching 1 thing', hidden: true })).toBeInTheDocument();
  });

  it('has no watch chip while the conversation watches nothing, and eyes the watching ones in the list', async () => {
    stubApi({
      ...conversationRoutes,
      'GET /api/pm': two,
      'GET /api/watches': [{
        id: 'w1', target: 'ticket', targetId: 't1', targetLabel: 'ALP-1', targetTitle: 'One', targetState: 'todo', workflowId: null,
        sessionId: 'c1', watcher: 'PM “CB cutover”', events: [], once: true, createdAt: 1,
      }],
    });
    renderWithQuery(<><PmList selection="c2" onSelect={() => {}} onNew={() => {}} /><PmView selection="c2" onSelect={() => {}} onNew={() => {}} /></>);
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    expect(await within(await within(open).findByRole('row', { name: /CB cutover/ })).findByRole('img', { name: 'Watching 1 thing' })).toBeInTheDocument();
    expect(within(within(open).getByRole('row', { name: /A PDT bug/ })).queryByRole('img', { name: /^Watching/ })).not.toBeInTheDocument();
    expect(await screen.findByRole('heading', { name: 'A PDT bug' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /^Watching/ })).not.toBeInTheDocument();
  });

  it('shows an ended conversation read-only with why it ended and its journal', async () => {
    const closed = { ...endedOne, reason: 'user' as const };
    stubApi({ ...conversationRoutes, 'GET /api/pm': { current: null, sessions: [closed] } });
    const onNew = vi.fn();
    renderWithQuery(<PmView selection="s1" onSelect={() => {}} onNew={onNew} />);
    expect(await screen.findByText(/This conversation ended because you closed it\./)).toBeInTheDocument();
    expect(screen.getByText(/Planned the PM/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Close…' })).not.toBeInTheDocument();
    await userEvent.click(screen.getAllByRole('button', { name: 'New conversation' })[0]);
    expect(onNew).toHaveBeenCalled();
  });

  it('resets the conversation on show after asking, and puts the fresh one in its place', async () => {
    const calls = stubApi({
      ...conversationRoutes,
      'GET /api/pm': two,
      'POST /api/pm/sessions/c1/reset': live('c3', 'CB cutover'),
    });
    const onSelect = vi.fn();
    renderWithQuery(<PmView selection="c1" onSelect={onSelect} onNew={() => {}} />);
    expect(await screen.findByRole('heading', { name: 'CB cutover' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset…' }));
    const dialog = await screen.findByRole('alertdialog');
    expect(dialog).toHaveTextContent('Starts this conversation fresh. Its journal is kept and the new one starts from it.');
    await userEvent.click(within(dialog).getByRole('button', { name: 'Reset' }));
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('c3'));
    expect(calls.some((c) => c.method === 'POST' && c.path === '/api/pm/sessions/c1/reset')).toBe(true);
  });

  it('resets from the list\'s menu, and the open row follows only when it was on show', async () => {
    stubApi({ 'GET /api/pm': two, 'POST /api/pm/sessions/c1/reset': live('c3', 'CB cutover') });
    const onSelect = vi.fn();
    renderWithQuery(<PmList selection="c1" onSelect={onSelect} onNew={() => {}} />);
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    const row = await within(open).findByRole('row', { name: /CB cutover/ });
    await userEvent.pointer({ keys: '[MouseRight]', target: row });
    await userEvent.click(await screen.findByRole('menuitem', { name: 'Reset…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Reset' }));
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('c3'));
  });

  it('shows a conversation being reset as resetting, never unread; Ended leaves out a reset one that carries on', async () => {
    const old = info({ ...session('c0', 1), title: 'CB cutover' }, { open: false, rotation: 'done', reason: 'reset', journal: 'Before.' });
    const cut = info({ ...session('c9', 1), title: 'Cut short' }, { open: false, rotation: 'done', reason: 'reset', journal: 'Half.' });
    stubApi({ 'GET /api/pm': { current: live('c2', 'A PDT bug'), sessions: [
      info(live('c2', 'A PDT bug')),
      info(live('c1', 'CB cutover', { unseen: true, activity: 'idle', attentionAt: 5 }), { rotation: 'journal', reason: 'reset', resetFrom: 'c0' }),
      old,
      cut,
    ] } satisfies PmState });
    renderWithQuery(<PmList selection="c2" onSelect={() => {}} onNew={() => {}} />);
    const open = await screen.findByRole('grid', { name: 'PM conversations' });
    const row = await within(open).findByRole('row', { name: /CB cutover/ });
    expect(row).toHaveTextContent('Resetting…');
    expect(within(row).queryByRole('img', { name: 'New reply' })).not.toBeInTheDocument();
    // c0 is reachable from c1; c9's reset never finished, so it is listed like a closed one.
    const ended = screen.getByRole('grid', { name: 'Ended PM conversations' });
    expect(within(ended).getAllByRole('row').map((r) => r.getAttribute('aria-label') ?? r.textContent)).toEqual(['Cut short']);
  });

  it('stays on the conversation the user moved to while a reset ran', async () => {
    let answer: (s: Session) => void = () => {};
    stubApi({
      ...conversationRoutes,
      'GET /api/pm': two,
      'POST /api/pm/sessions/c1/reset': () => new Promise<Session>((r) => { answer = r; }),
    });
    const onSelect = vi.fn();
    const view = renderWithQuery(<PmView selection="c1" onSelect={onSelect} onNew={() => {}} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Reset…' }));
    await userEvent.click(within(await screen.findByRole('alertdialog')).getByRole('button', { name: 'Reset' }));
    view.rerender(<PmView selection="c2" onSelect={onSelect} onNew={() => {}} />);
    expect(await screen.findByRole('heading', { name: 'A PDT bug' })).toBeInTheDocument();
    answer(live('c3', 'CB cutover'));
    await new Promise((r) => setTimeout(r, 50));
    expect(onSelect).not.toHaveBeenCalledWith('c3');
  });

  it('links a reset conversation to the one before it, which opens read-only', async () => {
    const at = Date.UTC(2026, 9, 30, 9);
    const old = info({ ...session('c0', 1), title: 'CB cutover' }, { open: false, rotation: 'done', reason: 'reset', journal: 'Before.', lastActivity: at });
    const fresh = info(live('c1', 'CB cutover'), { resetFrom: 'c0' });
    stubApi({ ...conversationRoutes, 'GET /api/pm': { current: fresh.session, sessions: [fresh, old] } satisfies PmState });
    const onSelect = vi.fn();
    const first = renderWithQuery(<PmView selection="c1" onSelect={onSelect} onNew={() => {}} />);
    const link = await screen.findByRole('button', { name: /^Previous conversation \(reset .*2026/ });
    await userEvent.click(link);
    expect(onSelect).toHaveBeenCalledWith('c0');
    first.unmount();
    renderWithQuery(<PmView selection="c0" onSelect={() => {}} onNew={() => {}} />);
    expect(await screen.findByText(/This conversation ended because you reset it\./)).toBeInTheDocument();
    expect(screen.queryByText(/Resetting:/)).not.toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Reset…' })).not.toBeInTheDocument();
  });
});
