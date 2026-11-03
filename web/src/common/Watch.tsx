// Watches (PDT-319): the user's Watch toggle on a ticket or workflow run, and who watches it (the user, PM
// conversations) with a way to remove each. A watch tells the user in the Inbox's Updates (and on the desktop, by the
// `watch.event` notification's setting); a PM's tells its conversation.
import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { Eye, X } from 'lucide-react';
import { IconButton, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { WatchInfo } from '../api/gen/WatchInfo';
import { runAction } from './useAction';
import { POLL } from './poll';

/** A watched target: a ticket's id or a workflow run's. */
export type WatchOf = { ticket: string } | { run: string };

const watchesKey = (of: WatchOf) => ['watches', of] as const;

export function useWatches(of: WatchOf) {
  // Polled with the rest of the UI: a PM's watch can start or end at any time.
  return useQuery({ queryKey: watchesKey(of), queryFn: () => api.watches.list(of), refetchInterval: POLL.live });
}

/** The user's own watch among a target's (a PM's has a session). */
export function ownWatch(watches: WatchInfo[] | undefined): WatchInfo | null {
  return watches?.find((w) => w.sessionId === null) ?? null;
}

async function toggle(qc: QueryClient, of: WatchOf, own: WatchInfo | null) {
  if (own) await api.watches.remove(own.id);
  else await api.watches.create(of);
  await qc.invalidateQueries({ queryKey: watchesKey(of) });
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
  const remove = (w: WatchInfo) => runAction(async () => {
    await api.watches.remove(w.id);
    await qc.invalidateQueries({ queryKey: watchesKey(of) });
  });
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
