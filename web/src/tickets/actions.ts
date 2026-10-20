import { createContext, useContext } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import { toast, type ConfirmOptions, type ConfirmResult, type MenuEntry } from '@ui';
import { api, ApiError } from '../api/client';
import type { AttemptSummary } from '../api/gen/AttemptSummary';
import type { Ticket } from '../api/gen/Ticket';
import type { Priority } from '../api/gen/Priority';
import type { FinishRequest } from '../api/gen/FinishRequest';
import type { PullResult } from '../api/gen/PullResult';
import type { TicketPatch } from '../api/gen/TicketPatch';
import type { TicketStage } from '../api/gen/TicketStage';
import type { PartsProgress } from '../api/gen/PartsProgress';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { runAction } from '../common/useAction';
import { isClosed, nextPart, STAGES } from './model';
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

/** Start is offered for Draft, Ready and To do tickets with no active attempt; never for a parent (it starts its parts). */
export const canStart = (t: { stage: TicketStage; attempt: Pick<AttemptSummary, 'state'> | null; parts?: PartsProgress | null }) =>
  (t.stage === 'draft' || t.stage === 'ready' || t.stage === 'todo') && t.attempt?.state !== 'active' && !t.parts;

/**
 * "Start next part" on a parent: opens the Start dialog for its next part (see `nextPart`), or says why there is
 * none.
 */
export async function startNextPart(start: (t: { id: string; projectId: string }) => void, parent: { id: string; key: string }) {
  const p = await api.getTicket(parent.id);
  const next = nextPart(p.partList);
  if (typeof next === 'string') {
    toast.info(`${parent.key} has no part to start: ${next}.`);
    return;
  }
  start(next);
}

/**
 * Moves a ticket to another stage. A parent whose parts are still open can't close (409 `parts_open`): with
 * `confirm`, PDT offers to close the open parts as Won't do first, then moves the parent.
 */
export async function moveTicket(qc: QueryClient, t: Pick<TicketRef, 'id' | 'key'>, stage: TicketStage, confirm?: Confirm) {
  try {
    return await patchTicket(qc, t.id, { stage });
  } catch (e) {
    if (!(confirm && e instanceof ApiError && e.code === 'parts_open')) throw e;
    const p = await api.getTicket(t.id);
    const open = p.partList.filter((x) => !isClosed(x.stage));
    const { ok } = await confirm({
      title: `${t.key} still has open parts`,
      body: `${open.map((x) => `${x.key} ${x.title}`).join(', ')} ${open.length === 1 ? 'is' : 'are'} not closed. Close ${open.length === 1 ? 'it' : 'them'} as Won't do and move ${t.key} to ${STAGE_LABEL[stage]}?`,
      action: `Close ${open.length === 1 ? 'it' : 'them'} and move`,
    });
    if (!ok) return null;
    await api.bulkTickets({ ids: open.map((x) => x.id), action: 'stage', value: 'wont_do' });
    return patchTicket(qc, t.id, { stage });
  }
}

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

/** "Move to ›": every stage but the ticket's own (with `confirm`, closing a parent offers to close its parts). */
export function moveToEntries(qc: QueryClient, t: TicketRef, confirm?: Confirm): MenuEntry[] {
  return STAGES.filter((s) => s !== t.stage).map((s) => ({
    label: STAGE_LABEL[s],
    onSelect: () => void runAction(() => moveTicket(qc, t, s, confirm)),
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
  /** Adds Make into parts… (callers offer it only where `canMakeParts` allows). */
  onMakeParts?: () => void;
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
    ...(opts.onMakeParts ? [{ label: 'Make into parts…', onSelect: opts.onMakeParts }] : []),
    { submenu: 'Move to', entries: moveToEntries(qc, t, confirm) },
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

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? '' : 's'}`;

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
 * The merged banner's action, through the server's `ticket.finish` (PDT-76): moves the ticket to Done, stops and
 * removes the sessions and runs in its worktree and workspace, removes the worktree, deletes the workspace Start made
 * and pulls the base branch (fast-forward only, PDT-56). The server checks first and changes nothing while something
 * would be lost: running sessions or runs (asked once, then stopped) and uncommitted changes (asked, then discarded).
 * Declining either moves the ticket to Done and keeps the rest. Results toast. `deleteBranch` also deletes the branch
 * with the worktree (Merge…'s choice).
 */
export async function finishMerged(confirm: Confirm, qc: QueryClient, t: Ticket, opts: { deleteBranch?: boolean } = {}) {
  const req: FinishRequest = { stopRunning: false, discardChanges: false, deleteBranch: !!opts.deleteBranch, keepWorktree: false };
  const workspace = !!t.attempt?.workspaceId;
  let r = await api.finishTicket(t.id, req);
  while (r.outcome === 'blocked' && r.blocked) {
    const b = r.blocked;
    if (b.sessions + b.runs > 0) {
      const n = b.sessions + b.runs;
      const what = [b.sessions && plural(b.sessions, 'session'), b.runs && plural(b.runs, 'run')].filter(Boolean).join(' and ');
      const { ok } = await confirm({
        title: `Stop ${t.key}'s sessions?`,
        body: `${what} ${n === 1 ? 'is' : 'are'} still running. Stop and remove ${n === 1 ? 'it' : 'them'}, the worktree${workspace ? ' and the workspace' : ''}?`,
        action: 'Stop and remove',
        danger: true,
      });
      if (ok) req.stopRunning = true;
      else req.keepWorktree = true;
    } else if (b.dirty) {
      const { ok } = await confirm({
        title: 'The worktree has uncommitted changes',
        body: `${t.attempt?.branch ?? 'The branch'} has changes that were never committed. Removing the worktree discards them.`,
        action: 'Discard and remove',
        danger: true,
      });
      if (ok) req.discardChanges = true;
      else req.keepWorktree = true;
    } else break;
    r = await api.finishTicket(t.id, req);
  }
  invalidateTickets(qc, [t.id]);
  void qc.invalidateQueries({ queryKey: ['worktrees', t.projectId] });
  void qc.invalidateQueries({ queryKey: ['sessions'] });
  void qc.invalidateQueries({ queryKey: ['workspaces'] });
  void qc.invalidateQueries({ queryKey: ['branches', t.projectId] });
  if (r.branchNote) toast.info(r.branchNote);
  const removed = [r.worktreeRemoved && 'worktree', r.workspaceRemoved && 'workspace'].filter(Boolean);
  const message =
    r.outcome === 'kept'
      ? `${t.key} moved to Done; the worktree was kept.`
      : removed.length
        ? `${t.key} is done and its ${removed.join(' and ')} ${removed.length === 1 ? 'is' : 'are'} removed.`
        : `${t.key} moved to Done.`;
  const note = pullNote(r.pull);
  (r.outcome === 'kept' ? toast.info : toast.success)(note.done ? `${message} ${note.done}` : message);
  if (note.problem) toast.info(note.problem);
}
