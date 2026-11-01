import { ApiError, enc, queryString, request } from './core';
import type { ApiErrorBody } from './gen/ApiErrorBody';
import type { BulkRequest } from './gen/BulkRequest';
import type { BulkResult } from './gen/BulkResult';
import type { EventPage } from './gen/EventPage';
import type { EventQuery } from './gen/EventQuery';
import type { FinishRequest } from './gen/FinishRequest';
import type { FinishResult } from './gen/FinishResult';
import type { NewComment } from './gen/NewComment';
import type { NewParts } from './gen/NewParts';
import type { NewTicket } from './gen/NewTicket';
import type { PartsCreated } from './gen/PartsCreated';
import type { RankMove } from './gen/RankMove';
import type { StartResult } from './gen/StartResult';
import type { StartTicket } from './gen/StartTicket';
import type { Ticket } from './gen/Ticket';
import type { TicketAttachment } from './gen/TicketAttachment';
import type { TicketEvent } from './gen/TicketEvent';
import type { TicketImportResult } from './gen/TicketImportResult';
import type { TicketLink } from './gen/TicketLink';
import type { TicketPage } from './gen/TicketPage';
import type { TicketPatch } from './gen/TicketPatch';
import type { TicketQuery } from './gen/TicketQuery';

/** Tickets: the board, links, comments, attachments, parts, and starting and finishing work. */
export const ticketsApi = {
  listTickets: (q: TicketQuery = {}) => request<TicketPage>(`/api/tickets${queryString(q)}`),
  createTicket: (projectId: string, t: NewTicket) =>
    request<Ticket>(`/api/projects/${enc(projectId)}/tickets`, { method: 'POST', body: JSON.stringify(t) }, { asUser: true }),
  /** One of a plugin's issues, by its key, as a ticket linked to it (or the ticket that already links it). */
  importIssue: (projectId: string, integration: string, key: string) =>
    request<TicketImportResult>(`/api/projects/${enc(projectId)}/import/${enc(integration)}`, { method: 'POST', body: JSON.stringify({ key }) }),
  getTicket: (id: string) => request<Ticket>(`/api/tickets/${enc(id)}`),
  updateTicket: (id: string, p: TicketPatch) =>
    request<Ticket>(`/api/tickets/${enc(id)}`, { method: 'PATCH', body: JSON.stringify(p) }, { asUser: true }),
  deleteTicket: (id: string) => request<void>(`/api/tickets/${enc(id)}`, { method: 'DELETE' }),
  rankTicket: (id: string, m: RankMove) =>
    request<Ticket>(`/api/tickets/${enc(id)}/rank`, { method: 'POST', body: JSON.stringify(m) }),
  bulkTickets: (b: BulkRequest) =>
    request<BulkResult>('/api/tickets/bulk', { method: 'POST', body: JSON.stringify(b) }, { asUser: true }),
  linkTicket: (id: string, l: TicketLink) =>
    request<Ticket>(`/api/tickets/${enc(id)}/links`, { method: 'POST', body: JSON.stringify(l) }),
  unlinkTicket: (id: string, l: TicketLink) =>
    request<Ticket>(`/api/tickets/${enc(id)}/links`, { method: 'DELETE', body: JSON.stringify(l) }),
  ticketEvents: (id: string, q: EventQuery = {}) =>
    request<EventPage>(`/api/tickets/${enc(id)}/events${queryString(q)}`),
  commentOnTicket: (id: string, body: string) =>
    request<TicketEvent>(`/api/tickets/${enc(id)}/comments`, { method: 'POST', body: JSON.stringify({ body, by: 'user' } satisfies NewComment) }),
  /**
   * Attaches a file to a ticket (an image up to 10 MB, or UTF-8 text up to 1 MB; the bytes decide), by the user.
   * `onProgress` hears the share sent so far (0–1).
   */
  uploadTicketAttachment: (id: string, file: Blob, name: string, onProgress?: (sent: number) => void) =>
    new Promise<TicketAttachment>((resolve, reject) => {
      const xhr = new XMLHttpRequest();
      xhr.open('POST', `/api/tickets/${enc(id)}/attachments${queryString({ name, by: 'user' })}`);
      xhr.setRequestHeader('content-type', file.type || 'application/octet-stream');
      xhr.upload.onprogress = (e) => { if (e.lengthComputable) onProgress?.(e.loaded / e.total); };
      xhr.onload = () => {
        let body: Partial<ApiErrorBody> & Partial<TicketAttachment> = {};
        try {
          body = JSON.parse(xhr.responseText);
        } catch {
          // non-JSON body (proxy, crash page)
        }
        if (xhr.status >= 200 && xhr.status < 300) resolve(body as TicketAttachment);
        else reject(new ApiError(xhr.status, body.code ?? 'http_error', body.message ?? `HTTP ${xhr.status}`));
      };
      xhr.onerror = () => reject(new ApiError(0, 'network_error', 'The upload failed: the server could not be reached'));
      xhr.send(file);
    }),
  ticketAttachments: (id: string) => request<TicketAttachment[]>(`/api/tickets/${enc(id)}/attachments`),
  deleteTicketAttachment: (id: string, attachmentId: string) =>
    request<void>(`/api/tickets/${enc(id)}/attachments/${enc(attachmentId)}`, { method: 'DELETE' }),
  /** Where a ticket's attachment is served (images inline, text as plain text). */
  ticketAttachmentUrl: (id: string, attachmentId: string) => `/api/tickets/${enc(id)}/attachments/${enc(attachmentId)}`,
  /** Several parts of a parent, in order after its existing parts (PDT-38). */
  createParts: (id: string, b: NewParts) =>
    request<PartsCreated>(`/api/tickets/${enc(id)}/parts`, { method: 'POST', body: JSON.stringify(b) }, { asUser: true }),
  startTicket: (id: string, b: StartTicket) =>
    request<StartResult>(`/api/tickets/${enc(id)}/start`, { method: 'POST', body: JSON.stringify(b) }, { asUser: true }),
  checkMerge: (id: string) => request<Ticket>(`/api/tickets/${enc(id)}/check-merge`, { method: 'POST' }),
  /** Asks the project's pipeline to refine a Draft or Ready ticket (Phase 7c). */
  refineTicket: (id: string) =>
    request<Ticket>(`/api/tickets/${enc(id)}/refine`, { method: 'POST', body: JSON.stringify({ by: 'user' }) }),
  /** `ticket.finish`: Done, then everything Start made goes (or `blocked` says what stops it, changing nothing). */
  finishTicket: (id: string, body: FinishRequest) =>
    request<FinishResult>(`/api/tickets/${enc(id)}/finish`, { method: 'POST', body: JSON.stringify(body) }),
};
