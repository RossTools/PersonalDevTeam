// Documents beside a conversation (PDT-317): an agent's or the PM's conversation with a side panel of document tabs,
// opened by the user (here, from the Documents view or a ticket's documents) or by the agent's own tool. The tabs and
// the one shown are PDT's (every device sees them); the panel's width and whether it is collapsed are this browser's,
// per conversation. On a phone the panel is a pane of its own beside the conversation's.
import { useEffect, useRef, useState, type DragEvent, type KeyboardEvent, type PointerEvent, type ReactNode } from 'react';
import { useQuery, useQueryClient } from '@tanstack/react-query';
import { ChevronLeft, ExternalLink, FileStack, PanelRightClose, Plus, X } from 'lucide-react';
import { ContextMenu, IconButton, SegmentedControl, dragData, parseDragData, tabIndexOf, tabListKeyDown, type MenuEntry } from '@ui';
import { cn } from '@ui/lib/utils';
import { api } from '../api/client';
import type { DocumentsBeside } from '../api/gen/DocumentsBeside';
import { runAction } from '../common/useAction';
import { usePm } from '../pm/usePm';
import { routePath } from '../shell/route';
import { navigate } from '../shell/router';
import { useOpenSession } from '../tickets/actions';
import { DocumentBody } from './DocumentBody';
import { LinkDocumentDialog } from './DocumentDialogs';
import { KindIcon } from './LinkedDocuments';
import { openDocument, providerLabel, useDocument, useDocumentProviders } from './queries';

const POLL = 2000;
export const BESIDE_WIDTH = { default: 440, min: 280, max: 960 };
const STEP = 16;
const DRAG_KIND = 'beside';

export const besideKey = (sessionId: string) => ['documents', 'beside', sessionId] as const;

/** The documents open beside a conversation, polled (an agent may open one at any time). */
export function useDocumentsBeside(sessionId: string) {
  return useQuery({ queryKey: besideKey(sessionId), queryFn: () => api.documentsBeside(sessionId), refetchInterval: POLL });
}

interface PanelPrefs { width: number; collapsed: boolean }

const prefsKey = (sessionId: string) => `pdt.beside.${sessionId}`;
const clamp = (n: number) => Math.min(BESIDE_WIDTH.max, Math.max(BESIDE_WIDTH.min, n));

function readPrefs(sessionId: string): PanelPrefs {
  try {
    const v = JSON.parse(localStorage.getItem(prefsKey(sessionId)) ?? 'null') as Partial<PanelPrefs> | null;
    const width = typeof v?.width === 'number' && Number.isFinite(v.width) ? clamp(v.width) : BESIDE_WIDTH.default;
    return { width, collapsed: v?.collapsed === true };
  } catch {
    return { width: BESIDE_WIDTH.default, collapsed: false };
  }
}

/** The panel's width and collapsed state, kept per conversation in this browser. */
function usePanelPrefs(sessionId: string): [PanelPrefs, (p: Partial<PanelPrefs>) => void] {
  const [prefs, setPrefs] = useState(() => readPrefs(sessionId));
  const set = (p: Partial<PanelPrefs>) => setPrefs((old) => {
    const next = { ...old, ...p, width: clamp(p.width ?? old.width) };
    try {
      localStorage.setItem(prefsKey(sessionId), JSON.stringify({ width: Math.round(next.width), collapsed: next.collapsed }));
    } catch {
      // Private window or blocked storage: the panel keeps its state for this visit only.
    }
    return next;
  });
  return [prefs, set];
}

/** The tabs' actions, applied at once on screen and then by PDT. */
function useTabs(sessionId: string, data: DocumentsBeside | undefined) {
  const qc = useQueryClient();
  const key = besideKey(sessionId);
  const settle = (b: DocumentsBeside) => qc.setQueryData(key, b);
  const local = (f: (b: DocumentsBeside) => DocumentsBeside) => {
    const now = qc.getQueryData<DocumentsBeside>(key) ?? data;
    if (now) qc.setQueryData(key, f(now));
  };
  return {
    show: (id: string) => void runAction(async () => {
      local((b) => ({ ...b, active: id }));
      settle(await api.openBeside(sessionId, id));
    }),
    close: (id: string) => void runAction(async () => {
      local((b) => {
        const tabs = b.tabs.filter((t) => t.document.id !== id);
        const next = tabs.reduce<(typeof tabs)[number] | null>((m, t) => (!m || t.focusedAt > m.focusedAt ? t : m), null);
        return { tabs, active: b.active === id ? next?.document.id ?? null : b.active };
      });
      settle(await api.closeBeside(sessionId, id));
    }),
    /** Moves a tab to `index` among the tabs. */
    move: (id: string, index: number) => void runAction(async () => {
      const now = qc.getQueryData<DocumentsBeside>(key) ?? data;
      if (!now) return;
      const ids = now.tabs.map((t) => t.document.id).filter((d) => d !== id);
      ids.splice(Math.max(0, Math.min(index, ids.length)), 0, id);
      local((b) => ({ ...b, tabs: ids.map((d) => b.tabs.find((t) => t.document.id === d)!).filter(Boolean) }));
      settle(await api.orderBeside(sessionId, ids));
    }),
  };
}

const coarsePointer = () => typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;

/** The tab strip: click shows a tab, drag (or its menu) reorders, × closes. */
function TabStrip({ data, tabs: act, trailing }: {
  data: DocumentsBeside;
  tabs: ReturnType<typeof useTabs>;
  trailing: ReactNode;
}) {
  const ids = data.tabs.map((t) => t.document.id);
  const anyActive = data.active !== null && ids.includes(data.active);
  const onDrop = (e: DragEvent, index: number) => {
    const d = parseDragData(e.dataTransfer.getData('text/plain'));
    if (d?.kind !== DRAG_KIND) return;
    e.preventDefault();
    act.move(d.id, index);
  };
  return (
    <div className="flex min-w-0 items-center gap-1 border-b border-border px-1">
      <div role="tablist" aria-label="Documents" onKeyDown={tabListKeyDown} className="flex min-w-0 flex-1 items-end gap-0.5 overflow-x-auto pt-1">
        {data.tabs.map((t, i) => {
          const id = t.document.id;
          const active = id === data.active;
          const entries = (): MenuEntry[] => [
            { label: 'Move left', onSelect: () => act.move(id, i - 1), disabled: i === 0 },
            { label: 'Move right', onSelect: () => act.move(id, i + 1), disabled: i === ids.length - 1 },
            { label: 'Open in Documents', onSelect: () => openDocument(id) },
            { separator: true },
            { label: 'Close', onSelect: () => act.close(id) },
          ];
          return (
            <div
              key={id}
              className={cn('group flex max-w-56 min-w-0 shrink-0 items-center rounded-t-md border border-b-0 border-transparent', active && 'border-border bg-background')}
              onDragOver={(e) => { if (e.dataTransfer.types.includes('text/plain')) e.preventDefault(); }}
              onDrop={(e) => onDrop(e, i)}
            >
              <ContextMenu label={`${t.document.title} actions`} entries={entries}>
                <button
                  type="button"
                  role="tab"
                  id={`beside-tab-${id}`}
                  aria-selected={active}
                  tabIndex={tabIndexOf(active, i, anyActive)}
                  title={t.document.title}
                  draggable={!coarsePointer()}
                  onDragStart={(e) => e.dataTransfer.setData('text/plain', dragData(DRAG_KIND, id))}
                  onClick={() => act.show(id)}
                  className={cn('flex min-w-0 items-center gap-1.5 py-1 pr-1 pl-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-ring', active ? 'text-foreground' : 'text-muted-foreground hover:text-foreground')}
                >
                  <KindIcon kind={t.document.kind} />
                  <span className="min-w-0 truncate">{t.document.title}</span>
                </button>
              </ContextMenu>
              <IconButton label={`Close ${t.document.title}`} size="sm" variant="ghost" className="size-5 shrink-0" onClick={() => act.close(id)}>
                <X aria-hidden />
              </IconButton>
            </div>
          );
        })}
      </div>
      {trailing}
    </div>
  );
}

/** Picks a document PDT has to open beside a conversation. */
export function useOpenBesidePicker(sessionId: string): { entry: MenuEntry; pick: () => void; dialog: ReactNode } {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const beside = useDocumentsBeside(sessionId);
  const dialog = open ? (
    <LinkDocumentDialog
      title="Open a document beside the conversation"
      exclude={beside.data?.tabs.map((t) => t.document.id) ?? []}
      onPick={async (d) => {
        qc.setQueryData(besideKey(sessionId), await api.openBeside(sessionId, d.id));
        setOpen(false);
      }}
      onClose={() => setOpen(false)}
    />
  ) : null;
  const pick = () => setOpen(true);
  return { entry: { label: 'Open a document beside…', onSelect: pick, icon: <FileStack aria-hidden /> }, pick, dialog };
}

/** The panel's body: the shown document, live. */
function Shown({ id }: { id: string | null }) {
  const providers = useDocumentProviders();
  const doc = useDocument(id);
  if (!id) return null;
  const provider = doc.data ? providerLabel(providers.data?.providers, doc.data.provider) : 'Its provider';
  return (
    <div role="tabpanel" aria-labelledby={`beside-tab-${id}`} className="flex min-h-0 flex-1 flex-col overflow-auto p-3">
      <DocumentBody key={id} id={id} providerLabel={provider} className="flex min-h-0 flex-1 flex-col" />
    </div>
  );
}

/**
 * A conversation with the documents open beside it: on a desktop a resizable, collapsible panel of tabs to its right;
 * on a phone a Conversation | Documents switch. With no document open it is the conversation alone.
 */
export function DocumentsBesideLayout({ sessionId, phone, children }: { sessionId: string; phone: boolean; children: ReactNode }) {
  const beside = useDocumentsBeside(sessionId);
  const data = beside.data;
  const act = useTabs(sessionId, data);
  const [prefs, setPrefs] = usePanelPrefs(sessionId);
  const [pane, setPane] = useState<'conversation' | 'documents'>('conversation');
  const picker = useOpenBesidePicker(sessionId);
  // A tab opened or shown by someone else (the agent, another device) brings a collapsed panel back.
  const lastActive = useRef<string | null | undefined>(undefined);
  const active = data?.active ?? null;
  useEffect(() => {
    if (data === undefined) return;
    if (lastActive.current !== undefined && active && active !== lastActive.current && prefs.collapsed) setPrefs({ collapsed: false });
    lastActive.current = active;
  }, [data, active, prefs.collapsed, setPrefs]);

  const drag = useRef<{ x: number; width: number } | null>(null);
  const onKeyDown = (e: KeyboardEvent) => {
    const by = { ArrowLeft: STEP, ArrowRight: -STEP }[e.key];
    if (by) setPrefs({ width: prefs.width + by });
    else if (e.key === 'Home') setPrefs({ width: BESIDE_WIDTH.max });
    else if (e.key === 'End') setPrefs({ width: BESIDE_WIDTH.min });
    else return;
    e.preventDefault();
  };
  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    drag.current = { x: e.clientX, width: prefs.width };
    e.currentTarget.setPointerCapture?.(e.pointerId);
  };
  const onPointerMove = (e: PointerEvent) => {
    if (drag.current) setPrefs({ width: drag.current.width - (e.clientX - drag.current.x) });
  };
  const onPointerUp = () => {
    drag.current = null;
  };

  const tabs = data?.tabs ?? [];
  if (tabs.length === 0) return <>{children}{picker.dialog}</>;
  const count = tabs.length;
  const actions = (
    <span className="flex shrink-0 items-center">
      <IconButton label="Open a document beside" size="sm" variant="ghost" onClick={picker.pick}><Plus aria-hidden /></IconButton>
      {active && <IconButton label="Open in Documents" size="sm" variant="ghost" onClick={() => openDocument(active)}><ExternalLink aria-hidden /></IconButton>}
      {!phone && (
        <IconButton label="Collapse the documents" size="sm" variant="ghost" aria-expanded onClick={() => setPrefs({ collapsed: true })}>
          <PanelRightClose aria-hidden />
        </IconButton>
      )}
    </span>
  );

  if (phone) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex justify-center border-b border-border px-2 py-1">
          <SegmentedControl
            aria-label="Show"
            size="sm"
            value={pane}
            onChange={setPane}
            options={[
              { value: 'conversation', label: 'Conversation' },
              { value: 'documents', label: `Documents (${count})` },
            ]}
          />
        </div>
        {pane === 'conversation' ? children : (
          <section aria-label="Documents beside the conversation" className="flex min-h-0 flex-1 flex-col">
            {data && <TabStrip data={data} tabs={act} trailing={actions} />}
            <Shown id={active} />
          </section>
        )}
        {picker.dialog}
      </div>
    );
  }

  return (
    <div className="flex min-h-0 min-w-0 flex-1">
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">{children}</div>
      {prefs.collapsed ? (
        <aside aria-label="Documents beside the conversation" className="flex w-10 shrink-0 flex-col border-l border-border bg-card">
          <button
            type="button"
            aria-label={`Show the documents (${count})`}
            aria-expanded={false}
            onClick={() => setPrefs({ collapsed: false })}
            className="flex flex-1 flex-col items-center gap-1 pt-2 text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
          >
            <FileStack aria-hidden className="size-4" />
            <span aria-hidden className="text-2xs font-medium tabular-nums">{count}</span>
            <ChevronLeft aria-hidden className="size-3" />
          </button>
        </aside>
      ) : (
        <aside aria-label="Documents beside the conversation" className="flex min-h-0 shrink-0 border-l border-border bg-card" style={{ width: prefs.width }}>
          <div
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize the documents"
            aria-valuenow={Math.round(prefs.width)}
            aria-valuemin={BESIDE_WIDTH.min}
            aria-valuemax={BESIDE_WIDTH.max}
            tabIndex={0}
            onKeyDown={onKeyDown}
            onPointerDown={onPointerDown}
            onPointerMove={onPointerMove}
            onPointerUp={onPointerUp}
            onPointerCancel={onPointerUp}
            className="w-1 shrink-0 cursor-col-resize touch-none outline-none hover:bg-accent focus-visible:bg-primary"
          />
          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            {data && <TabStrip data={data} tabs={act} trailing={actions} />}
            <Shown id={active} />
          </div>
        </aside>
      )}
      {picker.dialog}
    </div>
  );
}

/** A conversation a document can open beside: a PM conversation, or an agent's on one of its tickets. */
export interface BesideTarget {
  sessionId: string;
  projectId: string;
  label: string;
  pm: boolean;
}

/**
 * Where a document can open beside a conversation: the PM's open conversations and the conversations of `ticketIds`
 * (the tickets it is linked to, or the one on screen), with what opens it there and shows that conversation.
 */
export function useBesideTargets(ticketIds: string[]) {
  const qc = useQueryClient();
  const openSession = useOpenSession();
  const pm = usePm();
  const tickets = useQuery({
    queryKey: ['documents', 'beside-targets', ticketIds],
    queryFn: () => Promise.all(ticketIds.map((id) => api.getTicket(id))),
    enabled: ticketIds.length > 0,
    staleTime: 10_000,
  });
  const targets: BesideTarget[] = [
    ...(tickets.data ?? []).flatMap((t) => t.sessions
      .filter((s) => s.mode === 'conversation' && s.status !== 'finished')
      .map((s) => ({ sessionId: s.id, projectId: t.projectId, label: `${t.key} · ${s.persona ?? s.title}`, pm: false }))),
    ...(pm.data?.sessions ?? []).filter((s) => s.open).map((s) => ({ sessionId: s.session.id, projectId: s.session.projectId, label: `PM · ${s.title}`, pm: true })),
  ];
  const open = (t: BesideTarget, documentId: string) => void runAction(async () => {
    qc.setQueryData(besideKey(t.sessionId), await api.openBeside(t.sessionId, documentId));
    if (t.pm) navigate(routePath({ view: 'pm', pm: t.sessionId }));
    else openSession({ id: t.sessionId, projectId: t.projectId });
  });
  return { targets, open };
}

/** The conversations to open a document beside, as menu entries. */
export function besideEntries(b: ReturnType<typeof useBesideTargets>, documentId: string): MenuEntry[] {
  return b.targets.length > 0
    ? b.targets.map((t) => ({ label: t.label, onSelect: () => b.open(t, documentId) }))
    : [{ label: 'No conversation to open it beside', onSelect: () => {}, disabled: true }];
}

/** "Open beside" with the conversations to pick from. */
export function openBesideEntry(b: ReturnType<typeof useBesideTargets>, documentId: string): MenuEntry {
  return { submenu: 'Open beside', entries: besideEntries(b, documentId) };
}
