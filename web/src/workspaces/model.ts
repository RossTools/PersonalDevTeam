import type { OpenTab } from '../api/gen/OpenTab';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';

/** What a tab shows. */
export type Item = { kind: 'session'; id: string } | { kind: 'run'; id: string } | { kind: 'ticket'; id: string };

export function itemOf(tab: Tab): Item {
  if (tab.sessionId) return { kind: 'session', id: tab.sessionId };
  if (tab.ticketId) return { kind: 'ticket', id: tab.ticketId };
  return { kind: 'run', id: tab.runId! };
}

/** The `POST /api/workspaces/{id}/tabs` body naming an item. */
export function openTabBody(item: Item): OpenTab {
  switch (item.kind) {
    case 'session': return { sessionId: item.id };
    case 'run': return { runId: item.id };
    case 'ticket': return { ticketId: item.id };
  }
}

export const sameItem = (a: Item, b: Item) => a.kind === b.kind && a.id === b.id;

export function findTab(workspaces: Workspace[], item: Item): { workspace: Workspace; tab: Tab } | null {
  for (const workspace of workspaces) {
    const tab = workspace.tabs.find((t) => sameItem(itemOf(t), item));
    if (tab) return { workspace, tab };
  }
  return null;
}

/** Per-device choice: the workspace on screen and the last tab looked at in each workspace. */
export interface CurrentChoice {
  workspaceId: string | null;
  tabs: Record<string, string>;
}

export function resolveCurrent(workspaces: Workspace[], choice: CurrentChoice): { workspace: Workspace | null; tab: Tab | null } {
  const workspace = workspaces.find((w) => w.id === choice.workspaceId) ?? workspaces[0] ?? null;
  if (!workspace) return { workspace: null, tab: null };
  const tab = workspace.tabs.find((t) => t.id === choice.tabs[workspace.id]) ?? workspace.tabs[0] ?? null;
  return { workspace, tab };
}

/** The next tab (in workspace then tab order, wrapping) whose item needs the user, after `currentTabId`. */
export function nextNeedingYou(workspaces: Workspace[], currentTabId: string | null, needsYou: (item: Item) => boolean): Tab | null {
  const all = workspaces.flatMap((w) => w.tabs);
  const start = all.findIndex((t) => t.id === currentTabId);
  for (let i = 1; i <= all.length; i++) {
    const t = all[(start + i + all.length) % all.length];
    if (needsYou(itemOf(t))) return t;
  }
  return null;
}

/** True for a workspace a ticket's Start made: the sidebar lists these under Tickets, apart from the others. */
export const isTicketWorkspace = (w: Workspace) => !!w.ticket;

/** The workspaces listed together with `w` in the sidebar (ticket workspaces, or all the others), in order. */
export const sameGroup = (all: Workspace[], w: Workspace) => all.filter((x) => isTicketWorkspace(x) === isTicketWorkspace(w));

/**
 * The position in the full list (as `PATCH /api/workspaces` takes it) that puts `movingId` at index `to` of its group
 * (`to` counted without it). Workspaces of the other group keep their places.
 */
export function positionInAll(all: Workspace[], group: Workspace[], movingId: string, to: number): number {
  const rest = group.filter((w) => w.id !== movingId);
  const allRest = all.filter((w) => w.id !== movingId);
  if (to < rest.length) return allRest.findIndex((w) => w.id === rest[to]!.id);
  const last = rest[rest.length - 1];
  return last ? allRest.findIndex((w) => w.id === last.id) + 1 : allRest.length;
}
