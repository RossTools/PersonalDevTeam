import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { BuiltinSettingsView } from '../api/gen/BuiltinSettingsView';
import { renderWithQuery, stubApi } from '../test-utils';
import { StuckAgents } from './StuckAgents';

afterEach(() => {
  vi.unstubAllGlobals();
  vi.restoreAllMocks();
});

const defaults = {
  nudges: 2, firstNudge: 'You stopped without an outcome.', laterNudge: 'You stopped again.', restart: true,
  onLoopLimit: 'ask', timeoutRestarts: 1, idleMinutes: 20,
};

const view = (over: Partial<BuiltinSettingsView> = {}, values: Record<string, unknown> = {}): BuiltinSettingsView => ({
  workflowId: 'w1', name: 'When an agent is stuck', enabled: true, customized: false, params: [],
  values: { ...defaults, ...values }, defaults, channels: [], ...over,
});

describe('Settings → Agents → When an agent is stuck (PDT-367)', () => {
  it("changes the workflow's inputs: counts, switches and messages, and puts a message's default text back", async () => {
    let values: Record<string, unknown> = { firstNudge: 'Hurry.' };
    const calls = stubApi({
      'GET /api/stuck-agents': () => view({}, values),
      'PUT /api/stuck-agents': (c) => {
        values = { ...values, ...(c.body as { values: Record<string, unknown> }).values };
        return view({}, values);
      },
    });
    renderWithQuery(<StuckAgents />);
    await userEvent.click(await screen.findByRole('button', { name: /Nudges/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: '3' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').map((c) => c.body)).toEqual([{ values: { nudges: 3 } }]));
    await userEvent.click(screen.getByRole('switch', { name: 'Restart in a fresh session before asking you' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').at(-1)?.body).toEqual({ values: { restart: false } }));
    await userEvent.click(screen.getByRole('button', { name: /At a loop limit/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Stop the step' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').at(-1)?.body).toEqual({ values: { onLoopLimit: 'stop' } }));
    // Idle tickets (PDT-510): minutes, or never.
    await userEvent.click(screen.getByRole('button', { name: /Idle tickets/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: 'Never' }));
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').at(-1)?.body).toEqual({ values: { idleMinutes: 0 } }));
    const later = screen.getByRole('textbox', { name: 'Later nudges' });
    await userEvent.clear(later);
    await userEvent.type(later, 'Please finish.');
    await userEvent.tab();
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').at(-1)?.body).toEqual({ values: { laterNudge: 'Please finish.' } }));
    // The first nudge differs from its default: one click puts it back.
    const links = screen.getAllByRole('link', { name: 'Reset to default text' });
    expect(links).toHaveLength(2);
    await userEvent.click(links[0]);
    await waitFor(() => expect(calls.filter((c) => c.method === 'PUT').at(-1)?.body).toEqual({ values: { firstNudge: defaults.firstNudge } }));
    expect(screen.getByRole('link', { name: 'Customize as a workflow →' })).toBeInTheDocument();
  });

  it('names a workflow changed beyond its inputs, with Open and Reset to default', async () => {
    let customized = true;
    const calls = stubApi({
      'GET /api/stuck-agents': () => view({ customized }),
      'POST /api/stuck-agents/reset': () => {
        customized = false;
        return view();
      },
    });
    renderWithQuery(<StuckAgents />);
    expect(await screen.findByText('Stuck agents follow your customized workflow When an agent is stuck.')).toBeInTheDocument();
    expect(screen.queryByRole('textbox', { name: 'First nudge' })).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open' })).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/stuck-agents/reset')).toBe(true));
    expect(await screen.findByRole('textbox', { name: 'First nudge' })).toBeInTheDocument();
  });

  it('turns the workflow off, saying what happens then', async () => {
    let enabled = true;
    const calls = stubApi({
      'GET /api/stuck-agents': () => view({ enabled }),
      'PATCH /api/workflows/w1': (c) => {
        enabled = (c.body as { enabled: boolean }).enabled;
        return {};
      },
    });
    renderWithQuery(<StuckAgents />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Handle stuck agents' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ enabled: false }));
    expect(await screen.findByText(/Off: a stalled agent is nudged once/)).toBeInTheDocument();
  });
});
