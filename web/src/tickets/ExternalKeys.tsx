// The issues in other trackers a ticket is linked to (Phase 7e: the Jira issue it was imported from; PDT-154: the GitHub
// issue a bug report came in as), beside its key.
import { TicketKey } from '../common/TicketKey';
import type { ExternalLink } from '../api/gen/ExternalLink';

const SYSTEMS: Record<string, string> = { jira: 'Jira', github: 'GitHub' };

/** A GitHub issue (`owner/name#12`) shows as `#12`; the title names its repository. */
function shortKey(l: ExternalLink): string {
  return l.system === 'github' && l.key.includes('#') ? `#${l.key.split('#').pop()}` : l.key;
}

/** Each linked issue's key; with `linked`, a link to the issue (a board card is a button, so it can't hold links). */
export function ExternalKeys({ links, linked = false }: { links: ExternalLink[] | undefined; linked?: boolean }) {
  if (!links?.length) return null;
  return (
    <>
      {links.map((l) => (
        <TicketKey
          key={`${l.system}:${l.key}`}
          ticketKey={shortKey(l)}
          href={linked ? l.url : undefined}
          title={`${SYSTEMS[l.system] ?? l.system} issue ${l.key}`}
        />
      ))}
    </>
  );
}
