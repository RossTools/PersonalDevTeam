// The issues in other trackers a ticket is linked to (Phase 7e: the issue it was imported from; PDT-154: the forge
// issue a bug report came in as; PDT-481: issues linked by hand), beside its key, with the tracker's icon and the
// issue's state. Each tracker is a plugin, named as Settings → Plugins names it.
import { useQuery } from '@tanstack/react-query';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import { TicketKey } from '../common/TicketKey';
import type { ExternalLink } from '../api/gen/ExternalLink';
import { pluginsKey, type Plugin } from '../plugins/model';
import { PluginIcon } from '../plugins/parts';

/** An issue numbered within a repository (`owner/name#12`) shows as `#12`; the title names its repository. */
export function shortKey(l: ExternalLink): string {
  return l.key.includes('#') ? `#${l.key.split('#').pop()}` : l.key;
}

/** The plugins, for the trackers' names and icons. */
export function usePluginList(): Plugin[] | undefined {
  return useQuery({ queryKey: pluginsKey, queryFn: api.listIntegrations, staleTime: 60_000 }).data;
}

/** `GitHub issue o/r#12: Crash on start (closed)`. */
export function issueTitle(l: ExternalLink, plugin: Plugin | undefined): string {
  const head = `${plugin?.label ?? l.system} issue ${l.key}`;
  const state = l.state === 'closed' ? ' (closed)' : '';
  return l.title ? `${head}: ${l.title}${state}` : `${head}${state}`;
}

/** Each linked issue's key; with `linked`, a link to the issue (a board card is a button, so it can't hold links). */
export function ExternalKeys({ links, linked = false }: { links: ExternalLink[] | undefined; linked?: boolean }) {
  return links?.length ? <Keys links={links} linked={linked} /> : null;
}

function Keys({ links, linked }: { links: ExternalLink[]; linked: boolean }) {
  const plugins = usePluginList();
  return (
    <>
      {links.map((l) => {
        const plugin = plugins?.find((p) => p.id === l.system);
        return (
          <span key={`${l.system}:${l.key}`} className="inline-flex shrink-0 items-center gap-0.5">
            {plugin && <PluginIcon p={plugin} size="xs" />}
            <TicketKey
              ticketKey={shortKey(l)}
              href={linked ? l.url : undefined}
              title={issueTitle(l, plugin)}
              className={cn(l.state === 'closed' && 'text-muted-foreground line-through')}
            />
          </span>
        );
      })}
    </>
  );
}
