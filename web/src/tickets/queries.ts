import { keepPreviousData, useInfiniteQuery, useQuery, type QueryClient } from '@tanstack/react-query';
import { api } from '../api/client';
import type { TicketQuery } from '../api/gen/TicketQuery';

/** How often what's on screen refetches (like sessions). */
export const POLL = 2000;

export const ticketKeys = {
  /** Every list of tickets (backlog pages, counts, pickers). */
  lists: ['tickets'] as const,
  list: (q: TicketQuery) => ['tickets', 'list', q] as const,
  count: (projectId: string) => ['tickets', 'count', projectId] as const,
  search: (q: TicketQuery) => ['tickets', 'search', q] as const,
  one: (id: string) => ['ticket', id] as const,
  events: (id: string) => ['ticketEvents', id] as const,
  labels: (projectId: string) => ['labels', projectId] as const,
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
 * The backlog: pages over the cursor. Every loaded page refetches every 2 s (TanStack Query refetches an infinite
 * query page by page), and the previous filter's rows stay on screen while a new filter loads.
 */
export function useTicketPages(q: TicketQuery) {
  return useInfiniteQuery({
    queryKey: ticketKeys.list(q),
    queryFn: ({ pageParam }) => api.listTickets({ ...q, ...(pageParam ? { cursor: pageParam } : {}) }),
    initialPageParam: '' as string,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    refetchInterval: POLL,
    placeholderData: keepPreviousData,
  });
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
