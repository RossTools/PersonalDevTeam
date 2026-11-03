// The links row under an Inbox item's title (PDT-361): its ticket, session, changes, pull request and runs.
import type { ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { Button, Kbd, toast } from '@ui';
import { api } from '../api/client';
import type { InboxItem } from '../api/gen/InboxItem';
import { actionError } from '../common/useAction';
import { useOpenSession, useOpenTicket } from '../tickets/actions';
import { useOpenChanges } from '../review/open';
import { parseChoice, RunMenu, useRunChoices } from '../runs/RunMenu';
import { useStartRun } from '../runs/useStartRun';
import { prHref } from '../tickets/PullRequest';
import { useTicket } from '../tickets/queries';
import { useTicketTabs } from '../workspaces/ticketTabs';
import { changesKey } from '../review/queries';

/** A link the item's kind adds to the row (Storage settings, the workflow's versions). */
export interface ItemLink {
  label: string;
  /** The button's name when the label alone says too little. */
  'aria-label'?: string;
  onClick: () => void;
}

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

const link = 'h-6 gap-1.5 px-2 text-xs font-normal text-muted-foreground hover:text-foreground';

function LinkButton({ children, onClick, title, 'aria-label': ariaLabel }: { children: ReactNode; onClick: () => void; title?: string; 'aria-label'?: string }) {
  return <Button size="sm" variant="secondary" className={link} title={title} aria-label={ariaLabel} onClick={onClick}>{children}</Button>;
}

/**
 * Ticket (`o`), the item's session (else the attempt's agent session), the attempt's changes with their size, its
 * pull request (a gate's own one first) or local CI log, Run… over the runs and tests its worktree offers, and the
 * kind's own `extra` links.
 */
export function ItemLinks({ item, extra = [] }: { item: InboxItem; extra?: ItemLink[] }) {
  const openTicket = useOpenTicket();
  const openSession = useOpenSession();
  const openChanges = useOpenChanges();
  const tabs = useTicketTabs();
  const { attempt, coder } = useItemAttempt(item);
  const worktreeId = item.ticketId ? attempt?.worktreeId ?? null : null;
  const projectId = item.projectId ?? '';
  const size = useChangeSize(worktreeId);
  const { choices } = useRunChoices(projectId, worktreeId);
  const start = useStartRun(projectId, (id) => tabs.openRun({ id, projectId }));
  const pr = item.changeRequest ? null : item.ticketId ? attempt?.pr ?? null : null;
  const run = (choice: string) => {
    const tab = tabs.placementNear(item.ticketId!);
    if (!worktreeId) return;
    start.mutate({ worktreeId, ...parseChoice(choice), ...(tab ? { tab } : {}) }, { onError: (err) => toast.error(actionError(err)) });
  };
  const ownSession = item.sessionId && item.projectId && item.kind !== 'session';
  const persona = item.personaName ? `${item.personaName} session` : 'Session';
  const cr = item.changeRequest;
  const any = (item.ticketId && item.projectId) || ownSession || coder || worktreeId || cr || pr || extra.length > 0;
  if (!any) return null;
  return (
    <div role="group" aria-label="Links" className="flex flex-wrap items-center gap-1">
      {item.ticketId && item.projectId && (
        <LinkButton aria-label="Open ticket" onClick={() => openTicket({ id: item.ticketId!, projectId: item.projectId! })}>
          Ticket <Kbd>o</Kbd>
        </LinkButton>
      )}
      {ownSession ? (
        <LinkButton aria-label="Open session" onClick={() => openSession({ id: item.sessionId!, projectId: item.projectId! })}>{persona}</LinkButton>
      ) : coder && item.ticketId && (
        <LinkButton aria-label="Open coder session" title={coder.title}
          onClick={() => tabs.openNear({ kind: 'session', id: coder.id }, projectId, item.ticketId!)}>
          {coder.persona ? `${coder.persona} session` : 'Session'}
        </LinkButton>
      )}
      {worktreeId && (
        <LinkButton aria-label="Open changes" onClick={() => openChanges({ id: worktreeId, projectId }, { ticketId: item.ticketId! })}>
          Changes
          {size.known && size.files > 0 && (
            <span className="font-mono tabular-nums">
              <span className="text-diff-add-sign">+{size.added}</span> <span className="text-diff-del-sign">−{size.removed}</span>
            </span>
          )}
        </LinkButton>
      )}
      {cr && (
        <LinkButton aria-label={`Open ${cr.name}`} onClick={() => window.open(cr.url, '_blank', 'noopener')}>
          {cr.name.charAt(0).toUpperCase() + cr.name.slice(1)}
        </LinkButton>
      )}
      {pr && pr.url && (
        <LinkButton aria-label={pr.local ? 'Open CI log' : 'Open PR'} title={`#${pr.number} ${pr.title}`}
          onClick={() => window.open(prHref(pr, pr.url), '_blank', 'noopener')}>
          {pr.local ? 'CI log' : `PR #${pr.number}`}
        </LinkButton>
      )}
      {worktreeId && choices.length > 0 && (
        <RunMenu label="Run the ticket's branch" choices={choices} value="" onChange={run} disabled={start.isPending} className="w-28" />
      )}
      {extra.map((l) => <LinkButton key={l.label} aria-label={l['aria-label']} onClick={l.onClick}>{l.label}</LinkButton>)}
    </div>
  );
}
