import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, Reply, stubApi, type Call } from '../test-utils';
import { project, ticket } from './fixtures';
import { MakeTicketDialog } from './MakeTicketDialog';

afterEach(() => vi.unstubAllGlobals());

const chat: Session = {
  id: 's1', projectId: project.id, worktreeId: 'w1', kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Chat',
  cwd: '/r', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'idle', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: null, background: [],
};

const draft = { title: 'Fix CSV export', description: '## Goal\nKeep the last row.' };

function stub(draftReply: Parameters<typeof stubApi>[0][string] = draft): Call[] {
  return stubApi({
    'GET /api/projects': [project],
    [`GET /api/projects/${project.id}/labels`]: [],
    [`GET /api/projects/${project.id}/areas`]: { areas: [], error: null },
    'POST /api/sessions/s1/ticket-draft': draftReply,
    [`POST /api/projects/${project.id}/tickets`]: (c: Call) => ticket(7, { ...(c.body as object) }),
    'POST /api/tickets/t7/start': { ticket: ticket(7), attempt: {}, session: chat },
  });
}

const created = (calls: Call[]) => calls.filter((c) => c.method === 'POST' && c.path.endsWith('/tickets'));

describe('Make ticket', () => {
  it('drafts from the chat, then creates the ticket and can start it from the chat', async () => {
    const calls = stub();
    const onCreated = vi.fn();
    const onClose = vi.fn();
    renderWithQuery(<MakeTicketDialog session={chat} canStartHere onClose={onClose} onCreated={onCreated} />);
    expect(screen.getByText('Drafting a ticket from this chat…')).toBeInTheDocument();
    // The form replaces the drafting dialog.
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Title' })).toHaveValue('Fix CSV export'));
    const form = screen.getByRole('dialog', { name: 'Make ticket' });
    expect(within(form).getByRole('textbox', { name: 'Description' })).toHaveValue('## Goal\nKeep the last row.');
    const start = within(form).getByRole('checkbox', { name: 'Start from this chat' });
    expect(start).not.toBeChecked();
    await userEvent.click(start);
    await userEvent.click(within(form).getByRole('button', { name: 'Create ticket' }));
    await waitFor(() => expect(onCreated).toHaveBeenCalled());
    expect(created(calls)[0].body).toMatchObject({ title: 'Fix CSV export', description: '## Goal\nKeep the last row.' });
    expect(calls.find((c) => c.path === '/api/tickets/t7/start')?.body).toEqual({ agent: 'claude', fromSession: 's1', mode: 'force' });
    expect(onClose).toHaveBeenCalled();
  });

  it('only creates the ticket unless asked, and offers no start for a chat on the main checkout', async () => {
    const calls = stub();
    renderWithQuery(<MakeTicketDialog session={{ ...chat, worktreeId: null }} canStartHere={false} onClose={vi.fn()} />);
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Title' })).toHaveValue('Fix CSV export'));
    const form = screen.getByRole('dialog', { name: 'Make ticket' });
    expect(within(form).queryByRole('checkbox', { name: 'Start from this chat' })).toBeNull();
    await userEvent.click(within(form).getByRole('button', { name: 'Create ticket' }));
    await waitFor(() => expect(created(calls)).toHaveLength(1));
    expect(calls.some((c) => c.path.endsWith('/start'))).toBe(false);
  });

  it('says why there is no draft, and lets the user write it or try again', async () => {
    let n = 0;
    stub(() => (++n === 1 ? new Reply(409, { code: 'nothing_to_draft', message: 'the agent has no conversation yet' }) : draft));
    renderWithQuery(<MakeTicketDialog session={chat} canStartHere onClose={vi.fn()} />);
    expect(await screen.findByText('the agent has no conversation yet')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(screen.getByRole('textbox', { name: 'Title' })).toHaveValue('Fix CSV export'));
  });

  it('opens an empty form when the user writes it', async () => {
    stub(new Reply(502, { code: 'agent_error', message: 'the agent refused' }));
    renderWithQuery(<MakeTicketDialog session={chat} canStartHere onClose={vi.fn()} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Write it myself' }));
    expect(await screen.findByRole('textbox', { name: 'Title' })).toHaveValue('');
  });
});
