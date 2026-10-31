import { useEffect, useState } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ExternalLink, Pencil, Plus, X } from 'lucide-react';
import { Alert, Button, Dialog, EmptyState, IconButton, Link, MenuButton, RelativeTime, SegmentedControl, Select, Skeleton, useConfirm, type MenuEntry } from '@ui';
import { api } from '../api/client';
import type { Document } from '../api/gen/Document';
import type { DocumentContent } from '../api/gen/DocumentContent';
import type { Sheet } from '../api/gen/Sheet';
import { ProjectKey } from '../common/BranchChip';
import { InlineRename } from '../common/InlineRename';
import { Markdown } from '../common/Markdown';
import { actionError, runAction } from '../common/useAction';
import { useOpenTicket } from '../tickets/actions';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { TicketPicker } from '../tickets/TicketPicker';
import { KindIcon } from './LinkedDocuments';
import { KIND_LABEL, documentKeys, invalidateDocuments, providerLabel, useDocument, useDocumentContent, useDocumentProviders, useDocumentVersion } from './queries';

/** Who added it, for people. */
function addedBy(d: Document): string {
  if (d.actor === 'user') return 'you';
  return d.agent?.persona ?? 'an agent';
}

/** A sheet as a table: its rows from A1, with column letters and row numbers; a cell's formula shows on hover. */
function SheetTable({ sheet }: { sheet: Sheet }) {
  const width = Math.max(0, ...sheet.rows.map((r) => r.length));
  const letter = (i: number): string => (i < 26 ? String.fromCharCode(65 + i) : letter(Math.floor(i / 26) - 1) + letter(i % 26));
  if (sheet.rows.length === 0) return <p className="text-xs text-muted-foreground">This sheet is empty.</p>;
  return (
    <div className="overflow-auto rounded-md border border-border">
      <table className="border-collapse text-xs">
        <thead>
          <tr>
            <th className="sticky left-0 bg-muted px-2 py-1" />
            {Array.from({ length: width }, (_, i) => <th key={i} scope="col" className="border-l border-border bg-muted px-2 py-1 font-medium text-muted-foreground">{letter(i)}</th>)}
          </tr>
        </thead>
        <tbody>
          {sheet.rows.map((row, r) => (
            <tr key={r} className="border-t border-border">
              <th scope="row" className="sticky left-0 bg-muted px-2 py-1 text-right font-medium text-muted-foreground">{r + 1}</th>
              {Array.from({ length: width }, (_, c) => {
                const cell = row[c];
                return <td key={c} className="border-l border-border px-2 py-1 whitespace-nowrap" title={cell?.formula ?? undefined}>{cell?.value ?? ''}</td>;
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function Content({ content }: { content: DocumentContent }) {
  const [sheet, setSheet] = useState(0);
  if (content.kind === 'doc') {
    return content.markdown.trim() ? <Markdown>{content.markdown}</Markdown> : <p className="text-xs text-muted-foreground">This document is empty.</p>;
  }
  const shown = content.sheets[Math.min(sheet, content.sheets.length - 1)];
  return (
    <div className="flex flex-col gap-2">
      {content.sheets.length > 1 && (
        <SegmentedControl
          aria-label="Sheet"
          size="sm"
          className="self-start"
          options={content.sheets.map((s, i) => ({ value: String(i), label: s.name }))}
          value={String(Math.min(sheet, content.sheets.length - 1))}
          onChange={(v) => setSheet(Number(v))}
        />
      )}
      {shown ? <SheetTable sheet={shown} /> : <p className="text-xs text-muted-foreground">No sheets.</p>}
    </div>
  );
}

type View = 'live' | 'pdt';
const VIEW_KEY = 'pdt.documents.view';

/** The provider's own editor, embedded (Google's), live: what agents change shows as they change it. */
function LiveEditor({ url, title, provider }: { url: string; title: string; provider: string }) {
  return (
    <div className="flex flex-col gap-1">
      <iframe src={url} title={`${title} in ${provider}`} allow="clipboard-read; clipboard-write" className="h-[75dvh] w-full rounded-md border border-border bg-card" />
      <p className="text-xs text-muted-foreground">
        Asks you to sign in, or stays blank? This browser keeps {provider}’s sign-in out of pages inside PDT: switch to Read in PDT, or open it in {provider}.
      </p>
    </div>
  );
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
  const content = useDocumentContent(id);
  const d = doc.data;
  const provider = d ? providers.data?.providers.find((p) => p.id === d.provider) : undefined;
  const embedUrl = content.data?.embedUrl ?? null;
  const [view, setView] = useState<View>(() => (localStorage.getItem(VIEW_KEY) === 'pdt' ? 'pdt' : 'live'));
  const live = embedUrl !== null && view === 'live';
  // PDT's own view reads the document again when it changes (the embedded editor follows changes itself).
  const version = useDocumentVersion(id, content.isSuccess && !live);
  const shownVersion = content.data?.content.version ?? null;
  const nowVersion = version.data?.version ?? null;
  useEffect(() => {
    if (nowVersion !== null && shownVersion !== null && nowVersion !== shownVersion) {
      void qc.invalidateQueries({ queryKey: documentKeys.content(id) });
    }
  }, [qc, id, nowVersion, shownVersion]);
  const pickView = (v: View) => {
    localStorage.setItem(VIEW_KEY, v);
    setView(v);
  };
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
  const entries = (): MenuEntry[] => [
    { label: 'Rename', onSelect: () => setRenaming(true) },
    { label: 'Read again', onSelect: () => void qc.invalidateQueries({ queryKey: documentKeys.content(id) }) },
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
    <div className="min-h-0 flex-1 overflow-auto">
      <article aria-label={d.title} className="mx-auto flex max-w-4xl flex-col gap-4 p-4">
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
                  <span className="font-mono">{t.key}</span> {t.title}
                </button>
                <IconButton label={`Unlink ${t.key}`} size="sm" variant="ghost" className="size-5" onClick={() => unlinkTicket(t.id)}><X aria-hidden /></IconButton>
              </span>
            ))}
            <Button variant="ghost" size="sm" compact onClick={() => setLinkingTicket(true)}><Plus aria-hidden /> Link a ticket</Button>
          </div>
        </section>

        <section aria-label="Content" className="flex flex-col gap-2">
          {content.isPending && <Skeleton lines={6} label="Reading the document" />}
          {content.isError && <Alert tone="error">{provider?.label ?? d.provider} could not read it: {actionError(content.error)}</Alert>}
          {content.data && embedUrl !== null && (
            <SegmentedControl
              aria-label="How to show it"
              size="sm"
              className="self-start"
              options={[{ value: 'live', label: `${provider?.label ?? d.provider} editor` }, { value: 'pdt', label: 'Read in PDT' }]}
              value={view}
              onChange={(v) => pickView(v as View)}
            />
          )}
          {content.data && (live
            ? <LiveEditor url={embedUrl} title={d.title} provider={provider?.label ?? d.provider} />
            : <Content content={content.data.content} />)}
        </section>
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
