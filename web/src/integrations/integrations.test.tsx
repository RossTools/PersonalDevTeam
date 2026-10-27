import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderWithQuery, stubApi } from '../test-utils';
import type { IntegrationInfo } from '../api/gen/IntegrationInfo';
import { IntegrationsPage } from './IntegrationsPage';

const info = (id: string, label: string, over: Partial<IntegrationInfo> = {}): IntegrationInfo => ({
  id, label, description: `${label} things.`, enabled: false, built: true, ...over,
});

describe('integrations', () => {
  afterEach(() => vi.unstubAllGlobals());

  it('turns an integration on with the others kept as they are, and one not built in can\'t be', async () => {
    const calls = stubApi({
      'GET /api/integrations': [
        info('discord', 'Discord', { enabled: true }),
        info('jira', 'Jira'),
        info('plex', 'Plex', { built: false, description: '' }),
      ],
      'PUT /api/integrations': [info('discord', 'Discord', { enabled: true }), info('jira', 'Jira', { enabled: true })],
    });
    renderWithQuery(<IntegrationsPage />);
    expect(await screen.findByText('Jira things.')).toBeInTheDocument();
    expect(screen.getByText('Not in this build')).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Use Plex' })).toBeDisabled();
    expect(screen.getByRole('switch', { name: 'Use Discord' })).toBeChecked();
    await userEvent.click(screen.getByRole('switch', { name: 'Use Jira' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PUT')?.body).toEqual({ enabled: ['discord', 'jira'] }));
    await waitFor(() => expect(screen.getByRole('switch', { name: 'Use Jira' })).toBeChecked());
  });
});
