import { screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KindSetting } from '../api/gen/KindSetting';
import { renderWithQuery, stubApi } from '../test-utils';
import { NotificationSettingsPage } from './NotificationSettingsPage';

afterEach(() => vi.unstubAllGlobals());

const kinds: KindSetting[] = [
  { kind: 'session.needs_you', section: 'needs_you', label: 'Session waiting', when: 'A session waits.', show: 'desktop', defaultShow: 'desktop', allowed: ['off', 'dot', 'inbox', 'toast', 'desktop'] },
  { kind: 'inbox.gate', section: 'needs_you', label: 'Gate', when: 'A gate waits.', show: 'toast', defaultShow: 'toast', allowed: ['inbox', 'toast', 'desktop'] },
  { kind: 'pr.merged', section: 'updates', label: 'Merged', when: 'A PR merged.', show: 'toast', defaultShow: 'inbox', allowed: ['off', 'dot', 'inbox', 'toast', 'desktop'] },
];

describe('Settings → Notifications', () => {
  it('lists each kind under its section, sets its Show and resets to defaults', async () => {
    const calls = stubApi({
      'GET /api/notifications/settings': kinds,
      'PATCH /api/notifications/settings': kinds,
    });
    renderWithQuery(<NotificationSettingsPage />);
    expect(await screen.findByRole('region', { name: 'Needs you' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Updates' })).toBeInTheDocument();
    expect(screen.getByText('A PR merged. Default: Inbox.')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: /Session waiting: show/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Inbox' }));
    expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ 'session.needs_you': 'inbox' });

    await userEvent.click(screen.getByRole('button', { name: /Gate: show/ }));
    expect(screen.queryByRole('option', { name: 'Off' })).toBeNull();
    await userEvent.keyboard('{Escape}');

    await userEvent.click(screen.getByRole('button', { name: 'Reset to defaults' }));
    expect(calls.filter((c) => c.method === 'PATCH').at(-1)?.body).toEqual({ 'pr.merged': null });
  });
});
