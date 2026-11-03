import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { MessageList, type MessageListProps } from './MessageList';
import { fullText } from '../test-utils';

const DAY = 24 * 3600 * 1000;

const said = (seq: number, event: Extract<AgentEvent, { kind: 'user_message' }>, ts = Date.now()): ConversationEvent => ({
  seq, pos: seq, ts, itemId: `u:${seq}`, turnId: `t${seq}`, event,
});

const props = (over: Partial<MessageListProps>): MessageListProps => ({
  rows: [], provisional: [], streams: {}, turnRunning: false, pending: [], onAnswer: async () => {}, ...over,
});

describe("PDT's own messages to a PM conversation (PDT-211)", () => {
  it('show as one muted line that opens to the full text', async () => {
    const update = '[PDT] Since your briefing (1 d ago): what changed.\n\n## Tickets\n\n- PDT-208 Phone Inbox: PR #208 merged';
    const rows = [
      said(1, { kind: 'user_message', text: 'What is in flight?', images: [], source: 'pdt' }),
      said(2, { kind: 'user_message', text: update, images: [], source: 'notice', pdt: 'update' }, Date.now() - DAY),
      said(3, { kind: 'user_message', text: '[PDT] Your context was just compacted.', images: [], source: 'notice', pdt: 'checkpoint' }),
    ];
    render(<MessageList {...props({ rows })} />);
    expect(screen.getByText('What is in flight?')).toBeInTheDocument();
    const line = screen.getByText(fullText('PDT brought this conversation up to date · 1d ago'));
    expect(screen.getByText(fullText('PDT asked for a journal checkpoint · just now'))).toBeInTheDocument();
    const trigger = screen.getByRole('button', { name: 'PDT brought this conversation up to date · 1d ago' });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    // Not a user bubble labelled as news about a pull request.
    expect(screen.queryByText(/news about the pull request/)).not.toBeInTheDocument();
    await userEvent.click(line);
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText(/PDT-208 Phone Inbox: PR #208 merged/)).toBeVisible();
  });

  it('collapse while queued too', () => {
    const pending = [{ id: 'p1', text: '[PDT] The user closed this PM conversation.', delivery: 'next' as const, images: [], notice: true, pdt: 'journal' as const }];
    render(<MessageList {...props({ pending })} />);
    expect(screen.getByText("PDT asked for this conversation's journal entry · queued")).toBeInTheDocument();
    expect(screen.queryByText('Queued')).not.toBeInTheDocument();
  });
});
