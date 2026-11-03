import { useEffect, useRef, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { Upload } from 'lucide-react';
import { Alert, Button, CommandList, Dialog, DialogActions, Field, Input, SegmentedControl, Select, toast, type CommandItem } from '@ui';
import { api } from '../api/client';
import type { Document } from '../api/gen/Document';
import type { DocumentKind } from '../api/gen/DocumentKind';
import type { Project } from '../api/gen/Project';
import { actionError, useFormAction } from '../common/useAction';
import { useSignInAccounts } from '../plugins/AccountSignIn';
import { pickFiles } from './picker';
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

/**
 * Import or link: pick a provider that can search, then one of its files; PDT records it (or finds the record it has)
 * and links it to the project or ticket. Or a file from the computer (`.md`, `.xlsx`, `.csv`), made a local document.
 */
export function ImportDocumentDialog({ projectId = null, ticket, onClose, onImported }: {
  projectId?: string | null;
  ticket?: { id: string; key: string; projectId: string };
  onClose: () => void;
  onImported: (d: Document) => void;
}) {
  const qc = useQueryClient();
  const providers = useDocumentProviders(ticket?.projectId ?? projectId);
  const searchers = (providers.data?.providers ?? []).filter((p) => p.capabilities.search);
  const [provider, setProvider] = useState<string | null>(null);
  const chosen = provider ?? searchers.find((p) => p.id === providers.data?.default)?.id ?? searchers[0]?.id ?? null;
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
  const fileInput = useRef<HTMLInputElement>(null);
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
  // The provider's own file picker, for each signed-in account that opens it.
  const picker = searchers.find((p) => p.id === chosen)?.picker ?? null;
  const accounts = useSignInAccounts(picker?.accounts ?? null);
  const pickers = (accounts.data ?? []).filter((a) => a.signedIn && a.picker);
  // A provider's picker covers the page: the dialog steps aside while it is open.
  const [picking, setPicking] = useState(false);
  const items: CommandItem[] = (files.data ?? []).map((f) => ({
    id: f.reference,
    label: f.title,
    detail: KIND_LABEL[f.kind],
    keywords: query ? [query] : [],
  }));
  const importOne = (provider: string, reference: string) => api.importDocument({
    provider,
    reference,
    ...(ticket ? { ticketId: ticket.id } : projectId ? { projectId } : {}),
  });
  const pick = (reference: string) => act.run(async () => {
    if (!chosen) return;
    const d = await importOne(chosen, reference);
    invalidateDocuments(qc);
    onImported(d);
  });
  const pickWith = (module: string, provider: string, connectionId: string) => act.run(async () => {
    setPicking(true);
    try {
      const picked = await pickFiles(module, connectionId);
      let last: Document | null = null;
      for (const reference of picked) last = await importOne(provider, reference);
      invalidateDocuments(qc);
      if (last) onImported(last);
    } finally {
      setPicking(false);
    }
  });
  if (picking) return null;
  return (
    <Dialog onClose={onClose} title={ticket ? `Import a document to ${ticket.key}` : 'Import a document'} size="md">
      <div className="flex flex-col gap-2">
        {searchers.length > 1 && (
          <Select
            aria-label="Provider"
            size="sm"
            className="w-48 self-start"
            options={searchers.map((p) => ({ value: p.id, label: p.label }))}
            value={chosen}
            onChange={setProvider}
          />
        )}
        {picker && chosen && accounts.isSuccess && (pickers.length > 0 ? (
          <div className="flex flex-wrap items-center gap-2">
            {pickers.map((a) => (
              <Button key={a.connectionId} size="sm" variant="primary" onClick={() => void pickWith(picker.module, chosen, a.connectionId)}>
                {picker.label}{pickers.length > 1 ? ` (${a.email ?? a.name})` : ''}
              </Button>
            ))}
            <span className="text-xs text-muted-foreground">Or choose one PDT already has access to:</span>
          </div>
        ) : (
          <Alert tone="info">{picker.help}</Alert>
        ))}
        {providers.isSuccess && searchers.length === 0 ? (
          <Alert tone="info">No document provider here can look for files to import. Turn on a documents plugin in Settings → Plugins.</Alert>
        ) : (
          <CommandList
            label="Files"
            placeholder="Search by name…"
            items={items}
            query={query}
            onQueryChange={setQuery}
            autoFocus
            emptyText={files.isFetching ? 'Searching…' : files.isError ? actionError(files.error) : 'No files found'}
            onAction={(id) => void pick(String(id))}
            className="max-h-[50dvh]"
          />
        )}
        {act.error && <Alert tone="error">{act.error}</Alert>}
        {hasLocal && (
          <div className="flex items-center gap-2 border-t border-border pt-2">
            <Button variant="secondary" size="sm" disabled={uploading} onClick={() => fileInput.current?.click()}>
              <Upload aria-hidden /> {uploading ? 'Importing…' : 'From your computer…'}
            </Button>
            <span className="text-meta text-muted-foreground">A .md file becomes a doc; .xlsx or .csv a sheet.</span>
            <input
              ref={fileInput}
              type="file"
              accept=".md,.markdown,.txt,.xlsx,.csv,.tsv"
              aria-label="File to import"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                e.target.value = '';
                if (f) void upload(f);
              }}
            />
          </div>
        )}
      </div>
    </Dialog>
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
