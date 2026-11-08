// Watches (PDT-319): the user's Watch toggle on a ticket or workflow run, and who watches it (the user, PM
// conversations) with a way to remove each. A watch tells the user in the Inbox's Updates (and on the desktop, by the
// `watch.event` notification's setting); a PM's tells its conversation. Every watch at once (PDT-555) marks watched
// tickets on the board and backlog and watching conversations in the PM view, from one request per poll.
import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { Eye, X } from 'lucide-react';
import { IconButton, WatchMark, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { WatchInfo } from '../api/gen/WatchInfo';
import { runAction } from './useAction';
import { POLL } from './poll';
import { listText } from './text';

/** A watched target: a ticket's id or a workflow run's. */
export type WatchOf = { ticket: string } | { run: string };

const watchesKey = (of: WatchOf) => ['watches', of] as const;
/** Every query of watches: a change refreshes a target's list and the all-watches one alike. */
const ALL_WATCH_QUERIES = ['watches'] as const;

export function useWatches(of: WatchOf) {
  // Polled with the rest of the UI: a PM's watch can start or end at any time.
  return useQuery({ queryKey: watchesKey(of), queryFn: () => api.watches.list(of), refetchInterval: POLL.live });
}

/**
 * Every watch that hasn't ended (the user's and the PM conversations'): one query however many cards or rows ask, so
 * a board makes one request per poll.
 */
export function useAllWatches() {
  return useQuery({ queryKey: ['watches', 'all'], queryFn: () => api.watches.list({}), refetchInterval: POLL.live });
}

/** "Watched by you and PM “Docs”". */
function watchedByText(watches: Pick<WatchInfo, 'watcher'>[]): string {
  const names = watches.map((w) => w.watcher);
  return `Watched by ${listText(names)}`;
}

/** Stops a watch and refreshes every list of them. */
export async function stopWatch(qc: QueryClient, id: string) {
  await api.watches.remove(id);
  await qc.invalidateQueries({ queryKey: ALL_WATCH_QUERIES });
}

/** An eye on a ticket's card or row when anything watches it (the user or a PM), naming who on hover. */
export function TicketWatchMark({ ticketId, className }: { ticketId: string; className?: string }) {
  const all = useAllWatches().data;
  const watches = all?.filter((w) => w.target === 'ticket' && w.targetId === ticketId) ?? [];
  if (watches.length === 0) return null;
  return <WatchMark title={watchedByText(watches)} className={className} />;
}

/** The user's own watch among a target's (a PM's has a session). */
function ownWatch(watches: WatchInfo[] | undefined): WatchInfo | null {
  return watches?.find((w) => w.sessionId === null) ?? null;
}

async function toggle(qc: QueryClient, of: WatchOf, own: WatchInfo | null) {
  if (own) await api.watches.remove(own.id);
  else await api.watches.create(of);
  await qc.invalidateQueries({ queryKey: ALL_WATCH_QUERIES });
}

/** Watch / Stop watching, for a menu (the phone's ⋯). */
export function watchEntry(qc: QueryClient, of: WatchOf, watches: WatchInfo[] | undefined): MenuEntry {
  const own = ownWatch(watches);
  return { label: own ? 'Stop watching' : 'Watch', onSelect: () => void runAction(() => toggle(qc, of, own)) };
}

/** The Watch toggle: pressed while the user watches. */
export function WatchButton({ of }: { of: WatchOf }) {
  const qc = useQueryClient();
  const own = ownWatch(useWatches(of).data);
  return (
    <IconButton
      label={own ? 'Stop watching' : 'Watch: hear when it finishes, fails or waits on you'}
      pressed={!!own}
      onClick={() => void runAction(() => toggle(qc, of, own))}
    >
      <Eye aria-hidden />
    </IconButton>
  );
}

/** "Watched by you · PM “Docs and website”", each with a remove button; nothing when nobody watches. */
export function Watchers({ of, className }: { of: WatchOf; className?: string }) {
  const qc = useQueryClient();
  const watches = useWatches(of).data ?? [];
  if (watches.length === 0) return null;
  const remove = (w: WatchInfo) => runAction(() => stopWatch(qc, w.id));
  return (
    <p aria-label="Watched by" className={cn('flex flex-wrap items-center gap-x-1 text-meta text-muted-foreground', className)}>
      <span>Watched by </span>
      {watches.map((w, i) => (
        <span key={w.id} className="inline-flex items-center">
          {i > 0 && <span aria-hidden className="mr-1">· </span>}
          {w.watcher}
          <IconButton size="sm" label={w.sessionId === null ? 'Stop watching' : `Stop ${w.watcher} watching`} onClick={() => void remove(w)}>
            <X aria-hidden />
          </IconButton>
        </span>
      ))}
    </p>
  );
}
