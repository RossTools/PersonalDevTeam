// What a write marks stale (PDT-451): the views that show what it changed, not only the one it was made from.
import { QueryClient, QueryObserver } from '@tanstack/react-query';
import { describe, expect, it } from 'vitest';
import { invalidateTickets, ticketKeys } from '../tickets/queries';
import type { WorkflowDetail } from '../api/gen/WorkflowDetail';
import { invalidateProjects, projectKeys } from '../projects/queries';
import { sessionKeys } from '../sessions/queries';
import { invalidateWorkflow, savedWorkflow, workflowKeys } from '../workflows/queries';
import { workspaceKeys } from '../workspaces/queries';
import { ticketTabPoll } from '../workspaces/useWorkspaces';
import { invalidateWorktrees, worktreeKeys } from './worktrees';

const stale = (qc: QueryClient, key: readonly unknown[]) => qc.getQueryState(key)?.isInvalidated;

function cached(...keys: (readonly unknown[])[]) {
  const qc = new QueryClient();
  for (const k of keys) qc.setQueryData(k, {});
  return qc;
}

describe('invalidation', () => {
  it('a ticket write refreshes its hover card', () => {
    const qc = cached(ticketKeys.card('pdt-1'));
    invalidateTickets(qc);
    expect(stale(qc, ['tickets', 'card', 'PDT-1'])).toBe(true);
  });

  it('turning a workflow on or off refreshes the Settings sections that show a built-in', () => {
    const keys = [['workflows'], ['workflow', 'w1'], ['stuck-agents'], ['notifications', 'rules'], ['install-updates'], ['workflow', 'w2']];
    const qc = cached(...keys);
    invalidateWorkflow(qc, 'w1');
    expect(keys.map((k) => stale(qc, k))).toEqual([true, true, true, true, true, false]);
  });

  it('saving a workflow keeps the saved one and refreshes the list, its versions and the built-in sections', () => {
    const qc = cached(workflowKeys.all, workflowKeys.versions('w1'), ['stuck-agents'], workflowKeys.versions('w2'));
    savedWorkflow(qc, { id: 'w1', version: 3 } as WorkflowDetail);
    expect(qc.getQueryData(workflowKeys.one('w1'))).toEqual({ id: 'w1', version: 3 });
    expect([workflowKeys.all, workflowKeys.versions('w1'), ['stuck-agents'], workflowKeys.versions('w2')].map((k) => stale(qc, k))).toEqual([true, true, true, false]);
  });

  it('a project write refreshes the projects and the workspaces', () => {
    const qc = cached(projectKeys.all, workspaceKeys.all);
    void invalidateProjects(qc);
    expect([stale(qc, projectKeys.all), stale(qc, workspaceKeys.all)]).toEqual([true, true]);
  });

  it('a worktree write refreshes the sessions and workspaces in it', () => {
    const qc = cached(sessionKeys.project('p1'), workspaceKeys.all);
    invalidateWorktrees(qc, 'p1');
    expect([stale(qc, sessionKeys.project('p1')), stale(qc, workspaceKeys.all)]).toEqual([true, true]);
  });

  it("a worktree write refreshes the project's list, or every list", () => {
    const qc = cached(worktreeKeys.project('p1'), worktreeKeys.project('p2'));
    invalidateWorktrees(qc, 'p1');
    expect([stale(qc, ['worktrees', 'p1']), stale(qc, ['worktrees', 'p2'])]).toEqual([true, false]);
    invalidateWorktrees(qc);
    expect(stale(qc, ['worktrees', 'p2'])).toBe(true);
  });

  it("a ticket tab doesn't poll while the ticket's own view does", () => {
    const qc = cached(ticketKeys.one('t1'));
    const query = qc.getQueryCache().find({ queryKey: ticketKeys.one('t1') })!;
    const tab = new QueryObserver(qc, { queryKey: ticketKeys.one('t1'), enabled: false });
    const unsubTab = tab.subscribe(() => {});
    expect(ticketTabPoll(query)).toBe(5000);
    const view = new QueryObserver(qc, { queryKey: ticketKeys.one('t1'), enabled: false });
    const unsubView = view.subscribe(() => {});
    expect(ticketTabPoll(query)).toBe(false);
    unsubView();
    expect(ticketTabPoll(query)).toBe(5000);
    unsubTab();
  });
});
