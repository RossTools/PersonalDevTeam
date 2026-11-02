import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { PairedBrowsers } from './PairedBrowsers';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

describe('Settings › This host › Browsers (PDT-401)', () => {
  it('makes a pairing link on this origin, and unpairs every browser after asking', async () => {
    const calls = stubApi({
      'GET /api/ui/pairing': { paired: true, pairedBrowsers: 2 },
      'POST /api/ui/pair-links': { path: '/api/ui/pair?code=abc', expiresAt: 1 },
      'DELETE /api/ui/paired': undefined,
      'GET /api/host/settings': { remote: { active: true, url: 'https://box.tail.ts.net/' } },
    });
    renderWithQuery(<PairedBrowsers />);
    expect(await screen.findByText(/2 browsers are paired/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'New link' }));
    expect(await screen.findByRole('textbox', { name: 'Pairing link' })).toHaveValue(`${location.origin}/api/ui/pair?code=abc`);
    // The same code on the tailnet address, for a phone.
    expect(screen.getByRole('textbox', { name: 'Pairing link on the tailnet' })).toHaveValue('https://box.tail.ts.net/api/ui/pair?code=abc');

    await userEvent.click(screen.getByRole('button', { name: 'Unpair all…' }));
    await userEvent.click(await screen.findByRole('button', { name: 'Unpair all' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'DELETE' && c.path === '/api/ui/paired')).toBe(true));
  });
});
