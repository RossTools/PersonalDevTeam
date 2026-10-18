// The Tickets view's Board: committed work in three columns (To do, In progress, Done in the last two weeks). Ready
// tickets wait in the backlog until they are moved to To do.
import { useMemo, useState, type DragEvent } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { GitBranch } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import {
  Button, ContextMenu, dragData, MenuButton, parseDragData, SegmentedControl, Skeleton, useConfirm, type MenuEntry,
} from '@ui';
import type { Session } from '../api/gen/Session';
import type { TicketQuery } from '../api/gen/TicketQuery';
import type { TicketStage } from '../api/gen/TicketStage';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { AgentDot, itemState, rollup, type AgentStateValue } from '../common/AgentState';
import { PriorityIcon } from '../common/PriorityIcon';
import { TicketKey } from '../common/TicketKey';
import { runAction } from '../common/useAction';
import { canStart, patchTicket, ticketMenu, useOpenTicket, useStartTicket } from './actions';
import { useAllSessions, useTicketPages } from './queries';
import { useClearToStartToasts } from './clearToStart';
import { TicketBadges } from './TicketBadges';
import { LabelChips } from './TicketParts';

export type BoardStage = Extract<TicketStage, 'todo' | 'in_progress' | 'done'>;

export const BOARD_COLUMNS: { stage: BoardStage; title: string }[] = [
  { stage: 'todo', title: 'To do' },
  { stage: 'in_progress', title: 'In progress' },
  { stage: 'done', title: 'Done' },
];

/** The Done column shows tickets closed in the last two weeks. */
export const DONE_WINDOW_MS = 14 * 24 * 60 * 60 * 1000;
const BOARD_PAGE = 50;

/** A column's `GET /api/tickets` query. Done: closed since `closedAfter`, most recently updated first. */
export function columnQuery(projectId: string | null, stage: BoardStage, closedAfter: number): TicketQuery {
  const q: TicketQuery = { stage, limit: BOARD_PAGE };
  if (projectId) q.project = projectId;
  if (stage === 'done') {
    q.closedAfter = closedAfter;
    q.sort = 'updated';
  }
  return q;
}

/** The drag payload of a card (web/ui README: text/plain `pdt:<kind>:<id>`). */
export const CARD_DRAG_KIND = 'ticket';

const coarsePointer = () => typeof window !== 'undefined' && !!window.matchMedia?.('(pointer: coarse)').matches;

/** The most urgent state of the sessions working on a ticket's active attempt (null when none). */
export function cardActivity(t: Pick<TicketSummary, 'id' | 'attempt'>, sessions: Session[]): AgentStateValue | null {
  if (!t.attempt) return null;
  const mine = sessions.filter((s) => s.ticket?.id === t.id && s.attemptId === t.attempt!.id && s.kind === 'agent_tui');
  return rollup(mine.map((s) => itemState({ session: s, unseen: s.unseen })));
}

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
      draggable={draggable}
      onDragStart={draggable ? onDragStart : undefined}
      onDragEnd={() => setDragging(false)}
      className={cn(
        'group/card relative flex flex-col rounded-md border border-border bg-card',
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
            <PriorityIcon priority={t.priority} />
            <TicketBadges ticket={t} className="flex-wrap" />
          </span>
          <span className={cn('line-clamp-2 text-sm', activity?.unseen && 'font-semibold')}>{t.title}</span>
          {t.labels.length > 0 && <LabelChips labels={t.labels} className="flex-wrap" />}
          {t.attempt && (
            <span className="flex min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
              {activity ? <AgentDot state={activity} /> : <GitBranch aria-hidden className="size-3 shrink-0" />}
              <span className="min-w-0 truncate font-mono" title={t.attempt.branch}>{t.attempt.branch}</span>
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

function Column({ projectId, stage, title, closedAfter, sessions, onPeek, dropTarget, phone }: {
  projectId: string | null;
  stage: BoardStage;
  title: string;
  closedAfter: number;
  sessions: Session[];
  onPeek: (id: string) => void;
  dropTarget: boolean;
  phone: boolean;
}) {
  const qc = useQueryClient();
  const confirm = useConfirm();
  const open = useOpenTicket();
  const start = useStartTicket();
  const query = useMemo(() => columnQuery(projectId, stage, closedAfter), [projectId, stage, closedAfter]);
  const pages = useTicketPages(query);
  const items = useMemo(() => pages.data?.pages.flatMap((p) => p.items) ?? [], [pages.data]);
  const total = pages.data?.pages[0]?.total;
  useClearToStartToasts(items);
  const [over, setOver] = useState(false);

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
    if (items.some((t) => t.id === d.id)) return;
    void runAction(() => patchTicket(qc, d.id, { stage }));
  };

  const entries = (t: TicketSummary) => () =>
    ticketMenu({
      qc, confirm, ticket: t,
      onOpen: () => open(t),
      ...(canStart(t) ? { onStart: () => start(t) } : {}),
    });

  return (
    <section
      aria-label={title}
      onDragOver={onDragOver}
      onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(false); }}
      onDrop={onDrop}
      className={cn(
        'flex min-h-0 min-w-0 flex-1 flex-col rounded-md bg-surface-2',
        over && 'bg-accent ring-1 ring-primary',
      )}
    >
      {!phone && (
        <h3 className="flex items-center gap-2 px-3 pt-2 pb-1 text-xs font-medium">
          <span className="flex-1">{title}</span>
          {total !== undefined && <span className="text-meta text-muted-foreground tabular-nums">{total}</span>}
        </h3>
      )}
      <div className="min-h-0 flex-1 overflow-auto px-2 pb-2">
        {pages.isPending ? (
          <Skeleton lines={3} label={`Loading ${title}`} className="p-1" />
        ) : pages.isError && !pages.data ? (
          <p className="p-2 text-xs text-destructive">{pages.error.message}</p>
        ) : items.length === 0 ? (
          <p className="p-2 text-xs text-muted-foreground">{stage === 'done' ? 'Nothing done in the last two weeks.' : 'No tickets.'}</p>
        ) : (
          <ul aria-label={`${title} tickets`} className="flex flex-col gap-2 pt-1">
            {items.map((t) => (
              <BoardCard
                key={t.id}
                ticket={t}
                activity={cardActivity(t, sessions)}
                onOpen={() => onPeek(t.id)}
                entries={entries(t)}
                draggable={!phone && !coarsePointer()}
              />
            ))}
          </ul>
        )}
        {pages.hasNextPage && (
          <Button variant="ghost" size="sm" className="mt-2 w-full" pending={pages.isFetchingNextPage} onClick={() => void pages.fetchNextPage()}>
            Show more
          </Button>
        )}
      </div>
    </section>
  );
}

/**
 * To do / In progress / Done side by side, each its own paged, polled query; drag a card to another column to move
 * it (desktop). Phones show one column at a time with a picker.
 */
export function Board({ projectId, onPeek, phone }: { projectId: string | null; onPeek: (id: string) => void; phone: boolean }) {
  // Fixed while the board is on screen: a new value every render would be a new query every render.
  const [closedAfter] = useState(() => Date.now() - DONE_WINDOW_MS);
  const [stage, setStage] = useState<BoardStage>('todo');
  const sessions = useAllSessions();
  const columns = phone ? BOARD_COLUMNS.filter((c) => c.stage === stage) : BOARD_COLUMNS;
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-2 p-2 md:p-3">
      {phone && (
        <SegmentedControl
          aria-label="Board column"
          size="sm"
          className="self-start"
          options={BOARD_COLUMNS.map((c) => ({ value: c.stage, label: c.title }))}
          value={stage}
          onChange={setStage}
        />
      )}
      <div className="flex min-h-0 flex-1 gap-3">
        {columns.map((c) => (
          <Column
            key={c.stage}
            projectId={projectId}
            stage={c.stage}
            title={c.title}
            closedAfter={closedAfter}
            sessions={sessions}
            onPeek={onPeek}
            dropTarget={!phone}
            phone={phone}
          />
        ))}
      </div>
    </div>
  );
}
