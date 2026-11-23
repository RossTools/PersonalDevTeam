import { describe, expect, it } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Run } from '../api/gen/Run';
import type { Session } from '../api/gen/Session';
import { ticket } from '../tickets/fixtures';
import { closePrompt, runInfo, sessionInfo, ticketInfo } from './useWorkspaces';

const attempt = (over: Partial<AttemptSummary> = {}): AttemptSummary => ({
  id: 'a1', branch: 'pdt-ALP-1-x', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null, pr: null, ...over,
});

describe('ticketInfo', () => {
  it("places a ticket tab in its active attempt's worktree, else the main checkout", () => {
    expect(ticketInfo(ticket(1, { attempt: attempt() })).worktreeId).toBe('w1');
    expect(ticketInfo(ticket(1, { attempt: attempt({ state: 'merged' }) })).worktreeId).toBeNull();
    expect(ticketInfo(ticket(1, { attempt: null })).worktreeId).toBeNull();
  });
});

const session = (over: Partial<Session> = {}): Session => ({
  id: 's1', projectId: 'p1', worktreeId: null, kind: 'agent_tui', agent: 'claude', bypassPermissions: false, title: 'Chat',
  cwd: '/r/p1', status: 'finished', exitCode: null, createdAt: 0, endedAt: null, externalSessionId: null,
  freshStartedAt: null, activity: null, attentionAt: null, lastViewedAt: null, unseen: false, attemptId: null, ticket: null,
  mode: 'conversation', model: null, permissionMode: null, persona: null, background: [], instance: null, ...over,
});

const run = (over: Partial<Run> = {}): Run => ({
  id: 'r1', projectId: 'p1', worktreeId: null, branch: 'main', name: 'pdt', kind: 'run', state: 'stopped', port: null,
  url: null, dataDir: null, cwd: '/r/p1', exitCode: null, error: null, detached: false, createdAt: 0, updatedAt: 0, ...over,
});

describe('closePrompt', () => {
  it('asks before closing a finished conversation, whose history would be deleted', () => {
    const p = closePrompt(sessionInfo(session()));
    expect(p).toMatchObject({ title: 'Close Chat?', action: 'Close and remove', danger: true });
    expect(p?.body).toContain('conversation history is deleted');
  });

  it('asks before closing an interrupted conversation or a stopped agent terminal', () => {
    expect(closePrompt(sessionInfo(session({ status: 'interrupted' })))).not.toBeNull();
    expect(closePrompt(sessionInfo(session({ mode: 'terminal' }))))
      .toMatchObject({ body: expect.stringContaining('resumed') });
  });

  it('asks to stop a running session', () => {
    expect(closePrompt(sessionInfo(session({ status: 'running' })))).toMatchObject({ action: 'Stop and close' });
    expect(closePrompt(sessionInfo(session({ kind: 'shell', agent: null, mode: 'terminal', status: 'running' }))))
      .toMatchObject({ action: 'Stop and close' });
  });

  it('closes a stopped shell, a stopped run and other tabs without asking', () => {
    expect(closePrompt(sessionInfo(session({ kind: 'shell', agent: null, mode: 'terminal' })))).toBeNull();
    expect(closePrompt(runInfo(run()))).toBeNull();
    expect(closePrompt(ticketInfo(ticket(1, { attempt: null })))).toBeNull();
    expect(closePrompt(null)).toBeNull();
  });

  it('asks to stop an active run', () => {
    expect(closePrompt(runInfo(run({ state: 'ready' })))).toMatchObject({ action: 'Stop and close' });
  });
});
