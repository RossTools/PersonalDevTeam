import { useState, type KeyboardEvent } from 'react';
import { Plus } from 'lucide-react';
import { StatusDot } from '@ui';
import type { Workspace } from '../api/gen/Workspace';
import { useContextMenu } from './ContextMenu';
import { accepts, dragItem, dropped } from './dnd';
import { tabMenu, workspaceMenu } from './menus';
import { activityStatus, agentState, findTab, itemOf, worstActivity } from './model';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';
import { applyDrop } from './WorkspaceBar';

const WORKSPACE_MIME = 'application/x-pdt-workspace';
const heading = 'px-2 pt-3 pb-1 text-[10px] uppercase tracking-wider text-muted flex items-center';
const row = 'w-full flex items-center gap-2 text-left px-2 py-1 rounded-pdt text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring';

/** Up/Down move focus between the rows of a list (Enter clicks the focused one). */
function arrowKeys(e: KeyboardEvent<HTMLElement>) {
  if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
  e.preventDefault();
  const rows = [...e.currentTarget.querySelectorAll<HTMLElement>('[data-row]')];
  const i = rows.indexOf(document.activeElement as HTMLElement);
  rows[(i + (e.key === 'ArrowDown' ? 1 : -1) + rows.length) % rows.length]?.focus();
}

function StateDot({ activity, fallback }: { activity: ItemInfo['activity']; fallback: ItemInfo['status'] }) {
  return <StatusDot status={activityStatus(activity) ?? fallback} />;
}

/** A workspace's rolled-up state: its most urgent agent, "Done" when an idle one is unseen. */
function rollup(ws: WorkspacesApi, w: Workspace) {
  const infos = w.tabs.map((t) => ws.info(itemOf(t))).filter((i): i is ItemInfo => i !== null && i.activity !== null);
  const activity = worstActivity(infos.map((i) => i.activity));
  return { activity, state: agentState(activity, infos.some((i) => i.unseen)) };
}

function WorkspaceRow({ ws, w, onMenu, onProjectSettings }: {
  ws: WorkspacesApi;
  w: Workspace;
  onMenu: ReturnType<typeof useContextMenu>['bind'];
  onProjectSettings: (projectId: string) => void;
}) {
  const [renaming, setRenaming] = useState(false);
  const { activity, state } = rollup(ws, w);
  const current = ws.workspace?.id === w.id;
  const projectKey = w.projectId ? ws.projects.find((p) => p.id === w.projectId)?.key : undefined;
  if (renaming) {
    return (
      <form className="px-1" onSubmit={(e) => {
        e.preventDefault();
        const name = new FormData(e.currentTarget).get('name')?.toString().trim();
        setRenaming(false);
        if (name && name !== w.name) void ws.renameWorkspace(w.id, name);
      }}>
        <input autoFocus name="name" aria-label="Workspace name" defaultValue={w.name} onBlur={(e) => e.currentTarget.form?.requestSubmit()}
          onKeyDown={(e) => { if (e.key === 'Escape') setRenaming(false); }}
          className="w-full px-2 py-1 rounded-pdt text-sm bg-surface border border-border outline-none" />
      </form>
    );
  }
  return (
    <button
      type="button"
      data-row
      aria-current={current ? 'true' : undefined}
      draggable
      onDragStart={(e) => e.dataTransfer.setData(WORKSPACE_MIME, w.id)}
      onDragOver={(e) => { if (accepts(e) || e.dataTransfer.types.includes(WORKSPACE_MIME)) e.preventDefault(); }}
      onDrop={(e) => {
        e.preventDefault();
        const moving = e.dataTransfer.getData(WORKSPACE_MIME);
        if (moving) { if (moving !== w.id) void ws.moveWorkspace(moving, w.position); }
        else void applyDrop(ws, dropped(e), w.id);
      }}
      onClick={() => ws.selectWorkspace(w.id)}
      onDoubleClick={() => w.projectId === null && setRenaming(true)}
      {...onMenu(`Workspace ${w.name}`, () => workspaceMenu(ws, w, () => setRenaming(true), onProjectSettings))}
      className={`${row} ${current ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
    >
      {activity ? <StateDot activity={activity} fallback="idle" /> : <span className="inline-block size-2" />}
      <span className="flex-1 truncate">{w.name}</span>
      {projectKey && <span className="font-mono text-[10px] text-muted">{projectKey}</span>}
      {state && state !== 'Idle' && <span className={`text-[11px] ${state === 'Needs you' ? 'text-waiting font-semibold' : 'text-muted'}`}>{state}</span>}
      <span className="text-[11px] text-muted tabular-nums">{w.tabs.length}</span>
    </button>
  );
}

/** herdr's left panel: workspaces on top, every agent below with its state and where it is. */
export function Sidebar({ ws, onRenameTab, onProjectSettings, onAddProject }: {
  ws: WorkspacesApi;
  onRenameTab: (tabId: string) => void;
  /** Opens a project's page in Settings. */
  onProjectSettings: (projectId: string) => void;
  onAddProject: () => void;
}) {
  const { bind, menu } = useContextMenu();
  const [adding, setAdding] = useState(false);
  const agents = ws.allInfos
    .filter((i) => i.session?.kind === 'agent_tui')
    .map((i) => ({ info: i, where: findTab(ws.workspaces, i.item) }))
    .sort((a, b) => {
      const wa = a.where ? a.where.workspace.position : Infinity;
      const wb = b.where ? b.where.workspace.position : Infinity;
      return wa - wb || (a.where?.tab.position ?? 0) - (b.where?.tab.position ?? 0) || (a.info.session!.createdAt - b.info.session!.createdAt);
    });
  const branchOf = (i: ItemInfo) => {
    const project = ws.projects.find((p) => p.id === i.projectId);
    return i.session?.cwd && i.worktreeId ? i.session.cwd.split(/[\\/]/).pop() : project?.name;
  };

  return (
    <div className="flex flex-col">
      <section aria-label="Workspaces">
        <h2 className={heading}>
          <span className="flex-1">Workspaces</span>
          <button type="button" aria-label="New workspace" onClick={() => setAdding(true)} className="rounded-pdt p-0.5 hover:bg-surface-2 hover:text-fg">
            <Plus size={12} aria-hidden />
          </button>
        </h2>
        <div id="workspace-list" className="flex flex-col gap-0.5" onKeyDown={arrowKeys}>
          {ws.workspaces.map((w) => <WorkspaceRow key={w.id} ws={ws} w={w} onMenu={bind} onProjectSettings={onProjectSettings} />)}
        </div>
        {adding && (
          <form className="px-1 pt-0.5" onSubmit={(e) => {
            e.preventDefault();
            const name = new FormData(e.currentTarget).get('name')?.toString().trim();
            setAdding(false);
            if (name) void ws.createWorkspace(name);
          }}>
            <input autoFocus name="name" aria-label="New workspace name" placeholder="Workspace name"
              onBlur={() => setAdding(false)} onKeyDown={(e) => { if (e.key === 'Escape') setAdding(false); }}
              className="w-full px-2 py-1 rounded-pdt text-sm bg-surface border border-border outline-none" />
          </form>
        )}
      </section>
      <section aria-label="Agents">
        <h2 className={heading}>Agents</h2>
        {agents.length === 0 ? (
          <p className="px-2 text-xs text-muted">No agents yet. Start one with + in the tab bar (Ctrl+] c).</p>
        ) : (
          <ul className="flex flex-col gap-0.5" onKeyDown={arrowKeys}>
            {agents.map(({ info, where }) => {
              const state = agentState(info.activity, info.unseen);
              const current = ws.tab && where?.tab.id === ws.tab.id;
              return (
                <li key={info.item.id}>
                  <button
                    type="button"
                    data-row
                    aria-current={current ? 'true' : undefined}
                    draggable
                    onDragStart={(e) => dragItem(e, info.item, info.projectId)}
                    onClick={() => void ws.focusItem(info.item, info.projectId)}
                    {...bind(`Agent ${info.title}`, () => (where ? tabMenu(ws, where.tab, () => { ws.selectTab(where.tab); onRenameTab(where.tab.id); }) : [
                      { label: 'Open', onSelect: () => void ws.focusItem(info.item, info.projectId) },
                    ]))}
                    className={`${row} flex-col items-stretch gap-0 ${current ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}
                  >
                    <span className="flex items-center gap-2">
                      <StateDot activity={info.session?.status === 'running' ? info.activity : null} fallback={info.status} />
                      <span className={`flex-1 truncate ${info.unseen ? 'font-semibold' : ''}`}>{info.title}</span>
                      {state && (
                        <span className={`text-[11px] ${state === 'Needs you' ? 'text-waiting font-semibold' : state === 'Working' ? 'text-running' : 'text-muted'}`}>
                          {state}
                        </span>
                      )}
                      {info.unseen && <span role="img" aria-label="unseen" className="inline-block size-1.5 rounded-full bg-accent" />}
                    </span>
                    <span className="pl-4 text-[11px] text-muted truncate">
                      {where ? where.workspace.name : 'no tab'} · {branchOf(info)}
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </section>
      <button type="button" onClick={onAddProject} className="mt-3 mx-2 self-start inline-flex items-center gap-1 text-xs text-muted hover:text-fg">
        <Plus size={12} aria-hidden /> Add project
      </button>
      {menu}
    </div>
  );
}
