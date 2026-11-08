// The links row under an Inbox item's title (PDT-361): its ticket, session, changes, pull request and runs.
import type { ReactNode } from 'react';
import { Button, DiffStat, Kbd, toast } from '@ui';
import type { InboxItem } from '../api/gen/InboxItem';
import { actionError } from '../common/useAction';
import { useOpenSession, useOpenTicket, useOpenChanges, openExternal } from '../common/open';
import { useChangeSize, useItemAttempt } from './itemAttempt';
import { inboxTargets } from './model';
import { capitalize } from '../common/text';
import { parseChoice, RunMenu, useRunChoices } from '../runs/RunMenu';
import { useStartRun } from '../runs/useStartRun';
import { prHref } from '../tickets/PullRequest';
import { useTicketTabs } from '../workspaces/ticketTabs';

/** A link the item's kind adds to the row (Storage settings, the workflow's versions). */
export interface ItemLink {
  label: string;
  /** The button's name when the label alone says too little. */
  'aria-label'?: string;
  onClick: () => void;
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
  // Only a ticket's worktree has a Run menu; other items (the PM's, whose project has no repository) ask nothing.
  const { choices } = useRunChoices(projectId, worktreeId, !!worktreeId);
  const start = useStartRun(projectId, (id) => tabs.openRun({ id, projectId }));
  const pr = item.changeRequest ? null : item.ticketId ? attempt?.pr ?? null : null;
  const run = (choice: string) => {
    const tab = tabs.placementNear(item.ticketId!);
    if (!worktreeId) return;
    start.mutate({ worktreeId, ...parseChoice(choice), ...(tab ? { tab } : {}) }, { onError: (err) => toast.error(actionError(err)) });
  };
  const targets = inboxTargets(item);
  const ownSession = item.kind !== 'session' ? targets.session : null;
  const persona = item.personaName ? `${item.personaName} session` : 'Session';
  const cr = item.changeRequest;
  const any = targets.ticket || ownSession || coder || worktreeId || cr || pr || extra.length > 0;
  if (!any) return null;
  return (
    <div role="group" aria-label="Links" className="flex flex-wrap items-center gap-1">
      {targets.ticket && (
        <LinkButton onClick={() => openTicket(targets.ticket!)}>
          Ticket <Kbd>o</Kbd>
        </LinkButton>
      )}
      {ownSession ? (
        <LinkButton onClick={() => openSession(ownSession)}>{persona}</LinkButton>
      ) : coder && item.ticketId && (
        <LinkButton aria-label="Open coder session" title={coder.title}
          onClick={() => tabs.openNear({ kind: 'session', id: coder.id }, projectId, item.ticketId!)}>
          {coder.persona ? `${coder.persona} session` : 'Session'}
        </LinkButton>
      )}
      {worktreeId && (
        <LinkButton aria-label="Open changes" onClick={() => openChanges({ id: worktreeId, projectId }, { ticketId: item.ticketId! })}>
          Changes
          {size.known && size.files > 0 && <DiffStat added={size.added} removed={size.removed} />}
        </LinkButton>
      )}
      {cr && (
        <LinkButton aria-label={`Open ${cr.name}`} onClick={() => openExternal(cr.url)}>
          {capitalize(cr.name)}
        </LinkButton>
      )}
      {pr && pr.url && (
        <LinkButton title={`#${pr.number} ${pr.title}`}
          onClick={() => openExternal(prHref(pr, pr.url))}>
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
