import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { NotificationList } from '../api/gen/NotificationList';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { notification } from './fixtures';
import { useClearAllUpdates, useClearUpdate, useUpdates } from './queries';

function Updates() {
  const updates = useUpdates();
  const clear = useClearUpdate();
  const clearAll = useClearAllUpdates();
  return (
    <>
      <ul>{updates.data?.items.map((u) => <li key={u.id}>{u.title}</li>)}</ul>
      <button onClick={() => clear.mutate('a')}>Clear a</button>
      <button onClick={() => clearAll.mutate()}>Clear all</button>
    </>
  );
}

const both: NotificationList = { items: [notification('a'), notification('b')], unreadUpdates: 2 };

/** A route that answers each call when the test says (`next` holds the reply for the latest one). */
function held() {
  const waiting: ((v: unknown) => void)[] = [];
  return {
    route: () => new Promise((resolve) => waiting.push(resolve)),
    answer: (v: unknown) => waiting.splice(0).forEach((r) => r(v)),
    get pending() { return waiting.length; },
  };
}

afterEach(() => vi.unstubAllGlobals());

describe('clearing Updates (PDT-451)', () => {
  it('a poll in flight when one is cleared does not bring it back', async () => {
    const list = held();
    const clear = held();
    stubApi({ 'GET /api/notifications': list.route, 'POST /api/notifications/a/clear': clear.route });
    const { qc } = renderWithQuery(<Updates />);
    list.answer(both);
    await screen.findByText('Update a');

    // A poll (or the stream's refresh) starts before the user clears it and answers with the old list after.
    void qc.invalidateQueries({ queryKey: ['notifications', 'updates'] });
    await waitFor(() => expect(list.pending).toBe(1));
    fireEvent.click(screen.getByRole('button', { name: 'Clear a' }));
    await waitFor(() => expect(screen.queryByText('Update a')).toBeNull());
    await act(async () => list.answer(both));
    expect(screen.queryByText('Update a')).toBeNull();
    expect(screen.getByText('Update b')).toBeTruthy();

    // Once cleared, the refetch has it gone.
    await act(async () => clear.answer(undefined));
    await waitFor(() => expect(list.pending).toBe(1));
    await act(async () => list.answer({ items: [notification('b')], unreadUpdates: 1 }));
    expect(screen.queryByText('Update a')).toBeNull();
  });

  it('puts the Updates back when clearing fails', async () => {
    const clear = held();
    stubApi({ 'GET /api/notifications': both, 'POST /api/notifications/clear-all': clear.route });
    renderWithQuery(<Updates />);
    await screen.findByText('Update a');

    fireEvent.click(screen.getByRole('button', { name: 'Clear all' }));
    await waitFor(() => expect(screen.queryByText('Update a')).toBeNull());
    await act(async () => clear.answer(new Reply(500, { code: 'internal', message: 'disk full' })));
    await screen.findByText('Update a');
    expect(screen.getByText('Update b')).toBeTruthy();
    await screen.findByText(/Couldn't clear the updates/);
  });
});
