import { enc, queryString, request, send } from './core';
import type { ChartView } from './gen/ChartView';
import type { CommentReply } from './gen/CommentReply';
import type { DiffsApplied } from './gen/DiffsApplied';
import type { Document } from './gen/Document';
import type { DocumentComment } from './gen/DocumentComment';
import type { DocumentCommentInput } from './gen/DocumentCommentInput';
import type { DocumentEdit } from './gen/DocumentEdit';
import type { DocumentFilter } from './gen/DocumentFilter';
import type { DocumentKind } from './gen/DocumentKind';
import type { DocumentLinkInput } from './gen/DocumentLinkInput';
import type { DocumentProvidersView } from './gen/DocumentProvidersView';
import type { DocumentRead } from './gen/DocumentRead';
import type { DocumentsBeside } from './gen/DocumentsBeside';
import type { EditResult } from './gen/EditResult';
import type { ImportDocument } from './gen/ImportDocument';
import type { NewDocument } from './gen/NewDocument';
import type { ProjectDocumentSettings } from './gen/ProjectDocumentSettings';
import type { ProviderFile } from './gen/ProviderFile';
import type { SavedImage } from './gen/SavedImage';
import type { UploadedDocument } from './gen/UploadedDocument';

/** Documents (PDT-314) and the ones open beside a conversation (PDT-317). */
/** A path segment as written in Markdown (`My%20doc.assets`), decoded; as given when it isn't encoded. */
function safeDecode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    return s;
  }
}

export const documentsApi = {
  /** Documents (PDT-314): `project` and `ticket` take an id or a key. */
  documents: (f: DocumentFilter = {}) => request<Document[]>(`/api/documents${queryString(f)}`),
  document: (id: string) => request<Document>(`/api/documents/${enc(id)}`),
  /** The providers, and where new documents (in `project`) go. */
  documentProviders: (project?: string | null) =>
    request<DocumentProvidersView>(`/api/documents/providers${queryString({ project: project ?? undefined })}`),
  /** A provider's files to import (its picker). */
  providerFiles: (provider: string, q: string, kind?: DocumentKind) =>
    request<ProviderFile[]>(`/api/documents/providers/${enc(provider)}/files${queryString({ q, kind })}`),
  createDocument: (d: NewDocument) =>
    request<Document>('/api/documents', { method: 'POST', body: JSON.stringify({ ...d, by: 'user' } satisfies NewDocument) }),
  importDocument: (d: ImportDocument) =>
    request<Document>('/api/documents/import', { method: 'POST', body: JSON.stringify({ ...d, by: 'user' } satisfies ImportDocument) }),
  renameDocument: (id: string, title: string) =>
    request<Document>(`/api/documents/${enc(id)}`, { method: 'PATCH', body: JSON.stringify({ title }) }),
  /** Forgets it; with `file`, deletes it where it is kept too. */
  deleteDocument: (id: string, file = false) =>
    request<void>(`/api/documents/${enc(id)}${file ? '?file=true' : ''}`, { method: 'DELETE' }),
  linkDocument: (id: string, l: DocumentLinkInput) =>
    request<Document>(`/api/documents/${enc(id)}/links`, { method: 'POST', body: JSON.stringify(l) }),
  unlinkDocument: (id: string, l: DocumentLinkInput) =>
    request<Document>(`/api/documents/${enc(id)}/links`, { method: 'DELETE', body: JSON.stringify(l) }),
  documentContent: (id: string) => request<DocumentRead>(`/api/documents/${enc(id)}/content`),
  /** Changes a document's content; with `baseVersion`, a `conflict` (409) when it changed since. */
  editDocument: (id: string, edits: DocumentEdit[], baseVersion?: string | null) =>
    request<EditResult>(`/api/documents/${enc(id)}/edits`, {
      method: 'POST',
      body: JSON.stringify({ edits, ...(baseVersion ? { baseVersion } : {}) }),
    }),
  documentComments: (id: string) => request<DocumentComment[]>(`/api/documents/${enc(id)}/comments`),
  addDocumentComment: (id: string, c: DocumentCommentInput) =>
    request<DocumentComment>(`/api/documents/${enc(id)}/comments`, { method: 'POST', body: JSON.stringify({ ...c, by: 'user' } satisfies DocumentCommentInput) }),
  replyToDocumentComment: (id: string, comment: string, body: string) =>
    request<CommentReply>(`/api/documents/${enc(id)}/comments/${enc(comment)}/replies`, { method: 'POST', body: JSON.stringify({ body, by: 'user' }) }),
  resolveDocumentComment: (id: string, comment: string, resolved = true) =>
    request<void>(`/api/documents/${enc(id)}/comments/${enc(comment)}/${resolved ? 'resolve' : 'reopen'}`, { method: 'POST' }),
  /** A local sheet's live model (IronCalc's bytes) and the number of its last change. */
  documentSheet: async (id: string): Promise<{ bytes: Uint8Array; seq: number }> => {
    const res = await send(`/api/documents/${enc(id)}/sheet`);
    return { bytes: new Uint8Array(await res.arrayBuffer()), seq: Number(res.headers.get('x-sheet-seq') ?? 0) };
  },
  /** A view's edits to a local sheet (IronCalc diffs), made on top of change `seq`. */
  sendSheetDiffs: (id: string, view: string, seq: number, diffs: Uint8Array) =>
    request<DiffsApplied>(`/api/documents/${enc(id)}/sheet/diffs${queryString({ view, seq })}`, {
      method: 'POST',
      headers: { 'content-type': 'application/octet-stream' },
      body: new Blob([diffs as Uint8Array<ArrayBuffer>]),
    }),
  documentCharts: (id: string) => request<ChartView[]>(`/api/documents/${enc(id)}/charts`),
  documentEventsUrl: (id: string, view: string) => `/api/documents/${enc(id)}/events${queryString({ view })}`,
  documentExportUrl: (id: string, format: 'md' | 'xlsx' | 'csv', sheet?: string) =>
    `/api/documents/${enc(id)}/export${queryString({ format, sheet })}`,
  documentAssetUrl: (id: string, path: string) =>
    `/api/documents/${enc(id)}/assets/${path.split('/').map((p) => enc(safeDecode(p))).join('/')}`,
  /** A file from the user's computer made into a local document (`.md`, `.xlsx`, `.csv`). */
  uploadDocument: (file: File, link: { projectId?: string | null; ticketId?: string | null }) =>
    request<UploadedDocument>(`/api/documents/upload${queryString({ name: file.name, projectId: link.projectId, ticketId: link.ticketId, by: 'user' })}`, {
      method: 'POST',
      headers: { 'content-type': 'application/octet-stream' },
      body: file,
    }),
  /** An image saved beside a local doc; the path to put in its Markdown. */
  uploadDocumentImage: (id: string, file: File) =>
    request<SavedImage>(`/api/documents/${enc(id)}/images${queryString({ name: file.name || 'image.png' })}`, {
      method: 'POST',
      headers: { 'content-type': 'application/octet-stream' },
      body: file,
    }),
  /** Its version now (cheap): an open page reads it again when this changes. */
  documentVersion: (id: string) => request<EditResult>(`/api/documents/${enc(id)}/version`),
  /** A ticket's design doc, from the template, in its project's default provider. */
  createDesignDoc: (ticketId: string) =>
    request<Document>(`/api/tickets/${enc(ticketId)}/design-doc`, { method: 'POST', body: JSON.stringify({ by: 'user' }) }),
  /** The documents open beside a conversation (PDT-317), in tab order. */
  documentsBeside: (sessionId: string) => request<DocumentsBeside>(`/api/sessions/${enc(sessionId)}/documents`),
  /** Opens a document beside a conversation, or shows its tab there. */
  openBeside: (sessionId: string, documentId: string) =>
    request<DocumentsBeside>(`/api/sessions/${enc(sessionId)}/documents`, { method: 'POST', body: JSON.stringify({ documentId, by: 'user' }) }),
  orderBeside: (sessionId: string, documentIds: string[]) =>
    request<DocumentsBeside>(`/api/sessions/${enc(sessionId)}/documents`, { method: 'PUT', body: JSON.stringify({ documentIds }) }),
  closeBeside: (sessionId: string, documentId: string) =>
    request<DocumentsBeside>(`/api/sessions/${enc(sessionId)}/documents/${enc(documentId)}`, { method: 'DELETE' }),
  projectDocumentSettings: (projectId: string) =>
    request<ProjectDocumentSettings>(`/api/projects/${enc(projectId)}/document-settings`),
  putProjectDocumentSettings: (projectId: string, s: ProjectDocumentSettings) =>
    request<ProjectDocumentSettings>(`/api/projects/${enc(projectId)}/document-settings`, { method: 'PUT', body: JSON.stringify(s) }),
};
