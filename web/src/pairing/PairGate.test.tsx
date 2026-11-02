import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { PairGate } from './PairGate';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('PairGate (PDT-401)', () => {
  it('shows the app to a paired browser, and while pairing is unknown (an older PDT)', async () => {
    stubApi({ 'GET /api/ui/pairing': { paired: true, pairedBrowsers: 1 } });
    renderWithQuery(<PairGate><p>the app</p></PairGate>);
    expect(await screen.findByText('the app')).toBeInTheDocument();
    vi.unstubAllGlobals();
    stubApi({});
    renderWithQuery(<PairGate><p>older</p></PairGate>);
    expect(await screen.findByText('older')).toBeInTheDocument();
  });

  it('asks an unpaired browser to pair, and PDT opens itself paired', async () => {
    const calls = stubApi({
      'GET /api/ui/pairing': { paired: false, pairedBrowsers: 0 },
      'GET /api/host/whoami': { caller: 'local', login: null },
      'POST /api/ui/open': undefined,
    });
    renderWithQuery(<PairGate><p>the app</p></PairGate>);
    await userEvent.click(await screen.findByRole('button', { name: 'Pair this browser' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/ui/open')).toBe(true));
    expect(await screen.findByText(/PDT opened itself, paired/)).toBeInTheDocument();
    expect(screen.queryByText('the app')).not.toBeInTheDocument();
  });

  it('offers no Pair button over the tailnet: it would open a browser on the host', async () => {
    stubApi({
      'GET /api/ui/pairing': { paired: false, pairedBrowsers: 1 },
      'GET /api/host/whoami': { caller: 'tailnet', login: 'me@example.com' },
    });
    renderWithQuery(<PairGate><p>the app</p></PairGate>);
    expect(await screen.findByText(/Pair another browser/)).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Pair this browser' })).not.toBeInTheDocument());
  });
});
