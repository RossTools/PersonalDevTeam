import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PmSessionInfo } from '../api/gen/PmSessionInfo';
import type { PmState } from '../api/gen/PmState';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi } from '../test-utils';
import { PmList } from './PmList';
import { PmView } from './PmView';
import { firstLine } from './usePm';

afterEach(() => vi.unstubAllGlobals());

const session = (id: string, createdAt: number): Session => ({
  id, projectId: 'host', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PM',
  cwd: '/data/pm', status: 'finished', exitCode: null, createdAt, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: { id: 'pm', name: 'PM', version: 1, stepId: null },
});

const info = (s: Session, more: Partial<PmSessionInfo> = {}): PmSessionInfo => ({
  session: s, open: true, title: s.title, unseen: s.unseen, lastActivity: s.createdAt, rotation: 'none', reason: null, journal: null, ...more,
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
});
