import { enc, queryString, request } from './core';
import type { Document } from './gen/Document';
import type { DocumentFilter } from './gen/DocumentFilter';
import type { DocumentKind } from './gen/DocumentKind';
import type { DocumentLinkInput } from './gen/DocumentLinkInput';
import type { DocumentProvidersView } from './gen/DocumentProvidersView';
import type { DocumentRead } from './gen/DocumentRead';
import type { DocumentsBeside } from './gen/DocumentsBeside';
import type { ImportDocument } from './gen/ImportDocument';
import type { NewDocument } from './gen/NewDocument';
import type { ProjectDocumentSettings } from './gen/ProjectDocumentSettings';
import type { ProviderFile } from './gen/ProviderFile';

/** Documents (PDT-314) and the ones open beside a conversation (PDT-317). */
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
