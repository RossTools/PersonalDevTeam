// The merged banner above every tab of a ticket's workspace (PDT-71), so Move to Done is there from the session or
// run tab too, not only the ticket's.
import { MergedBanner } from './Attempt';
import { useTicket } from './queries';

export function WorkspaceMergedBanner({ ticketId }: { ticketId: string }) {
  const q = useTicket(ticketId);
  return q.data ? <MergedBanner ticket={q.data} /> : null;
}
