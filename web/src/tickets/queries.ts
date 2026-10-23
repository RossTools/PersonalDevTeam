import { useEffect, useMemo, useRef } from 'react';
import { keepPreviousData, useInfiniteQuery, useQueries, useQuery, useQueryClient, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import { useSameList } from '../common/useSameList';
import type { Session } from '../api/gen/Session';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketQuery } from '../api/gen/TicketQuery';

/** How often what's on screen refetches (like sessions). */
export const POLL = 2000;

/**
 * A paged query's poll: TanStack refetches every loaded page, so past two pages (a long scroll back) it polls every
 * 15 s, so that a 1,000-row list isn't ten requests every 2 s.
 */
export const pagedPoll = (query: { state: { data?: { pages: unknown[] } } }) => ((query.state.data?.pages.length ?? 0) > 2 ? 15_000 : POLL);

export const ticketKeys = {
  /** Every list of tickets (backlog pages, counts, pickers). */
  lists: ['tickets'] as const,
  list: (q: TicketQuery) => ['tickets', 'list', q] as const,
  count: (projectId: string) => ['tickets', 'count', projectId] as const,
  search: (q: TicketQuery) => ['tickets', 'search', q] as const,
  one: (id: string) => ['ticket', id] as const,
  events: (id: string) => ['ticketEvents', id] as const,
  labels: (projectId: string) => ['labels', projectId] as const,
  areas: (projectId: string) => ['areas', projectId] as const,
};

/** After a write: every list, and the tickets it touched (their view, timeline). Labels may have changed too. */
export function invalidateTickets(qc: QueryClient, ids: string[] = []) {
  void qc.invalidateQueries({ queryKey: ticketKeys.lists });
  void qc.invalidateQueries({ queryKey: ['labels'] });
  for (const id of ids) {
    void qc.invalidateQueries({ queryKey: ticketKeys.one(id) });
    void qc.invalidateQueries({ queryKey: ticketKeys.events(id) });
  }
}

/**
 * The backlog: pages over the cursor. Loaded pages refetch every 2 s (every 15 s once more than two are loaded), and the previous filter's rows stay on screen while a new filter loads.
 */
export function useTicketPages(q: TicketQuery, enabled = true) {
  return useInfiniteQuery({
    queryKey: ticketKeys.list(q),
    enabled,
    queryFn: ({ pageParam }) => api.listTickets({ ...q, ...(pageParam ? { cursor: pageParam } : {}) }),
    initialPageParam: '' as string,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    refetchInterval: pagedPoll,
    placeholderData: keepPreviousData,
  });
}

/** A ticket key (`PDT-12`), as a path names a ticket by (an id otherwise). */
export const isTicketKey = (ref: string) => /^[A-Za-z][A-Za-z0-9]*-\d+$/.test(ref);

/**
 * The id of the ticket a path names, by key or id (PDT-111). Null while a key is being looked up, or when no ticket has
 * it (`missing`).
 */
export function useTicketId(ref: string | null): { id: string | null; missing: boolean } {
  const byKey = !!ref && isTicketKey(ref);
  const q = useQuery({
    queryKey: ['ticket-by-key', ref?.toUpperCase() ?? ''],
    queryFn: async () => (await api.listTickets({ q: ref!, limit: 5 })).items.find((t) => t.key.toUpperCase() === ref!.toUpperCase())?.id ?? null,
    enabled: byKey,
    // A key's ticket never changes, but a miss can (the ticket is made later, or the lookup ran before it synced).
    staleTime: (q) => (q.state.data ? Infinity : 0),
  });
  if (!ref) return { id: null, missing: false };
  if (!byKey) return { id: ref, missing: false };
  return { id: q.data ?? null, missing: (q.isSuccess && q.data === null) || q.isError };
}

export function useTicket(id: string | null) {
  return useQuery({
    queryKey: ticketKeys.one(id ?? ''),
    queryFn: () => api.getTicket(id!),
    enabled: !!id,
    refetchInterval: POLL,
  });
}

export function useLabels(projectId: string | null) {
  return useQuery({
    queryKey: ticketKeys.labels(projectId ?? ''),
    queryFn: () => api.projectLabels(projectId!),
    enabled: !!projectId,
    staleTime: 10_000,
  });
}

/** The project's configured areas (they change rarely: refetched on focus and once a minute). */
export function useProjectAreas(projectId: string | null) {
  return useQuery({
    queryKey: ticketKeys.areas(projectId ?? ''),
    queryFn: () => api.getProjectAreas(projectId!),
    enabled: !!projectId,
    staleTime: 30_000,
    refetchInterval: 60_000,
  });
}

/** Tickets matching `q` (key, number or words), for pickers and the palette. */
export function useTicketSearch(q: TicketQuery, enabled = true) {
  return useQuery({
    queryKey: ticketKeys.search(q),
    queryFn: () => api.listTickets(q),
    enabled,
    placeholderData: keepPreviousData,
    staleTime: 2000,
  });
}

/** Every project's sessions (the same queries the workspaces poll), for the attempt and board activity dots. */
export function useAllSessions(): Session[] {
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const qs = useQueries({
    queries: (projects.data ?? []).map((p) => ({ queryKey: ['sessions', p.id], queryFn: () => api.listSessions(p.id), refetchInterval: POLL })),
  });
  // The result arrays are new on every render; their data keeps its identity until a poll brings something new.
  // (Reading `dataUpdatedAt` here would re-render every caller on every poll.)
  const data = useSameList(qs.map((q) => q.data));
  return useMemo(() => data.flatMap((d) => d ?? []), [data]);
}

/**
 * Asks pdtd to check now whether the ticket's active attempt is merged (once per ticket per mount: the 60 s loop does
 * the rest) and keeps the answer in the ticket's cache.
 */
export function useCheckMerge(t: Pick<Ticket, 'id' | 'attempt'> | undefined) {
  const qc = useQueryClient();
  const checked = useRef(new Set<string>());
  const id = t?.id;
  const active = t?.attempt?.state === 'active';
  useEffect(() => {
    if (!id || !active || checked.current.has(id)) return;
    checked.current.add(id);
    api.checkMerge(id).then(
      (fresh) => {
        qc.setQueryData(ticketKeys.one(id), fresh);
        if (fresh.attempt?.state !== 'active') invalidateTickets(qc, [id]);
      },
      () => { /* The 60 s loop checks again; nothing to tell the user. */ },
    );
  }, [id, active, qc]);
}
