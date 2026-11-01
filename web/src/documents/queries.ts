// Documents (PDT-314): their queries, polled with the rest of the UI, and what invalidates them.
import { useQuery, type QueryClient } from '@tanstack/react-query';
import { api, ApiError } from '../api/client';
import type { DocumentFilter } from '../api/gen/DocumentFilter';
import type { DocumentKind } from '../api/gen/DocumentKind';
import type { ProviderInfo } from '../api/gen/ProviderInfo';
import { routePath } from '../shell/route';
import { navigate } from '../shell/router';

const POLL = 2000;

export const documentKeys = {
  all: ['documents'] as const,
  list: (f: DocumentFilter) => ['documents', 'list', f] as const,
  one: (id: string) => ['documents', 'one', id] as const,
  content: (id: string) => ['documents', 'content', id] as const,
  comments: (id: string) => ['documents', 'comments', id] as const,
  charts: (id: string) => ['documents', 'charts', id] as const,
  version: (id: string) => ['documents', 'version', id] as const,
  providers: (project: string | null) => ['documents', 'providers', project] as const,
};

export const KIND_LABEL: Record<DocumentKind, string> = { doc: 'Doc', sheet: 'Sheet' };

/** The documents a filter lets through, the most recently changed first. */
export function useDocuments(f: DocumentFilter, enabled = true) {
  return useQuery({ queryKey: documentKeys.list(f), queryFn: () => api.documents(f), refetchInterval: POLL, enabled });
}

/** One document (null once it is gone). */
export function useDocument(id: string | null) {
  return useQuery({
    queryKey: documentKeys.one(id ?? ''),
    queryFn: () => api.document(id!).catch((e: unknown) => {
      if (e instanceof ApiError && e.status === 404) return null;
      throw e;
    }),
    enabled: id !== null,
    refetchInterval: POLL,
  });
}

/** A document's content, read from its provider (not polled: providers may be slow or rate-limited). */
export function useDocumentContent(id: string) {
  return useQuery({ queryKey: documentKeys.content(id), queryFn: () => api.documentContent(id), retry: false });
}

/** A document's version now, every 15 s while `enabled` (cheaper than its content: an open page reads that again when
 * this changes). */
export function useDocumentVersion(id: string, enabled: boolean) {
  return useQuery({ queryKey: documentKeys.version(id), queryFn: () => api.documentVersion(id), refetchInterval: 15_000, retry: false, enabled });
}

/** The providers this host has, and where new documents (in `project`) go. They change only with plugins. */
export function useDocumentProviders(project: string | null = null) {
  return useQuery({ queryKey: documentKeys.providers(project), queryFn: () => api.documentProviders(project), staleTime: 30_000 });
}

export const providerLabel = (providers: ProviderInfo[] | undefined, id: string) =>
  providers?.find((p) => p.id === id)?.label ?? id;

export function invalidateDocuments(qc: QueryClient) {
  void qc.invalidateQueries({ queryKey: documentKeys.all });
}

/** Shows a document in the Documents view. */
export function openDocument(id: string) {
  navigate(routePath({ view: 'documents', doc: id }));
}

/** A document's comments (refetched when its event stream says they changed, and with the rest of the UI). */
export function useDocumentComments(id: string, enabled: boolean) {
  return useQuery({ queryKey: documentKeys.comments(id), queryFn: () => api.documentComments(id), enabled, refetchInterval: POLL, retry: false });
}

/** A local sheet's charts with their numbers (refetched after every change). */
export function useDocumentCharts(id: string, enabled: boolean) {
  return useQuery({ queryKey: documentKeys.charts(id), queryFn: () => api.documentCharts(id), enabled, retry: false });
}
