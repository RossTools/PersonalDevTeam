import { useEffect, useRef, type DragEvent, type ReactNode, type Ref } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ChevronLeft, ChevronRight, FileDiff, Plus, Ticket, X } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { ContextMenu, IconButton, Kbd } from '@ui';
import { api } from '../api/client';
import type { Tab } from '../api/gen/Tab';
import { AgentDot, itemState } from '../common/AgentState';
import { SessionKindIcon } from '../common/SessionKindIcon';
import { InlineRename } from '../common/InlineRename';
import { runAction } from '../common/useAction';
import { accepts, dragTab, dropped, type Dropped } from './dnd';
import { tabMenu } from './menus';
import { SessionTicketChip } from './SessionTicketChip';
import { itemOf } from './model';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';

const coarsePointer = () => typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;

/** Applies something dropped on a workspace's tab strip (at `index`) or on the workspace itself (the end). */
export async function applyDrop(ws: WorkspacesApi, d: Dropped, workspaceId: string, index?: number) {
  if (!d) return;
  if ('tabId' in d) {
    const t = ws.workspaces.flatMap((w) => w.tabs).find((x) => x.id === d.tabId);
    if (t) await ws.moveTab(t, workspaceId, index);
  } else {
    await ws.openIn(d.item, workspaceId, index);
  }
}

function TabButton({ ws, tab, info, selected, renaming, setRenaming, onDrop }: {
  ws: WorkspacesApi;
  tab: Tab;
  info: ItemInfo | null;
  selected: boolean;
  renaming: boolean;
  setRenaming: (id: string | null) => void;
  onDrop: (e: DragEvent, index: number) => void;
}) {
  const qc = useQueryClient();
  const title = info?.title ?? (tab.sessionId ? 'Session' : tab.ticketId ? 'Ticket' : tab.worktreeId ? 'Changes' : 'Run');
  const rename = (t: string) =>
    void runAction(async () => {
      try {
        await api.renameSession(tab.sessionId!, t);
      } finally {
        if (info) void qc.invalidateQueries({ queryKey: ['sessions', info.projectId] });
      }
    });
  return (
    <div
      className={cn(
        'relative flex shrink-0 items-center gap-1 rounded-md py-0.5 pr-0.5 pl-2',
        selected ? 'bg-accent text-foreground' : 'text-muted-foreground hover:bg-muted',
      )}
      onDragOver={(e) => { if (accepts(e)) e.preventDefault(); }}
      onDrop={(e) => onDrop(e, tab.position)}
    >
      {renaming && tab.sessionId ? (
        <InlineRename
          label="Tab name"
          initial={title}
          size="sm"
          className="w-32 text-xs"
          onCommit={(t) => { setRenaming(null); rename(t); }}
          onCancel={() => setRenaming(null)}
        />
      ) : (
        <ContextMenu label={`Tab ${title}`} entries={() => tabMenu(ws, tab, () => setRenaming(tab.id))}>
          <button
            type="button"
            role="tab"
            aria-selected={selected}
            // Touch screens move tabs from the long-press menu (workspaces spec §4.6): a natively draggable element turns
            // Android's long press into a drag, so the menu would never open.
            draggable={!coarsePointer()}
            onDragStart={(e) => dragTab(e, tab.id)}
            onClick={() => ws.selectTab(tab)}
            onDoubleClick={() => tab.sessionId && setRenaming(tab.id)}
            className="flex items-center gap-1.5 rounded-md text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring"
          >
            {tab.ticketId ? (
              <Ticket aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
            ) : tab.worktreeId ? (
              <FileDiff aria-hidden className="size-3.5 shrink-0 text-muted-foreground" />
            ) : (
              info && <AgentDot state={itemState(info)} />
            )}
            <SessionKindIcon session={info?.session} />
            <span className={cn('max-w-48 truncate', info?.unseen && 'font-semibold')}>{title}</span>
          </button>
        </ContextMenu>
      )}
      <SessionTicketChip session={info?.session} className="flex" />
      <IconButton label={`Close ${title}`} size="sm" noTooltip className="size-5" onClick={() => void runAction(() => ws.closeTab(tab))}>
        <X aria-hidden />
      </IconButton>
    </div>
  );
}

/**
 * The current workspace's name and tab strip. `hideName` leaves the name out (phones: the top bar shows it); `plusRef`
 * is the "+" button, which the new-tab chooser anchors to.
 */
export function WorkspaceBar({ ws, renamingTabId, setRenamingTabId, onNewTab, trailing, hideName = false, plusRef }: {
  ws: WorkspacesApi;
  renamingTabId: string | null;
  setRenamingTabId: (id: string | null) => void;
  onNewTab: () => void;
  trailing?: ReactNode;
  hideName?: boolean;
  plusRef?: Ref<HTMLButtonElement>;
}) {
  const w = ws.workspace;
  // Keep the current tab in view when it changes from elsewhere (sidebar, palette, keys): on a phone the strip
  // shows about one tab.
  const strip = useRef<HTMLDivElement>(null);
  const currentTabId = ws.tab?.id;
  useEffect(() => {
    strip.current?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [currentTabId]);
  const touch = useRef<{ x: number; y: number } | null>(null);
  const step = (by: number) => {
    if (!w || !ws.tab) return;
    const next = w.tabs[(ws.tab.position + by + w.tabs.length) % w.tabs.length];
    if (next) ws.selectTab(next);
  };
  const drop = (e: DragEvent, index?: number) => {
    e.preventDefault();
    e.stopPropagation();
    const d = dropped(e);
    if (w) void runAction(() => applyDrop(ws, d, w.id, index));
  };

  return (
    <div
      className="flex min-w-0 items-center gap-1 border-b border-border px-2 py-1"
      onTouchStart={(e) => { const t = e.touches[0]; touch.current = { x: t.clientX, y: t.clientY }; }}
      onTouchEnd={(e) => {
        const start = touch.current;
        touch.current = null;
        const t = e.changedTouches[0];
        if (!start || !t) return;
        const dx = t.clientX - start.x;
        if (Math.abs(dx) > 60 && Math.abs(t.clientY - start.y) < 40) step(dx < 0 ? 1 : -1);
      }}
    >
      {!hideName && <span className="max-w-32 shrink-0 truncate px-1 text-sm font-medium" title="Workspace">{w?.name ?? 'No workspace'}</span>}
      <IconButton label="Previous tab" size="sm" className="md:hidden" onClick={() => step(-1)}><ChevronLeft aria-hidden /></IconButton>
      <div
        ref={strip}
        role="tablist"
        aria-label="Tabs"
        className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto"
        onDragOver={(e) => { if (accepts(e)) e.preventDefault(); }}
        onDrop={(e) => drop(e)}
      >
        {w?.tabs.map((t) => (
          <TabButton
            key={t.id}
            ws={ws}
            tab={t}
            info={ws.info(itemOf(t))}
            selected={t.id === ws.tab?.id}
            renaming={renamingTabId === t.id}
            setRenaming={setRenamingTabId}
            onDrop={drop}
          />
        ))}
        <IconButton ref={plusRef} label="New tab" size="sm" shortcut={<Kbd keys={['Ctrl', ']', 'c']} />} disabled={!w} onClick={onNewTab}>
          <Plus aria-hidden />
        </IconButton>
      </div>
      <IconButton label="Next tab" size="sm" className="md:hidden" onClick={() => step(1)}><ChevronRight aria-hidden /></IconButton>
      {trailing}
    </div>
  );
}
