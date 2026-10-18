import type { ThemePref } from '@ui';
import type { Project } from '../api/gen/Project';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';
import { itemState, type AgentStateValue } from '../common/AgentState';
import type { Place } from '../common/PlacePicker';
import { findTab, itemOf, nextNeedingYou, type Item } from '../workspaces/model';
import type { ItemInfo } from '../workspaces/useWorkspaces';

export const GO_TO = 'Go to';
export const ACTIONS = 'Actions';

/** Which icon the palette draws for an item (kept as data so this module stays pure). */
export type PaletteIcon =
  | 'workspace' | 'tab' | 'agent' | 'settings' | 'host' | 'claude' | 'codex' | 'shell' | 'run' | 'rename' | 'close'
  | 'move' | 'needsYou' | 'theme' | 'keys' | 'check' | 'add';

/** A second step: another list (Move tab → workspaces), or a text field (New workspace → its name). */
export type PaletteStep =
  | { kind: 'list'; title: string; placeholder: string; items: PaletteItem[] }
  | { kind: 'input'; title: string; label: string; placeholder?: string; submitLabel: string; run: (value: string) => void };

export interface PaletteItem {
  id: string;
  label: string;
  group: string;
  /** Secondary text after the label (a tab's workspace). Also matched by the filter. */
  detail?: string;
  /** Extra words the filter matches (project key, branch). */
  keywords?: string[];
  /** The Ctrl+] key hint, e.g. "Ctrl+] x". */
  shortcut?: string;
  icon?: PaletteIcon;
  /** Tabs and agents show their state dot. */
  state?: AgentStateValue;
  /** Runs the item; the palette closes first. */
  run?: () => void;
  /** Opens a second step instead of running. */
  next?: PaletteStep;
}

export type NewKind = 'claude' | 'codex' | 'shell';

/** What the items do. AppShell implements them with the workspaces API (through runAction). */
export interface PaletteHandlers {
  selectWorkspace: (workspaceId: string) => void;
  selectTab: (tab: Tab) => void;
  /** Shows an item without a tab (opens one in its project's workspace). */
  focusItem: (item: Item, projectId: string) => void;
  projectSettings: (projectId: string) => void;
  hostSettings: () => void;
  newSession: (kind: NewKind, place: Place) => void;
  /** Opens the new-tab chooser (runs, tests, other branches). */
  openChooser: () => void;
  renameTab: (tab: Tab) => void;
  closeTab: (tab: Tab) => void;
  moveTab: (tab: Tab, workspaceId: string) => void;
  /** Makes a workspace with this name and moves the tab there. */
  moveTabToNewWorkspace: (tab: Tab, name: string) => void;
  setTheme: (pref: ThemePref) => void;
  showKeys: () => void;
}

export interface PaletteContext {
  workspaces: Workspace[];
  /** Every session and run (WorkspacesApi.allInfos). */
  infos: ItemInfo[];
  projects: Project[];
  /** The tab on screen. */
  tab: Tab | null;
  /** Where "New … here" starts: the current tab's checkout, else the current workspace's project. */
  place: Place | null;
  theme: ThemePref;
  handlers: PaletteHandlers;
}

const key = (k: string) => `Ctrl+] ${k}`;
const infoKey = (item: Item) => `${item.kind}:${item.id}`;

/** A run's branch, or a worktree session's folder name (the branch); null for the main checkout. */
export function branchOf(info: ItemInfo): string | null {
  if (info.run) return info.run.branch;
  if (info.session && info.worktreeId) return info.session.cwd.split(/[\\/]/).filter(Boolean).pop() ?? null;
  return null;
}

const needsYou = (info: ItemInfo | undefined) => info?.activity === 'needs_you';

/** The command palette's items: "Go to" (workspaces, tabs, agents without a tab, settings) then "Actions". */
export function paletteItems(ctx: PaletteContext): PaletteItem[] {
  const { workspaces, projects, tab, place, handlers: h } = ctx;
  const byItem = new Map(ctx.infos.map((i) => [infoKey(i.item), i]));
  const projectOf = (id: string) => projects.find((p) => p.id === id);
  const wordsFor = (info: ItemInfo | undefined) => {
    if (!info) return [];
    const branch = branchOf(info);
    const project = projectOf(info.projectId);
    return [project?.key, project?.name, branch].filter((w): w is string => !!w);
  };

  const goTo: PaletteItem[] = [];
  for (const w of workspaces) {
    goTo.push({ id: `workspace:${w.id}`, label: `Workspace · ${w.name}`, group: GO_TO, icon: 'workspace', run: () => h.selectWorkspace(w.id) });
  }
  for (const w of workspaces) {
    for (const t of w.tabs) {
      const info = byItem.get(infoKey(itemOf(t)));
      goTo.push({
        id: `tab:${t.id}`,
        label: info?.title ?? 'Untitled',
        group: GO_TO,
        detail: w.name,
        keywords: wordsFor(info),
        icon: 'tab',
        ...(info ? { state: itemState(info) } : {}),
        run: () => h.selectTab(t),
      });
    }
  }
  for (const info of ctx.infos) {
    if (info.session?.kind !== 'agent_tui' || findTab(workspaces, info.item)) continue;
    goTo.push({
      id: `agent:${info.item.id}`,
      label: info.title,
      group: GO_TO,
      detail: 'no tab',
      keywords: wordsFor(info),
      icon: 'agent',
      state: itemState(info),
      run: () => h.focusItem(info.item, info.projectId),
    });
  }
  for (const p of projects) {
    goTo.push({
      id: `settings:${p.id}`, label: `Settings · ${p.name}`, group: GO_TO, keywords: [p.key], icon: 'settings',
      run: () => h.projectSettings(p.id),
    });
  }
  goTo.push({ id: 'settings:host', label: 'Settings · This host', group: GO_TO, icon: 'host', run: h.hostSettings });

  const actions: PaletteItem[] = [];
  if (place) {
    const words = [projectOf(place.projectId)?.key].filter((w): w is string => !!w);
    actions.push(
      { id: 'new:claude', label: 'New Claude here', group: ACTIONS, icon: 'claude', keywords: words, run: () => h.newSession('claude', place) },
      { id: 'new:codex', label: 'New Codex here', group: ACTIONS, icon: 'codex', keywords: words, run: () => h.newSession('codex', place) },
      { id: 'new:shell', label: 'New shell here', group: ACTIONS, icon: 'shell', keywords: words, run: () => h.newSession('shell', place) },
    );
  }
  actions.push({ id: 'run', label: 'Run…', group: ACTIONS, icon: 'run', shortcut: key('c'), keywords: ['new tab', 'test'], run: h.openChooser });
  if (tab) {
    const t = tab;
    // Only sessions can be renamed (a run's title is its name and branch).
    if (t.sessionId) actions.push({ id: 'tab:rename', label: 'Rename tab', group: ACTIONS, icon: 'rename', shortcut: key('r'), run: () => h.renameTab(t) });
    actions.push({ id: 'tab:close', label: 'Close tab', group: ACTIONS, icon: 'close', shortcut: key('x'), run: () => h.closeTab(t) });
    const others = workspaces.filter((w) => w.id !== t.workspaceId);
    actions.push({
      id: 'tab:move',
      label: 'Move tab to workspace…',
      group: ACTIONS,
      icon: 'move',
      next: {
        kind: 'list',
        title: 'Move tab to workspace',
        placeholder: 'Filter workspaces…',
        items: [
          ...others.map((w): PaletteItem => ({
            id: `move:${w.id}`, label: w.name, group: 'Workspaces', icon: 'workspace', run: () => h.moveTab(t, w.id),
          })),
          {
            id: 'move:new',
            label: 'New workspace…',
            group: 'Workspaces',
            icon: 'add',
            next: { kind: 'input', title: 'New workspace', label: 'Name', placeholder: 'Workspace name', submitLabel: 'Create and move', run: (name) => h.moveTabToNewWorkspace(t, name) },
          },
        ],
      },
    });
  }
  const next = nextNeedingYou(workspaces, tab?.id ?? null, (item) => needsYou(byItem.get(infoKey(item))));
  if (next) actions.push({ id: 'needs-you', label: 'Next needs you', group: ACTIONS, icon: 'needsYou', shortcut: key('a'), run: () => h.selectTab(next) });
  for (const [pref, name] of [['system', 'System'], ['light', 'Light'], ['dark', 'Dark']] as const) {
    actions.push({
      id: `theme:${pref}`, label: `Theme: ${name}`, group: ACTIONS, icon: ctx.theme === pref ? 'check' : 'theme',
      keywords: ['appearance', 'mode'], run: () => h.setTheme(pref),
    });
  }
  actions.push({ id: 'keys', label: 'Keyboard shortcuts', group: ACTIONS, icon: 'keys', shortcut: key('?'), keywords: ['help', 'keys'], run: h.showKeys });

  return [...goTo, ...actions];
}
