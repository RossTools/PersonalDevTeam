// Test data for the ticket screens.
import type { Project } from '../api/gen/Project';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketEvent } from '../api/gen/TicketEvent';
import type { TicketPage } from '../api/gen/TicketPage';
import type { TicketSummary } from '../api/gen/TicketSummary';

export const project: Project = {
  id: 'p1', name: 'Alpha', key: 'ALP', repoPath: '/r', baseBranch: 'main', createdAt: 1, forge: null, kind: 'repo',
};

export function summary(n: number, over: Partial<TicketSummary> = {}): TicketSummary {
  return {
    id: `t${n}`, projectId: project.id, key: `ALP-${n}`, number: n, title: `Ticket ${n}`, stage: 'draft', priority: 'none',
    labels: [], areas: [], rank: `a${String(n).padStart(4, '0')}`, waiting: false, clearToStart: false, blockedBy: 0, attempt: null, overlap: 'none', overlapWith: [],
    createdAt: 1, updatedAt: 1, closedAt: null, parent: null, parts: null, pipelineStage: null, agent: null, autoStartAt: null, pickupHold: null, external: [], source: 'user', admitted: false, ...over,
  };
}

export function ticket(n: number, over: Partial<Ticket> = {}): Ticket {
  return { ...summary(n), description: '', links: [], attempts: [], defaultBranch: `pdt/ALP-${n}-ticket-${n}`, sessions: [], overlaps: [], partList: [], ...over };
}

export const page = (items: TicketSummary[], nextCursor: string | null = null, total = items.length): TicketPage => ({ items, nextCursor, total });

export function event(id: string, over: Partial<TicketEvent> = {}): TicketEvent {
  return { id, ticketId: 't1', kind: 'created', actor: 'user', body: null, data: null, createdAt: 1, agent: null, ...over };
}

/** The query of a request path, as an object. */
export const params = (path: string) => Object.fromEntries(new URLSearchParams(path.split('?')[1] ?? ''));
