import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PmState } from '../api/gen/PmState';
import type { Session } from '../api/gen/Session';
import { renderWithQuery, stubApi } from '../test-utils';
import { PmList } from './PmList';
import { PmView } from './PmView';
import { firstLine } from './usePm';

afterEach(() => vi.unstubAllGlobals());

const session = (id: string, createdAt: number): Session => ({
  id, projectId: 'host', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PM',
  cwd: '/data/pm', status: 'finished', exitCode: null, createdAt, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: { id: 'pm', name: 'PM', version: 1, stepId: null },
});

const ended: PmState = {
  current: null,
  sessions: [{ session: session('s1', Date.UTC(2026, 9, 20, 9)), rotation: 'done', reason: 'age', journal: '## Today\nPlanned the PM.' }],
};

describe('the PM', () => {
  it('reads a journal’s first line without its heading marks', () => {
    expect(firstLine('\n## Today\nPlanned.')).toBe('Today');
    expect(firstLine(null)).toBe('');
  });

  it('lists the current session and earlier sessions with their journal, and no notes (they have a view of their own)', async () => {
    stubApi({ 'GET /api/pm': ended });
    const onSelect = vi.fn();
    renderWithQuery(<PmList selection="current" onSelect={onSelect} onNew={() => {}} />);
    expect(await screen.findByText('Ask something to start one')).toBeInTheDocument();
    expect(await screen.findByText('Today')).toBeInTheDocument();
    expect(screen.queryByText('Notes')).not.toBeInTheDocument();
    await userEvent.click(screen.getByText('Today'));
    expect(onSelect).toHaveBeenCalledWith('s1');
  });

  it('starts a session with the first message when there is none', async () => {
    const calls = stubApi({ 'GET /api/pm': ended, 'POST /api/pm/sessions': session('s2', 2) });
    const onSelect = vi.fn();
    renderWithQuery(<PmView selection="current" onSelect={onSelect} onNew={() => {}} />);
    const box = await screen.findByRole('textbox', { name: 'Ask the PM' });
    await userEvent.type(box, "What's in flight?{Enter}");
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('current'));
    expect(calls.find((c) => c.method === 'POST')?.body).toEqual({ text: "What's in flight?" });
  });

  it('keeps the start box’s unsent text when the page goes and comes back; starting clears it', async () => {
    stubApi({ 'GET /api/pm': ended, 'POST /api/pm/sessions': session('s2', 2) });
    const first = renderWithQuery(<PmView selection="current" onSelect={() => {}} onNew={() => {}} />);
    await userEvent.type(await screen.findByRole('textbox', { name: 'Ask the PM' }), 'half a thought');
    first.unmount();
    const onSelect = vi.fn();
    const second = renderWithQuery(<PmView selection="current" onSelect={onSelect} onNew={() => {}} />);
    const box = await screen.findByRole('textbox', { name: 'Ask the PM' });
    expect(box).toHaveValue('half a thought');
    await userEvent.type(box, '{Enter}');
    await waitFor(() => expect(onSelect).toHaveBeenCalledWith('current'));
    second.unmount();
    renderWithQuery(<PmView selection="current" onSelect={() => {}} onNew={() => {}} />);
    expect(await screen.findByRole('textbox', { name: 'Ask the PM' })).toHaveValue('');
  });
});
