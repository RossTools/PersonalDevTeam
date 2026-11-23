import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { InstancesUpdateView } from '../api/gen/InstancesUpdateView';
import { renderWithQuery, Reply, stubApi } from '../test-utils';
import { InstanceUpdates } from './InstanceUpdates';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const view = (o: Partial<InstancesUpdateView> = {}): InstancesUpdateView => ({
  name: 'studio',
  version: '0.7.3',
  channel: 'stable',
  instances: [{
    instanceId: 'i1', name: 'win-box', version: '0.6.9', protocol: 0, state: 'incompatible', projects: ['VAL'],
    needed: 'update win-box to v0.7.3 to link it with studio',
  }],
  ...o,
});

describe('Updates → Remote instances', () => {
  it('says which instance needs updating, and updates it after asking when it is busy', async () => {
    let n = 0;
    const calls = stubApi({
      'GET /api/host/update/instances': view(),
      'POST /api/host/update/instances/i1': () => (n++ === 0
        ? new Reply(409, { code: 'host_busy', message: 'win-box: Busy: Claude is working. Update anyway?' })
        : { name: 'win-box', outcome: 'installing', version: '0.7.3', channel: 'stable' }),
    });
    renderWithQuery(<InstanceUpdates />);
    const section = await screen.findByRole('region', { name: 'Remote instances' });
    expect(section).toHaveTextContent("Each updates on this host's stable channel");
    expect(within(section).getByText('Update win-box to v0.7.3 to link it with studio.')).toBeInTheDocument();
    expect(within(section).getByText('needs an update')).toBeInTheDocument();
    expect(section).toHaveTextContent('v0.6.9 · VAL');

    await userEvent.click(within(section).getByRole('button', { name: 'Update win-box' }));
    expect(await screen.findByText(/Busy: Claude is working/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Update anyway' }));
    await waitFor(() => expect(calls).toContainEqual({ method: 'POST', path: '/api/host/update/instances/i1', body: { force: true } }));
    expect(calls).toContainEqual({ method: 'POST', path: '/api/host/update/instances/i1', body: { force: false } });
    expect(await screen.findByText(/win-box is installing v0.7.3/)).toBeInTheDocument();
  });

  it('shows nothing without remote instances', async () => {
    const calls = stubApi({ 'GET /api/host/update/instances': view({ instances: [] }) });
    renderWithQuery(<InstanceUpdates />);
    await waitFor(() => expect(calls.length).toBe(1));
    expect(screen.queryByRole('region', { name: 'Remote instances' })).not.toBeInTheDocument();
  });
});
