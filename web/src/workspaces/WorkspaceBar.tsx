import { useRef, useState, type DragEvent, type ReactNode } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronLeft, ChevronRight, MoreHorizontal, Plus, X } from 'lucide-react';
import { StatusDot } from '@ui';
import { api } from '../api/client';
import type { Tab } from '../api/gen/Tab';
import type { Workspace } from '../api/gen/Workspace';
import { accepts, dragTab, dropped, type Dropped } from './dnd';
import { activityStatus, itemOf, worstActivity } from './model';
import type { ItemInfo, WorkspacesApi } from './useWorkspaces';

const small = 'rounded-pdt p-0.5 text-muted hover:text-fg hover:bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-ring';

/** Most urgent activity and whether anything is unseen, over some items. */
export function summarize(infos: (ItemInfo | null)[]) {
  const present = infos.filter((i): i is ItemInfo => i !== null);
  return { activity: worstActivity(present.map((i) => i.activity)), unseen: present.some((i) => i.unseen) };
}

export function workspaceSummary(ws: WorkspacesApi, w: Workspace) {
  return summarize(w.tabs.map((t) => ws.info(itemOf(t))));
}

/** A coloured dot for the most urgent activity, plus the unseen dot. */
export function AttentionDots({ activity, unseen, label }: { activity: ItemInfo['activity']; unseen: boolean; label?: string }) {
  const status = activityStatus(activity);
  return (
    <>
      {status && activity !== 'idle' && activity !== 'unknown' && (
        <span title={label} className="inline-flex"><StatusDot status={status} /></span>
      )}
      {unseen && <span role="img" aria-label="unseen" className="inline-block size-1.5 rounded-full bg-accent" />}
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

function Popover({ children, onClose, className = '' }: { children: ReactNode; onClose: () => void; className?: string }) {
  return (
    <>
      <div className="fixed inset-0 z-10" onClick={onClose} aria-hidden />
      <div
        onKeyDown={(e) => { if (e.key === 'Escape') onClose(); }}
        className={`absolute z-20 mt-1 rounded-pdt border border-border bg-surface shadow-lg p-1 flex flex-col gap-0.5 text-sm ${className}`}
      >
        {children}
      </div>
    </>
  );
}

function TabMenu({ ws, tab, title, onClose }: { ws: WorkspacesApi; tab: Tab; title: string; onClose: () => void }) {
  const w = ws.workspaces.find((x) => x.id === tab.workspaceId);
  const others = ws.workspaces.filter((x) => x.id !== tab.workspaceId);
  const [newName, setNewName] = useState('');
  const run = (f: () => Promise<unknown>) => { onClose(); void f(); };
  const item = 'text-left px-2 py-1 rounded-pdt hover:bg-surface-2';
  return (
    <Popover onClose={onClose} className="right-0 w-56">
      <div role="menu" aria-label={`Tab menu for ${title}`} className="flex flex-col gap-0.5">
        <button type="button" role="menuitem" className={item} disabled={tab.position === 0}
          onClick={() => run(() => ws.moveTab(tab, tab.workspaceId, tab.position - 1))}>Move left</button>
        <button type="button" role="menuitem" className={item} disabled={!w || tab.position >= w.tabs.length - 1}
          onClick={() => run(() => ws.moveTab(tab, tab.workspaceId, tab.position + 1))}>Move right</button>
        <p className="px-2 pt-1 text-[10px] uppercase tracking-wider text-muted">Move to workspace</p>
        {others.map((o) => (
          <button key={o.id} type="button" role="menuitem" className={item} onClick={() => run(() => ws.moveTab(tab, o.id))}>{o.name}</button>
        ))}
        <form
          className="flex gap-1 px-1"
          onSubmit={(e) => {
            e.preventDefault();
            const name = newName.trim();
            if (!name) return;
            run(async () => {
              const created = await api.createWorkspace({ name });
              await ws.moveTab(tab, created.id);
              ws.selectWorkspace(created.id);
            });
          }}
        >
          <input aria-label="New workspace name" placeholder="New workspace…" value={newName} onChange={(e) => setNewName(e.target.value)}
            className="min-w-0 flex-1 px-1 rounded-pdt text-xs bg-bg border border-border outline-none" />
        </form>
      </div>
    </Popover>
  );
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
  const [menu, setMenu] = useState(false);
  const title = info?.title ?? (tab.sessionId ? 'Session' : 'Run');
  const rename = useMutation({
    mutationFn: (t: string) => api.renameSession(tab.sessionId!, t),
    onSettled: () => info && qc.invalidateQueries({ queryKey: ['sessions', info.projectId] }),
  });
  return (
    <div
      className={`relative flex shrink-0 items-center gap-1 rounded-pdt pl-2 pr-0.5 py-0.5 ${selected ? 'bg-accent-soft text-fg' : 'text-muted hover:bg-surface-2'}`}
      onDragOver={(e) => { if (accepts(e)) e.preventDefault(); }}
      onDrop={(e) => onDrop(e, tab.position)}
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
          title={tab.sessionId ? 'Double-click to rename' : undefined}
          className="flex items-center gap-1.5 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-pdt"
        >
          {info && <StatusDot status={info.status} />}
          <span className={`max-w-48 truncate ${info?.unseen ? 'font-semibold' : ''}`}>{title}</span>
          {info && <AttentionDots activity={info.activity} unseen={info.unseen} label={info.activity ?? undefined} />}
        </button>
      )}
      <button type="button" aria-label={`Tab menu for ${title}`} aria-haspopup="menu" className={small} onClick={() => setMenu((m) => !m)}>
        <MoreHorizontal size={12} aria-hidden />
      </button>
      <button type="button" aria-label={`Close ${title}`} className={small} onClick={() => void ws.closeTab(tab)}>
        <X size={12} aria-hidden />
      </button>
      {menu && <TabMenu ws={ws} tab={tab} title={title} onClose={() => setMenu(false)} />}
    </div>
  );
}

export function WorkspaceSwitcher({ ws, open, setOpen }: { ws: WorkspacesApi; open: boolean; setOpen: (o: boolean) => void }) {
  const [filter, setFilter] = useState('');
  const [newName, setNewName] = useState('');
  const [renaming, setRenaming] = useState(false);
  const current = ws.workspace;
  const qc = useQueryClient();
  const refresh = () => qc.invalidateQueries({ queryKey: ['workspaces'] });
  const create = useMutation({
    mutationFn: (name: string) => api.createWorkspace({ name }),
    onSuccess: (w) => { void refresh(); ws.selectWorkspace(w.id); setNewName(''); setOpen(false); },
  });
  const rename = useMutation({ mutationFn: (name: string) => api.updateWorkspace(current!.id, { name }), onSettled: refresh });
  const remove = useMutation({ mutationFn: () => api.deleteWorkspace(current!.id), onSuccess: () => { void refresh(); setOpen(false); } });
  const move = useMutation({ mutationFn: ({ id, position }: { id: string; position: number }) => api.updateWorkspace(id, { position }), onSettled: refresh });
  const shown = ws.workspaces.filter((w) => w.name.toLowerCase().includes(filter.trim().toLowerCase()));
  const close = () => { setOpen(false); setFilter(''); setRenaming(false); };
  const go = (w: Workspace) => { ws.selectWorkspace(w.id); close(); };
  const summary = current ? workspaceSummary(ws, current) : { activity: null, unseen: false };
  const error = create.error ?? rename.error ?? remove.error ?? move.error;
  const row = 'flex-1 flex items-center gap-2 text-left px-2 py-1 rounded-pdt';

  return (
    <div className="relative">
      <button
        type="button"
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-label={`Workspace: ${current?.name ?? 'none'}`}
        onClick={() => (open ? close() : setOpen(true))}
        onDragOver={(e) => { if (accepts(e)) { e.preventDefault(); setOpen(true); } }}
        className="flex items-center gap-1.5 rounded-pdt px-2 py-1 text-sm font-medium hover:bg-surface-2 outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="max-w-40 truncate">{current?.name ?? 'No workspace'}</span>
        <AttentionDots activity={summary.activity} unseen={summary.unseen} />
        <ChevronDown size={14} aria-hidden />
      </button>
      {open && (
        <Popover onClose={close} className="left-0 w-[calc(100vw-2rem)] md:w-72">
          <div role="dialog" aria-label="Workspaces" className="flex flex-col gap-0.5">
            <input
              autoFocus
              aria-label="Find workspace"
              placeholder="Find workspace…"
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Enter' && shown[0]) go(shown[0]); }}
              className="px-2 py-1 rounded-pdt text-sm bg-bg border border-border outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
            <ul aria-label="Workspace list" className="flex flex-col gap-0.5 max-h-80 overflow-auto">
              {shown.map((w) => {
                const s = workspaceSummary(ws, w);
                return (
                  <li
                    key={w.id}
                    draggable
                    onDragStart={(e) => e.dataTransfer.setData('application/x-pdt-workspace', w.id)}
                    onDragOver={(e) => { e.preventDefault(); }}
                    onDrop={(e) => {
                      e.preventDefault();
                      const moving = e.dataTransfer.getData('application/x-pdt-workspace');
                      if (moving && moving !== w.id) move.mutate({ id: moving, position: w.position });
                      else void applyDrop(ws, dropped(e), w.id);
                    }}
                    className="flex items-center"
                  >
                    <button type="button" aria-current={w.id === current?.id ? 'true' : undefined} onClick={() => go(w)}
                      className={`${row} ${w.id === current?.id ? 'bg-accent-soft' : 'hover:bg-surface-2'}`}>
                      <span className="flex-1 truncate">{w.name}</span>
                      <span className="text-[11px] text-muted">{w.tabs.length}</span>
                      <AttentionDots activity={s.activity} unseen={s.unseen} />
                    </button>
                  </li>
                );
              })}
            </ul>
            {current && current.projectId === null && (
              renaming ? (
                <form className="flex gap-1 px-1" onSubmit={(e) => {
                  e.preventDefault();
                  const name = new FormData(e.currentTarget).get('name')?.toString().trim();
                  if (name) rename.mutate(name);
                  setRenaming(false);
                }}>
                  <input autoFocus name="name" aria-label="Workspace name" defaultValue={current.name}
                    className="min-w-0 flex-1 px-1 rounded-pdt text-xs bg-bg border border-border outline-none" />
                </form>
              ) : (
                <div className="flex gap-1 px-1 text-xs">
                  <button type="button" className="px-2 py-0.5 rounded-pdt text-muted hover:bg-surface-2" onClick={() => setRenaming(true)}>Rename “{current.name}”</button>
                  <button type="button" className="px-2 py-0.5 rounded-pdt text-failed hover:bg-surface-2" onClick={() => remove.mutate()}>Delete</button>
                </div>
              )
            )}
            <form className="flex gap-1 px-1 pt-1 border-t border-border" onSubmit={(e) => {
              e.preventDefault();
              if (newName.trim()) create.mutate(newName.trim());
            }}>
              <input aria-label="New workspace name" placeholder="New workspace…" value={newName} onChange={(e) => setNewName(e.target.value)}
                className="min-w-0 flex-1 px-2 py-1 rounded-pdt text-xs bg-bg border border-border outline-none" />
              <button type="submit" className="px-2 rounded-pdt text-xs text-muted hover:bg-surface-2" disabled={!newName.trim()}>Create</button>
            </form>
            {error && <p role="alert" className="px-2 text-xs text-failed">{error.message}</p>}
          </div>
        </Popover>
      )}
    </div>
  );
}

/** The workspace switcher and the current workspace's tab strip. */
export function WorkspaceBar({ ws, switcherOpen, setSwitcherOpen, renamingTabId, setRenamingTabId, onNewTab, trailing }: {
  ws: WorkspacesApi;
  switcherOpen: boolean;
  setSwitcherOpen: (o: boolean) => void;
  renamingTabId: string | null;
  setRenamingTabId: (id: string | null) => void;
  onNewTab: () => void;
  trailing?: ReactNode;
}) {
  const w = ws.workspace;
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
      <WorkspaceSwitcher ws={ws} open={switcherOpen} setOpen={setSwitcherOpen} />
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
          />
        ))}
        <button type="button" aria-label="New tab" title="New tab (Ctrl+] c)" className={small} disabled={!w} onClick={onNewTab}>
          <Plus size={14} aria-hidden />
        </button>
      </div>
      <button type="button" aria-label="Next tab" className={`${small} md:hidden`} onClick={() => step(1)}><ChevronRight size={14} aria-hidden /></button>
      {trailing}
    </div>
  );
}
