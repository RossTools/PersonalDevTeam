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
    // The name says the state; it isn't also a pressed toggle (PDT-668).
    expect(eye).not.toHaveAttribute('aria-pressed');
    await userEvent.click(eye);
    expect(await screen.findByRole('button', { name: 'Watching' })).not.toHaveAttribute('aria-pressed');
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ ticket: 't1' });
    await userEvent.click(screen.getByRole('button', { name: 'Watching' }));
    expect(await screen.findByRole('button', { name: 'Watch' })).toBeInTheDocument();
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
    const line = await screen.findByRole('group', { name: 'Watched by' });
    expect(line).toHaveTextContent('Watched by you· PM “Docs”');
    expect(calls[0].path).toBe('/api/watches?run=r1');
    await userEvent.click(screen.getByRole('button', { name: 'Stop PM “Docs” watching' }));
    await waitFor(() => expect(screen.getByLabelText('Watched by')).not.toHaveTextContent('Docs'));
  });

  it('moves focus to the next watcher\'s remove, and from the last to the Watch toggle (PDT-668)', async () => {
    let watches = [watch({}), watch({ id: 'w2', sessionId: 's1', watcher: 'PM “Docs”' })];
    const gone = (id: string) => () => { watches = watches.filter((w) => w.id !== id); return undefined; };
    stubApi({ 'GET /api/watches': () => watches, 'DELETE /api/watches/w1': gone('w1'), 'DELETE /api/watches/w2': gone('w2') });
    renderWithQuery(<><WatchButton of={{ ticket: 't1' }} /><Watchers of={{ ticket: 't1' }} /></>);
    await userEvent.click(await screen.findByRole('button', { name: 'Stop watching' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Stop PM “Docs” watching' })).toHaveFocus());
    await userEvent.click(screen.getByRole('button', { name: 'Stop PM “Docs” watching' }));
    await waitFor(() => expect(screen.queryByRole('group', { name: 'Watched by' })).not.toBeInTheDocument());
    await waitFor(() => expect(screen.getByRole('button', { name: 'Watch' })).toHaveFocus());
  });
});
