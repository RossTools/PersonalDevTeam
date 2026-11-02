import { act, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import { ticket } from '../tickets/fixtures';
import { renderWithQuery, stubApi } from '../test-utils';
import { TicketPill } from './TicketPill';

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const attempt: AttemptSummary = {
  id: 'a1', branch: 'pdt-ALP-19-save-the-cart', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null,
  pr: {
    number: 7, url: 'https://github.com/acme/shop/pull/7', title: 'Save the cart', state: 'open', draft: false, baseBranch: 'main', headSha: 'abc',
    mergeable: 'mergeable', behind: false, autoMerge: false, inMergeQueue: false, queuePosition: null,
    ci: { state: 'failed', url: 'https://ci', runs: [] }, queueCi: null, checkedAt: 1, error: null, local: false, mergeHeld: false,
  },
};

describe('TicketPill', () => {
  it('links to the ticket and fetches its card only when hovered, once', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    const calls = stubApi({
      'GET /api/tickets/ALP-19': ticket(19, {
        title: 'Save the cart across visits', stage: 'in_progress', priority: 'medium', labels: ['agentic'], attempt,
        agent: { persona: 'Coder', state: 'stalled', sessionId: 's', busy: false, runId: 'r', background: [] },
      }),
    });
    renderWithQuery(<TicketPill ticketKey="ALP-19" />);
    const pill = screen.getByRole('link', { name: 'ALP-19' });
    expect(pill).toHaveAttribute('href', '/tickets/ALP-19');
    expect(calls).toHaveLength(0);

    await user.click(document.body);
    await user.hover(pill);
    act(() => vi.advanceTimersByTime(500));
    const card = await screen.findByRole('tooltip');
    expect(await screen.findByText('Save the cart across visits')).toBeInTheDocument();
    for (const text of ['In progress', 'Medium', 'agentic', 'Coder stalled', 'pdt-ALP-19-save-the-cart', 'PR #7', 'CI failed']) {
      expect(card).toHaveTextContent(text);
    }
    expect(calls.map((c) => c.path)).toEqual(['/api/tickets/ALP-19']);

    await user.unhover(pill);
    act(() => vi.advanceTimersByTime(500));
    await user.hover(pill);
    act(() => vi.advanceTimersByTime(500));
    expect(await screen.findByText('Save the cart across visits')).toBeInTheDocument();
    expect(calls).toHaveLength(1);
  });

  it('says so when no ticket has the key', async () => {
    stubApi({});
    renderWithQuery(<TicketPill ticketKey="ALP-404" />);
    await userEvent.tab();
    expect(await screen.findByText('No ticket ALP-404')).toBeInTheDocument();
  });
});
