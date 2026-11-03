import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import { StartAtLogin } from './StartAtLogin';

afterEach(() => vi.unstubAllGlobals());

describe('Start PDT at login', () => {
  it('turns starting at login off and on again', async () => {
    const calls = stubApi({
      'GET /api/host/start-at-login': { on: true, unavailable: null },
      'PUT /api/host/start-at-login': (c) => ({ on: (c.body as { on: boolean }).on, unavailable: null }),
    });
    renderWithQuery(<StartAtLogin />);
    const sw = await screen.findByRole('switch', { name: 'Start PDT at login' });
    expect(sw).toBeChecked();
    await userEvent.click(sw);
    await waitFor(() => expect(sw).not.toBeChecked());
    await userEvent.click(sw);
    await waitFor(() => expect(sw).toBeChecked());
    expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ on: false }, { on: true }]);
  });

  it('says why it cannot be changed here', async () => {
    stubApi({ 'GET /api/host/start-at-login': { on: false, unavailable: 'only the stable PDT starts at login' } });
    renderWithQuery(<StartAtLogin />);
    expect(await screen.findByRole('switch', { name: 'Start PDT at login' })).toBeDisabled();
    expect(screen.getByText("Can't be changed here: only the stable PDT starts at login.")).toBeInTheDocument();
  });
});
