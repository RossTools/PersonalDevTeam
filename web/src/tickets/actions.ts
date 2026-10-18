import { createContext, useContext } from 'react';
import type { QueryClient } from '@tanstack/react-query';
import type { ConfirmOptions, ConfirmResult, MenuEntry } from '@ui';
import { api } from '../api/client';
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
