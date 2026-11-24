import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { InboxItem } from '../api/gen/InboxItem';
import { Reply, renderWithQuery, stubApi } from '../test-utils';
import { InboxList } from './InboxList';
import { InboxView } from './InboxView';
import { tomorrowMorning } from './snooze';
import { useInbox } from './useInbox';

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
    // The item leaves the screen: a toast says where it went (PDT-667).
    expect(await screen.findByText(/^Snoozed until /)).toBeInTheDocument();
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

  it('leaves out its own ticket when that ticket waits on it, and says why a snooze was refused', async () => {
    const user = userEvent.setup();
    stubApi({
      'GET /api/inbox': { items: [], snoozed: [], total: 0 },
      'POST /api/inbox/q/snooze': new Reply(400, { code: 'invalid', message: 'PDT-7 is waiting on this' }),
    });
    renderWithQuery(<InboxView items={[item('q', { kind: 'question', group: 'question', ticketWaits: true })]} selectedId="q" onSelect={() => {}} />);
    await user.click(screen.getByRole('button', { name: 'Snooze' }));
    await user.click(screen.getByRole('menuitem', { name: 'Until a ticket is done…' }));
    const field = screen.getByRole('textbox', { name: 'Until a ticket is done…' });
    expect(field).toHaveValue('');
    await user.type(field, 'PDT-7{Enter}');
    expect(await screen.findByText(/PDT-7 is waiting on this/)).toBeInTheDocument();
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
    expect(section).not.toHaveTextContent('waits on it');
    await user.click(screen.getByRole('button', { name: 'Unsnooze' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/inbox/g/snooze')).toBe(true));
  });
});

it('says when a snoozed item holds its ticket', async () => {
  const held = item('q', { kind: 'question', ticketWaits: true, snooze: { itemId: 'q', until: 1e15, ticketId: null, ticketKey: null, createdAt: 1 } });
  stubApi({ 'GET /api/inbox': { items: [], snoozed: [held], total: 0 } });
  renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
  expect(await screen.findByRole('region', { name: 'Snoozed' })).toHaveTextContent('PDT-7 waits on it');
});

describe('focus in Snoozed (PDT-668)', () => {
  const away = (id: string) => item(id, { snooze: { itemId: id, until: 1e15, ticketId: null, ticketKey: null, createdAt: 1 } });
  const row = (title: string) => screen.getByText(title).closest('[role="row"]');

  /** The Inbox list on the server's items, as the app has it: an unsnoozed item comes back to Needs you. */
  function Live() {
    const data = useInbox().data;
    return <InboxList items={data?.items ?? []} selectedId={null} onSelect={() => {}} />;
  }

  it('Show all moves focus to the first row it shows', async () => {
    const user = userEvent.setup();
    stubApi({ 'GET /api/inbox': { items: [], snoozed: ['a', 'b', 'c', 'd', 'e'].map(away), total: 0 } });
    renderWithQuery(<InboxList items={[]} selectedId={null} onSelect={() => {}} />);
    const section = await screen.findByRole('region', { name: 'Snoozed' });
    within(section).getByRole('button', { name: 'Show all 5' }).focus();
    await user.keyboard('{Enter}');
    await waitFor(() => expect(document.activeElement).toBe(row('Item d')));
    expect(within(section).queryByRole('button', { name: /^Show all/ })).not.toBeInTheDocument();
  });

  it('Unsnooze moves focus to the next row, and from the last to the item back in Needs you', async () => {
    const user = userEvent.setup();
    let snoozed = [away('a'), away('b')];
    let items: InboxItem[] = [];
    const back = (id: string) => () => {
      items = [...items, ...snoozed.filter((i) => i.id === id).map((i) => ({ ...i, snooze: undefined }))];
      snoozed = snoozed.filter((i) => i.id !== id);
      return undefined;
    };
    stubApi({
      'GET /api/inbox': () => ({ items, snoozed, total: items.length }),
      'DELETE /api/inbox/a/snooze': back('a'),
      'DELETE /api/inbox/b/snooze': back('b'),
    });
    renderWithQuery(<Live />);
    const section = await screen.findByRole('region', { name: 'Snoozed' });
    await user.click(within(row('Item a') as HTMLElement).getByRole('button', { name: 'Unsnooze' }));
    await waitFor(() => expect(document.activeElement).toBe(row('Item b')));
    await user.click(within(section).getByRole('button', { name: 'Unsnooze' }));
    await waitFor(() => expect(screen.queryByRole('region', { name: 'Snoozed' })).not.toBeInTheDocument());
    await waitFor(() => expect(document.activeElement).toBe(row('Item b')));
    expect(screen.getByRole('region', { name: 'Ready to accept' })).toContainElement(document.activeElement as HTMLElement);
  });
});
