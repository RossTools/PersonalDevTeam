import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import { renderWithQuery, stubApi } from '../test-utils';
import { BuiltinSettingsBanner } from '../workflows/BuiltinSettingsBanner';
import { PmCheckpointSettings } from './PmCheckpointSettings';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const ALL = ['mon', 'tue', 'wed', 'thu', 'fri', 'sat', 'sun'];
const defaults = { atTime: false, time: '09:00', days: ALL, chat: '' };

const view = (over: Partial<BuiltinSettingsView> = {}, values: Record<string, unknown> = {}): BuiltinSettingsView => ({
  workflowId: 'w1', name: 'PM daily checkpoint', enabled: true, customized: false, params: [],
  values: { ...defaults, ...values }, defaults, channels: [], ...over,
});

const puts = (calls: { method: string; body: unknown }[]) => calls.filter((c) => c.method === 'PUT').map((c) => c.body);

describe('Settings → This host → PM → Daily checkpoint (PDT-384)', () => {
  it("changes the workflow's inputs: a set time and the days", async () => {
    let values: Record<string, unknown> = {};
    const calls = stubApi({
      'GET /api/pm-checkpoint': () => view({}, values),
      'PUT /api/pm-checkpoint': (c) => {
        values = { ...values, ...(c.body as { values: Record<string, unknown> }).values };
        return view({}, values);
      },
    });
    renderWithQuery(<PmCheckpointSettings />);
    expect(screen.queryByRole('textbox', { name: 'Time' })).not.toBeInTheDocument();
    await userEvent.click(await screen.findByRole('switch', { name: 'At a set time' }));
    await waitFor(() => expect(puts(calls)).toEqual([{ values: { atTime: true } }]));
    const time = await screen.findByRole('textbox', { name: 'Time' });
    await userEvent.clear(time);
    await userEvent.type(time, '07:30');
    await userEvent.tab();
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { time: '07:30' } }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Sat' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { days: ['mon', 'tue', 'wed', 'thu', 'fri', 'sun'] } }));
    await userEvent.click(screen.getByRole('checkbox', { name: 'Sun' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { days: ['mon', 'tue', 'wed', 'thu', 'fri'] } }));
    expect(screen.getByRole('link', { name: 'Customize as a workflow →' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Chat channel/ })).not.toBeInTheDocument();
  });

  it('keeps the last day picked', async () => {
    stubApi({ 'GET /api/pm-checkpoint': () => view({}, { days: ['fri'] }) });
    renderWithQuery(<PmCheckpointSettings />);
    expect(await screen.findByRole('checkbox', { name: 'Fri' })).toBeDisabled();
    expect(screen.getByRole('checkbox', { name: 'Mon' })).toBeEnabled();
  });

  it('names a workflow changed beyond its inputs, and resets it to the default', async () => {
    let customized = true;
    const calls = stubApi({
      'GET /api/pm-checkpoint': () => view({ customized }),
      'POST /api/pm-checkpoint/reset': () => {
        customized = false;
        return view();
      },
    });
    renderWithQuery(<PmCheckpointSettings />);
    expect(await screen.findByText('The daily checkpoint follows your customized workflow PM daily checkpoint.')).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: 'At a set time' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/pm-checkpoint/reset')).toBe(true));
    expect(await screen.findByRole('switch', { name: 'At a set time' })).toBeInTheDocument();
  });

  it('says when the workflow is off, and Reset to default turns it back on', async () => {
    let enabled = false;
    stubApi({
      'GET /api/pm-checkpoint': () => view({ enabled }),
      'POST /api/pm-checkpoint/reset': () => {
        enabled = true;
        return view();
      },
    });
    renderWithQuery(<PmCheckpointSettings />);
    expect(await screen.findByText('The workflow PM daily checkpoint is off: PM conversations get no daily update.')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
    expect(await screen.findByRole('switch', { name: 'At a set time' })).toBeInTheDocument();
  });

  it('points the workflow editor at Settings', () => {
    renderWithQuery(<BuiltinSettingsBanner builtin="pm-daily-checkpoint" />);
    expect(screen.getByText(/Its settings are in Settings → This host → PM: the time, the days and the chat channel/)).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open Settings' })).toBeInTheDocument();
  });

  it('shows no banner on a built-in Settings does not set', () => {
    renderWithQuery(<BuiltinSettingsBanner builtin="pr-notices" />);
    expect(screen.queryByText(/Its settings are in/)).not.toBeInTheDocument();
  });
});
