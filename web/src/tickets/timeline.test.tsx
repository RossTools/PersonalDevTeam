import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { event } from './fixtures';
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
});
