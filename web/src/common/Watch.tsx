// Watches (PDT-319): the user's Watch toggle on a ticket, workflow run or document (PDT-616), and who watches it (the user, PM
// conversations) with a way to remove each. A watch tells the user in the Inbox's Updates (and on the desktop, by the
// `watch.event` notification's setting); a PM's tells its conversation. Every watch at once (PDT-555) marks watched
// tickets on the board and backlog and watching conversations in the PM view, from one request per poll.
import { useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { Bell, BellRing, Check, X } from 'lucide-react';
import { Button, IconButton, WatchMark, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { WatchInfo } from '../api/gen/WatchInfo';
import { useKeepFocus } from './focusLost';
import { runAction } from './useAction';
import { POLL } from './poll';
import { listText } from './text';

/** A watched target: a ticket's id, a workflow run's or a document's. */
export type WatchOf = { ticket: string } | { run: string } | { document: string };

export const watchKeys = {
  /** Every query of watches: a change refreshes a target's list, a session's and the all-watches one alike. */
  all: ['watches'] as const,
  of: (of: WatchOf) => ['watches', of] as const,
  session: (session: string | null) => ['watches', { session }] as const,
  every: ['watches', 'all'] as const,
};

/** After a watch starts or stops: every list of them. */
export function invalidateWatches(qc: QueryClient) {
  return qc.invalidateQueries({ queryKey: watchKeys.all });
}

export function useWatches(of: WatchOf) {
  // Polled with the rest of the UI: a PM's watch can start or end at any time.
  return useQuery({ queryKey: watchKeys.of(of), queryFn: () => api.watches.list(of), refetchInterval: POLL.live });
}

/** A session's watches (a conversation's watch card), polled: the PM may stop one at any time. Off without one. */
export function useSessionWatches(session: string | null) {
  return useQuery({
    queryKey: watchKeys.session(session),
    queryFn: () => api.watches.list({ session: session! }),
    enabled: !!session,
    refetchInterval: POLL.live,
  });
}

/**
 * Every watch that hasn't ended (the user's and the PM conversations'): one query however many cards or rows ask, so
 * a board makes one request per poll.
 */
export function useAllWatches() {
  return useQuery({ queryKey: watchKeys.every, queryFn: () => api.watches.list({}), refetchInterval: POLL.live });
}

/** "Watched by you and PM “Docs”". */
function watchedByText(watches: Pick<WatchInfo, 'watcher'>[]): string {
  const names = watches.map((w) => w.watcher);
  return `Watched by ${listText(names)}`;
}

/** Stops a watch and refreshes every list of them. */
export async function stopWatch(qc: QueryClient, id: string) {
  await api.watches.remove(id);
  await invalidateWatches(qc);
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
  await invalidateWatches(qc);
}

/** Watch / Stop watching, for a menu (the phone's ⋯). */
export function watchEntry(qc: QueryClient, of: WatchOf, watches: WatchInfo[] | undefined): MenuEntry {
  const own = ownWatch(watches);
  return { label: own ? 'Stop watching' : 'Watch', onSelect: () => void runAction(() => toggle(qc, of, own)) };
}

/** What marks a target's Watch toggle in the page, for focus to find (`data-watch-toggle`). */
function toggleMark(of: WatchOf): string {
  return JSON.stringify(of);
}

/**
 * The Watch toggle, labelled (PDT-679): "Watch", or "Watching ✓" while the user watches. Its name says the state, so
 * it isn't also `aria-pressed` ("Watching, pressed").
 */
export function WatchButton({ of }: { of: WatchOf }) {
  const qc = useQueryClient();
  const own = ownWatch(useWatches(of).data);
  return (
    <Button
      variant="ghost"
      size="sm"
      title={own ? 'Stop watching' : 'document' in of ? 'Hear when it changes or someone comments' : 'Hear when it finishes, fails or waits on you'}
      data-watch-toggle={toggleMark(of)}
      onClick={() => void runAction(() => toggle(qc, of, own))}
    >
      {own ? <><BellRing aria-hidden /> Watching <Check aria-hidden /></> : <><Bell aria-hidden /> Watch</>}
    </Button>
  );
}

/**
 * "Watched by you · PM “Docs and website”", each with a remove button; nothing when nobody watches. `inline` puts it in
 * a meta line (after a `·`) instead of a line of its own.
 */
export function Watchers({ of, inline, className }: { of: WatchOf; inline?: boolean; className?: string }) {
  const qc = useQueryClient();
  const watches = useWatches(of).data ?? [];
  // A remove takes its own button away (the whole line, with the last): focus goes to the next watcher's, else the
  // target's Watch toggle when the page has one (PDT-668).
  const keep = useKeepFocus();
  if (watches.length === 0) return null;
  const remove = (w: WatchInfo, from: HTMLElement) => {
    const line = from.closest('[data-watchers]');
    const at = [...(line?.querySelectorAll('button') ?? [])].indexOf(from as HTMLButtonElement);
    keep(from, () => {
      const left = line?.isConnected ? [...line.querySelectorAll<HTMLElement>('button')] : [];
      return left[Math.min(at, left.length - 1)] ?? document.querySelector<HTMLElement>(`[data-watch-toggle="${CSS.escape(toggleMark(of))}"]`);
    });
    return runAction(() => stopWatch(qc, w.id));
  };
  const Line = inline ? 'span' : 'p';
  return (
    <>
      {inline && <span aria-hidden>·</span>}
      <Line role="group" aria-label="Watched by" data-watchers className={cn('flex flex-wrap items-center gap-x-1 text-meta text-muted-foreground', inline && 'inline-flex', className)}>
        <span>Watched by </span>
        {watches.map((w, i) => (
          <span key={w.id} className="inline-flex items-center">
            {i > 0 && <span aria-hidden className="mr-1">· </span>}
            {w.watcher}
            <IconButton size="sm" label={w.sessionId === null ? 'Stop watching' : `Stop ${w.watcher} watching`} onClick={(e) => void remove(w, e.target as HTMLElement)}>
              <X aria-hidden />
            </IconButton>
          </span>
        ))}
      </Line>
    </>
  );
}
