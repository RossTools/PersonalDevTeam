import { useState, type MouseEvent, type ReactNode } from 'react';
import { useQueries } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { Button, EmptyState, IconButton, List, ListRow, useMenuAt, type ListDropEvent } from '@ui';
import { api } from '../api/client';
import type { Workspace } from '../api/gen/Workspace';
import { AgentDot, AgentLabel, itemState, rollup, UnseenDot } from '../common/AgentState';
import { ProjectKey } from '../common/BranchChip';
import { InlineRename } from '../common/InlineRename';
import { runAction } from '../common/useAction';
import { DROP_KINDS, droppedOf } from './dnd';
import { agentMenu, workspaceMenu } from './menus';
import { findTab, itemOf } from './model';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';
import { SessionTicketChip } from './SessionTicketChip';
import { applyDrop } from './WorkspaceBar';

const heading = 'flex items-center px-2 pt-3 pb-1 text-2xs tracking-wider text-muted-foreground uppercase';

/** Keeps keys and clicks inside an in-row text field away from the row (typeahead, selection, drag). */
function RowField({ children }: { children: ReactNode }) {
  const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();
  return (
    <span className="block" onKeyDown={stop} onKeyUp={stop} onPointerDown={stop} onMouseDown={stop} onClick={stop} onDoubleClick={stop}>
      {children}
    </span>
  );
}

/** A workspace's rolled-up state: its most urgent item ("Done" when an idle agent is unseen). */
function workspaceState(ws: WorkspacesApi, w: Workspace) {
  return rollup(w.tabs.map((t) => ws.info(itemOf(t))).filter((i): i is ItemInfo => i !== null).map(itemState));
}

/** Where a workspace dropped at `target` goes: before/on a row takes its place, after goes behind it. */
function reorderTo(list: Workspace[], movingId: string, target: ListDropEvent['target']): number | null {
  const from = list.findIndex((w) => w.id === movingId);
  if (from < 0) return null;
  if (!target) return list.length - 1;
  if (target.id === movingId) return null;
  const to = list.findIndex((w) => w.id === target.id);
  if (to < 0) return null;
  const at = to + (target.position === 'after' ? 1 : 0);
  return at > from ? at - 1 : at;
}

// Rows are static children, not `items` + a render function: GridList caches rendered items by object, and these rows
// also depend on outside state (the current workspace or tab, renaming).

/**
 * An agent row's second line: its branch (nothing for the main checkout), then its workspace when that isn't the one
 * on screen ("no tab" when it has none). Null when there is nothing to say.
 */
export function agentSecondary(branch: string | null, workspaceName: string | null, isCurrentWorkspace: boolean): string | null {
  const parts = [branch, workspaceName === null ? 'no tab' : isCurrentWorkspace ? null : workspaceName].filter((p): p is string => !!p);
  return parts.length > 0 ? parts.join(' · ') : null;
}

/** herdr's left panel: workspaces on top, every agent below with its state and where it is. */
export function Sidebar({ ws, onRenameTab, onProjectSettings, onChoose }: {
  ws: WorkspacesApi;
  onRenameTab: (tabId: string) => void;
  /** Opens a project's page in Settings. */
  onProjectSettings: (projectId: string) => void;
  /** Called after a workspace or an agent is chosen (the phone sheet closes). */
  onChoose?: () => void;
}) {
  const { openMenu, menu } = useMenuAt();
  const [adding, setAdding] = useState(false);
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const agents = ws.allInfos
    .filter((i) => i.session?.kind === 'agent_tui')
    .map((i) => ({ id: i.item.id, info: i, where: findTab(ws.workspaces, i.item) }))
    .sort((a, b) => {
      const wa = a.where ? a.where.workspace.position : Infinity;
      const wb = b.where ? b.where.workspace.position : Infinity;
      return wa - wb || (a.where?.tab.position ?? 0) - (b.where?.tab.position ?? 0) || (a.info.session!.createdAt - b.info.session!.createdAt);
    });
  // Shared with the place picker and the views' cached queries.
  const worktreeQs = useQueries({
    queries: ws.projects.map((p) => ({ queryKey: ['worktrees', p.id], queryFn: () => api.listWorktrees(p.id), staleTime: 30_000 })),
  });
  const branches = new Map(worktreeQs.flatMap((q) => (q.data ?? []).map((w) => [w.id, w.branch] as const)));
  /** The worktree's branch (its folder name until the worktrees load); null for the main checkout. */
  const branchOf = (i: ItemInfo) => {
    if (!i.worktreeId) return null;
    return branches.get(i.worktreeId) ?? i.session?.cwd.split(/[\\/]/).filter(Boolean).pop() ?? null;
  };

  const onWorkspaceDrop = (e: ListDropEvent) => {
    if (e.kind === 'workspace') {
      const to = reorderTo(ws.workspaces, e.id, e.target);
      if (to !== null) void runAction(() => ws.moveWorkspace(e.id, to));
      return;
    }
    // A tab or an agent dropped on (or next to) a workspace row goes to the end of that workspace.
    if (e.target) void runAction(() => applyDrop(ws, droppedOf(e), e.target!.id));
  };
  // Double-clicking a free-form workspace renames it.
  const onWorkspaceDoubleClick = (e: MouseEvent<HTMLElement>) => {
    const id = (e.target as HTMLElement).closest<HTMLElement>('[data-key]')?.dataset.key;
    const w = ws.workspaces.find((x) => x.id === id);
    if (w && w.projectId === null) setRenamingId(w.id);
  };

  return (
    <div className="flex flex-col">
      <section aria-label="Workspaces">
        <h2 className={heading}>
          <span className="flex-1">Workspaces</span>
          <IconButton label="New workspace" size="sm" noTooltip className="size-5" onClick={() => setAdding(true)}>
            <Plus aria-hidden />
          </IconButton>
        </h2>
        <div onDoubleClick={onWorkspaceDoubleClick}>
          <List
            id="workspace-list"
            aria-label="Workspaces"
            onAction={(id) => { ws.selectWorkspace(id); onChoose?.(); }}
            dragKind="workspace"
            acceptKinds={['workspace', ...DROP_KINDS]}
            onDrop={onWorkspaceDrop}
          >
            {ws.workspaces.map((w) => {
              const state = workspaceState(ws, w);
              const projectKey = w.projectId ? ws.projects.find((p) => p.id === w.projectId)?.key : undefined;
              const renaming = renamingId === w.id;
              return (
                <ListRow
                  key={w.id}
                  id={w.id}
                  textValue={w.name}
                  current={ws.workspace?.id === w.id}
                  title={renaming ? (
                    <RowField>
                      <InlineRename
                        label="Workspace name"
                        initial={w.name}
                        onCommit={(name) => { setRenamingId(null); void runAction(() => ws.renameWorkspace(w.id, name)); }}
                        onCancel={() => setRenamingId(null)}
                      />
                    </RowField>
                  ) : w.name}
                  leading={state ? <AgentDot state={state} unseenDot={false} /> : <span className="inline-block size-2" />}
                  trailing={
                    <>
                      {projectKey && <ProjectKey projectKey={projectKey} />}
                      {state && state.label !== 'Idle' && <AgentLabel state={state} />}
                      <span className="text-meta text-muted-foreground tabular-nums">{w.tabs.length}</span>
                    </>
                  }
                  onMenu={(at) => openMenu(at, `Workspace ${w.name}`, () => workspaceMenu(ws, w, () => setRenamingId(w.id), onProjectSettings))}
                />
              );
            })}
          </List>
        </div>
        {adding && (
          <div className="px-1 pt-0.5">
            <InlineRename
              label="New workspace name"
              placeholder="Workspace name"
              onCommit={(name) => { setAdding(false); void runAction(() => ws.createWorkspace(name)); }}
              onCancel={() => setAdding(false)}
            />
          </div>
        )}
      </section>
      <section aria-label="Agents">
        <h2 className={heading}>Agents</h2>
        <List
          aria-label="Agents"
          onAction={(id) => {
            const a = agents.find((x) => x.id === id);
            if (a) void runAction(() => ws.focusItem(a.info.item, a.info.projectId));
            onChoose?.();
          }}
          renderEmpty={() => <EmptyState>No agents yet. Start one with + in the tab bar (Ctrl+] c).</EmptyState>}
        >
          {agents.map(({ id, info, where }) => {
            const state = itemState(info);
            return (
              <ListRow
                key={id}
                id={id}
                textValue={info.title}
                title={info.title}
                bold={info.unseen}
                current={!!ws.tab && where?.tab.id === ws.tab.id}
                leading={<AgentDot state={state} unseenDot={false} />}
                trailing={
                  state.label || info.unseen || info.session?.ticket ? (
                    <>
                      <SessionTicketChip session={info.session} className="flex" />
                      {state.label && <AgentLabel state={state} />}
                      {info.unseen && <UnseenDot />}
                    </>
                  ) : undefined
                }
                secondary={agentSecondary(branchOf(info), where?.workspace.name ?? null, where?.workspace.id === ws.workspace?.id) ?? undefined}
                onMenu={(at) => openMenu(at, `Agent ${info.title}`, () => agentMenu(
                  () => void runAction(() => ws.focusItem(info.item, info.projectId)),
                  where ? () => { ws.selectTab(where.tab); onRenameTab(where.tab.id); } : undefined,
                ))}
              />
            );
          })}
        </List>
      </section>
      {menu}
    </div>
  );
}
