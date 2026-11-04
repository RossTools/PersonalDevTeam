import { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, Cloud, HardDrive } from 'lucide-react';
import {
  Alert, Badge, Button, CommandList, Dialog, DialogActions, DropZone, EmptyState, Field, Input, Kbd, Link, MenuButton,
  RelativeTime, SectionLabel, SegmentedControl, Select, SourceCard, Spinner, toast, useFileDrop, type CommandItem,
  type FileDrag,
} from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { Connection } from '../api/gen/Connection';
import type { Document } from '../api/gen/Document';
import type { DocumentKind } from '../api/gen/DocumentKind';
import type { Project } from '../api/gen/Project';
import type { ProviderPicker } from '../api/gen/ProviderPicker';
import type { SignInAccount } from '../api/gen/SignInAccount';
import { actionError, useFormAction } from '../common/useAction';
import { ConnectionDialog } from '../connections/ConnectionDialog';
import { useConnections } from '../connections/queries';
import { useSignInAccounts } from '../plugins/AccountSignIn';
import { pickFiles } from './picker';
import { useMediaQuery } from '../common/useMediaQuery';
import { useProjects } from '../projects/queries';
import { KindIcon } from './LinkedDocuments';
import { KIND_LABEL, invalidateDocuments, useDocumentProviders, useDocuments } from './queries';

const KINDS: { value: DocumentKind; label: string }[] = [
  { value: 'doc', label: 'Doc' },
  { value: 'sheet', label: 'Sheet' },
];

/** No project: the document is linked to none (host-wide). */
const NO_PROJECT = '';

/** Waits until typing pauses before asking the API. */
function useDebounced<T>(value: T, ms = 250): T {
  const [v, setV] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setV(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return v;
}

/**
 * New document: a title, Doc or Sheet, the provider (the project's default first) and the project it is linked to.
 * With a ticket, it is linked to the ticket (and its project) and the project can't change.
 */
export function NewDocumentDialog({ projects, projectId = null, ticket, onClose, onCreated }: {
  projects: Project[];
  projectId?: string | null;
  ticket?: { id: string; key: string; projectId: string };
  onClose: () => void;
  onCreated: (d: Document) => void;
}) {
  const qc = useQueryClient();
  const [title, setTitle] = useState('');
  const [kind, setKind] = useState<DocumentKind>('doc');
  const [project, setProject] = useState<string>(ticket?.projectId ?? projectId ?? NO_PROJECT);
  const providers = useDocumentProviders(project || null);
  const [provider, setProvider] = useState<string | null>(null);
  const makers = (providers.data?.providers ?? []).filter((p) => p.capabilities.create && (kind === 'doc' ? p.capabilities.docs : p.capabilities.sheets));
  const chosen = provider && makers.some((p) => p.id === provider) ? provider
    : makers.find((p) => p.id === providers.data?.default)?.id ?? makers[0]?.id ?? null;
  const save = useFormAction(async () => {
    if (!title.trim() || !chosen) return;
    const d = await api.createDocument({
      title,
      kind,
      provider: chosen,
      ...(ticket ? { ticketId: ticket.id } : project ? { projectId: project } : {}),
    });
    invalidateDocuments(qc);
    onCreated(d);
  });
  const create = save.run;
  const none = providers.isSuccess && makers.length === 0;
  return (
    <Dialog
      onClose={onClose}
      title={ticket ? `New document for ${ticket.key}` : 'New document'}
      size="sm"
      footer={<DialogActions onCancel={onClose} action="Create" pending={save.pending} disabled={!title.trim() || !chosen} onAction={() => void create()} />}
    >
      <form className="flex flex-col gap-3" onSubmit={(e) => { e.preventDefault(); void create(); }}>
        <Field label="Title">
          <Input autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Q4 plan" />
        </Field>
        <Field label="Kind">
          <SegmentedControl size="sm" options={KINDS} value={kind} onChange={setKind} />
        </Field>
        <Field label="Provider" hint={providers.data && chosen === 'local' ? `Saved in ${providers.data.folder}` : undefined}>
          <Select
            options={makers.map((p) => ({ value: p.id, label: p.label }))}
            value={chosen}
            onChange={setProvider}
            placeholder="No provider"
            disabled={makers.length === 0}
          />
        </Field>
        {!ticket && (
          <Field label="Project">
            <Select
              options={[{ value: NO_PROJECT, label: 'None' }, ...projects.map((p) => ({ value: p.id, label: p.name }))]}
              value={project}
              onChange={setProject}
            />
          </Field>
        )}
        {none && (
          <Alert tone="info">
            No document provider here makes {kind === 'doc' ? 'docs' : 'sheets'}. Turn on a documents plugin in Settings → Plugins.
          </Alert>
        )}
        {save.error && <Alert tone="error">{save.error}</Alert>}
      </form>
    </Dialog>
  );
}

/** What a dragged or chosen file becomes, by its extension or, while dragged (no name yet), its type. */
function becomes(f: FileDrag): DocumentKind | null {
  const ext = f.name?.includes('.') ? f.name.split('.').pop()!.toLowerCase() : null;
  if (ext) return ['md', 'markdown', 'txt'].includes(ext) ? 'doc' : ['xlsx', 'csv', 'tsv'].includes(ext) ? 'sheet' : null;
  if (f.type === 'text/markdown' || f.type === 'text/plain') return 'doc';
  if (f.type === 'text/csv' || f.type === 'text/tab-separated-values' || f.type.includes('spreadsheetml')) return 'sheet';
  return null;
}

function dropText(f: FileDrag) {
  const kind = becomes(f);
  const name = f.name ? <strong className="font-medium">{f.name}</strong> : 'it';
  return kind ? <>Drop to import {name} as a {kind === 'doc' ? 'doc' : 'sheet'}</> : <>Drop to import {name}</>;
}

function ProviderIcon({ id }: { id: string }) {
  return id === 'local' ? <HardDrive /> : <Cloud />;
}

/** A provider's picker: a button for one account, a menu of them for several. */
function BrowseButton({ label, accounts, onPick, variant = 'primary', size = 'md', autoFocus }: {
  label: string;
  accounts: SignInAccount[];
  onPick: (connectionId: string) => void;
  variant?: 'primary' | 'secondary';
  size?: 'sm' | 'md';
  /** Back from the picker: this is where focus was. */
  autoFocus?: boolean;
}) {
  if (accounts.length === 1) {
    return <Button variant={variant} size={size} autoFocus={autoFocus} onClick={() => onPick(accounts[0].connectionId)}>{label}</Button>;
  }
  return (
    <MenuButton
      label={label}
      entries={() => accounts.map((a) => ({ label: a.email ?? a.name, onSelect: () => onPick(a.connectionId) }))}
    >
      <Button variant={variant} size={size} autoFocus={autoFocus}>{label} <ChevronDown aria-hidden /></Button>
    </MenuButton>
  );
}

/**
 * Import or link: pick a provider that can search, then one of its files; PDT records it (or finds the record it has)
 * and links it to the project or ticket. Or a file from the computer (`.md`, `.xlsx`, `.csv`), made a local document:
 * dropped anywhere on the dialog, or chosen.
 */
export function ImportDocumentDialog({ projectId = null, ticket, onClose, onImported }: {
  projectId?: string | null;
  ticket?: { id: string; key: string; projectId: string };
  onClose: () => void;
  onImported: (d: Document) => void;
}) {
  const qc = useQueryClient();
  const into = ticket?.projectId ?? projectId;
  const providers = useDocumentProviders(into);
  const projects = useProjects({ enabled: into !== null });
  const projectName = projects.data?.find((p) => p.id === into)?.name;
  const searchers = (providers.data?.providers ?? []).filter((p) => p.capabilities.search);
  const [provider, setProvider] = useState<string | null>(null);
  const chosen = provider ?? searchers.find((p) => p.id === providers.data?.default)?.id ?? searchers[0]?.id ?? null;
  const source = searchers.find((p) => p.id === chosen);
  const [query, setQuery] = useState('');
  const q = useDebounced(query.trim());
  const files = useQuery({
    queryKey: ['documents', 'provider-files', chosen, q],
    queryFn: () => api.providerFiles(chosen!, q),
    enabled: chosen !== null,
  });
  // Uploading, importing a found file and the provider's picker share one error line.
  const act = useFormAction((work: () => Promise<void>) => work());
  const [uploading, setUploading] = useState(false);
  // The found file being imported: its row shows a spinner.
  const [importing, setImporting] = useState<string | null>(null);
  const hasLocal = (providers.data?.providers ?? []).some((p) => p.id === 'local');
  const upload = (file: File) => act.run(async () => {
    setUploading(true);
    try {
      const r = await api.uploadDocument(file, ticket ? { ticketId: ticket.id } : { projectId });
      invalidateDocuments(qc);
      if (r.dropped.length > 0) {
        toast.info(`“${r.document.title}” is imported without its ${r.dropped.join(', ')}: PDT's sheets don't keep those.`);
      }
      onImported(r.document);
    } finally {
      setUploading(false);
    }
  });
  // Dropping a file anywhere on the dialog drops it into the zone.
  const fileDrop = useFileDrop((f) => void upload(f), !hasLocal || uploading);
  // The provider's own file picker, for each signed-in account that opens it.
  const picker = source?.picker ?? null;
  const accounts = useSignInAccounts(picker?.accounts ?? null);
  const pickers = (accounts.data ?? []).filter((a) => a.signedIn && a.picker);
  // What its search can't find (Google's drive.file: only files PDT made or opened), and where those are.
  const searchNote = picker?.searchNote ?? null;
  const browseNote = picker && pickers.length > 0 ? `${picker.label} for the rest.` : null;
  // A provider's picker covers the page: the dialog steps aside while it is open.
  const [picking, setPicking] = useState(false);
  // Which Browse button opened it: focus goes back there, not to the search, when the dialog returns (PDT-547).
  const [pickedFrom, setPickedFrom] = useState<'primary' | 'secondary' | null>(null);
  // Once: a Browse button shown later (an empty search's) mustn't take focus from the search.
  useEffect(() => {
    if (!picking && pickedFrom) setPickedFrom(null);
  }, [picking, pickedFrom]);
  // The keyboard hints, only where there is a keyboard: not on phones or touch screens.
  const touch = useMediaQuery('(max-width: 639.98px), (pointer: coarse)');
  const found = files.data ?? [];
  const items: CommandItem[] = found.map((f) => {
    const onTicket = !!ticket && f.ticketIds.includes(ticket.id);
    return {
      id: f.reference,
      label: f.title,
      icon: <span className="flex size-7 shrink-0 items-center justify-center rounded-md bg-muted"><KindIcon kind={f.kind} /></span>,
      description: f.modified !== null
        ? <>{KIND_LABEL[f.kind]} · edited <RelativeTime time={f.modified} /></>
        : KIND_LABEL[f.kind],
      trailing: (
        <>
          {importing === f.reference && <Spinner aria-label="Importing" />}
          {onTicket ? <Badge tone="running">On this ticket</Badge> : f.documentId !== null && <Badge tone="accent">In PDT</Badge>}
          {f.url && <Link href={f.url} external className="text-xs" aria-label={`Open ${f.title} in ${source?.label ?? 'its provider'}`}>Open</Link>}
        </>
      ),
      keywords: query ? [query] : [],
    };
  });
  const importOne = (provider: string, reference: string) => api.importDocument({
    provider,
    reference,
    ...(ticket ? { ticketId: ticket.id } : projectId ? { projectId } : {}),
  });
  const pick = (reference: string) => act.run(async () => {
    if (!chosen) return;
    setImporting(reference);
    try {
      const d = await importOne(chosen, reference);
      invalidateDocuments(qc);
      onImported(d);
    } finally {
      setImporting(null);
    }
  });
  const pickWith = (connectionId: string, from: 'primary' | 'secondary') => act.run(async () => {
    if (!picker || !chosen) return;
    setPickedFrom(from);
    setPicking(true);
    try {
      const picked = await pickFiles(picker.module, connectionId);
      let last: Document | null = null;
      for (const reference of picked) last = await importOne(chosen, reference);
      invalidateDocuments(qc);
      if (last) onImported(last);
    } finally {
      setPicking(false);
    }
  });
  if (picking) return null;
  const browse = (variant: 'primary' | 'secondary', size: 'sm' | 'md', label = `${picker?.label ?? ''}…`) =>
    picker && pickers.length > 0 && (
      <BrowseButton label={label} accounts={pickers} onPick={(c) => void pickWith(c, variant)} variant={variant} size={size} autoFocus={pickedFrom === variant} />
    );
  const account = pickers.length === 1 ? (pickers[0].email ?? pickers[0].name) : pickers.length > 1 ? `${pickers.length} accounts` : undefined;
  const description = ticket
    ? <>Into <Badge className="font-mono">{ticket.key}</Badge>{projectName && ` · ${projectName}`}</>
    : projectName && `Into ${projectName}`;
  const empty = files.isFetching ? 'Searching…' : files.isError ? actionError(files.error) : (
    <EmptyState size="inline" className="justify-center py-2 text-center" action={browse('secondary', 'sm', `${picker?.label ?? ''} for it`)}>
      <span className="w-full font-medium text-foreground">{q ? `No files named “${q}” that PDT can open` : 'No files PDT can open yet'}</span>
      {searchNote && <span className="w-full">{searchNote}</span>}
    </EmptyState>
  );
  const keys = (
    <p className="mr-auto flex items-center gap-1 text-meta text-muted-foreground">
      <Kbd>↑</Kbd> <Kbd>↓</Kbd> move · <Kbd>Enter</Kbd> import · <Kbd>Esc</Kbd> close
    </p>
  );
  return (
    <Dialog onClose={onClose} title="Import a document" description={description || undefined} size="md" footer={touch ? undefined : keys}>
      <div className="flex flex-col gap-3" {...(hasLocal ? fileDrop.dropProps : {})}>
        {searchers.length > 1 && chosen && (
          <SegmentedControl
            aria-label="Provider"
            size="sm"
            className="self-start"
            options={searchers.map((p) => ({ value: p.id, label: p.label }))}
            value={chosen}
            onChange={setProvider}
          />
        )}
        {source && (
          <SourceCard
            aria-label="Provider"
            icon={<ProviderIcon id={source.id} />}
            name={source.label}
            detail={account ?? (source.id === 'local' ? providers.data?.folder : undefined)}
            action={browse('primary', 'sm')}
          >
            {picker && accounts.isSuccess && pickers.length === 0 && <PickerHelp picker={picker} accounts={accounts.data} />}
          </SourceCard>
        )}
        {providers.isSuccess && searchers.length === 0 ? (
          <Alert tone="info">No document provider here can look for files to import. Turn on a documents plugin in Settings → Plugins.</Alert>
        ) : (
          <section className="flex flex-col gap-1.5">
            <SectionLabel>Files PDT can open</SectionLabel>
            <CommandList
              label="Files PDT can open"
              placeholder="Search by name…"
              items={items}
              query={query}
              onQueryChange={setQuery}
              autoFocus={pickedFrom === null}
              emptyText={empty}
              onAction={(id) => void pick(String(id))}
              className="max-h-[45dvh] p-0"
            />
            {found.length > 0 && (searchNote || browseNote) && (
              <p className="text-meta text-muted-foreground">{[searchNote, browseNote].filter(Boolean).join(' ')}</p>
            )}
          </section>
        )}
        {act.error && <Alert tone="error">{act.error}</Alert>}
        {hasLocal && (
          <section className="flex flex-col gap-1.5">
            <SectionLabel>From your computer</SectionLabel>
            <DropZone
              onFile={(f) => void upload(f)}
              accept=".md,.markdown,.txt,.xlsx,.csv,.tsv"
              describe={dropText}
              pending={uploading}
              drop={fileDrop}
            >
              Drop a <strong className="font-medium text-foreground">.md</strong> (becomes a doc),{' '}
              <strong className="font-medium text-foreground">.xlsx</strong> or{' '}
              <strong className="font-medium text-foreground">.csv</strong> (a sheet)
            </DropZone>
          </section>
        )}
      </div>
    </Dialog>
  );
}

/**
 * Why no account opens the provider's picker, and what to do about it, in a muted line or two: sign in; nothing, for an
 * account with PDT's own sign-in in a build without a picker; or give an account with its own client what the picker
 * needs, in its connection, edited from here.
 */
function PickerHelp({ picker, accounts }: { picker: ProviderPicker; accounts: SignInAccount[] }) {
  const without = accounts.filter((a) => a.signedIn && !a.picker);
  const own = picker.setup ? without.filter((a) => !a.builtIn) : [];
  const connections = useConnections(own.length > 0);
  const [editing, setEditing] = useState<Connection | null>(null);
  const builtIn = without.some((a) => a.builtIn);
  const line = 'text-meta text-muted-foreground';
  if (!builtIn && own.length === 0) return <p className={line}>{picker.help}</p>;
  return (
    <>
      {builtIn && <p className={line}>{picker.unavailable ?? picker.help}</p>}
      {own.map((a) => {
        const c = connections.data?.find((x) => x.id === a.connectionId);
        return (
          <p key={a.connectionId} className={cn(line, 'flex flex-wrap items-center gap-2')}>
            <span className="min-w-0 flex-1">{own.length > 1 ? `${a.name}: ${picker.setup}` : picker.setup}</span>
            {c && <Button size="sm" variant="secondary" onClick={() => setEditing(c)}>Add to {a.name}</Button>}
          </p>
        );
      })}
      {editing && <ConnectionDialog connection={editing} onClose={() => setEditing(null)} />}
    </>
  );
}

/** Link a document PDT has to a ticket or project: search them by title. */
export function LinkDocumentDialog({ title, exclude, onPick, onClose }: {
  title: string;
  /** Documents not offered (the ones linked already). */
  exclude: string[];
  onPick: (d: Document) => Promise<void>;
  onClose: () => void;
}) {
  const [query, setQuery] = useState('');
  const q = useDebounced(query.trim());
  // The best matches only: typing narrows them.
  const docs = useDocuments({ ...(q ? { q } : {}), limit: 50 + exclude.length });
  const [error, setError] = useState<string | null>(null);
  const list = (docs.data ?? []).filter((d) => !exclude.includes(d.id));
  const items: CommandItem[] = list.map((d) => ({ id: d.id, label: d.title, detail: KIND_LABEL[d.kind], keywords: query ? [query] : [] }));
  return (
    <Dialog onClose={onClose} title={title} size="md">
      <div className="flex flex-col gap-2">
        <CommandList
          label="Documents"
          placeholder="Search by title…"
          items={items}
          query={query}
          onQueryChange={setQuery}
          autoFocus
          emptyText={docs.isFetching ? 'Searching…' : 'No documents found'}
          onAction={(id) => {
            const d = list.find((x) => x.id === id);
            if (d) onPick(d).catch((e: unknown) => setError(actionError(e)));
          }}
          className="max-h-[50dvh]"
        />
        {error && <Alert tone="error">{error}</Alert>}
      </div>
    </Dialog>
  );
}
