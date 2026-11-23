import { act, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import { FakeSocket } from '../conversation/fakeSocket';
import { renderWithQuery, stubApi } from '../test-utils';
import { OpenSessionContext, OpenTicketContext, StartTicketContext } from './actions';
import { project, summary, ticket } from './fixtures';
import { TicketConversation } from './TicketConversation';

afterEach(() => {
  FakeSocket.all = [];
  vi.unstubAllGlobals();
});

const session = (id: string, over: Partial<Session> = {}): Session => ({
  id, projectId: project.id, worktreeId: 'w', kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: `Session ${id}`,
  cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'idle', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: 'a1', ticket: { id: 't1', key: 'ALP-1' },
  mode: 'conversation', model: null, permissionMode: 'ask', persona: null, background: [], instance: null, ...over,
});

const row = (s: Session) => ({ id: s.id, title: s.title, status: s.status, agent: s.agent, attemptId: 'a1', persona: null, mode: s.mode });

const caps = { conversation: true, permissionModes: ['ask'], models: [], commands: [] };

function render(sessions: Session[], props: { onStart?: () => void; onOpen?: () => void; onOpenTicket?: () => void; over?: Partial<Ticket> } = {}) {
  const calls = stubApi({
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/sessions`]: sessions,
    'GET /api/agents/claude/capabilities': caps,
    'GET /api/agents/codex/capabilities': caps,
    'POST /api/attempts/a1/session': session('s9'),
  });
  const t = ticket(1, { stage: 'ready', sessions: sessions.map(row), ...props.over });
  renderWithQuery(
    <StartTicketContext.Provider value={props.onStart ?? vi.fn()}>
      <OpenSessionContext.Provider value={props.onOpen ?? vi.fn()}>
        <OpenTicketContext.Provider value={props.onOpenTicket ?? vi.fn()}>
          <TicketConversation ticket={t} socketFactory={(u) => new FakeSocket(u)} />
        </OpenTicketContext.Provider>
      </OpenSessionContext.Provider>
    </StartTicketContext.Provider>,
  );
  return calls;
}

const attempt = (over: Partial<AttemptSummary> = {}): AttemptSummary => ({
  id: 'a1', branch: 'feature/login-form', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null, pr: null, ...over,
});

const server = () => FakeSocket.all[FakeSocket.all.length - 1];

describe('ticket Conversation', () => {
  it('offers Start when there is no session yet', async () => {
    const onStart = vi.fn();
    render([], { onStart });
    expect(screen.getByText('No agent has worked on this yet.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Start' }));
    expect(onStart).toHaveBeenCalled();
  });

  it('offers an agent on the branch of an attempt with none at work, or Abandon', async () => {
    const calls = render([], { over: { stage: 'in_progress', attempt: attempt(), attempts: [attempt()] } });
    expect(screen.getByText(/No agent is working on/)).toHaveTextContent('No agent is working on feature/login-form.');
    expect(screen.queryByRole('button', { name: 'Start' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Abandon…' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: /Start an agent here/ }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST' && c.path === '/api/attempts/a1/session')?.body)
      .toEqual({ agent: 'claude', sessionMode: 'conversation', permissionMode: 'ask' }));
  });

  it('points a parent at its parts', async () => {
    const onOpenTicket = vi.fn();
    const parts = [summary(2, { title: 'First part', stage: 'done' }), summary(3, { title: 'Second part' })];
    render([], { onOpenTicket, over: { stage: 'in_progress', partList: parts } });
    expect(screen.getByText('Each part has its own conversation.')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Start' })).toBeNull();
    expect(screen.getByRole('button', { name: /Start next part/ })).toBeInTheDocument();
    await userEvent.click(within(screen.getByRole('list', { name: 'Parts' })).getByText('ALP-3'));
    expect(onOpenTicket).toHaveBeenCalledWith({ id: 't3', projectId: project.id });
  });

  it('offers nothing on a closed ticket', () => {
    render([], { over: { stage: 'done', attempt: attempt({ state: 'merged' }) } });
    expect(screen.getByText('No agent worked on this ticket.')).toBeInTheDocument();
    expect(screen.queryByRole('button')).toBeNull();
  });

  it('shows the newest conversation and switches between sessions', async () => {
    const newest = session('s2', { title: 'Coder', agent: 'codex' });
    const terminal = session('s3', { title: 'Old terminal', mode: 'terminal' });
    const older = session('s1', { title: 'First try' });
    const onOpen = vi.fn();
    render([terminal, newest, older], { onOpen });
    const group = await screen.findByRole('group', { name: 'Sessions' });
    // The first conversation in the ticket's (newest first) list is shown.
    expect(await within(group).findByRole('button', { name: /Coder/ })).toHaveAttribute('aria-pressed', 'true');
    await waitFor(() => expect(server()?.url).toMatch(/\/api\/sessions\/s2\/events/));
    act(() => {
      server().open();
      server().send({
        type: 'event',
        event: { seq: 1, pos: 1, ts: 1, itemId: 'i1', turnId: 'u1', event: { kind: 'user_message', text: 'ALP-1: Fix it', images: [], source: 'pdt' } },
      });
      server().send({ type: 'ready' });
    });
    expect(screen.getByText('ALP-1: Fix it')).toBeInTheDocument();
    await userEvent.click(within(group).getByRole('button', { name: /First try/ }));
    await waitFor(() => expect(server().url).toMatch(/\/api\/sessions\/s1\/events/));
    // A terminal session is opened in its own tab.
    await userEvent.click(within(group).getByRole('button', { name: /Old terminal/ }));
    expect(screen.getByText(/runs in the agent's terminal/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Open its tab' }));
    expect(onOpen).toHaveBeenCalledWith({ id: 's3', projectId: project.id });
  });
});
