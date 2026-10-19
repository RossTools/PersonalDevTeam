import { createContext, useContext } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import { toast, type ConfirmOptions, type ConfirmResult, type MenuEntry } from '@ui';
import { api, ApiError } from '../api/client';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Ticket } from '../api/gen/Ticket';
import type { Priority } from '../api/gen/Priority';
import type { PullResult } from '../api/gen/PullResult';
import type { TicketPatch } from '../api/gen/TicketPatch';
import type { TicketStage } from '../api/gen/TicketStage';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { runAction } from '../common/useAction';
import { STAGES } from './model';
import { invalidateTickets } from './queries';

/** Enough of a ticket to act on it (a backlog row or a full ticket). */
export interface TicketRef {
  id: string;
  projectId: string;
  key: string;
  title: string;
  stage: TicketStage;
  priority: Priority;
  /** Start chose "Wait until clear". */
  waiting?: boolean;
}

/** Opens (or focuses) a ticket's tab in its project's workspace. AppShell provides it. */
export const OpenTicketContext = createContext<(t: { id: string; projectId: string }) => void>(() => {});
export const useOpenTicket = () => useContext(OpenTicketContext);

/**
 * Opens the Start dialog for a ticket. `here`: started from the ticket's own tab, so its tabs open in the workspace on
 * screen (after the current tab) rather than at the end of the project workspace. AppShell provides it.
 */
export const StartTicketContext = createContext<(t: { id: string; projectId: string }, opts?: { here?: boolean }) => void>(() => {});
export const useStartTicket = () => useContext(StartTicketContext);

/** Shows a session's tab (opening one in its project's workspace when it has none). AppShell provides it. */
export const OpenSessionContext = createContext<(s: { id: string; projectId: string }) => void>(() => {});
export const useOpenSession = () => useContext(OpenSessionContext);

/** Start is offered for Draft, Ready and To do tickets with no active attempt. */
export const canStart = (t: { stage: TicketStage; attempt: Pick<AttemptSummary, 'state'> | null }) =>
  (t.stage === 'draft' || t.stage === 'ready' || t.stage === 'todo') && t.attempt?.state !== 'active';

export async function patchTicket(qc: QueryClient, id: string, patch: TicketPatch) {
  const t = await api.updateTicket(id, patch);
  qc.setQueryData(['ticket', id], t);
  invalidateTickets(qc, [id]);
  // A ticket tab's title follows the ticket.
  void qc.invalidateQueries({ queryKey: ['workspaces'] });
  return t;
}

/** "Wait until clear" (Start's Wait) or stop waiting; the result toasts. */
export async function setWaiting(qc: QueryClient, t: Pick<TicketRef, 'id' | 'key'>, waiting: boolean) {
  const fresh = await patchTicket(qc, t.id, { waiting });
  toast.success(waiting ? `${t.key} will wait until clear` : `${t.key} stopped waiting`);
  return fresh;
}

/** "Stop waiting", for menus of a waiting ticket. */
export const stopWaitingEntry = (qc: QueryClient, t: Pick<TicketRef, 'id' | 'key'>): MenuEntry => ({
  label: 'Stop waiting',
  onSelect: () => void runAction(() => setWaiting(qc, t, false)),
});

type Confirm = (o: ConfirmOptions) => Promise<ConfirmResult>;

/** Asks, then deletes the ticket (its tab goes with it). Resolves true when it was deleted. */
export async function deleteTicket(confirm: Confirm, qc: QueryClient, t: Pick<TicketRef, 'id' | 'key' | 'title'>): Promise<boolean> {
  const { ok } = await confirm({
    title: `Delete ${t.key}?`,
    body: `“${t.title}”, its comments and its links are deleted. This can't be undone.`,
    action: 'Delete ticket',
    danger: true,
  });
  if (!ok) return false;
  await api.deleteTicket(t.id);
  qc.removeQueries({ queryKey: ['ticket', t.id] });
  invalidateTickets(qc);
  void qc.invalidateQueries({ queryKey: ['workspaces'] });
  return true;
}

/** "Move to ›": every stage but the ticket's own. */
export function moveToEntries(qc: QueryClient, t: TicketRef): MenuEntry[] {
  return STAGES.filter((s) => s !== t.stage).map((s) => ({
    label: STAGE_LABEL[s],
    onSelect: () => void runAction(() => patchTicket(qc, t.id, { stage: s })),
  }));
}

export function priorityEntries(qc: QueryClient, t: TicketRef): MenuEntry[] {
  return PRIORITIES.map((p) => ({
    label: PRIORITY_LABEL[p],
    disabled: p === t.priority,
    onSelect: () => void runAction(() => patchTicket(qc, t.id, { priority: p })),
  }));
}

/**
 * A ticket's menu (backlog row right-click / long press, the peek's and the tab's ⋯): Open, Start…, Stop waiting,
 * Move to ›, Priority ›,
 * optionally Move to top / bottom (backlog), Delete….
 */
export function ticketMenu(opts: {
  qc: QueryClient;
  confirm: Confirm;
  ticket: TicketRef;
  onOpen?: () => void;
  /** Adds Start… (when the ticket can be started). */
  onStart?: () => void;
  /** Adds Move to top / bottom (the backlog sorted by rank). */
  rank?: boolean;
  onDeleted?: () => void;
}): MenuEntry[] {
  const { qc, confirm, ticket: t } = opts;
  const rankTo = (to: 'top' | 'bottom') => () =>
    void runAction(async () => {
      await api.rankTicket(t.id, { to });
      invalidateTickets(qc, [t.id]);
    });
  return [
    ...(opts.onOpen ? [{ label: 'Open', onSelect: opts.onOpen }] : []),
    ...(opts.onStart ? [{ label: 'Start…', onSelect: opts.onStart }] : []),
    ...(t.waiting ? [stopWaitingEntry(qc, t)] : []),
    { submenu: 'Move to', entries: moveToEntries(qc, t) },
    { submenu: 'Priority', entries: priorityEntries(qc, t) },
    ...(opts.rank
      ? [{ label: 'Move to top', onSelect: rankTo('top') }, { label: 'Move to bottom', onSelect: rankTo('bottom') }]
      : []),
    { separator: true },
    {
      label: 'Delete…',
      danger: true,
      onSelect: () => void runAction(async () => {
        if (await deleteTicket(confirm, qc, t)) opts.onDeleted?.();
      }),
    },
  ];
}

/** Asks, then abandons the ticket's active attempt (its worktree and sessions are kept). */
export async function abandonAttempt(confirm: Confirm, qc: QueryClient, t: Pick<Ticket, 'id' | 'key'>, attempt: Pick<AttemptSummary, 'id' | 'branch'>) {
  const { ok } = await confirm({
    title: `Abandon ${t.key}'s attempt?`,
    body: `The attempt on ${attempt.branch} ends. Its worktree, branch and sessions are kept; the ticket can be started again.`,
    action: 'Abandon attempt',
    danger: true,
  });
  if (!ok) return;
  const fresh = await api.abandonAttempt(attempt.id);
  qc.setQueryData(['ticket', t.id], fresh);
  invalidateTickets(qc, [t.id]);
}

const isCode = (e: unknown, code: string) => e instanceof ApiError && e.code === code;

const BUSY_RETRIES = 10;
const BUSY_RETRY_MS = 300;
const pause = () => new Promise((r) => setTimeout(r, BUSY_RETRY_MS));
const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

/** Retries `f` while it fails with `code` (a stopped process takes a moment to exit). */
async function retryWhile(code: string, f: () => Promise<unknown>) {
  for (let i = 0; ; i += 1) {
    try {
      return await f();
    } catch (e) {
      if (!isCode(e, code) || i >= BUSY_RETRIES) throw e;
      await pause();
    }
  }
}

const ignoreMissing = (e: unknown) => {
  if (!(e instanceof ApiError && e.status === 404)) throw e;
};

/** The base branch's pull, for the toast: a sentence to add to the success, or a problem to show on its own. */
function pullNote(r: PullResult | null): { done?: string; problem?: string } {
  if (!r) return { problem: 'The base branch could not be pulled.' };
  const b = r.branch;
  const why = r.message?.split('\n')[0]?.trim();
  switch (r.outcome) {
    case 'updated':
      return { done: `${b} is pulled.` };
    case 'up_to_date':
      return { done: `${b} is up to date.` };
    case 'diverged':
      return { problem: `${b} was not pulled: it has commits origin doesn't, and origin has commits it doesn't.` };
    case 'blocked':
      return { problem: `${b} was not pulled${why ? `: ${why}` : '; its checkout has changes in the way.'}` };
    case 'fetch_failed':
      return { problem: `${b} was not pulled: fetching it from origin failed${why ? ` (${why})` : ''}.` };
    default:
      return {};
  }
}

/**
 * The merged banner's action: moves the ticket to Done, stops what is still going (the sessions in the ticket's
 * workspace, the attempt's sessions, every session and run in its worktree, the workspace's runs) after asking once,
 * removes the worktree (asking to discard uncommitted changes), then deletes the workspace's sessions, closes the
 * ticket's tab and deletes the workspace Start made for it. Meanwhile the base branch is pulled (fast-forward only,
 * PDT-56) so the main checkout has the merged work. Results toast.
 */
export async function finishMerged(confirm: Confirm, qc: QueryClient, t: Ticket) {
  const attempt = t.attempt;
  await patchTicket(qc, t.id, { stage: 'done' });
  const pulling = api.pullBranch(t.projectId, attempt ? { branch: attempt.baseBranch } : {}).catch(() => null);
  const report = async (show: (m: string) => void, message: string) => {
    const note = pullNote(await pulling);
    void qc.invalidateQueries({ queryKey: ['branches', t.projectId] });
    show(note.done ? `${message} ${note.done}` : message);
    if (note.problem) toast.info(note.problem);
  };
  if (!attempt) return report(toast.success, `${t.key} moved to Done.`);
  const worktreeId = attempt.worktreeId;
  const workspaces = await api.listWorkspaces();
  const workspace = attempt.workspaceId ? workspaces.find((w) => w.id === attempt.workspaceId) : undefined;
  const tabs = workspace?.tabs ?? [];
  const wsSessionIds = tabs.flatMap((tab) => (tab.sessionId ? [tab.sessionId] : []));
  const wsRunIds = new Set(tabs.flatMap((tab) => (tab.runId ? [tab.runId] : [])));

  // Everything still going: one question, then stop it all.
  const projectSessions = await api.listSessions(t.projectId);
  const wsSessions = (await Promise.all(wsSessionIds.map((id) => api.getSession(id).catch(() => null)))).filter((s) => s !== null);
  const running = new Set([
    ...t.sessions.filter((s) => s.attemptId === attempt.id && s.status === 'running').map((s) => s.id),
    ...projectSessions.filter((s) => worktreeId && s.worktreeId === worktreeId && s.status === 'running').map((s) => s.id),
    ...wsSessions.filter((s) => s.status === 'running').map((s) => s.id),
  ]);
  const runs = (await api.listActiveRuns()).filter((r) => (worktreeId && r.worktreeId === worktreeId) || wsRunIds.has(r.id));
  if (running.size > 0 || runs.length > 0) {
    const what = [running.size && plural(running.size, 'session'), runs.length && plural(runs.length, 'run')].filter(Boolean).join(' and ');
    const { ok } = await confirm({
      title: `Stop ${t.key}'s sessions?`,
      body: `${what} ${running.size + runs.length === 1 ? 'is' : 'are'} still running. Stop and remove ${running.size + runs.length === 1 ? 'it' : 'them'}, the worktree${workspace ? ' and the workspace' : ''}?`,
      action: 'Stop and remove',
      danger: true,
    });
    if (!ok) return report(toast.info, `${t.key} moved to Done; the worktree was kept.`);
    for (const id of running) await api.killSession(id).catch(ignoreMissing);
    for (const r of runs) await api.stopRun(r.id).catch(ignoreMissing);
  }

  if (worktreeId) {
    let force = false;
    for (;;) {
      try {
        // A stopped session's process takes a moment to exit, and the worktree stays busy until it has.
        await retryWhile('worktree_busy', () => api.removeWorktree(worktreeId, { force }));
        break;
      } catch (e) {
        if (isCode(e, 'worktree_dirty') && !force) {
          const { ok } = await confirm({
            title: 'The worktree has uncommitted changes',
            body: `${attempt.branch} has changes that were never committed. Removing the worktree discards them.`,
            action: 'Discard and remove',
            danger: true,
          });
          if (!ok) return report(toast.info, `${t.key} moved to Done; the worktree was kept.`);
          force = true;
          continue;
        }
        throw e;
      }
    }
  }

  // With a workspace of its own: its sessions are deleted (removing the worktree already deleted the ones in it), the
  // ticket's tab closes wherever it is, and the workspace goes with every tab in it.
  for (const s of wsSessions) await retryWhile('session_running', () => api.deleteSession(s.id)).catch(ignoreMissing);
  const ticketTab = workspaces.flatMap((w) => w.tabs).find((tab) => tab.ticketId === t.id);
  if (workspace && ticketTab && ticketTab.workspaceId !== workspace.id) await api.closeTab(ticketTab.id).catch(ignoreMissing);
  let workspaceRemoved = false;
  if (workspace) {
    workspaceRemoved = await api.deleteWorkspace(workspace.id, { closeTabs: true }).then(() => true, (e) => (ignoreMissing(e), false));
  }

  void qc.invalidateQueries({ queryKey: ['worktrees', t.projectId] });
  void qc.invalidateQueries({ queryKey: ['sessions'] });
  void qc.invalidateQueries({ queryKey: ['workspaces'] });
  const removed = [worktreeId && 'worktree', workspaceRemoved && 'workspace'].filter(Boolean);
  await report(
    toast.success,
    removed.length
      ? `${t.key} is done and its ${removed.join(' and ')} ${removed.length === 1 ? 'is' : 'are'} removed.`
      : `${t.key} moved to Done.`,
  );
}
