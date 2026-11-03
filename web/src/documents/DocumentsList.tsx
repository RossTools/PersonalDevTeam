import { useState } from 'react';
import { FileDown, Search, SquarePen, X } from 'lucide-react';
import { Button, Dialog, EmptyState, IconButton, Input, RelativeTime, Select, Skeleton } from '@ui';
import { cn } from '@ui/lib/utils';
import type { Document } from '../api/gen/Document';
import type { DocumentFilter } from '../api/gen/DocumentFilter';
import type { DocumentKind } from '../api/gen/DocumentKind';
import { ProjectKey } from '../common/BranchChip';
import { TicketKey } from '../common/TicketKey';
import { TicketPicker } from '../tickets/TicketPicker';
import { KindIcon } from './LinkedDocuments';
import { providerLabel, useDocumentProviders, useDocuments } from './queries';
import { useProjects } from '../projects/queries';
import { usePageLimit } from '../common/usePageLimit';
import { LoadError } from '../common/QueryStatus';

const ANY = '';
/** Documents loaded at a time; Show more loads the next ones. */
const PAGE = 100;
/** "Any" in a filter's Select (an empty value would read as none chosen). */
const ALL = 'all';
const fromSelect = <V extends string>(v: V | typeof ALL): V | typeof ANY => (v === ALL ? ANY : (v as V));

/** The list's choices, kept by the shell so the list and New agree: project, ticket, kind, provider and the search. */
export interface DocumentsFilters {
  project: string;
  setProject: (p: string) => void;
  /** A ticket's key or id (set from a ticket's link; cleared from the list). */
  ticket: string;
  setTicket: (t: string) => void;
  kind: DocumentKind | typeof ANY;
  setKind: (k: DocumentKind | typeof ANY) => void;
  provider: string;
  setProvider: (p: string) => void;
  q: string;
  setQ: (q: string) => void;
}

export function useDocumentsFilters(): DocumentsFilters {
  const [project, setProject] = useState(ANY);
  const [ticket, setTicket] = useState(ANY);
  const [kind, setKind] = useState<DocumentKind | typeof ANY>(ANY);
  const [provider, setProvider] = useState(ANY);
  const [q, setQ] = useState('');
  return { project, setProject, ticket, setTicket, kind, setKind, provider, setProvider, q, setQ };
}

/** The filter the API takes. */
function filterOf(f: DocumentsFilters): DocumentFilter {
  return {
    ...(f.project ? { project: f.project } : {}),
    ...(f.ticket ? { ticket: f.ticket } : {}),
    ...(f.kind ? { kind: f.kind } : {}),
    ...(f.provider ? { provider: f.provider } : {}),
    ...(f.q.trim() ? { q: f.q.trim() } : {}),
  };
}

function DocumentItem({ doc, provider, selected, onOpen }: { doc: Document; provider: string; selected: boolean; onOpen: (id: string) => void }) {
  return (
    <button
      type="button"
      aria-current={selected ? 'true' : undefined}
      onClick={() => onOpen(doc.id)}
      className={cn(
        'relative flex w-full flex-col gap-0.5 overflow-hidden rounded-md px-2.5 pt-2 pb-2.5 text-left outline-none',
        'hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring',
        selected && 'bg-accent before:absolute before:inset-y-0 before:left-0 before:w-0.5 before:bg-primary',
      )}
    >
      <span className="flex min-w-0 items-center gap-1.5">
        <KindIcon kind={doc.kind} />
        <span className="truncate text-sm font-medium">{doc.title}</span>
      </span>
      <span className="mt-0.5 flex min-w-0 flex-wrap items-center gap-1.5 text-meta text-muted-foreground">
        {doc.projects.map((p) => <ProjectKey key={p.id} projectKey={p.key} />)}
        {doc.tickets.slice(0, 3).map((t) => <TicketKey key={t.id} ticketKey={t.key} />)}
        <span>{provider}</span>
        <span aria-hidden>·</span>
        <RelativeTime time={doc.updatedAt} />
      </span>
    </button>
  );
}

/**
 * The Documents view's list column (PDT-314): New and Import, a search by title, filters (project, kind, provider;
 * a ticket's when opened from one), then the documents, the most recently changed first. On a phone it is the view's
 * screen.
 */
export function DocumentsList({ filters, selectedId, onOpen, onNew, onImport }: {
  filters: DocumentsFilters;
  selectedId: string | null;
  onOpen: (id: string) => void;
  onNew: () => void;
  onImport: () => void;
}) {
  const filter = filterOf(filters);
  const page = usePageLimit(PAGE, JSON.stringify(filter));
  const docs = useDocuments({ ...filter, limit: page.limit });
  const providers = useDocumentProviders();
  const projects = useProjects();
  const list = docs.data ?? [];
  const filtered = Object.keys(filter).length > 0;
  const none = providers.isSuccess && providers.data.providers.length === 0;
  const [pickingTicket, setPickingTicket] = useState(false);
  return (
    <div className="flex flex-col pb-2">
      <div className="flex items-center gap-1 pt-2.5 pr-1.5 pb-2 pl-3">
        <h2 className="flex-1 text-sm font-semibold tracking-tight">Documents</h2>
        <IconButton label="Import a document" onClick={onImport} disabled={none}>
          <FileDown aria-hidden />
        </IconButton>
        <IconButton label="New document" onClick={onNew} disabled={none}>
          <SquarePen aria-hidden />
        </IconButton>
      </div>
      <div className="relative mx-2">
        <Search aria-hidden className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-muted-foreground" />
        <Input
          type="search"
          aria-label="Search documents"
          placeholder="Search documents"
          value={filters.q}
          onChange={(e) => filters.setQ(e.target.value)}
          className="pl-8"
        />
      </div>
      <div className="mx-2 mt-2 flex flex-wrap items-center gap-1.5">
        <Select
          aria-label="Project"
          size="sm"
          quiet
          className="w-auto max-w-32"
          options={[{ value: ALL, label: 'All projects' }, ...(projects.data ?? []).map((p) => ({ value: p.id, label: p.name }))]}
          value={filters.project || ALL}
          onChange={(v) => filters.setProject(fromSelect(v))}
        />
        <Select<DocumentKind | typeof ALL>
          aria-label="Kind"
          size="sm"
          quiet
          className="w-auto"
          options={[{ value: ALL, label: 'Docs and sheets' }, { value: 'doc', label: 'Docs' }, { value: 'sheet', label: 'Sheets' }]}
          value={filters.kind || ALL}
          onChange={(v) => filters.setKind(fromSelect(v))}
        />
        {(providers.data?.providers.length ?? 0) > 1 && (
          <Select
            aria-label="Provider"
            size="sm"
            quiet
            className="w-auto"
            options={[{ value: ALL, label: 'Any provider' }, ...(providers.data?.providers ?? []).map((p) => ({ value: p.id, label: p.label }))]}
            value={filters.provider || ALL}
            onChange={(v) => filters.setProvider(fromSelect(v))}
          />
        )}
        {filters.ticket ? (
          <Button variant="secondary" size="sm" compact aria-label={`Linked to ${filters.ticket}; show documents of every ticket`} onClick={() => filters.setTicket(ANY)}>
            <span className="font-mono">{filters.ticket}</span> <X aria-hidden />
          </Button>
        ) : (
          <Button variant="ghost" size="sm" compact className="text-muted-foreground" onClick={() => setPickingTicket(true)}>Ticket…</Button>
        )}
      </div>
      {pickingTicket && (
        <Dialog onClose={() => setPickingTicket(false)} title="Documents linked to a ticket" size="md">
          <TicketPicker
            projectId={filters.project || null}
            label="Ticket"
            onPick={(t) => { filters.setTicket(t.key); setPickingTicket(false); }}
            className="max-h-[50dvh]"
          />
        </Dialog>
      )}
      {docs.isPending && <Skeleton lines={4} label="Loading documents" className="m-3" />}
      <LoadError what="Documents" error={docs.error} className="m-2" />
      {docs.isSuccess && list.length === 0 && (
        <EmptyState className="mt-6 border-0">
          {filtered ? 'No document matches.'
            : none ? 'No document provider yet. Turn on a documents plugin in Settings → Plugins.'
            : 'No documents yet. Make one with New, or import one.'}
        </EmptyState>
      )}
      {list.length > 0 && (
        <ul aria-label="Documents" className="mt-2 flex flex-col gap-px px-1">
          {list.map((d) => (
            <li key={d.id}>
              <DocumentItem doc={d} provider={providerLabel(providers.data?.providers, d.provider)} selected={d.id === selectedId} onOpen={onOpen} />
            </li>
          ))}
        </ul>
      )}
      {page.hasMore(list.length) && (
        <Button variant="ghost" size="sm" className="mx-2 mt-2" pending={docs.isPlaceholderData} onClick={page.more}>Show more</Button>
      )}
    </div>
  );
}
