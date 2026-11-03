import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import { renderWithQuery, stubApi } from '../test-utils';
import { InstallUpdates } from './InstallUpdates';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const defaults = {
  stableOnly: false, window: false, windowFrom: '02:00', windowTo: '06:00', installAnywayAfter: 0, checkAfter: true,
  announce: false, chat: '',
};

const view = (over: Partial<BuiltinSettingsView> = {}, values: Record<string, unknown> = {}): BuiltinSettingsView => ({
  workflowId: 'w1', name: 'Install updates', enabled: true, customized: false, params: [],
  values: { ...defaults, ...values }, defaults, channels: [], ...over,
});

const puts = (calls: { method: string; body: unknown }[]) => calls.filter((c) => c.method === 'PUT').map((c) => c.body);

describe('Settings → Updates → Installing updates (PDT-369)', () => {
  it("changes the workflow's inputs: the window, waiting for agents and the check after installing", async () => {
    let values: Record<string, unknown> = {};
    const calls = stubApi({
      'GET /api/install-updates': () => view({}, values),
      'PUT /api/install-updates': (c) => {
        values = { ...values, ...(c.body as { values: Record<string, unknown> }).values };
        return view({}, values);
      },
    });
    renderWithQuery(<InstallUpdates policy="auto_when_idle" />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Only in a time window' }));
    await waitFor(() => expect(puts(calls)).toEqual([{ values: { window: true } }]));
    const from = await screen.findByRole('textbox', { name: 'Window from' });
    await userEvent.clear(from);
    await userEvent.type(from, '01:30');
    await userEvent.tab();
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { windowFrom: '01:30' } }));
    await userEvent.click(screen.getByRole('button', { name: /While agents work/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Install anyway after 4 hours' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { installAnywayAfter: 4 * 3_600_000 } }));
    await userEvent.click(screen.getByRole('switch', { name: 'Check PDT after installing' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { checkAfter: false } }));
    expect(screen.getByRole('link', { name: 'Customize as a workflow →' })).toBeInTheDocument();
  });

  it('marks what applies only while updates install by themselves', async () => {
    stubApi({ 'GET /api/install-updates': () => view() });
    renderWithQuery(<InstallUpdates policy="notify" />);
    expect(await screen.findByRole('switch', { name: 'Only in a time window' })).toBeDisabled();
    expect(screen.getByText(/The rest applies once Install updates is set to install automatically when idle/)).toBeInTheDocument();
    expect(screen.getByRole('switch', { name: 'Check PDT after installing' })).toBeEnabled();
  });

  it('names a workflow changed beyond its inputs, with Open and Reset to default', async () => {
    let customized = true;
    const calls = stubApi({
      'GET /api/install-updates': () => view({ customized }),
      'POST /api/install-updates/reset': () => {
        customized = false;
        return view();
      },
    });
    renderWithQuery(<InstallUpdates policy="auto_when_idle" />);
    expect(await screen.findByText('Updates follow your customized workflow Install updates.')).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: 'Only in a time window' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/install-updates/reset')).toBe(true));
    expect(await screen.findByRole('switch', { name: 'Only in a time window' })).toBeInTheDocument();
  });

  it('says when the workflow is off, and turns it on', async () => {
    let enabled = false;
    const calls = stubApi({
      'GET /api/install-updates': () => view({ enabled }),
      'PATCH /api/workflows/w1': (c) => {
        enabled = (c.body as { enabled: boolean }).enabled;
        return {};
      },
    });
    renderWithQuery(<InstallUpdates policy="auto_when_idle" />);
    expect(await screen.findByText('The workflow Install updates is off: updates install only when you say.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Turn on' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ enabled: true }));
    expect(await screen.findByRole('switch', { name: 'Only in a time window' })).toBeInTheDocument();
  });
});
