import { screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { KindSetting } from '../api/gen/KindSetting';
import type { NotificationRulesView } from '../api/gen/NotificationRulesView';
import { renderWithQuery, stubApi } from '../test-utils';
import { NotificationSettingsPage } from './NotificationSettingsPage';

afterEach(() => vi.unstubAllGlobals());

const kinds: KindSetting[] = [
  { kind: 'session.needs_you', section: 'needs_you', label: 'Session waiting', when: 'A session waits.', show: 'desktop', defaultShow: 'desktop', allowed: ['off', 'dot', 'inbox', 'toast', 'desktop'] },
  { kind: 'inbox.gate', section: 'needs_you', label: 'Gate', when: 'A gate waits.', show: 'toast', defaultShow: 'toast', allowed: ['inbox', 'toast', 'desktop'] },
  { kind: 'pr.merged', section: 'updates', label: 'Merged', when: 'A PR merged.', show: 'toast', defaultShow: 'inbox', allowed: ['off', 'dot', 'inbox', 'toast', 'desktop'] },
];

const defaults = {
  escalate: false, escalateAfter: 7_200_000, quietHours: false, quietFrom: '22:00', quietTo: '07:00', chat: '',
  chatQuestions: true, chatApprovals: true, chatFailedRuns: true,
};

const view = (over: Partial<NotificationRulesView> = {}, values: Record<string, unknown> = {}): NotificationRulesView => ({
  workflowId: 'w1', name: 'Notification rules', enabled: true, customized: false, kinds,
  values: { ...defaults, ...values }, defaults, channels: [], ...over,
});

const puts = (calls: { method: string; path: string; body: unknown }[]) =>
  calls.filter((c) => c.method === 'PUT' && c.path === '/api/notification-rules').map((c) => c.body);

describe('Settings → Notifications', () => {
  it("lists each kind under its section, sets its level as the rules' input and resets to defaults", async () => {
    const calls = stubApi({
      'GET /api/notification-rules': () => view(),
      'PUT /api/notification-rules': () => view(),
    });
    renderWithQuery(<NotificationSettingsPage />);
    expect(await screen.findByRole('region', { name: 'Needs you' })).toBeInTheDocument();
    expect(screen.getByRole('region', { name: 'Updates' })).toBeInTheDocument();
    expect(screen.getByText('A PR merged. Default: Inbox.')).toBeInTheDocument();

    await userEvent.click(screen.getByRole('button', { name: /Session waiting: show/ }));
    await userEvent.click(await screen.findByRole('option', { name: 'Inbox' }));
    await waitFor(() => expect(puts(calls)).toEqual([{ levels: { 'session.needs_you': 'inbox' } }]));

    await userEvent.click(screen.getByRole('button', { name: /Gate: show/ }));
    expect(screen.queryByRole('option', { name: 'Off' })).toBeNull();
    await userEvent.keyboard('{Escape}');

    await userEvent.click(screen.getByRole('button', { name: 'Reset to defaults' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ levels: { 'pr.merged': null } }));
    expect(screen.getByRole('link', { name: 'Customize as a workflow →' })).toBeInTheDocument();
  });

  it('turns on escalation, quiet hours and a chat channel', async () => {
    let values: Record<string, unknown> = {};
    const calls = stubApi({
      'GET /api/notification-rules': () => view({ channels: [{ label: 'Discord', connectionKind: 'discord_webhook' }] }, values),
      'PUT /api/notification-rules': (c) => {
        values = { ...values, ...(c.body as { values: Record<string, unknown> }).values };
        return view({ channels: [{ label: 'Discord', connectionKind: 'discord_webhook' }] }, values);
      },
      'GET /api/connections': [
        { id: 'c1', name: 'alerts', kind: 'discord_webhook', url: '', config: {}, hasSecret: true, createdAt: 1, updatedAt: 1 },
        { id: 'c2', name: 'jira', kind: 'jira', url: '', config: {}, hasSecret: true, createdAt: 1, updatedAt: 1 },
      ],
    });
    renderWithQuery(<NotificationSettingsPage />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Escalate what waits' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { escalate: true } }));
    await userEvent.click(await screen.findByRole('button', { name: /Escalate after/ }));
    await userEvent.click(within(screen.getByRole('listbox')).getByRole('option', { name: '4 hours' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { escalateAfter: 4 * 3_600_000 } }));

    await userEvent.click(screen.getByRole('switch', { name: 'Quiet hours' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { quietHours: true } }));
    const from = await screen.findByRole('textbox', { name: 'Quiet from' });
    await userEvent.clear(from);
    await userEvent.type(from, '21:30');
    await userEvent.tab();
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { quietFrom: '21:30' } }));

    await userEvent.click(screen.getByRole('button', { name: /Chat channel/ }));
    const list = within(screen.getByRole('listbox'));
    expect(list.queryByRole('option', { name: /jira/ })).toBeNull();
    await userEvent.click(list.getByRole('option', { name: 'alerts (Discord)' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { chat: 'alerts' } }));
    await userEvent.click(await screen.findByRole('switch', { name: 'Post failed runs' }));
    await waitFor(() => expect(puts(calls).at(-1)).toEqual({ values: { chatFailedRuns: false } }));
  });

  it('shows no chat channel while no plugin posts notifications', async () => {
    stubApi({ 'GET /api/notification-rules': () => view() });
    renderWithQuery(<NotificationSettingsPage />);
    expect(await screen.findByRole('switch', { name: 'Quiet hours' })).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: /Chat channel/ })).toBeNull();
  });

  it('names a customized workflow, with Open and Reset to default, and keeps its levels read-only', async () => {
    let customized = true;
    const calls = stubApi({
      'GET /api/notification-rules': () => view({ customized }),
      'POST /api/notification-rules/reset': () => {
        customized = false;
        return view();
      },
    });
    renderWithQuery(<NotificationSettingsPage />);
    expect(await screen.findByText('Notifications follow your customized workflow Notification rules.')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open' })).toBeInTheDocument();
    expect(screen.queryByRole('switch', { name: 'Quiet hours' })).toBeNull();
    expect(screen.getByRole('button', { name: /Gate: show/ })).toBeDisabled();
    await userEvent.click(screen.getByRole('button', { name: 'Reset to default' }));
    await waitFor(() => expect(calls.some((c) => c.method === 'POST' && c.path === '/api/notification-rules/reset')).toBe(true));
    expect(await screen.findByRole('switch', { name: 'Quiet hours' })).toBeInTheDocument();
  });

  it('turns the rules off', async () => {
    const calls = stubApi({
      'GET /api/notification-rules': () => view(),
      'PATCH /api/workflows/w1': { id: 'w1' },
    });
    renderWithQuery(<NotificationSettingsPage />);
    await userEvent.click(await screen.findByRole('switch', { name: 'Notification rules' }));
    await waitFor(() => expect(calls.find((c) => c.method === 'PATCH')?.body).toEqual({ enabled: false }));
  });
});
