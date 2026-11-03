import { memo, useMemo, useState, type MouseEvent, type ReactNode } from 'react';
import { useQueries } from '@tanstack/react-query';
import { Plus } from 'lucide-react';
import { Badge, EmptyState, IconButton, List, ListHeading, ListRow, useMenuAt, type ListDropEvent } from '@ui';
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
import { startedByStep } from '../sessions/status';
import { applyDrop } from './WorkspaceBar';
import { RowField } from '../common/RowField';
import { worktreesQuery } from '../common/worktrees';
import { useSameList } from '../common/useSameList';

const stop = (e: { stopPropagation: () => void }) => e.stopPropagation();

/** A workspace's rolled-up state: its most urgent item ("Done" when an idle agent is unseen). */
export function workspaceState(ws: WorkspacesApi, w: Workspace) {
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

/**
 * A phone's rows: 56px tall, and moved from their menus rather than dragged. The coarse-pointer variant replaces the
 * row's own 44px touch minimum, whose media query would otherwise win over a plain `min-h-14`.
 */
const TOUCH_ROW = 'min-h-14 pointer-coarse:min-h-14';

/**
 * herdr's left panel: workspaces on top, every agent below with its state and where it is. `touch` is the phone's
 * full-screen version (`/sessions`): taller rows, no dragging. `adding`/`onAddingChange` control the New workspace
 * field from outside (the phone's ⋯).
 */
export const Sidebar = memo(function Sidebar({ ws, onRenameTab, onProjectSettings, onChoose, touch = false, adding: addingProp, onAddingChange }: {
  ws: WorkspacesApi;
  onRenameTab: (tabId: string) => void;
  /** Opens a project's page in Settings. */
  onProjectSettings: (projectId: string) => void;
  /** Called after a workspace or an agent is chosen (the phone sheet closes). */
  onChoose?: () => void;
  touch?: boolean;
  adding?: boolean;
  onAddingChange?: (adding: boolean) => void;
}) {
  const { openMenu, menu } = useMenuAt();
  const [addingState, setAddingState] = useState(false);
  const adding = addingProp ?? addingState;
  const setAdding = onAddingChange ?? setAddingState;
  const drag = (group: Workspace[]) => (touch ? {} : { dragKind: 'workspace', acceptKinds: ['workspace', ...DROP_KINDS], onDrop: onDropIn(group) });
  const [renamingId, setRenamingId] = useState<string | null>(null);

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
        <ListHeading
          className="px-2"
          trailing={
            <IconButton label="New workspace" size="sm" noTooltip className="size-5" onClick={() => setAdding(true)}>
              <Plus aria-hidden />
            </IconButton>
          }
        >
          Workspaces
        </ListHeading>
        <div onDoubleClick={onWorkspaceDoubleClick}>
          <List
            id="workspace-list"
            aria-label="Workspaces"
            onAction={(id) => { ws.selectWorkspace(id); onChoose?.(); }}
            {...drag(general)}
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
                  className={touch ? TOUCH_ROW : undefined}
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
          <ListHeading className="px-2">Tickets</ListHeading>
          <div onDoubleClick={onWorkspaceDoubleClick}>
            <List
              id="ticket-workspace-list"
              aria-label="Ticket workspaces"
              onAction={(id) => { ws.selectWorkspace(id); onChoose?.(); }}
              {...drag(tickets)}
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
                    className={touch ? TOUCH_ROW : undefined}
                    title={renamingId === w.id ? nameOrRename(w) : t.title}
                    leading={state ? <AgentDot state={state} unseenDot={false} /> : <span className="inline-block size-2" />}
                    trailing={state && state.label !== 'Idle' ? <AgentLabel state={state} /> : undefined}
                    secondary={
                      <span className="flex min-w-0 items-center gap-1">
                        <span className="flex" onPointerDown={stop} onPointerUp={stop} onMouseDown={stop} onKeyDown={stop} onClick={stop} onDoubleClick={stop}>
                          <TicketKey ticketKey={t.key} title={`Open ${t.key}`} onClick={() => openTicket({ id: t.id, projectId: t.projectId })} />
                        </span>
                        {t.pr ? <PrBadge pr={t.pr} attemptMerged={t.attemptState === 'merged'} /> : t.attemptState === 'merged' ? <Badge tone="accent" className="shrink-0">Merged</Badge> : null}
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
      <AgentsSection ws={ws} onRenameTab={onRenameTab} onChoose={onChoose} touch={touch} />
      {menu}
    </div>
  );
});

/** Every agent session the user started, with its state and where it is (the Sessions sidebar, the phone drawer). */
export function AgentsSection({ ws, onRenameTab, onChoose, touch = false }: {
  ws: WorkspacesApi;
  onRenameTab: (tabId: string) => void;
  /** Called after an agent is chosen (the phone drawer closes). */
  onChoose?: () => void;
  /** 56px rows (the phone's `/sessions`). */
  touch?: boolean;
}) {
  const { openMenu, menu } = useMenuAt();
  // The user's own conversations: a workflow step's session never, even in a tab (PDT-299).
  const { everyInfo, workspaces } = ws;
  const agents = useMemo(() => everyInfo
    .filter((i) => i.session?.kind === 'agent_tui' && !startedByStep(i.session))
    .map((i) => ({ id: i.item.id, info: i, where: findTab(workspaces, i.item) }))
    .sort((a, b) => {
      const wa = a.where ? a.where.workspace.position : Infinity;
      const wb = b.where ? b.where.workspace.position : Infinity;
      return wa - wb || (a.where?.tab.position ?? 0) - (b.where?.tab.position ?? 0) || (a.info.session!.createdAt - b.info.session!.createdAt);
    }), [everyInfo, workspaces]);
  // Shared with the place picker and the views' cached queries.
  const worktreeQs = useQueries({
    queries: ws.projects.map((p) => worktreesQuery(p.id)),
  });
  const worktreeData = useSameList(worktreeQs.map((q) => q.data));
  const branches = useMemo(
    () => new Map(worktreeData.flatMap((d) => (d ?? []).map((w) => [w.id, w.branch] as const))),
    [worktreeData],
  );
  /** The worktree's branch (its folder name until the worktrees load); null for the main checkout. */
  const branchOf = (i: ItemInfo) => {
    if (!i.worktreeId) return null;
    return branches.get(i.worktreeId) ?? i.session?.cwd.split(/[\\/]/).filter(Boolean).pop() ?? null;
  };
  return (
    <>
      <section aria-label="Agents">
        <ListHeading className="px-2">Agents</ListHeading>
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
                title={
                  info.session?.ticket ? (
                    // The key leads the title row only, so the second line still runs under it.
                    <span className="flex min-w-0 items-center gap-1.5">
                      <SessionTicketChip session={info.session} className="flex shrink-0" />
                      <span className="truncate">{info.title}</span>
                    </span>
                  ) : info.title
                }
                bold={info.unseen}
                current={!!ws.tab && where?.tab.id === ws.tab.id}
                className={touch ? TOUCH_ROW : undefined}
                leading={
                  <span className="flex items-center gap-1.5">
                    <AgentDot state={state} unseenDot={false} />
                    <SessionKindIcon session={info.session} />
                  </span>
                }
                trailing={
                  state.label || info.unseen ? (
                    <>
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
    </>
  );
}
