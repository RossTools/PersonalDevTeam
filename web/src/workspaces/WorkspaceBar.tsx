import { useRef, useState, type DragEvent, type ReactNode } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ChevronLeft, ChevronRight, Plus, X } from 'lucide-react';
import { StatusDot } from '@ui';
import { api } from '../api/client';
import type { Tab } from '../api/gen/Tab';
import { useContextMenu } from './ContextMenu';
import { accepts, dragTab, dropped, type Dropped } from './dnd';
import { tabMenu } from './menus';
import { activityStatus, itemOf, worstActivity } from './model';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';

const small = 'rounded-pdt p-0.5 text-muted-foreground hover:text-fg hover:bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-ring';

/** Most urgent activity and whether anything is unseen, over some items. */
export function summarize(infos: (ItemInfo | null)[]) {
  const present = infos.filter((i): i is ItemInfo => i !== null);
  return { activity: worstActivity(present.map((i) => i.activity)), unseen: present.some((i) => i.unseen) };
}

/** A coloured dot for an agent that is working or needs you, plus the unseen dot. */
export function AttentionDots({ activity, unseen, label }: { activity: ItemInfo['activity']; unseen: boolean; label?: string }) {
  const status = activityStatus(activity);
  return (
    <>
      {status && activity !== 'idle' && activity !== 'unknown' && (
        <span title={label} className="inline-flex"><StatusDot status={status} /></span>
      )}
      {unseen && <span role="img" aria-label="unseen" className="inline-block size-1.5 rounded-full bg-primary" />}
    </>
  );
}

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

function RenameTab({ initial, onDone }: { initial: string; onDone: (title: string | null) => void }) {
  const [value, setValue] = useState(initial);
  const done = useRef(false);
  const finish = (save: boolean) => {
    if (done.current) return;
    done.current = true;
    const t = value.trim();
    onDone(save && t && t !== initial ? t : null);
  };
  return (
    <input
      autoFocus
      aria-label="Tab name"
      value={value}
      maxLength={100}
      onChange={(e) => setValue(e.target.value)}
      onFocus={(e) => e.currentTarget.select()}
      onBlur={() => finish(true)}
      onKeyDown={(e) => {
        if (e.key === 'Enter') finish(true);
        else if (e.key === 'Escape') finish(false);
      }}
      className="w-32 px-1 rounded-pdt text-xs bg-surface text-fg border border-border outline-none"
    />
  );
}

function TabButton({ ws, tab, info, selected, renaming, setRenaming, onDrop, menu }: {
  ws: WorkspacesApi;
  tab: Tab;
  info: ItemInfo | null;
  selected: boolean;
  renaming: boolean;
  setRenaming: (id: string | null) => void;
  onDrop: (e: DragEvent, index: number) => void;
  menu: ReturnType<typeof useContextMenu>['bind'];
}) {
  const qc = useQueryClient();
  const title = info?.title ?? (tab.sessionId ? 'Session' : 'Run');
  const rename = useMutation({
    mutationFn: (t: string) => api.renameSession(tab.sessionId!, t),
    onSettled: () => info && qc.invalidateQueries({ queryKey: ['sessions', info.projectId] }),
  });
  return (
    <div
      className={`relative flex shrink-0 items-center gap-1 rounded-pdt pl-2 pr-0.5 py-0.5 ${selected ? 'bg-accent text-fg' : 'text-muted-foreground hover:bg-surface-2'}`}
      onDragOver={(e) => { if (accepts(e)) e.preventDefault(); }}
      onDrop={(e) => onDrop(e, tab.position)}
      {...menu(`Tab ${title}`, () => tabMenu(ws, tab, () => setRenaming(tab.id)))}
    >
      {renaming && tab.sessionId ? (
        <RenameTab initial={title} onDone={(t) => { setRenaming(null); if (t) rename.mutate(t); }} />
      ) : (
        <button
          type="button"
          role="tab"
          aria-selected={selected}
          draggable
          onDragStart={(e) => dragTab(e, tab.id)}
          onClick={() => ws.selectTab(tab)}
          onDoubleClick={() => tab.sessionId && setRenaming(tab.id)}
          title="Right-click for more"
          className="flex items-center gap-1.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-pdt"
        >
          {info && <StatusDot status={info.status} />}
          <span className={`max-w-48 truncate ${info?.unseen ? 'font-semibold' : ''}`}>{title}</span>
          {info && <AttentionDots activity={info.activity} unseen={info.unseen} label={info.activity ?? undefined} />}
        </button>
      )}
      <button type="button" aria-label={`Close ${title}`} className={small} onClick={() => void ws.closeTab(tab)}>
        <X size={12} aria-hidden />
      </button>
    </div>
  );
}

/** The current workspace's name and tab strip. */
export function WorkspaceBar({ ws, renamingTabId, setRenamingTabId, onNewTab, trailing }: {
  ws: WorkspacesApi;
  renamingTabId: string | null;
  setRenamingTabId: (id: string | null) => void;
  onNewTab: () => void;
  trailing?: ReactNode;
}) {
  const w = ws.workspace;
  const { bind, menu } = useContextMenu();
  const touch = useRef<{ x: number; y: number } | null>(null);
  const step = (by: number) => {
    if (!w || !ws.tab) return;
    const next = w.tabs[(ws.tab.position + by + w.tabs.length) % w.tabs.length];
    if (next) ws.selectTab(next);
  };
  const drop = (e: DragEvent, index?: number) => {
    e.preventDefault();
    e.stopPropagation();
    if (w) void applyDrop(ws, dropped(e), w.id, index);
  };

  return (
    <div
      className="flex items-center gap-1 border-b border-border px-2 py-1 min-w-0"
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
      <span className="shrink-0 max-w-32 truncate px-1 text-sm font-medium" title="Workspace">{w?.name ?? 'No workspace'}</span>
      <button type="button" aria-label="Previous tab" className={`${small} md:hidden`} onClick={() => step(-1)}><ChevronLeft size={14} aria-hidden /></button>
      <div
        role="tablist"
        aria-label="Tabs"
        className="flex flex-1 min-w-0 items-center gap-1 overflow-x-auto"
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
            menu={bind}
          />
        ))}
        <button type="button" aria-label="New tab" title="New tab (Ctrl+] c)" className={small} disabled={!w} onClick={onNewTab}>
          <Plus size={14} aria-hidden />
        </button>
      </div>
      <button type="button" aria-label="Next tab" className={`${small} md:hidden`} onClick={() => step(1)}><ChevronRight size={14} aria-hidden /></button>
      {trailing}
      {menu}
    </div>
  );
}
