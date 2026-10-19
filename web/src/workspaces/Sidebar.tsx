import { useState, type MouseEvent, type ReactNode } from 'react';
import { useQueries } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { Badge, EmptyState, IconButton, List, ListRow, useMenuAt, type ListDropEvent } from '@ui';
import { api } from '../api/client';
import type { Workspace } from '../api/gen/Workspace';
import { AgentDot, AgentLabel, itemState, rollup, UnseenDot } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { ProjectKey } from '../common/BranchChip';
import { InlineRename } from '../common/InlineRename';
import { runAction } from '../common/useAction';
import { DROP_KINDS, droppedOf } from './dnd';
import { agentMenu, workspaceMenu } from './menus';
import { findTab, isTicketWorkspace, itemOf, positionInAll } from './model';
import { TicketKey } from '../common/TicketKey';
import { PrBadge } from '../tickets/PullRequest';
import { useOpenTicket } from '../tickets/actions';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';
import { SessionTicketChip } from './SessionTicketChip';
import { applyDrop } from './WorkspaceBar';

const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

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

  const openTicket = useOpenTicket();
  const general = ws.workspaces.filter((w) => !isTicketWorkspace(w));
  const tickets = ws.workspaces.filter(isTicketWorkspace);
  /** Drops on one section's list: workspaces reorder within it; a tab or agent goes to the end of the row's workspace. */
  const onDropIn = (group: Workspace[]) => (e: ListDropEvent) => {
    if (e.kind === 'workspace') {
      if (!group.some((w) => w.id === e.id)) return;
      const to = reorderTo(group, e.id, e.target);
      if (to !== null) void runAction(() => ws.moveWorkspace(e.id, positionInAll(ws.workspaces, group, e.id, to)));
      return;
    }
    if (e.target) void runAction(() => applyDrop(ws, droppedOf(e), e.target!.id));
  };
  // Double-clicking a free-form workspace renames it.
  const onWorkspaceDoubleClick = (e: MouseEvent<HTMLElement>) => {
    const id = (e.target as HTMLElement).closest<HTMLElement>('[data-key]')?.dataset.key;
    const w = ws.workspaces.find((x) => x.id === id);
    if (w && w.projectId === null) setRenamingId(w.id);
  };

  /** A row's name, or the rename field while it is being renamed. */
  const nameOrRename = (w: Workspace): ReactNode => renamingId === w.id ? (
    <RowField>
      <InlineRename
        label="Workspace name"
        initial={w.name}
        onCommit={(name) => { setRenamingId(null); void runAction(() => ws.renameWorkspace(w.id, name)); }}
        onCancel={() => setRenamingId(null)}
      />
    </RowField>
  ) : w.name;

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
            onDrop={onDropIn(general)}
          >
            {general.map((w) => {
              const state = workspaceState(ws, w);
              const projectKey = w.projectId ? ws.projects.find((p) => p.id === w.projectId)?.key : undefined;
              return (
                <ListRow
                  key={w.id}
                  id={w.id}
                  textValue={w.name}
                  current={ws.workspace?.id === w.id}
                  title={nameOrRename(w)}
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
      {tickets.length > 0 && (
        <section aria-label="Ticket workspaces">
          <h2 className={heading}>Tickets</h2>
          <div onDoubleClick={onWorkspaceDoubleClick}>
            <List
              id="ticket-workspace-list"
              aria-label="Ticket workspaces"
              onAction={(id) => { ws.selectWorkspace(id); onChoose?.(); }}
              dragKind="workspace"
              acceptKinds={['workspace', ...DROP_KINDS]}
              onDrop={onDropIn(tickets)}
            >
              {tickets.map((w) => {
                const t = w.ticket!;
                const state = workspaceState(ws, w);
                return (
                  <ListRow
                    key={w.id}
                    id={w.id}
                    textValue={`${t.key} ${t.title}`}
                    current={ws.workspace?.id === w.id}
                    title={renamingId === w.id ? nameOrRename(w) : t.title}
                    leading={state ? <AgentDot state={state} unseenDot={false} /> : <span className="inline-block size-2" />}
                    trailing={state && state.label !== 'Idle' ? <AgentLabel state={state} /> : undefined}
                    secondary={
                      <span className="flex min-w-0 items-center gap-1">
                        <span className="flex" onPointerDown={stop} onPointerUp={stop} onMouseDown={stop} onKeyDown={stop} onClick={stop} onDoubleClick={stop}>
                          <TicketKey ticketKey={t.key} title={`Open ${t.key}`} onClick={() => openTicket({ id: t.id, projectId: t.projectId })} />
                        </span>
                        {t.pr ? <PrBadge pr={t.pr} /> : t.attemptState === 'merged' ? <Badge tone="accent" className="shrink-0">Merged</Badge> : null}
                      </span>
                    }
                    onMenu={(at) => openMenu(at, `Workspace ${w.name}`, () => workspaceMenu(ws, w, () => setRenamingId(w.id), onProjectSettings))}
                  />
                );
              })}
            </List>
          </div>
        </section>
      )}
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
                leading={
                  <span className="flex items-center gap-1.5">
                    <AgentDot state={state} unseenDot={false} />
                    <SessionKindIcon session={info.session} />
                  </span>
                }
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
