import type { Status } from '@ui';
import type { Activity } from '../api/gen/Activity';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';

/** What a tab shows. */
export type Item = { kind: 'session'; id: string } | { kind: 'run'; id: string };

export function itemOf(tab: Tab): Item {
  return tab.sessionId ? { kind: 'session', id: tab.sessionId } : { kind: 'run', id: tab.runId! };
}

export const sameItem = (a: Item, b: Item) => a.kind === b.kind && a.id === b.id;

export function findTab(workspaces: Workspace[], item: Item): { workspace: Workspace; tab: Tab } | null {
  for (const workspace of workspaces) {
    const tab = workspace.tabs.find((t) => sameItem(itemOf(t), item));
    if (tab) return { workspace, tab };
  }
  return null;
}

const RANK: Record<Activity, number> = { needs_you: 3, working: 2, idle: 1, unknown: 0 };

/** The most urgent activity: needs you, then working, then idle. */
export function worstActivity(list: (Activity | null | undefined)[]): Activity | null {
  let worst: Activity | null = null;
  for (const a of list) if (a && (worst === null || RANK[a] > RANK[worst])) worst = a;
  return worst;
}

/** The status-dot colour for an activity (same colours as the activity badge). */
export function activityStatus(a: Activity | null): Status | null {
  switch (a) {
    case 'needs_you': return 'waiting';
    case 'working': return 'running';
    case 'idle':
    case 'unknown': return 'idle';
    default: return null;
  }
}

export type AgentState = 'Needs you' | 'Working' | 'Done' | 'Idle' | 'Unknown';

/** herdr-style state words: an idle agent nobody has looked at since it finished is "Done". */
export function agentState(activity: Activity | null, unseen: boolean): AgentState | null {
  switch (activity) {
    case 'needs_you': return 'Needs you';
    case 'working': return 'Working';
    case 'idle': return unseen ? 'Done' : 'Idle';
    case 'unknown': return 'Unknown';
    default: return null;
  }
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
