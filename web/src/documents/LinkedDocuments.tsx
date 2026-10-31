import { useState, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, FilePlus2, FileSpreadsheet, FileText, Link2, Unlink } from 'lucide-react';
import { Button, IconButton, MenuButton } from '@ui';
import { api } from '../api/client';
import type { Document } from '../api/gen/Document';
import type { DocumentLinkInput } from '../api/gen/DocumentLinkInput';
import { actionError, runAction, useAction } from '../common/useAction';
import { ImportDocumentDialog, LinkDocumentDialog, NewDocumentDialog } from './DocumentDialogs';
import { invalidateDocuments, openDocument, providerLabel, useDocumentProviders, useDocuments } from './queries';

export function KindIcon({ kind }: { kind: Document['kind'] }) {
  return kind === 'sheet'
    ? <FileSpreadsheet aria-label="Sheet" className="size-3.5 shrink-0 text-muted-foreground" />
    : <FileText aria-label="Doc" className="size-3.5 shrink-0 text-muted-foreground" />;
}

/** One linked document: its title opens it in the Documents view; Unlink takes the link away (the document stays). */
function DocumentRow({ doc, provider, onUnlink }: { doc: Document; provider: string; onUnlink: () => void }) {
  return (
    <li className="flex min-w-0 items-center gap-2 text-xs">
      <KindIcon kind={doc.kind} />
      <button type="button" className="min-w-0 flex-1 truncate text-left hover:underline focus-visible:underline focus-visible:outline-none" title={doc.title} onClick={() => openDocument(doc.id)}>
        {doc.title}
      </button>
      <span className="shrink-0 text-meta text-muted-foreground">{provider}</span>
      <IconButton label={`Unlink ${doc.title}`} size="sm" variant="ghost" onClick={onUnlink}>
        <Unlink aria-hidden />
      </IconButton>
    </li>
  );
}

type Dialogs = 'new' | 'import' | 'link' | null;

/**
 * The documents linked to a ticket or project, with Link (one PDT has), New and Import; a ticket also has Create design
 * doc, which makes one from the template in the project's default provider.
 */
function LinkedDocuments({ link, projectId, ticket, empty, wrap }: {
  link: DocumentLinkInput;
  projectId: string;
  ticket?: { id: string; key: string; projectId: string };
  empty: string;
  /** Lays out the heading with the actions. */
  wrap: (actions: ReactNode, body: ReactNode) => ReactNode;
}) {
  const qc = useQueryClient();
  const projects = useQuery({ queryKey: ['projects'], queryFn: api.listProjects }).data ?? [];
  const filter = ticket ? { ticket: ticket.id } : { project: projectId };
  const docs = useDocuments(filter);
  const providers = useDocumentProviders(projectId);
  const [dialog, setDialog] = useState<Dialogs>(null);
  const list = docs.data ?? [];
  const [designDoc, making] = useAction(async () => {
    const d = await api.createDesignDoc(ticket!.id);
    invalidateDocuments(qc);
    openDocument(d.id);
  });
  const unlink = (d: Document) => void runAction(async () => {
    await api.unlinkDocument(d.id, link);
    invalidateDocuments(qc);
  });
  const done = () => setDialog(null);
  const canMake = (providers.data?.providers ?? []).some((p) => p.capabilities.create);
  const canImport = (providers.data?.providers ?? []).some((p) => p.capabilities.search);
  const actions = (
    <>
      {ticket && canMake && (
        <Button variant="ghost" size="sm" compact pending={making} onClick={() => void designDoc()}>
          <FilePlus2 aria-hidden /> Create design doc
        </Button>
      )}
      <MenuButton
        label="Add a document"
        placement="bottom end"
        entries={() => [
          { label: 'Link a document…', onSelect: () => setDialog('link'), icon: <Link2 aria-hidden /> },
          { label: 'New document…', onSelect: () => setDialog('new'), disabled: !canMake },
          { label: 'Import…', onSelect: () => setDialog('import'), disabled: !canImport },
        ]}
      >
        <Button variant="ghost" size="sm" compact>Add <ChevronDown aria-hidden /></Button>
      </MenuButton>
    </>
  );
  const body = (
    <>
      {docs.isSuccess && list.length === 0 && <p className="text-xs text-muted-foreground">{empty}</p>}
      {list.length > 0 && (
        <ul aria-label="Linked documents" className="flex flex-col gap-1.5">
          {list.map((d) => <DocumentRow key={d.id} doc={d} provider={providerLabel(providers.data?.providers, d.provider)} onUnlink={() => unlink(d)} />)}
        </ul>
      )}
      {docs.isError && <p role="alert" className="text-xs text-destructive">{actionError(docs.error)}</p>}
      {dialog === 'new' && (
        <NewDocumentDialog projects={projects} projectId={projectId} ticket={ticket} onClose={done} onCreated={(d) => { done(); openDocument(d.id); }} />
      )}
      {dialog === 'import' && <ImportDocumentDialog projectId={projectId} ticket={ticket} onClose={done} onImported={done} />}
      {dialog === 'link' && (
        <LinkDocumentDialog
          title={ticket ? `Link a document to ${ticket.key}` : 'Link a document'}
          exclude={list.map((d) => d.id)}
          onClose={done}
          onPick={async (d) => {
            await api.linkDocument(d.id, link);
            invalidateDocuments(qc);
            done();
          }}
        />
      )}
    </>
  );
  return <>{wrap(actions, body)}</>;
}

export function TicketDocumentsBody({ ticket, wrap }: {
  ticket: { id: string; key: string; projectId: string };
  wrap: (actions: ReactNode, body: ReactNode) => ReactNode;
}) {
  return (
    <LinkedDocuments
      link={{ ticketId: ticket.id }}
      projectId={ticket.projectId}
      ticket={ticket}
      empty="None. Create a design doc, or link one."
      wrap={wrap}
    />
  );
}

export function ProjectDocumentsBody({ projectId, wrap }: {
  projectId: string;
  wrap: (actions: ReactNode, body: ReactNode) => ReactNode;
}) {
  return (
    <LinkedDocuments link={{ projectId }} projectId={projectId} empty="No documents are linked to this project." wrap={wrap} />
  );
}
