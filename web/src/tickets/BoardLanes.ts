// The board's lanes (PDT-38): cards grouped by their parent. Pure, so the grouping is tested on its own.
import type { PullRequestStatus } from '../api/gen/PullRequestStatus';
import type { TicketParent } from '../api/gen/TicketParent';
import type { TicketSummary } from '../api/gen/TicketSummary';
import type { BoardStage } from './Board';

export interface Lane {
  /** The parent's id; null for the lane of tickets that are not parts. */
  id: string | null;
  /** The parent as a part names it (always set for a parent lane). */
  parent: TicketParent | null;
  /** The parent's own row, when a lane-header query returned it (stage, rank, roll-up, menu). */
  ticket: TicketSummary | null;
  items: Record<BoardStage, TicketSummary[]>;
}

const STAGES: BoardStage[] = ['todo', 'in_progress', 'done'];
const empty = (): Record<BoardStage, TicketSummary[]> => ({ todo: [], in_progress: [], done: [] });

/**
 * The lanes for the board's columns and the parents the lane-header queries found: the lane of tickets that are not
 * parts first, then one lane per parent, by the parent's rank (parents known only from a part's `parent`, whose rank
 * is not loaded, come after, in the order they were met). A parent with nothing on the board still gets its lane.
 */
export function buildLanes(columns: Record<BoardStage, TicketSummary[]>, parents: TicketSummary[]): Lane[] {
  const none: Lane = { id: null, parent: null, ticket: null, items: empty() };
  const byId = new Map<string, Lane>();
  for (const p of [...parents].sort((a, b) => (a.rank < b.rank ? -1 : a.rank > b.rank ? 1 : a.id.localeCompare(b.id)))) {
    if (byId.has(p.id)) continue;
    byId.set(p.id, { id: p.id, parent: { id: p.id, key: p.key, title: p.title }, ticket: p, items: empty() });
  }
  for (const stage of STAGES) {
    for (const t of columns[stage]) {
      if (!t.parent) {
        none.items[stage].push(t);
        continue;
      }
      let lane = byId.get(t.parent.id);
      if (!lane) {
        lane = { id: t.parent.id, parent: t.parent, ticket: null, items: empty() };
        byId.set(t.parent.id, lane);
      }
      lane.items[stage].push(t);
    }
  }
  return [none, ...byId.values()];
}

/** Every ticket id in a lane, all columns. */
export const laneIds = (lane: Lane) => new Set(STAGES.flatMap((s) => lane.items[s].map((t) => t.id)));

/** The PR of a part in review in the lane (an active attempt with an open PR), if one is on the board. */
export function lanePr(lane: Lane): PullRequestStatus | null {
  for (const s of STAGES) {
    for (const t of lane.items[s]) {
      if (t.attempt?.state === 'active' && t.attempt.pr?.state === 'open') return t.attempt.pr;
    }
  }
  return null;
}

/** The lanes collapsed in this browser (localStorage; any failure reads as none and writes nothing). */
export const COLLAPSED_KEY = 'pdt.board.collapsedLanes';

export function readCollapsed(): Set<string> {
  try {
    const v = JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? '[]');
    return new Set(Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
  } catch {
    return new Set();
  }
}

export function writeCollapsed(ids: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...ids]));
  } catch {
    // Private window or blocked storage: the lane stays collapsed for this visit only.
  }
}
