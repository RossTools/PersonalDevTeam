import { useCallback, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ExternalLink, Pencil, Plus, X } from 'lucide-react';
import { Alert, Button, Dialog, EmptyState, IconButton, Link, MenuButton, RelativeTime, Select, Skeleton, useConfirm, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { Document } from '../api/gen/Document';
import { ProjectKey } from '../common/BranchChip';
import { TicketKey } from '../common/TicketKey';
import { useRememberedScroll } from '../common/viewState';
import { InlineRename } from '../common/InlineRename';
import { cn } from '@ui/lib/utils';
import { runAction } from '../common/useAction';
import { useOpenTicket } from '../tickets/actions';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { TicketPicker } from '../tickets/TicketPicker';
import { KindIcon } from './LinkedDocuments';
import { openBesideEntry, useBesideTargets } from './BesidePanel';
import { DocumentBody, type SheetHandle } from './DocumentBody';
import { DocumentComments, headingsOf } from './DocumentComments';
import { KIND_LABEL, documentKeys, invalidateDocuments, providerLabel, useDocument, useDocumentContent, useDocumentProviders } from './queries';

/** Downloads a file pdtd serves. */
function download(url: string) {
  const a = document.createElement('a');
  a.href = url;
  a.download = '';
  document.body.append(a);
  a.click();
  a.remove();
}

/** The ids of the tickets a document is linked to (their conversations are where it can open beside). */
const linkedTickets = (d: Document | null | undefined) => d?.tickets.map((t) => t.id) ?? [];

/** Who added it, for people. */
function addedBy(d: Document): string {
  if (d.actor === 'user') return 'you';
  return d.agent?.persona ?? 'an agent';
}

const linkHeading = 'text-2xs font-semibold tracking-wider text-muted-foreground uppercase';

/**
 * One document (PDT-314): its title (rename in place), kind, provider and who added it; where it opens; its links to
 * projects and tickets (add and remove); and its content as the provider reads it. Removing it from PDT keeps the
 * file; Delete removes the file too, where the provider can.
 */
export function DocumentPage({ id, phone, onGone }: { id: string; phone: boolean; onGone: () => void }) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const openTicket = useOpenTicket();
  const doc = useDocument(id);
  const providers = useDocumentProviders();
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects });
  const [renaming, setRenaming] = useState(false);
  const [linkingTicket, setLinkingTicket] = useState(false);
  const besideTargets = useBesideTargets(linkedTickets(doc.data));
  const [sheetHandle, setSheetHandle] = useState<SheetHandle | null>(null);
  const onSheet = useCallback((h: SheetHandle | null) => setSheetHandle(h), []);
  const content = useDocumentContent(id);
  // Where it was scrolled comes back when it is left and opened again (PDT-379).
  const scroll = useRememberedScroll(`document:${id}`);
  const d = doc.data;
  const provider = d ? providers.data?.providers.find((p) => p.id === d.provider) : undefined;
  const settle = () => invalidateDocuments(qc);
  const rename = (title: string) => void runAction(async () => {
    await api.renameDocument(id, title);
    settle();
  });
  const remove = (file: boolean) => void runAction(async () => {
    if (!d) return;
    const { ok } = await confirm(file ? {
      title: `Delete “${d.title}”?`,
      body: `It is deleted from ${provider?.label ?? d.provider} too, and its links go.`,
      action: 'Delete document',
      danger: true,
    } : {
      title: `Remove “${d.title}” from PDT?`,
      body: `PDT forgets it and its links. The file stays in ${provider?.label ?? d.provider}.`,
      action: 'Remove from PDT',
      danger: true,
    });
    if (!ok) return;
    await api.deleteDocument(id, file);
    settle();
    onGone();
  });
  const local = d?.provider === 'local';
  const exports = (): MenuEntry[] => !d ? [] : d.kind === 'doc'
    ? [{ label: 'Export as Markdown (.md)', onSelect: () => download(api.documentExportUrl(id, 'md')) }]
    : [
        ...(local ? [{ label: 'Export as Excel (.xlsx)', onSelect: () => download(api.documentExportUrl(id, 'xlsx')) }] : []),
        { label: sheetHandle ? `Export “${sheetHandle.sheet()}” as CSV (.csv)` : 'Export as CSV (.csv)', onSelect: () => download(api.documentExportUrl(id, 'csv', sheetHandle?.sheet())) },
      ];
  const entries = (): MenuEntry[] => [
    { label: 'Rename', onSelect: () => setRenaming(true) },
    { label: 'Read again', onSelect: () => void qc.invalidateQueries({ queryKey: documentKeys.content(id) }) },
    ...(d ? [openBesideEntry(besideTargets, d.id)] : []),
    ...exports(),
    { separator: true },
    { label: 'Remove from PDT…', onSelect: () => remove(false), danger: true },
    ...(provider?.capabilities.delete ? [{ label: 'Delete document…', onSelect: () => remove(true), danger: true }] : []),
  ];
  useTopBarActions(phone && d ? entries : null);

  if (doc.isPending) return <Skeleton lines={8} className="p-4" />;
  if (doc.isError) return <Alert tone="error" className="m-4">The document could not be loaded: {doc.error.message}</Alert>;
  if (!d) return <EmptyState className="mt-16 border-0">This document is gone.</EmptyState>;

  const unlinkProject = (projectId: string) => void runAction(async () => {
    await api.unlinkDocument(id, { projectId });
    settle();
  });
  const unlinkTicket = (ticketId: string) => void runAction(async () => {
    await api.unlinkDocument(id, { ticketId });
    settle();
  });
  const unlinked = (projects.data ?? []).filter((p) => !d.projects.some((l) => l.id === p.id));
  return (
    <div ref={scroll.ref} onScroll={scroll.onScroll} className="min-h-0 flex-1 overflow-auto">
      <article aria-label={d.title} className={cn('mx-auto flex flex-col gap-4 p-4', d.kind === 'sheet' && provider?.capabilities.live ? 'max-w-none' : 'max-w-4xl')}>
        <header className="flex flex-col gap-1">
          <div className="flex min-w-0 items-center gap-2">
            <KindIcon kind={d.kind} />
            {renaming ? (
              <InlineRename label="Document title" initial={d.title} maxLength={200} size="md" className="flex-1"
                onCommit={(t) => { setRenaming(false); rename(t); }} onCancel={() => setRenaming(false)} />
            ) : (
              <h1 className="min-w-0 flex-1 truncate text-lg font-semibold tracking-tight" title={d.title}>{d.title}</h1>
            )}
            {!phone && !renaming && (
              <>
                <IconButton label="Rename" variant="ghost" size="sm" onClick={() => setRenaming(true)}><Pencil aria-hidden /></IconButton>
                <MenuButton label="Document actions" entries={entries} placement="bottom end" />
              </>
            )}
          </div>
          <p className="flex flex-wrap items-center gap-1.5 text-meta text-muted-foreground">
            <span>{KIND_LABEL[d.kind]}</span>
            <span aria-hidden>·</span>
            <span>{providerLabel(providers.data?.providers, d.provider)}</span>
            <span aria-hidden>·</span>
            <span>Added by {addedBy(d)} <RelativeTime time={d.createdAt} /></span>
            {d.url && (
              <>
                <span aria-hidden>·</span>
                <Link external href={d.url} className="inline-flex items-center gap-1">Open in {provider?.label ?? d.provider} <ExternalLink aria-hidden className="size-3" /></Link>
              </>
            )}
          </p>
        </header>

        <section aria-label="Links" className="flex flex-col gap-2 rounded-md border border-border bg-card p-3">
          <div className="flex flex-wrap items-center gap-1.5">
            <h2 className={linkHeading}>Projects</h2>
            {d.projects.length === 0 && <span className="text-xs text-muted-foreground">None</span>}
            {d.projects.map((p) => (
              <span key={p.id} className="inline-flex items-center gap-1 rounded-sm bg-accent py-0.5 pr-0.5 pl-1.5 text-xs">
                <ProjectKey projectKey={p.key} /> {p.name}
                <IconButton label={`Unlink ${p.name}`} size="sm" variant="ghost" className="size-5" onClick={() => unlinkProject(p.id)}><X aria-hidden /></IconButton>
              </span>
            ))}
            {unlinked.length > 0 && (
              <Select
                aria-label="Link a project"
                size="sm"
                quiet
                className="w-auto"
                placeholder="Link a project"
                options={unlinked.map((p) => ({ value: p.id, label: p.name }))}
                value={null}
                onChange={(projectId) => void runAction(async () => {
                  await api.linkDocument(id, { projectId });
                  settle();
                })}
              />
            )}
          </div>
          <div className="flex flex-wrap items-center gap-1.5">
            <h2 className={linkHeading}>Tickets</h2>
            {d.tickets.length === 0 && <span className="text-xs text-muted-foreground">None</span>}
            {d.tickets.map((t) => (
              <span key={t.id} className="inline-flex max-w-full min-w-0 items-center gap-1 rounded-sm bg-accent py-0.5 pr-0.5 pl-1.5 text-xs">
                <button type="button" className="min-w-0 truncate hover:underline" onClick={() => openTicket({ id: t.id, projectId: t.projectId })} title={t.title}>
                  <TicketKey ticketKey={t.key} /> {t.title}
                </button>
                <IconButton label={`Unlink ${t.key}`} size="sm" variant="ghost" className="size-5" onClick={() => unlinkTicket(t.id)}><X aria-hidden /></IconButton>
              </span>
            ))}
            <Button variant="ghost" size="sm" compact onClick={() => setLinkingTicket(true)}><Plus aria-hidden /> Link a ticket</Button>
          </div>
        </section>

        <section aria-label="Content" className="flex flex-col gap-2">
          <DocumentBody id={id} providerLabel={provider?.label ?? d.provider} onSheet={onSheet} />
        </section>

        {provider?.capabilities.comments && content.data && (
          <DocumentComments
            id={id}
            headings={content.data.content.kind === 'doc' ? headingsOf(content.data.content.markdown) : undefined}
            selection={content.data.content.kind === 'sheet' && sheetHandle ? sheetHandle.selection : undefined}
            onShow={sheetHandle ? sheetHandle.show : undefined}
          />
        )}
      </article>
      {linkingTicket && (
        <Dialog open onOpenChange={(o) => !o && setLinkingTicket(false)} title={`Link “${d.title}” to a ticket`} size="md">
          <TicketPicker
            projectId={d.projects.length === 1 ? d.projects[0]!.id : null}
            exclude={d.tickets.map((t) => t.id)}
            label="Ticket to link"
            onPick={(t) => void runAction(async () => {
              await api.linkDocument(id, { ticketId: t.id });
              settle();
              setLinkingTicket(false);
            })}
            className="max-h-[50dvh]"
          />
        </Dialog>
      )}
    </div>
  );
}
