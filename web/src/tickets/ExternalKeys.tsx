// The issues in other trackers a ticket is linked to (Phase 7e: the issue it was imported from; PDT-154: the forge
// issue a bug report came in as), beside its key. Each tracker is a plugin, named as Settings → Plugins names it.
import { useQuery } from '@tanstack/react-query';
import { api } from '../api/client';
import { TicketKey } from '../common/TicketKey';
import type { ExternalLink } from '../api/gen/ExternalLink';
import { pluginsKey } from '../plugins/model';

/** An issue numbered within a repository (`owner/name#12`) shows as `#12`; the title names its repository. */
function shortKey(l: ExternalLink): string {
  return l.key.includes('#') ? `#${l.key.split('#').pop()}` : l.key;
}

/** Each linked issue's key; with `linked`, a link to the issue (a board card is a button, so it can't hold links). */
export function ExternalKeys({ links, linked = false }: { links: ExternalLink[] | undefined; linked?: boolean }) {
  return links?.length ? <Keys links={links} linked={linked} /> : null;
}

function Keys({ links, linked }: { links: ExternalLink[]; linked: boolean }) {
  const plugins = useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations, staleTime: 60_000 });
  const label = (system: string) => plugins.data?.find((p) => p.id === system)?.label ?? system;
  return (
    <>
      {links.map((l) => (
        <TicketKey
          key={`${l.system}:${l.key}`}
          ticketKey={shortKey(l)}
          href={linked ? l.url : undefined}
          title={`${label(l.system)} issue ${l.key}`}
        />
      ))}
    </>
  );
}
