import type { MenuEntry } from '@ui';
import { api } from '../api/client';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';
import { runAction } from '../common/useAction';
import type { WorkspacesApi } from './useWorkspaces';

/** The right-click menu of a tab (also used for an agent row that has a tab). Failures toast. */
export function tabMenu(ws: WorkspacesApi, tab: Tab, onRename: () => void): MenuEntry[] {
  const w = ws.workspaces.find((x) => x.id === tab.workspaceId);
  const others = ws.workspaces.filter((x) => x.id !== tab.workspaceId);
  return [
    ...(tab.sessionId ? [{ label: 'Rename', onSelect: onRename }] : []),
    { label: 'Move left', disabled: tab.position === 0, onSelect: () => void runAction(() => ws.moveTab(tab, tab.workspaceId, tab.position - 1)) },
    { label: 'Move right', disabled: !w || tab.position >= w.tabs.length - 1, onSelect: () => void runAction(() => ws.moveTab(tab, tab.workspaceId, tab.position + 1)) },
    { heading: 'Move to workspace' },
    ...others.map((o) => ({ label: o.name, onSelect: () => void runAction(() => ws.moveTab(tab, o.id)) })),
    {
      input: 'New workspace…',
      onSubmit: (name: string) => void runAction(async () => {
        const created = await api.createWorkspace({ name });
        await ws.moveTab(tab, created.id);
        ws.selectWorkspace(created.id);
      }),
    },
    { separator: true },
    { label: 'Close tab', onSelect: () => void runAction(() => ws.closeTab(tab)) },
  ];
}

/** The right-click menu of a workspace in the sidebar. Failures toast. */
export function workspaceMenu(ws: WorkspacesApi, w: Workspace, onRename: () => void, onProjectSettings?: (projectId: string) => void): MenuEntry[] {
  const i = ws.workspaces.findIndex((x) => x.id === w.id);
  const free = w.projectId === null;
  return [
    ...(w.projectId && onProjectSettings ? [{ label: 'Project settings…', onSelect: () => onProjectSettings(w.projectId!) }] : []),
    ...(free ? [{ label: 'Rename', onSelect: onRename }] : []),
    { label: 'Move up', disabled: i <= 0, onSelect: () => void runAction(() => ws.moveWorkspace(w.id, i - 1)) },
    { label: 'Move down', disabled: i >= ws.workspaces.length - 1, onSelect: () => void runAction(() => ws.moveWorkspace(w.id, i + 1)) },
    ...(free ? [{ label: 'Delete (tabs go back to their projects)', danger: true, onSelect: () => void runAction(() => ws.deleteWorkspace(w.id)) }] : []),
  ];
}
