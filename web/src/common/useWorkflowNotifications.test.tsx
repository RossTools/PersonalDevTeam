import { act, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { useWorkflowNotifications } from './useWorkflowNotifications';

function Follower({ onTicket }: { onTicket: (key: string) => void }) {
  useWorkflowNotifications(onTicket);
  return null;
}

const event = (seq: number, title: string, link: string | null = null) => ({
  seq, id: `e${seq}`, type: 'pdt.notified', key: `k${seq}`, projectId: null, ticketId: null, attemptId: null,
  sessionId: null, source: 'workflow', createdAt: 0, processedAt: null,
  payload: { title, body: 'details', level: 'info', link, workflowId: 'w', workflowName: 'Watchdog', runId: 'r' },
});

describe('workflow notifications', () => {
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it('shows notifications from after the page loaded, each once, and opens a linked ticket', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const after: string[] = [];
    stubApi({
      'GET /api/events': (c) => {
        if (c.path.includes('limit=1')) return [event(4, 'Old news')];
        const from = Number(new URL(c.path, 'http://x').searchParams.get('after'));
        after.push(String(from));
        return from < 5 ? [event(5, 'NAS is down', 'PDT-7')] : [];
      },
    });
    const onTicket = vi.fn();
    renderWithQuery(<Follower onTicket={onTicket} />);
    await act(async () => { await vi.advanceTimersByTimeAsync(2100); });
    expect(await screen.findByText('NAS is down')).toBeTruthy();
    expect(screen.getByText('details · From Watchdog')).toBeTruthy();
    expect(screen.queryByText('Old news')).toBeNull();
    await act(async () => { await vi.advanceTimersByTimeAsync(2100); });
    expect(screen.getAllByText('NAS is down')).toHaveLength(1);
    expect(after).toEqual(['4', '5']);
    await userEvent.click(screen.getByRole('button', { name: 'Open' }));
    expect(onTicket).toHaveBeenCalledWith('PDT-7');
  });
});
