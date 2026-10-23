import { describe, expect, it } from 'vitest';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import { ticket } from '../tickets/fixtures';
import { ticketInfo } from './useWorkspaces';

const attempt = (over: Partial<AttemptSummary> = {}): AttemptSummary => ({
  id: 'a1', branch: 'pdt-ALP-1-x', baseBranch: 'main', worktreeId: 'w1', state: 'active', createdAt: 1, endedAt: null,
  lastHead: null, workspaceId: null, ci: null, pr: null, ...over,
});

describe('ticketInfo', () => {
  it("places a ticket tab in its active attempt's worktree, else the main checkout", () => {
    expect(ticketInfo(ticket(1, { attempt: attempt() }), 0).worktreeId).toBe('w1');
    expect(ticketInfo(ticket(1, { attempt: attempt({ state: 'merged' }) }), 0).worktreeId).toBeNull();
    expect(ticketInfo(ticket(1, { attempt: null }), 0).worktreeId).toBeNull();
  });
});
