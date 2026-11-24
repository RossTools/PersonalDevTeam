import { useState } from 'react';
import { FileDown, ListFilter, Search, X } from 'lucide-react';
import { Button, Dialog, EmptyState, IconButton, Input, List, ListRow, MenuButton, RelativeTime, Skeleton, type MenuEntry } from '@ui';
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
import { ListColumnHeader } from '../common/ListColumnHeader';

const ANY = '';
/** Documents loaded at a time; Show more loads the next ones. */
const PAGE = 100;
const KINDS: { value: DocumentKind; label: string }[] = [{ value: 'doc', label: 'Docs' }, { value: 'sheet', label: 'Sheets' }];

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

function DocumentItem({ doc, provider, current }: { doc: Document; provider: string; current: boolean }) {
  return (
    <ListRow
      id={doc.id}
      textValue={doc.title}
      leading={<KindIcon kind={doc.kind} />}
      title={<span className="font-medium">{doc.title}</span>}
      current={current}
      className="px-2.5 pt-2 pb-2.5"
      details={
        <span className="mt-0.5 flex min-w-0 flex-wrap items-center gap-1.5 text-meta">
          {doc.projects.map((p) => <ProjectKey key={p.id} projectKey={p.key} />)}
          {doc.tickets.slice(0, 3).map((t) => <TicketKey key={t.id} ticketKey={t.key} />)}
          <span>{provider}</span>
          <span aria-hidden>·</span>
          <RelativeTime time={doc.updatedAt} />
        </span>
      }
    />
  );
}

/**
 * The Documents view's list column (PDT-314): New and Import, a search by title and one Filter menu (project, kind,
 * provider, ticket) with the filters in force as removable chips beside it (PDT-681), then the documents, the most recently changed first. On a phone it is the view's
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
  const providerList = providers.data?.providers ?? [];
  const projectName = (pid: string) => projects.data?.find((p) => p.id === pid)?.name ?? pid;
  // The filters in force, as chips that clear them.
  const active: { key: string; label: string; clear: () => void }[] = [
    ...(filters.project ? [{ key: 'project', label: projectName(filters.project), clear: () => filters.setProject(ANY) }] : []),
    ...(filters.kind ? [{ key: 'kind', label: KINDS.find((k) => k.value === filters.kind)?.label ?? filters.kind, clear: () => filters.setKind(ANY) }] : []),
    ...(filters.provider ? [{ key: 'provider', label: providerLabel(providerList, filters.provider), clear: () => filters.setProvider(ANY) }] : []),
    ...(filters.ticket ? [{ key: 'ticket', label: filters.ticket, clear: () => filters.setTicket(ANY) }] : []),
  ];
  const filterEntries = (): MenuEntry[] => [
    {
      submenu: 'Project',
      entries: [
        { label: 'All projects', checked: !filters.project, onSelect: () => filters.setProject(ANY) },
        ...(projects.data ?? []).map((p) => ({ label: p.name, checked: filters.project === p.id, onSelect: () => filters.setProject(p.id) })),
      ],
    },
    {
      submenu: 'Kind',
      entries: [
        { label: 'Docs and sheets', checked: !filters.kind, onSelect: () => filters.setKind(ANY) },
        ...KINDS.map((k) => ({ label: k.label, checked: filters.kind === k.value, onSelect: () => filters.setKind(k.value) })),
      ],
    },
    ...(providerList.length > 1 ? [{
      submenu: 'Provider',
      entries: [
        { label: 'Any provider', checked: !filters.provider, onSelect: () => filters.setProvider(ANY) },
        ...providerList.map((p) => ({ label: p.label, checked: filters.provider === p.id, onSelect: () => filters.setProvider(p.id) })),
      ],
    }] : []),
    { label: 'Ticket…', detail: filters.ticket || undefined, onSelect: () => setPickingTicket(true) },
  ];
  return (
    <div className="flex flex-col pb-2">
      <ListColumnHeader
        title="Documents"
        newLabel="New document"
        onNew={onNew}
        newDisabled={none}
        extra={(
          <IconButton label="Import a document" size="sm" onClick={onImport} disabled={none}>
            <FileDown aria-hidden />
          </IconButton>
        )}
      />
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
        <MenuButton label="Filter documents" entries={filterEntries}>
          <Button variant="ghost" size="sm" compact className="text-muted-foreground">
            <ListFilter aria-hidden /> Filter{active.length > 0 ? ` · ${active.length}` : ''}
          </Button>
        </MenuButton>
        {active.map((c) => (
          <span key={c.key} className="inline-flex max-w-full min-w-0 items-center gap-1 rounded-sm bg-accent py-0.5 pr-0.5 pl-1.5 text-xs">
            <span className="min-w-0 truncate">{c.label}</span>
            <IconButton label={`Clear the filter ${c.label}`} size="sm" variant="ghost" className="size-5" onClick={c.clear}><X aria-hidden /></IconButton>
          </span>
        ))}
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
        <List aria-label="Documents" onAction={onOpen} className="mt-2 gap-px px-1">
          {list.map((d) => (
            <DocumentItem key={d.id} doc={d} provider={providerLabel(providers.data?.providers, d.provider)} current={d.id === selectedId} />
          ))}
        </List>
      )}
      {page.hasMore(list.length) && (
        <Button variant="ghost" size="sm" className="mx-2 mt-2" pending={docs.isPlaceholderData} onClick={page.more}>Show more</Button>
      )}
    </div>
  );
}
