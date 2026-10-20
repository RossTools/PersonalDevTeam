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
});
