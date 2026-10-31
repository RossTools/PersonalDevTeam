import type { Ticket } from '../api/gen/Ticket';
import { TicketDocumentsBody } from '../documents/LinkedDocuments';
import { OverviewSection } from './TicketParts';

/** The ticket's documents (PDT-314): its design doc and any other linked, with Create design doc, Link, New, Import. */
export function TicketDocuments({ ticket }: { ticket: Ticket }) {
  return (
    <TicketDocumentsBody
      ticket={{ id: ticket.id, key: ticket.key, projectId: ticket.projectId }}
      wrap={(actions, body) => <OverviewSection title="Documents" actions={actions}>{body}</OverviewSection>}
    />
  );
}
