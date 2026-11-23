import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import { renderWithQuery, stubApi } from '../test-utils';
import { InboxList } from './InboxList';
import { InboxView } from './InboxView';
import { tomorrowMorning } from './snooze';

afterEach(() => vi.unstubAllGlobals());

const item = (id: string, over: Partial<InboxItem> = {}): InboxItem => ({
  id, kind: 'gate', group: 'gate', status: 'open', projectId: 'p', ticketId: 't', ticketKey: 'PDT-7', ticketTitle: 'Pipeline',
  attemptId: null, sessionId: null, runId: 'r', stepId: 's', personaId: null, personaName: null, changeRequest: null, title: `Item ${id}`,
  body: {}, choices: [{ id: 'ship', title: 'ship', primary: true }], resolution: null, createdAt: Date.now(), resolvedAt: null, ...over,
});

describe('snooze (PDT-641)', () => {
  it('ends tomorrow at 9:00 local time', () => {
    const t = new Date(tomorrowMorning(new Date(2026, 10, 22, 23, 30).getTime()));
    expect([t.getFullYear(), t.getMonth(), t.getDate(), t.getHours(), t.getMinutes()]).toEqual([2026, 10, 23, 9, 0]);
  });

  it('snoozes an item for a week, or until its ticket (or another) is done', async () => {
    const user = userEvent.setup();
    const calls = stubApi({ 'GET /api/inbox': { items: [], snoozed: [], total: 0 }, 'POST /api/inbox/g/snooze': undefined });
    renderWithQuery(<InboxView items={[item('g')]} selectedId="g" onSelect={() => {}} />);
    const snoozes = () => calls.filter((c) => c.method === 'POST' && c.path === '/api/inbox/g/snooze').map((c) => c.body as { until?: number; ticket?: string });

    await user.click(screen.getByRole('button', { name: 'Snooze' }));
    const before = Date.now();
    await user.click(screen.getByRole('menuitem', { name: 'For 1 week' }));
    await waitFor(() => expect(snoozes()).toHaveLength(1));
    const week = snoozes()[0].until!;
    expect(week - before).toBeGreaterThanOrEqual(7 * 24 * 3_600_000 - 1000);
    expect(week - before).toBeLessThan(7 * 24 * 3_600_000 + 5000);

    await user.click(screen.getByRole('button', { name: 'Snooze' }));
    await user.click(screen.getByRole('menuitem', { name: 'Until a ticket is done…' }));
    const field = screen.getByRole('textbox', { name: 'Until a ticket is done…' });
    expect(field).toHaveValue('PDT-7');
    await user.keyboard('{Enter}');
    await waitFor(() => expect(snoozes()[1]).toEqual({ ticket: 'PDT-7' }));
  });

  it('lists the snoozed items with until when, and Unsnooze brings one back', async () => {
    const user = userEvent.setup();
    const away = item('g', {
      title: 'Merge #633?',
      snooze: { itemId: 'g', until: null, ticketId: 't9', ticketKey: 'PDT-639', createdAt: 1 },
    });
    const calls = stubApi({ 'GET /api/inbox': { items: [], snoozed: [away], total: 0 }, 'DELETE /api/inbox/g/snooze': undefined });
    renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
    const section = await screen.findByRole('region', { name: 'Snoozed' });
    expect(section).toHaveTextContent('Merge #633?');
    expect(section).toHaveTextContent('until PDT-639 is done');
    await user.click(screen.getByRole('button', { name: 'Unsnooze' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/inbox/g/snooze')).toBe(true));
  });
});
