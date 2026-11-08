import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import type { AgentEvent } from '../api/gen/AgentEvent';
import type { ConversationEvent } from '../api/gen/ConversationEvent';
import { requestAnchor } from './MessageList';
import { PinnedApprovalBar } from './PinnedApprovalBar';

const ev = (seq: number, event: AgentEvent): ConversationEvent => ({ seq, pos: seq, ts: seq, event, itemId: `i${seq}` });

const request = (seq: number, id: string, over: Partial<Extract<AgentEvent, { kind: 'request' }>> = {}) =>
  ev(seq, {
    kind: 'request', id, requestKind: 'approval', tool: 'mcp__pdt__create_ticket', title: 'x', input: {}, canAlways: false, state: 'open',
    ...over,
  } as AgentEvent);

const bar = () => screen.queryByRole('region', { name: 'Open request' });

describe('PinnedApprovalBar', () => {
  it('says what each kind of request asks for', () => {
    const { rerender } = render(<PinnedApprovalBar rows={[request(1, 'r', { requestKind: 'question', tool: 'AskUserQuestion' })]} />);
    expect(bar()).toHaveTextContent('has a question');
    rerender(<PinnedApprovalBar rows={[request(1, 'r', { requestKind: 'plan', tool: 'ExitPlanMode' })]} />);
    expect(bar()).toHaveTextContent('wants its plan approved');
    // Any other tool by its short name: an MCP server's prefix dropped, a built-in tool as it is.
    rerender(<PinnedApprovalBar rows={[request(1, 'r', { tool: 'mcp__github__merge_pr' })]} />);
    expect(bar()).toHaveTextContent('wants merge_pr');
    rerender(<PinnedApprovalBar rows={[request(1, 'r', { tool: 'Bash' })]} />);
    expect(bar()).toHaveTextContent('wants Bash');
  });

  it('shows the oldest open request and counts the rest; nothing once none is open', () => {
    const rows = [
      request(1, 'done', { tool: 'Write', state: 'answered', answer: { type: 'approve', always: false } } as never),
      ev(2, { kind: 'user_message', text: 'go', images: [], source: 'pdt' }),
      request(3, 'a', { tool: 'Bash' }),
      request(4, 'b', { tool: 'Edit' }),
      request(5, 'c', { requestKind: 'question' }),
    ];
    const { rerender } = render(<PinnedApprovalBar rows={rows} />);
    expect(bar()).toHaveTextContent('wants Bash · 2 more');
    rerender(<PinnedApprovalBar rows={rows.slice(0, 4).filter((r) => r.seq !== 3)} />);
    expect(bar()).toHaveTextContent('wants Edit');
    expect(bar()).not.toHaveTextContent('more');
    rerender(<PinnedApprovalBar rows={rows.slice(0, 2)} />);
    expect(bar()).toBeNull();
  });

  it('Review does nothing when the request has no card on screen', async () => {
    render(<PinnedApprovalBar rows={[request(1, 'gone')]} />);
    await userEvent.click(screen.getByRole('button', { name: 'Review' }));
    expect(document.getElementById(requestAnchor('gone'))).toBeNull();
    expect(bar()).toBeInTheDocument();
  });
});
