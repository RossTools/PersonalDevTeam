// The ticket attempt an Inbox item is about, and the size of its changes: for the links row and the phone's merge check.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import { changesKey } from '../review/queries';
import { useTicket } from '../tickets/queries';

/** The ticket attempt an item is about (its own, else the ticket's active one) and the attempt's agent session. */
export function useItemAttempt(item: InboxItem) {
  const ticket = useTicket(item.ticketId);
  const t = ticket.data;
  const attempt = t?.attempts.find((x) => x.id === item.attemptId) ?? t?.attempts.find((x) => x.state === 'active') ?? null;
  // The attempt's agent session (the Coder's): the newest one, running ones first.
  const mine = (t?.sessions ?? []).filter((x) => attempt && x.attemptId === attempt.id);
  const coder = mine.find((x) => x.status === 'running') ?? mine[0] ?? null;
  return { ticket, attempt, coder };
}

/** The size of a worktree's changes: files, lines added and removed. */
export function useChangeSize(worktreeId: string | null) {
  const changes = useQuery({ queryKey: changesKey(worktreeId ?? ''), queryFn: () => api.getChanges(worktreeId!), enabled: !!worktreeId });
  const files = changes.data?.files ?? [];
  return {
    loading: changes.isLoading,
    known: !!changes.data,
    files: files.length,
    added: files.reduce((n, f) => n + f.added, 0),
    removed: files.reduce((n, f) => n + f.removed, 0),
  };
}
