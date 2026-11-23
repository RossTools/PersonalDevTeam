import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { WatchInfo } from '../api/gen/WatchInfo';
import { renderWithQuery, stubApi } from '../test-utils';
import { WatchButton, Watchers } from './Watch';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const watch = (over: Partial<WatchInfo>): WatchInfo => ({
  id: 'w1', target: 'ticket', targetId: 't1', targetLabel: 'ABC-12', targetTitle: 'Fix it', targetState: 'todo', workflowId: null, sessionId: null, watcher: 'you',
  events: ['finished', 'failed', 'needs_you'], once: true, createdAt: 1, ...over,
});

describe('watches', () => {
  it('Watch starts and stops the user’s watch, and reads Watching while on', async () => {
    let watches: WatchInfo[] = [];
    const calls = stubApi({
      'GET /api/watches': () => watches,
      'POST /api/watches': () => { watches = [watch({})]; return watches[0]; },
      'DELETE /api/watches/w1': () => { watches = []; return undefined; },
    });
    renderWithQuery(<WatchButton of={{ ticket: 't1' }} />);
    const eye = await screen.findByRole('button', { name: 'Watch' });
    expect(eye).toHaveAttribute('aria-pressed', 'false');
    await userEvent.click(eye);
    await waitFor(() => expect(screen.getByRole('button', { name: 'Watching' })).toHaveAttribute('aria-pressed', 'true'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ ticket: 't1' });
    await userEvent.click(screen.getByRole('button', { name: 'Watching' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Watch' })).toHaveAttribute('aria-pressed', 'false'));
  });

  it('watches a document', async () => {
    const calls = stubApi({
      'GET /api/watches': () => [],
      'POST /api/watches': () => watch({ target: 'document', targetId: 'd1', targetLabel: 'Plan', events: ['changed', 'comment'], once: false }),
    });
    renderWithQuery(<WatchButton of={{ document: 'd1' }} />);
    await userEvent.click(await screen.findByRole('button', { name: 'Watch' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ document: 'd1' }));
    expect(calls[0].path).toBe('/api/watches?document=d1');
  });

  it('lists who watches, and removes a PM’s watch', async () => {
    let watches = [watch({}), watch({ id: 'w2', sessionId: 's1', watcher: 'PM “Docs”' })];
    const calls = stubApi({
      'GET /api/watches': () => watches,
      'DELETE /api/watches/w2': () => { watches = watches.filter((w) => w.id !== 'w2'); return undefined; },
    });
    renderWithQuery(<Watchers of={{ run: 'r1' }} />);
    const line = await screen.findByLabelText('Watched by');
    expect(line).toHaveTextContent('Watched by you· PM “Docs”');
    expect(calls[0].path).toBe('/api/watches?run=r1');
    await userEvent.click(screen.getByRole('button', { name: 'Stop PM “Docs” watching' }));
    await waitFor(() => expect(screen.getByLabelText('Watched by')).not.toHaveTextContent('Docs'));
  });
});
