// The Tickets view's Board: committed work in three columns (To do, In progress, Done in the project's window). Ready
// tickets wait in the backlog until they are moved to To do. Rows are lanes (PDT-38): tickets that are not parts
// first, then one lane per parent with its parts, under a header that tracks the parent. With a pipeline workflow at
// work (Phase 7c), In progress splits into its stages: Code, Review, QA, Accept and Merge.
import { memo, useEffect, useMemo, useRef, useState, type DragEvent, type ReactNode, type Ref, type TouchEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { ChevronDown, ChevronRight, GitBranch } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import {
  Button, ContextMenu, dragData, IconButton, MenuButton, parseDragData, Skeleton, useConfirm, type MenuEntry,
} from '@ui';
import type { Session } from '../api/gen/Session';
import type { TicketQuery } from '../api/gen/TicketQuery';
import type { TicketStage } from '../api/gen/TicketStage';
import type { TicketSummary } from '../api/gen/TicketSummary';
import type { PipelineStage } from '../api/gen/PipelineStage';
import type { TicketAgent } from '../api/gen/TicketAgent';
import { AgentDot, itemState, rollup, type AgentStateValue } from '../common/AgentState';
import { PartsStepper, partsCount } from '../common/PartsStepper';
import { STAGE_LABEL } from '../common/StageStepper';
import { PriorityIcon } from '../common/PriorityIcon';
import { TicketKey } from '../common/TicketKey';
import { agentLine } from '../common/ticketAgent';
import { runAction } from '../common/useAction';
import { canStart, moveTicket, startNextPart, ticketMenu, useOpenTicket, useStartTicket } from './actions';
import { buildLanes, laneIds, readCollapsed, writeCollapsed, type Lane } from './BoardLanes';
import { useAllSessions, useTicketPages } from './queries';
import { useClearToStartToasts } from './clearToStart';
import { CiBadge, PrBadge } from './PullRequest';
import { ExternalKeys } from './ExternalKeys';
import { TicketBadges } from './TicketBadges';
import { LabelChips } from './TicketParts';
import { DAY_MS, DEFAULT_DONE_DAYS, doneWindowLabel, useDoneWindow } from './BoardViewMenu';

export { agentLine };

export type BoardStage = Extract<TicketStage, 'todo' | 'in_progress' | 'done'>;

export const BOARD_COLUMNS: { stage: BoardStage; title: string }[] = [
  { stage: 'todo', title: 'To do' },
  { stage: 'in_progress', title: 'In progress' },
  { stage: 'done', title: 'Done' },
];

/** A column on screen: a board stage, or (In progress split by the pipeline) one of its stages. */
export interface DisplayColumn {
  id: string;
  stage: BoardStage;
  title: string;
  pipeline?: Exclude<PipelineStage, 'refine' | 'merge_blocked'>;
}

export const PIPELINE_COLUMNS: DisplayColumn[] = [
  { id: 'code', stage: 'in_progress', title: 'Code', pipeline: 'code' },
  { id: 'review', stage: 'in_progress', title: 'Review', pipeline: 'review' },
  { id: 'qa', stage: 'in_progress', title: 'QA', pipeline: 'qa' },
  { id: 'accept', stage: 'in_progress', title: 'Accept', pipeline: 'accept' },
  { id: 'merge', stage: 'in_progress', title: 'Merge', pipeline: 'merge' },
];

/** A pipeline stage's name (rows and cards). */
export const PIPELINE_LABEL: Record<PipelineStage, string> = { refine: 'Refine', code: 'Code', review: 'Review', qa: 'QA', accept: 'Accept',
  merge: 'Merge', merge_blocked: 'Merge blocked' };

/** In progress splits into the pipeline's stages once any card in it has one. */
export function boardSplits(inProgress: Pick<TicketSummary, 'pipelineStage'>[]): boolean {
  return inProgress.some((t) => !!t.pipelineStage && t.pipelineStage !== 'refine');
}

/** The columns on screen. */
export function displayColumns(split: boolean): DisplayColumn[] {
  const plain = BOARD_COLUMNS.map((c) => ({ id: c.stage, stage: c.stage, title: c.title }));
  return split ? [plain[0], ...PIPELINE_COLUMNS, plain[2]] : plain;
}

/**
 * Whether a card of the column's stage shows in it: a card with no pipeline stage shows under Code, a blocked merge
 * under Merge (its badge says so).
 */
export function inColumn(t: Pick<TicketSummary, 'pipelineStage'>, c: DisplayColumn): boolean {
  if (!c.pipeline) return true;
  const at = t.pipelineStage === 'merge_blocked' ? 'merge'
    : t.pipelineStage && t.pipelineStage !== 'refine' ? t.pipelineStage : 'code';
  return at === c.pipeline;
}

/** The Done column shows tickets closed in the last two weeks unless the project's board says otherwise. */
export const DONE_WINDOW_MS = DEFAULT_DONE_DAYS * DAY_MS;
const BOARD_PAGE = 50;

/**
 * A column's `GET /api/tickets` query: its cards, never parents (they head lanes). Done: closed since `closedAfter`,
 * most recently updated first.
 */
export function columnQuery(projectId: string | null, stage: BoardStage, closedAfter: number): TicketQuery {
  const q: TicketQuery = { stage, hasParts: false, limit: BOARD_PAGE };
  if (projectId) q.project = projectId;
  if (stage === 'done') {
    q.closedAfter = closedAfter;
    q.sort = 'updated';
  }
  return q;
}

/** The lane headers' queries: open parents (To do, In progress) and parents closed since `closedAfter`. */
export function laneQueries(projectId: string | null, closedAfter: number): TicketQuery[] {
  const scope = projectId ? { project: projectId } : {};
  return [
    { ...scope, hasParts: true, stage: 'todo,in_progress', limit: 100 },
    { ...scope, hasParts: true, stage: 'done,wont_do', closedAfter, limit: 100 },
  ];
}

/** The drag payload of a card (web/ui README: text/plain `pdt:<kind>:<id>`). */
export const CARD_DRAG_KIND = 'ticket';

const coarsePointer = () => typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;

/**
 * The most urgent state of the sessions working on a ticket's active attempt (null when none): its own, and the ones
 * workflow steps started on the ticket (Reviewer, QA), which have no attempt. `sessions` may be every session, or
 * just the ticket's (`sessionsByTicket`).
 */
export function cardActivity(t: Pick<TicketSummary, 'id' | 'attempt'>, sessions: Session[]): AgentStateValue | null {
  if (!t.attempt) return null;
  const mine = sessions.filter((s) => s.ticket?.id === t.id && (s.attemptId === null || s.attemptId === t.attempt!.id)
    && s.kind === 'agent_tui');
  return rollup(mine.map((s) => itemState({ session: s, unseen: s.unseen })));
}

/** The sessions on a ticket, by ticket id: one pass over every session instead of one per card. */
export function sessionsByTicket(sessions: Session[]): Map<string, Session[]> {
  const m = new Map<string, Session[]>();
  for (const s of sessions) {
    if (!s.ticket) continue;
    const list = m.get(s.ticket.id);
    if (list) list.push(s);
    else m.set(s.ticket.id, [s]);
  }
  return m;
}

const sameActivity = (a: AgentStateValue | null | undefined, b: AgentStateValue | null | undefined) =>
  a === b || (!!a && !!b && a.dot === b.dot && a.pulse === b.pulse && a.label === b.label && a.tone === b.tone && a.unseen === b.unseen);

export interface BoardCardProps {
  ticket: TicketSummary;
  /** Its sessions' rolled-up state (the dot next to the branch). */
  activity?: AgentStateValue | null;
  onOpen: () => void;
  /** The card menu (right-click, long press, ⋯). */
  entries: () => MenuEntry[];
  /** Desktop: the card can be dragged to another column. */
  draggable?: boolean;
}

/** A board card: key, title (two lines), priority, labels, blocked/overlap/waiting, and the attempt's branch with its activity dot. */
export function BoardCard({ ticket: t, activity, onOpen, entries, draggable = false }: BoardCardProps) {
  const [dragging, setDragging] = useState(false);
  const onDragStart = (e: DragEvent) => {
    e.dataTransfer.setData('text/plain', dragData(CARD_DRAG_KIND, t.id));
    e.dataTransfer.effectAllowed = 'move';
    setDragging(true);
  };
  return (
    <li
      aria-label={`${t.key} ${t.title}`}
      data-peek
      draggable={draggable}
      onDragStart={draggable ? onDragStart : undefined}
      onDragEnd={() => setDragging(false)}
      className={cn(
        'group/card relative flex flex-col rounded-md border border-border bg-card shadow-card',
        'transition-colors duration-(--pdt-duration) hover:border-input motion-reduce:transition-none',
        draggable && 'cursor-grab active:cursor-grabbing',
        dragging && 'opacity-50',
      )}
    >
      <ContextMenu label={`Ticket ${t.key}`} entries={entries}>
        <button
          type="button"
          onClick={onOpen}
          className="flex min-w-0 flex-col gap-1.5 rounded-md p-2 pr-8 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
        >
          <span className="flex min-w-0 flex-wrap items-center gap-1.5">
            <TicketKey ticketKey={t.key} />
            <ExternalKeys links={t.external} />
            <PriorityIcon priority={t.priority} />
            <TicketBadges ticket={t} className="flex-wrap" />
          </span>
          <span className={cn('line-clamp-2 text-sm', activity?.unseen && 'font-semibold')}>{t.title}</span>
          {t.labels.length > 0 && <LabelChips labels={t.labels} className="flex-wrap" />}
          {t.agent && (() => {
            const line = agentLine(t.agent);
            return (
              <span className="flex min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
                <AgentDot state={line.dot} />
                <span className="min-w-0 truncate">{line.text}</span>
              </span>
            );
          })()}
          {t.attempt && (
            <span className="flex min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
              {activity ? <AgentDot state={activity} /> : <GitBranch aria-hidden className="size-3 shrink-0" />}
              <span className="min-w-0 truncate font-mono" title={t.attempt.branch}>{t.attempt.branch}</span>
              {t.attempt.pr && <PrBadge pr={t.attempt.pr} attemptMerged={t.attempt.state === 'merged'} mergeUpdate={t.attempt.mergeUpdate ?? null} />}
              {!t.attempt.pr && t.attempt.ci && <CiBadge ci={t.attempt.ci} />}
            </span>
          )}
        </button>
      </ContextMenu>
      <MenuButton
        label={`Actions for ${t.key}`}
        size="sm"
        entries={entries}
        className="absolute top-1.5 right-1.5 opacity-0 group-hover/card:opacity-100 focus-visible:opacity-100 data-pressed:opacity-100 pointer-coarse:opacity-100"
      />
    </li>
  );
}

type Columns = Record<BoardStage, TicketSummary[]>;

/** One column's paged query, its loaded cards and its total. */
function useColumn(projectId: string | null, stage: BoardStage, closedAfter: number) {
  const query = useMemo(() => columnQuery(projectId, stage, closedAfter), [projectId, stage, closedAfter]);
  const pages = useTicketPages(query);
  const items = useMemo(() => pages.data?.pages.flatMap((p) => p.items) ?? [], [pages.data]);
  return { pages, items, total: pages.data?.pages[0]?.total };
}

type ColumnData = ReturnType<typeof useColumn>;

/** The parents heading lanes: the first page of each lane-header query. */
function useLaneParents(projectId: string | null, closedAfter: number): TicketSummary[] {
  const [open, closed] = useMemo(() => laneQueries(projectId, closedAfter), [projectId, closedAfter]);
  const a = useTicketPages(open);
  const b = useTicketPages(closed);
  return useMemo(() => [...(a.data?.pages[0]?.items ?? []), ...(b.data?.pages[0]?.items ?? [])], [a.data, b.data]);
}

/** The lanes collapsed in this browser. */
function useCollapsed(): [Set<string>, (id: string) => void] {
  const [collapsed, setCollapsed] = useState(readCollapsed);
  const toggle = (id: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id); else next.add(id);
      writeCollapsed(next);
      return next;
    });
  return [collapsed, toggle];
}

/** What every cell needs to show and move cards. */
interface CellEnv {
  /** The sessions on each ticket (the activity dots). */
  sessions: Map<string, Session[]>;
  /** The rest keeps its identity from render to render, so a card re-renders only when its ticket or dot changes. */
  cards: CardEnv;
}

interface CardEnv {
  onPeek: (id: string) => void;
  entries: (t: TicketSummary) => () => MenuEntry[];
  phone: boolean;
}

/**
 * A card in a cell. The board re-renders on every poll that changes anything; a card whose ticket (the same object
 * while its data is unchanged) and dot are the same skips it (PDT-136).
 */
const CellCard = memo(
  function CellCard({ ticket: t, activity, env }: { ticket: TicketSummary; activity: AgentStateValue | null; env: CardEnv }) {
    return (
      <BoardCard
        ticket={t}
        activity={activity}
        onOpen={() => env.onPeek(t.id)}
        entries={env.entries(t)}
        draggable={!env.phone && !coarsePointer()}
      />
    );
  },
  (a, b) => a.ticket === b.ticket && a.env === b.env && sameActivity(a.activity, b.activity),
);

/**
 * One lane's column: its cards, and a drop target for cards of the same lane (a card dropped from another lane is
 * ignored: lanes don't change a ticket's parent).
 */
function Cell({ label, stage, items, lane, env, height, children }: {
  /** The region's name: the column's title for the first lane, `<key> · <column>` for a parent's. */
  label: string;
  stage: BoardStage;
  items: TicketSummary[];
  /** The ids of every card in this lane. */
  lane: Set<string>;
  env: CellEnv;
  /**
   * `fill`: fills the board's height and scrolls on its own (the board without parent lanes). `capped`: at most half
   * the screen, scrolling on its own, so a long column (Done) doesn't push the parent lanes below it off screen.
   * `auto`: as tall as its cards.
   */
  height: 'fill' | 'capped' | 'auto';
  /** Shown above the cards (the column's title and count, without lanes). */
  children?: ReactNode;
}) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const [over, setOver] = useState(false);
  const dropTarget = !env.cards.phone;
  const onDragOver = (e: DragEvent) => {
    if (!dropTarget || !Array.from(e.dataTransfer?.types ?? []).includes('text/plain')) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
    if (!over) setOver(true);
  };
  const onDrop = (e: DragEvent) => {
    setOver(false);
    const d = parseDragData(e.dataTransfer.getData('text/plain') ?? '');
    if (!d || d.kind !== CARD_DRAG_KIND) return;
    e.preventDefault();
    if (!lane.has(d.id) || items.some((t) => t.id === d.id)) return;
    // Back to To do from In progress asks before stopping its work.
    void runAction(() => moveTicket(qc, { id: d.id }, stage, confirm));
  };
  return (
    <section
      aria-label={label}
      onDragOver={onDragOver}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(false); }}
      onDrop={onDrop}
      className={cn(
        'flex min-w-0 flex-1 flex-col rounded-md bg-surface-2',
        height === 'fill' && 'min-h-0',
        height === 'capped' && 'max-h-[50dvh] min-h-0',
        over && 'bg-accent ring-1 ring-primary',
      )}
    >
      {children}
      {/* Phones: room under the last card for the floating ＋. */}
      <div className={cn('px-2 pb-2', height !== 'auto' && 'min-h-0 flex-1 overflow-auto', env.cards.phone && height === 'fill' && 'pb-20')}>
        {items.length > 0 && (
          <ul aria-label={`${label} tickets`} className="flex flex-col gap-2 pt-2">
            {items.map((t) => (
              <CellCard key={t.id} ticket={t} activity={cardActivity(t, env.sessions.get(t.id) ?? [])} env={env.cards} />
            ))}
          </ul>
        )}
      </div>
    </section>
  );
}

/** A column's loading, error and empty states, and Show more. */
function ColumnStatus({ col, title, stage, showEmpty, doneDays }: { col: ColumnData; title: string; stage: BoardStage; showEmpty: boolean; doneDays: number }) {
  const { pages, items } = col;
  return (
    <>
      {pages.isPending ? (
        <Skeleton lines={3} label={`Loading ${title}`} className="p-1" />
      ) : pages.isError && !pages.data ? (
        <p className="p-2 text-xs text-destructive">{pages.error.message}</p>
      ) : showEmpty && items.length === 0 ? (
        <p className="p-2 text-xs text-muted-foreground">{stage === 'done' ? `Nothing done in the ${doneWindowLabel(doneDays).toLowerCase()}.` : 'No tickets.'}</p>
      ) : null}
      {pages.hasNextPage && (
        <Button variant="ghost" size="sm" className="mt-2 w-full" pending={pages.isFetchingNextPage} onClick={() => void pages.fetchNextPage()}>
          Show more
        </Button>
      )}
    </>
  );
}

/**
 * A parent's lane header: its key (opens the peek), title, stage, the parts' stepper and count, Start next part, its
 * menu, and the collapse toggle. The title gives way first (it truncates, down to about 12rem for the toggle, key and
 * title); past that the stage, stepper, Start next part and menu wrap to a line of their own, on the right, so nothing
 * runs past the edge. No PR here: the parts run side by side, and each card shows its own.
 */
export function LaneHeader({ lane, collapsed, onToggle, onPeek, compact }: {
  lane: Lane;
  collapsed: boolean;
  onToggle: () => void;
  onPeek: (id: string) => void;
  /** Phones: a group heading inside the one column. */
  compact: boolean;
}) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const open = useOpenTicket();
  const start = useStartTicket();
  const parent = lane.parent!;
  const t = lane.ticket;
  const openStage = t ? t.stage !== 'done' && t.stage !== 'wont_do' : true;
  const entries = (): MenuEntry[] =>
    t ? ticketMenu({ qc, confirm, ticket: t, onOpen: () => open(t) }) : [{ label: 'Open', onSelect: () => open({ id: parent.id, projectId: '' }) }];
  return (
    <div
      role="group"
      aria-label={`${parent.key} ${parent.title}`}
      data-peek
      className={cn('flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 px-1 pt-3', compact ? 'pb-1' : 'pb-1.5')}
    >
      <div className="flex min-w-0 flex-1 basis-48 items-center gap-2">
        <IconButton label={collapsed ? `Expand ${parent.key}` : `Collapse ${parent.key}`} size="sm" onClick={onToggle}>
          {collapsed ? <ChevronRight aria-hidden /> : <ChevronDown aria-hidden />}
        </IconButton>
        <TicketKey ticketKey={parent.key} title={parent.title} onClick={() => onPeek(parent.id)} className="shrink-0" />
        <h4 className="min-w-0 truncate text-xs font-medium" title={parent.title}>{parent.title}</h4>
      </div>
      <div className="ml-auto flex max-w-full flex-wrap items-center justify-end gap-x-2 gap-y-1">
        {t && <span className="shrink-0 text-meta text-muted-foreground">{STAGE_LABEL[t.stage]}</span>}
        {t?.parts && <PartsStepper parts={t.parts} showCount className={cn('shrink-0', compact ? 'w-24' : 'w-40')} />}
        {openStage && !compact && (
          <Button variant="ghost" size="sm" className="shrink-0" onClick={() => void runAction(() => startNextPart(start, parent))}>
            Start next part
          </Button>
        )}
        <MenuButton label={`Actions for ${parent.key}`} size="sm" entries={entries} />
      </div>
    </div>
  );
}

/** "No parts on the board yet" or the count, for a lane whose cards are all elsewhere. */
const laneEmpty = (lane: Lane) => (['todo', 'in_progress', 'done'] as BoardStage[]).every((s) => lane.items[s].length === 0);

/** The column after `id` (by +1 or -1), or null at either end. */
export function nextColumn(columns: DisplayColumn[], id: string, by: 1 | -1): DisplayColumn | null {
  const i = columns.findIndex((c) => c.id === id);
  return i < 0 ? null : columns[i + by] ?? null;
}

/** A horizontal swipe far enough to change column (and more sideways than up or down). */
const SWIPE_PX = 60;
/** Touches that start this close to the screen's left edge belong to the system Back gesture. */
const EDGE_PX = 24;

/**
 * Phones: the board's columns as a row of chips with their counts, scrolling sideways when they don't fit; the chosen
 * one is kept in view.
 */
export function ColumnChips({ ref, columns, counts, value, onChange }: {
  ref?: Ref<HTMLDivElement>;
  columns: DisplayColumn[];
  counts: Record<string, number | undefined>;
  value: string;
  onChange: (id: string) => void;
}) {
  const strip = useRef<HTMLDivElement | null>(null);
  const setStrip = (el: HTMLDivElement | null) => {
    strip.current = el;
    if (typeof ref === 'function') ref(el);
    else if (ref) ref.current = el;
  };
  useEffect(() => {
    strip.current?.querySelector<HTMLElement>('[aria-pressed="true"]')?.scrollIntoView?.({ block: 'nearest', inline: 'nearest' });
  }, [value]);
  return (
    <div ref={setStrip} role="group" aria-label="Board column" className="-mx-2 flex shrink-0 gap-1.5 overflow-x-auto px-2 pb-0.5 md:-mx-3 md:px-3">
      {columns.map((c) => {
        const n = counts[c.id];
        const on = c.id === value;
        return (
          <Button
            key={c.id}
            size="sm"
            variant={on ? 'primary' : 'secondary'}
            aria-pressed={on}
            aria-label={n !== undefined ? `${c.title} (${n})` : c.title}
            className="shrink-0 rounded-full"
            onClick={() => onChange(c.id)}
          >
            {c.title}
            {n !== undefined && <span className={cn('tabular-nums', !on && 'text-muted-foreground')}>{n}</span>}
          </Button>
        );
      })}
    </div>
  );
}

/**
 * To do / In progress / Done side by side, each its own paged, polled query; drag a card to another column of its lane
 * to move it (desktop). With parents on the board the rows are lanes and the board scrolls as a whole; the first lane's
 * columns stop at half the screen and scroll on their own, so the parents' lanes stay in reach. Phones show one
 * column at a time, picked from chips with counts or by a swipe sideways, its parts under a heading per parent; cards
 * move from their menu (Move to), never by drag.
 */
export function Board({ projectId, onPeek, phone }: { projectId: string | null; onPeek: (id: string) => void; phone: boolean }) {
  // Fixed while the board is on screen and its window stays the same: a new value every render would be a new query
  // every render.
  const doneDays = useDoneWindow(projectId);
  const closedAfter = useMemo(() => Date.now() - doneDays * DAY_MS, [doneDays]);
  const [shown, setShown] = useState<string>('todo');
  const sessions = useAllSessions();
  const qc = useQueryClient();
  const confirm = useConfirm();
  const open = useOpenTicket();
  const start = useStartTicket();
  const cols: Record<BoardStage, ColumnData> = {
    todo: useColumn(projectId, 'todo', closedAfter),
    in_progress: useColumn(projectId, 'in_progress', closedAfter),
    done: useColumn(projectId, 'done', closedAfter),
  };
  const parents = useLaneParents(projectId, closedAfter);
  const [collapsed, toggle] = useCollapsed();
  const items: Columns = { todo: cols.todo.items, in_progress: cols.in_progress.items, done: cols.done.items };
  const lanes = useMemo(() => buildLanes(items, parents), [items.todo, items.in_progress, items.done, parents]);
  useClearToStartToasts(useMemo(() => [...items.todo, ...items.in_progress, ...items.done], [items.todo, items.in_progress, items.done]));

  const byTicket = useMemo(() => sessionsByTicket(sessions), [sessions]);
  // The menu reads the latest hooks' values when it opens; the cards' env stays the same object.
  const menuDeps = useRef({ qc, confirm, open, start });
  menuDeps.current = { qc, confirm, open, start };
  const cardEnv = useMemo<CardEnv>(() => ({
    onPeek,
    phone,
    entries: (t) => () => {
      const d = menuDeps.current;
      return ticketMenu({
        qc: d.qc, confirm: d.confirm, ticket: t,
        onOpen: () => d.open(t),
        ...(canStart(t) ? { onStart: () => d.start(t) } : {}),
      });
    },
  }), [onPeek, phone]);
  const env: CellEnv = { sessions: byTicket, cards: cardEnv };
  const all = displayColumns(boardSplits([...items.in_progress, ...lanes.flatMap((l) => l.items.in_progress)]));
  // Phones: one column; In progress's id falls back to Code when it splits (and back).
  const current = all.find((c) => c.id === shown) ?? all.find((c) => c.stage === shown || c.id === 'in_progress') ?? all[0];
  const columns = phone ? [current] : all;
  const [first, ...parentLanes] = lanes;
  const withLanes = parentLanes.length > 0;
  const firstIds = laneIds(first);
  const cards = (list: TicketSummary[], c: DisplayColumn) => (c.pipeline ? list.filter((t) => inColumn(t, c)) : list);
  // A pipeline column's status (loading, Show more) is In progress's, shown once under Code.
  const ownsStatus = (c: DisplayColumn) => !c.pipeline || c.pipeline === 'code';

  const columnCount = (c: DisplayColumn) => (c.pipeline ? cards(cols.in_progress.items, c).length : cols[c.stage].total);
  const columnHead = (c: DisplayColumn) => {
    const count = columnCount(c);
    return (
      <h3 className="flex items-center gap-2 px-3 pt-2 pb-1 text-xs font-medium">
        <span className="flex-1">{c.title}</span>
        {count !== undefined && <span className="text-meta text-muted-foreground tabular-nums">{count}</span>}
      </h3>
    );
  };
  const status = (c: DisplayColumn, showEmpty: boolean) =>
    ownsStatus(c) ? (
      <ColumnStatus col={cols[c.stage]} title={c.title} stage={c.stage} showEmpty={showEmpty} doneDays={doneDays} />
    ) : showEmpty && cards(cols.in_progress.items, c).length === 0 ? (
      <p className="p-2 text-xs text-muted-foreground">No tickets.</p>
    ) : null;

  // Phones: a swipe sideways shows the next or previous column.
  const swipe = useRef<{ x: number; y: number } | null>(null);
  const chips = useRef<HTMLDivElement>(null);
  const onTouchStart = (e: TouchEvent) => {
    const t = e.touches[0];
    // The chips scroll sideways on their own: a swipe there scrolls them, it doesn't change column.
    const onChips = chips.current?.contains(e.target as Node) ?? false;
    swipe.current = e.touches.length === 1 && t && t.clientX > EDGE_PX && !onChips ? { x: t.clientX, y: t.clientY } : null;
  };
  const onTouchEnd = (e: TouchEvent) => {
    const from = swipe.current;
    const t = e.changedTouches[0];
    swipe.current = null;
    if (!from || !t) return;
    const dx = t.clientX - from.x;
    const dy = t.clientY - from.y;
    if (Math.abs(dx) < SWIPE_PX || Math.abs(dx) < Math.abs(dy) * 2) return;
    const to = nextColumn(all, current.id, dx < 0 ? 1 : -1);
    if (to) setShown(to.id);
  };

  return (
    <div
      className="flex min-h-0 flex-1 flex-col gap-2 p-2 md:p-3"
      {...(phone ? { onTouchStart, onTouchEnd, onTouchCancel: () => { swipe.current = null; } } : {})}
    >
      {phone && (
        <ColumnChips
          ref={chips}
          columns={all}
          counts={Object.fromEntries(all.map((c) => [c.id, columnCount(c)]))}
          value={current.id}
          onChange={setShown}
        />
      )}
      {!withLanes ? (
        <div className="flex min-h-0 flex-1 gap-3">
          {columns.map((c) => (
            <Cell key={c.id} label={c.title} stage={c.stage} items={cards(first.items[c.stage], c)} lane={firstIds} env={env} height="fill">
              {!phone && columnHead(c)}
              <div className="px-2">{status(c, true)}</div>
            </Cell>
          ))}
        </div>
      ) : phone ? (
        // One column: the first lane's cards, then a heading per parent with its parts in this column (room at the
        // end for the floating ＋).
        <div className="min-h-0 flex-1 overflow-auto pb-20">
          {columns.map((c) => (
            <div key={c.id} className="flex flex-col">
              <Cell label={c.title} stage={c.stage} items={cards(first.items[c.stage], c)} lane={firstIds} env={env} height="capped">
                <div className="px-2">{status(c, false)}</div>
              </Cell>
              {parentLanes.map((lane) => {
                const shut = collapsed.has(lane.id!);
                return (
                  <div key={lane.id}>
                    <LaneHeader lane={lane} collapsed={shut} onToggle={() => toggle(lane.id!)} onPeek={onPeek} compact />
                    {!shut && (cards(lane.items[c.stage], c).length > 0 ? (
                      <Cell label={`${lane.parent!.key} · ${c.title}`} stage={c.stage} items={cards(lane.items[c.stage], c)} lane={laneIds(lane)} env={env} height="auto" />
                    ) : (
                      <p className="px-3 pb-1 text-xs text-muted-foreground">{laneEmpty(lane) ? 'No parts on the board yet.' : `No parts in ${c.title}.`}</p>
                    ))}
                  </div>
                );
              })}
            </div>
          ))}
        </div>
      ) : (
        <div className="min-h-0 flex-1 overflow-auto">
          <div className="flex gap-3">
            {columns.map((c) => <div key={c.id} className="min-w-0 flex-1">{columnHead(c)}</div>)}
          </div>
          <div className="flex gap-3">
            {columns.map((c) => (
              <Cell key={c.id} label={c.title} stage={c.stage} items={cards(first.items[c.stage], c)} lane={firstIds} env={env} height="capped">
                <div className="px-2">{status(c, false)}</div>
              </Cell>
            ))}
          </div>
          {parentLanes.map((lane) => {
            const shut = collapsed.has(lane.id!);
            const ids = laneIds(lane);
            return (
              <div key={lane.id} className="flex flex-col">
                <LaneHeader lane={lane} collapsed={shut} onToggle={() => toggle(lane.id!)} onPeek={onPeek} compact={false} />
                {!shut && (laneEmpty(lane) ? (
                  <p className="rounded-md bg-surface-2 px-3 py-2 text-xs text-muted-foreground">No parts on the board yet.</p>
                ) : (
                  <div className="flex gap-3">
                    {columns.map((c) => (
                      <Cell key={c.id} label={`${lane.parent!.key} · ${c.title}`} stage={c.stage} items={cards(lane.items[c.stage], c)} lane={ids} env={env} height="auto" />
                    ))}
                  </div>
                ))}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
