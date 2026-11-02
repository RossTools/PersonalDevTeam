import { act, waitFor } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { PmState } from '../api/gen/PmState';
import type { Session } from '../api/gen/Session';
import { usePmSummary } from '../pm/usePm';
import { renderWithQuery, stubApi } from '../test-utils';
import { useWorkspaces, type WorkspacesApi } from '../workspaces/useWorkspaces';

// The shell re-renders only when what it shows changed, not on every poll (PDT-454).

afterEach(() => vi.unstubAllGlobals());

const session = (id: string, more: Partial<Session> = {}): Session => ({
  id, projectId: 'host', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'PM',
  cwd: '/data/pm', status: 'running', exitCode: null, createdAt: 1, endedAt: null, externalSessionId: null, freshStartedAt: null,
  activity: 'working', attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null, mode: 'conversation',
  model: null, permissionMode: 'ask', persona: { id: 'pm', name: 'PM', version: 1, stepId: null }, background: [],
  ...more,
});

const pmState = (s: Session, lastActivity: number): PmState => ({
  current: s,
  sessions: [{ session: s, open: true, title: 'Plans', unseen: s.unseen, lastActivity, rotation: 'none', reason: null, journal: null, via: null, resetFrom: null }],
});

describe('usePmSummary', () => {
  it('keeps its data while a turn runs, and changes when a reply is unread', async () => {
    let state = pmState(session('c1'), 1);
    stubApi({ 'GET /api/pm': () => state });
    const seen: unknown[] = [];
    function Probe() {
      const data = usePmSummary().data;
      if (data) seen.push(data);
      return null;
    }
    const { qc } = renderWithQuery(<Probe />);
    await waitFor(() => expect(seen.length).toBeGreaterThan(0));
    const first = seen.at(-1);
    expect(first).toEqual({ current: 'c1', conversations: [{ id: 'c1', title: 'Plans', unread: false }] });

    // The turn moves on: the session's activity time changes, nothing the shell shows does.
    state = pmState(session('c1', { attentionAt: 7 }), 2);
    await act(() => qc.refetchQueries({ queryKey: ['pm'] }));
    expect(seen.at(-1)).toBe(first);

    state = pmState(session('c1', { activity: 'idle', unseen: true }), 3);
    await act(() => qc.refetchQueries({ queryKey: ['pm'] }));
    await waitFor(() => expect(seen.at(-1)).toEqual({ current: 'c1', conversations: [{ id: 'c1', title: 'Plans', unread: true }] }));
  });
});

describe('useWorkspaces', () => {
  it('returns the same object while nothing in it changed', async () => {
    stubApi({ 'GET /api/projects': [], 'GET /api/workspaces': [] });
    const seen: WorkspacesApi[] = [];
    let bump = () => {};
    function Probe() {
      const [, setN] = useState(0);
      bump = () => setN((n) => n + 1);
      const ws = useWorkspaces();
      seen.push(ws);
      return null;
    }
    const { qc } = renderWithQuery(<Probe />);
    await waitFor(() => expect(seen.at(-1)?.projectsLoaded).toBe(true));
    const loaded = seen.at(-1);
    // An unrelated re-render, and a poll that brings the same answer.
    act(() => bump());
    await act(() => qc.refetchQueries({ queryKey: ['workspaces'] }));
    expect(seen.at(-1)).toBe(loaded);
  });
});
