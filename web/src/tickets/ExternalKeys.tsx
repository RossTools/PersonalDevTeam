// The issues in other trackers a ticket is linked to (Phase 7e: the Jira issue it was imported from), beside its key.
import { TicketKey } from '../common/TicketKey';
import type { ExternalLink } from '../api/gen/ExternalLink';

/** Each linked issue's key; with `linked`, a link to the issue (a board card is a button, so it can't hold links). */
export function ExternalKeys({ links, linked = false }: { links: ExternalLink[] | undefined; linked?: boolean }) {
  if (!links?.length) return null;
  return (
    <>
      {links.map((l) => (
        <TicketKey
          key={`${l.system}:${l.key}`}
          ticketKey={l.key}
          href={linked ? l.url : undefined}
          title={`${l.system === 'jira' ? 'Jira' : l.system} issue ${l.key}`}
        />
      ))}
    </>
  );
}
