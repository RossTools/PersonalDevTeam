import type { SyntheticEvent } from 'react';
import type { Session } from '../api/gen/Session';
import { TicketKey } from '../common/TicketKey';
import { useOpenTicket } from '../common/open';

const stop = (e: SyntheticEvent) => e.stopPropagation();

/**
 * The ticket key of a session started from a ticket (sidebar agent rows, tab titles); clicking it opens the ticket.
 * Presses stay on the chip, so the row or tab around it doesn't also act.
 */
export function SessionTicketChip({ session, className }: { session: Pick<Session, 'projectId' | 'ticket'> | undefined; className?: string }) {
  const open = useOpenTicket();
  const t = session?.ticket;
  if (!session || !t) return null;
  return (
    <span className={className} onPointerDown={stop} onPointerUp={stop} onMouseDown={stop} onKeyDown={stop} onClick={stop} onDoubleClick={stop}>
      <TicketKey ticketKey={t.key} title={`Open ${t.key}`} onClick={() => open({ id: t.id, projectId: session.projectId })} />
    </span>
  );
}
