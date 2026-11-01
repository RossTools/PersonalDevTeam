import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { event } from './fixtures';
import { OpenSessionContext } from './actions';
import { TicketTimeline } from './TicketParts';

afterEach(() => vi.unstubAllGlobals());

describe('TicketTimeline', () => {
  it('keeps the review comments a feedback event sent, folded', async () => {
    stubApi({
      'GET /api/tickets/t1/events': {
        items: [event('e1', { kind: 'feedback', body: '**`a.txt:2`**\n\n```\ntwo\n```\n\nMake it TWO.', data: { count: 1 } })],
        nextCursor: null,
      },
    });
    renderWithQuery(<TicketTimeline ticketId="t1" />);
    expect(await screen.findByText('Sent 1 review comment to the agent')).toBeInTheDocument();
    const show = screen.getByText('Show the comments');
    expect(show.closest('details')).not.toHaveAttribute('open');
    await userEvent.click(show);
    expect(screen.getByText('Make it TWO.')).toBeVisible();
    expect(screen.getByText('a.txt:2')).toBeInTheDocument();
  });

  it("shows an agent step's outcome with its summary and workflow, by the persona", async () => {
    stubApi({
      'GET /api/tickets/t1/events': {
        items: [
          event('e2', { kind: 'comment', actor: 'agent', body: 'Which login page?', data: { persona: 'Refiner' } }),
          event('e1', {
            kind: 'outcome',
            actor: 'agent',
            data: { outcome: 'ready', summary: 'Scoped to the **login** form.', persona: 'Refiner', personaVersion: 2, by: 'agent', sessionId: 's1', workflowName: 'Pipeline', runId: 'r1' },
          }),
        ],
        nextCursor: null,
      },
    });
    renderWithQuery(<TicketTimeline ticketId="t1" />);
    expect(await screen.findByText('Refiner: ready')).toBeInTheDocument();
    expect(screen.getByText('login')).toBeInTheDocument();
    expect(screen.getByText('from workflow Pipeline')).toBeInTheDocument();
    expect(screen.getByText('Which login page?')).toBeInTheDocument();
    expect(screen.getAllByText(/^Refiner ·/)).toHaveLength(2);
    expect(screen.queryByText(/^You ·/)).toBeNull();
  });

  it("names an agent's edits and comments by its persona and ticket, opening its session", async () => {
    const agent = { sessionId: 's9', projectId: 'p1', persona: 'Coder', fromTicket: 'CB-34', fromTicketId: 't34' };
    stubApi({
      'GET /api/tickets/t1/events': {
        items: [
          event('e3', { kind: 'comment', actor: 'agent', body: 'CB-34 needs this.', agent, createdAt: 3 }),
          event('e2', { kind: 'edited', actor: 'agent', data: { fields: ['title', 'description'], agent }, agent, createdAt: 2 }),
          // Before PDT-163: only the persona.
          event('e1', { kind: 'comment', actor: 'agent', body: 'Old one.', data: { persona: 'Refiner' }, createdAt: 1 }),
        ],
        nextCursor: null,
      },
    });
    const onSession = vi.fn();
    renderWithQuery(
      <OpenSessionContext.Provider value={onSession}>
        <TicketTimeline ticketId="t1" />
      </OpenSessionContext.Provider>,
    );
    expect(await screen.findByText('Edited title, description')).toBeInTheDocument();
    const labels = screen.getAllByRole('button', { name: 'Coder · CB-34' });
    expect(labels).toHaveLength(2);
    await userEvent.click(labels[0]!);
    expect(onSession).toHaveBeenCalledWith({ id: 's9', projectId: 'p1' });
    expect(screen.getByText(/^Refiner ·/)).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Refiner/ })).toBeNull();
  });

  it('says which ticket an agent was working on when it made this one', async () => {
    const agent = { sessionId: 's9', projectId: 'p1', persona: 'Coder', fromTicket: 'PDT-345', fromTicketId: 't345' };
    stubApi({
      'GET /api/tickets/t1/events': {
        items: [event('e1', { kind: 'created', actor: 'agent', data: { stage: 'draft', agent }, agent })],
        nextCursor: null,
      },
    });
    renderWithQuery(<TicketTimeline ticketId="t1" />);
    expect(await screen.findByRole('button', { name: 'Coder · from PDT-345' })).toBeInTheDocument();
  });
});
