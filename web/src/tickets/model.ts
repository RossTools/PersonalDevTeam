// Pure helpers for the Tickets view and the ticket tab: filters and the list query, saved filters, grouping, link
// and timeline wording.
import type { LinkedTicket } from '../api/gen/LinkedTicket';
import type { Priority } from '../api/gen/Priority';
import type { TicketEvent } from '../api/gen/TicketEvent';
import type { TicketQuery } from '../api/gen/TicketQuery';
import type { TicketSort } from '../api/gen/TicketSort';
import type { TicketStage } from '../api/gen/TicketStage';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';

export const STAGES: TicketStage[] = ['draft', 'ready', 'todo', 'in_progress', 'done', 'wont_do'];
export const OPEN_STAGES: TicketStage[] = ['draft', 'ready', 'todo', 'in_progress'];
/** What the backlog shows with no stage filter: tickets not on the board yet. */
export const BACKLOG_STAGES: TicketStage[] = ['draft', 'ready'];
export const isClosed = (s: TicketStage) => s === 'done' || s === 'wont_do';

export type GroupBy = 'none' | 'stage' | 'priority' | 'label';

export interface BacklogFilters {
  /** Empty: Draft and Ready, the tickets not on the board (every stage with `showClosed`). */
  stages: TicketStage[];
  priorities: Priority[];
  /** All of them must match. */
  labels: string[];
  showClosed: boolean;
  group: GroupBy;
  sort: TicketSort;
  /** Search text (FTS prefix words, or a key). */
  q: string;
}

export const DEFAULT_FILTERS: BacklogFilters = {
  stages: [], priorities: [], labels: [], showClosed: false, group: 'none', sort: 'rank', q: '',
};

/** Rows per page of the backlog. */
export const PAGE_SIZE = 100;

/** The `GET /api/tickets` query for a scope (one project, or every project with null) and filters. */
export function toQuery(projectId: string | null, f: BacklogFilters, limit = PAGE_SIZE): TicketQuery {
  const stages = f.stages.length > 0 ? f.stages : f.showClosed ? [] : BACKLOG_STAGES;
  const q: TicketQuery = { limit };
  if (projectId) q.project = projectId;
  if (stages.length > 0) q.stage = stages.join(',');
  if (f.priorities.length > 0) q.priority = f.priorities.join(',');
  if (f.labels.length > 0) q.label = f.labels.join(',');
  if (f.q.trim()) q.q = f.q.trim();
  if (f.sort !== 'rank') q.sort = f.sort;
  return q;
}

/** Ranks are per project: drag and Alt+↑/↓ only make sense in one project's list, sorted by rank, not grouped. */
export const canRank = (projectId: string | null, f: BacklogFilters) => !!projectId && f.sort === 'rank' && f.group === 'none';

/** True when nothing narrows the list beyond the default (Draft and Ready tickets). */
export const isDefaultFilter = (f: BacklogFilters) =>
  f.stages.length === 0 && f.priorities.length === 0 && f.labels.length === 0 && !f.showClosed && !f.q.trim();

// ---------------------------------------------------------------------------------------------------------------
// Grouping

export interface Group {
  id: string;
  title: string;
  items: TicketSummary[];
}

/**
 * The loaded rows in groups, in a fixed order (stage flow, priority high → none, labels A → Z then "No label"). A
 * ticket with several labels is listed under its first one (a row can appear only once in the list).
 */
export function groupTickets(items: TicketSummary[], by: Exclude<GroupBy, 'none'>): Group[] {
  const groups = new Map<string, TicketSummary[]>();
  const keyOf = (t: TicketSummary) => (by === 'stage' ? t.stage : by === 'priority' ? t.priority : t.labels[0] ?? '');
  for (const t of items) {
    const k = keyOf(t);
    groups.set(k, [...(groups.get(k) ?? []), t]);
  }
  const order: string[] =
    by === 'stage' ? STAGES : by === 'priority' ? PRIORITIES : [...groups.keys()].filter((k) => k).sort().concat('');
  const title = (k: string) =>
    by === 'stage' ? STAGE_LABEL[k as TicketStage] : by === 'priority' ? PRIORITY_LABEL[k as Priority] : k || 'No label';
  return order.filter((k) => groups.has(k)).map((k) => ({ id: k || 'none', title: title(k), items: groups.get(k)! }));
}

// ---------------------------------------------------------------------------------------------------------------
// Keys

/** `PDT-12`, `pdt-12`, `#12` or `12`: worth asking the API for (the palette's Go to). */
export const looksLikeKey = (q: string) => /^([A-Za-z][A-Za-z0-9]*-\d+|#?\d+)$/.test(q.trim());

// ---------------------------------------------------------------------------------------------------------------
// Saved filters (per browser, Phase 4)

export interface SavedFilter {
  id: string;
  name: string;
  /** The project it was saved in; null for all projects. */
  projectId: string | null;
  filters: BacklogFilters;
}

const SAVED_KEY = 'pdt.tickets.savedFilters';

export function loadSavedFilters(): SavedFilter[] {
  try {
    const v = JSON.parse(localStorage.getItem(SAVED_KEY) ?? '[]');
    if (!Array.isArray(v)) return [];
    return v
      .filter((x) => x && typeof x.id === 'string' && typeof x.name === 'string' && x.filters && typeof x.filters === 'object')
      .map((x) => ({ id: x.id, name: x.name, projectId: x.projectId ?? null, filters: { ...DEFAULT_FILTERS, ...x.filters } }));
  } catch {
    return [];
  }
}

export function storeSavedFilters(list: SavedFilter[]) {
  try {
    localStorage.setItem(SAVED_KEY, JSON.stringify(list));
  } catch {
    // Storage blocked (private window): saved filters last for this page only.
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Links and the timeline

export type LinkRole = 'blockedBy' | 'blocks' | 'related';

export const LINK_ROLE_LABEL: Record<LinkRole, string> = { blockedBy: 'Blocked by', blocks: 'Blocks', related: 'Related' };

export const linkRole = (l: Pick<LinkedTicket, 'kind' | 'direction'>): LinkRole =>
  l.kind === 'relates' ? 'related' : l.direction === 'outgoing' ? 'blocks' : 'blockedBy';

/**
 * The API call that adds or removes a link of `role` between `self` and `other`: `blocks` links go from the blocker,
 * so "blocked by X" is X's link to this ticket.
 */
export function linkCall(selfId: string, otherId: string, role: LinkRole): { from: string; to: string; kind: 'blocks' | 'relates' } {
  if (role === 'blockedBy') return { from: otherId, to: selfId, kind: 'blocks' };
  return { from: selfId, to: otherId, kind: role === 'blocks' ? 'blocks' : 'relates' };
}

interface LinkData { kind?: string; direction?: string; key?: string }
interface MovedData { from?: string; to?: string }
interface EditedData { fields?: string[] }

const stageName = (s: string | undefined) => (s && s in STAGE_LABEL ? STAGE_LABEL[s as TicketStage] : s ?? '?');

/** One line for a timeline entry ("Moved from Draft to Ready", "Linked: blocked by PDT-12"). */
export function eventTitle(e: TicketEvent): string {
  const data = (e.data ?? {}) as LinkData & MovedData & EditedData;
  switch (e.kind) {
    case 'created': return 'Created';
    case 'moved': return `Moved from ${stageName(data.from)} to ${stageName(data.to)}`;
    case 'edited': return data.fields?.length ? `Edited ${data.fields.join(', ')}` : 'Edited';
    case 'linked':
    case 'unlinked': {
      const role = data.kind === 'relates' ? 'related to' : data.direction === 'incoming' ? 'blocked by' : 'blocks';
      return `${e.kind === 'linked' ? 'Linked' : 'Unlinked'}: ${role} ${data.key ?? 'a ticket'}`;
    }
    case 'comment': return 'Commented';
    case 'unblocked': return 'Unblocked';
    case 'started': return 'Started';
    case 'session': return 'Session';
    case 'merged': return 'Merged';
    case 'abandoned': return 'Abandoned';
    case 'overlap': return 'Overlap';
  }
}

/** A label typed by hand, as the API stores it: spaces collapsed, lower-case, 1–30 characters, no commas. */
export function normaliseLabel(s: string): string | null {
  const v = s.trim().split(/\s+/).join(' ').toLowerCase();
  return v.length >= 1 && v.length <= 30 && !v.includes(',') ? v : null;
}

/** An area name: trimmed and kept as written, 1–60 characters, no commas. */
export function normaliseArea(s: string): string | null {
  const v = s.trim();
  return v.length >= 1 && v.length <= 60 && !v.includes(',') ? v : null;
}
