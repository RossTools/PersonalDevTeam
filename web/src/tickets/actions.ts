import { createContext, useContext } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import { toast, type ConfirmOptions, type ConfirmResult, type MenuEntry } from '@ui';
import { api, ApiError } from '../api/client';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Ticket } from '../api/gen/Ticket';
import type { Priority } from '../api/gen/Priority';
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

/** Start is offered for Draft and Ready tickets with no active attempt. */
export const canStart = (t: { stage: TicketStage; attempt: Pick<AttemptSummary, 'state'> | null }) =>
  (t.stage === 'draft' || t.stage === 'ready') && t.attempt?.state !== 'active';

export async function patchTicket(qc: QueryClient, id: string, patch: TicketPatch) {
  const t = await api.updateTicket(id, patch);
  qc.setQueryData(['ticket', id], t);
  invalidateTickets(qc, [id]);
  // A ticket tab's title follows the ticket.
  void qc.invalidateQueries({ queryKey: ['workspaces'] });
  return t;
}

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
 * A ticket's menu (backlog row right-click / long press, the peek's and the tab's ⋯): Open, Move to ›, Priority ›,
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

/**
 * The merged banner's action: moves the ticket to Done, then removes the attempt's worktree. A busy worktree asks to
 * stop the attempt's running sessions first; a dirty one asks to discard its changes. Results toast.
 */
export async function finishMerged(confirm: Confirm, qc: QueryClient, t: Ticket) {
  const attempt = t.attempt;
  await patchTicket(qc, t.id, { stage: 'done' });
  const worktreeId = attempt?.worktreeId;
  if (!attempt || !worktreeId) {
    toast.success(`${t.key} moved to Done.`);
    return;
  }
  let force = false;
  let stopped = false;
  for (;;) {
    try {
      await api.removeWorktree(worktreeId, { force });
      break;
    } catch (e) {
      if (isCode(e, 'worktree_busy') && !stopped) {
        const { ok } = await confirm({
          title: 'The worktree is in use',
          body: `Sessions are still running in ${attempt.branch}. Stop its sessions and remove the worktree?`,
          action: 'Stop and remove',
          danger: true,
        });
        if (!ok) return void toast.info(`${t.key} moved to Done; the worktree was kept.`);
        stopped = true;
        const running = t.sessions.filter((s) => s.attemptId === attempt.id && s.status === 'running');
        for (const s of running) await api.killSession(s.id);
        continue;
      }
      if (isCode(e, 'worktree_dirty') && !force) {
        const { ok } = await confirm({
          title: 'The worktree has uncommitted changes',
          body: `${attempt.branch} has changes that were never committed. Removing the worktree discards them.`,
          action: 'Discard and remove',
          danger: true,
        });
        if (!ok) return void toast.info(`${t.key} moved to Done; the worktree was kept.`);
        force = true;
        continue;
      }
      throw e;
    }
  }
  void qc.invalidateQueries({ queryKey: ['worktrees', t.projectId] });
  void qc.invalidateQueries({ queryKey: ['sessions', t.projectId] });
  void qc.invalidateQueries({ queryKey: ['workspaces'] });
  toast.success(`${t.key} is done and its worktree is removed.`);
}
