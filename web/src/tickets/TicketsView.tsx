import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { useQuery, useQueryClient, type InfiniteData, type QueryClient, type QueryKey } from '@tanstack/react-query';
import { Check, ChevronDown, Plus } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import {
  Badge, BulkActionBar, Button, Checkbox, DataList, DataListRow, EmptyState, FilterBar, Input, MenuButton, Popover,
  RelativeTime, SegmentedControl, Select, Sheet, Skeleton, Switch, useConfirm, useMenuAt, ViewHeader, type MenuEntry,
} from '@ui';
import { api } from '../api/client';
import type { Priority } from '../api/gen/Priority';
import type { Project } from '../api/gen/Project';
import type { TicketPage } from '../api/gen/TicketPage';
import type { TicketSort } from '../api/gen/TicketSort';
import type { TicketStage } from '../api/gen/TicketStage';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketSummary } from '../api/gen/TicketSummary';
import { PriorityIcon, PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL, StageStepper } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { actionError, runAction } from '../common/useAction';
import { PHONE_QUERY, useMediaQuery } from '../common/useMediaQuery';
import { canStart, deleteTicket, moveToEntries, startNextPart, stopWaitingEntry, ticketMenu, useOpenTicket, useStartTicket } from './actions';
import { canMakeParts, MakePartsDialog, ParentChip, PartsSection } from './PartsSection';
import { PartsStepper } from '../common/PartsStepper';
import { AgentDot } from '../common/AgentState';
import { useTopBarActions } from '../shell/PhoneTopBar';
import { AttemptSection, MergedBanner } from './Attempt';
import { agentLine, Board, PIPELINE_LABEL } from './Board';
import { NewTicketFab, PhoneToolbar } from './PhoneTickets';
import { PhoneTicket } from './PhoneTicket';
import { useClearToStartToasts } from './clearToStart';
import { TicketBadges } from './TicketBadges';
import { ImportJiraDialog } from './ImportJiraDialog';
import {
  BACKLOG_STAGES, canRank, groupTickets, isClosed, isDefaultFilter, normaliseLabel, STAGES, toQuery, type GroupBy,
} from './model';
import { invalidateTickets, ticketKeys, useCheckMerge, useLabels, useTicket, useTicketId, useTicketPages } from './queries';
import { LabelChips, TicketDescription, TicketDetails, TicketLinks, TicketOverlaps, TicketTimeline, TicketTitle } from './TicketParts';
import type { BacklogState } from './useBacklog';

// ---------------------------------------------------------------------------------------------------------------
// Filter chips

interface Option<V extends string> { value: V; label: string }

/** A filter chip that opens a checkbox list (several can be picked without reopening it). */
function MultiChip<V extends string>({ label, options, selected, onChange, none }: {
  label: string;
  /** What an empty selection means, shown on the chip ("Draft, Ready"). */
  none?: string;
  options: Option<V>[];
  selected: V[];
  onChange: (v: V[]) => void;
}) {
  const names = options.filter((o) => selected.includes(o.value)).map((o) => o.label);
  const text = names.length === 0 ? (none ? `${label}: ${none}` : label) : names.length <= 2 ? `${label}: ${names.join(', ')}` : `${label}: ${names.length}`;
  return (
    <Popover
      label={label}
      placement="bottom start"
      className="w-56 p-1"
      trigger={
        <Button variant={selected.length > 0 ? 'secondary' : 'ghost'} size="sm" compact aria-label={`${label} filter`}>
          {text} <ChevronDown aria-hidden />
        </Button>
      }
    >
      <div className="flex max-h-72 flex-col gap-0.5 overflow-auto p-1">
        {options.length === 0 && <p className="p-1 text-xs text-muted-foreground">None yet.</p>}
        {options.map((o) => (
          <Checkbox
            key={o.value}
            checked={selected.includes(o.value)}
            onChange={(on) => onChange(on ? [...selected, o.value] : selected.filter((v) => v !== o.value))}
          >
            {o.label}
          </Checkbox>
        ))}
        {selected.length > 0 && (
          <Button variant="ghost" size="sm" compact className="mt-1 self-start" onClick={() => onChange([])}>Clear</Button>
        )}
      </div>
    </Popover>
  );
}

/** A filter chip with one choice of three: either (null), yes (true) or no (false). */
function ChoiceChip({ label, value, onChange, yes, no }: {
  label: string;
  value: boolean | null;
  onChange: (v: boolean | null) => void;
  /** What true and false are called ("With overlap", "No overlap"). */
  yes: string;
  no: string;
}) {
  const text = value === null ? label : `${label}: ${value ? yes : no}`;
  const entry = (v: boolean | null, name: string): MenuEntry => ({
    label: name,
    icon: value === v ? <Check aria-hidden /> : <span aria-hidden className="size-4" />,
    onSelect: () => onChange(v),
  });
  return (
    <MenuButton label={`${label} filter`} placement="bottom start" entries={() => [entry(null, 'Any'), entry(true, yes), entry(false, no)]}>
      <Button variant={value !== null ? 'secondary' : 'ghost'} size="sm" compact aria-label={`${label} filter`}>
        {text} <ChevronDown aria-hidden />
      </Button>
    </MenuButton>
  );
}

const GROUPS: Option<GroupBy>[] = [
  { value: 'none', label: 'No grouping' },
  { value: 'stage', label: 'Group: stage' },
  { value: 'priority', label: 'Group: priority' },
  { value: 'label', label: 'Group: label' },
];
const SORTS: Option<TicketSort>[] = [
  { value: 'rank', label: 'Sort: rank' },
  { value: 'priority', label: 'Sort: priority' },
  { value: 'updated', label: 'Sort: updated' },
  { value: 'created', label: 'Sort: created' },
];

// ---------------------------------------------------------------------------------------------------------------
// Rank moves (optimistic)

/** The pages with `ids` moved directly before or after `target`, page sizes kept (so the cursor pages still line up). */
export function movedPages(data: InfiniteData<TicketPage>, ids: string[], target: string, place: 'before' | 'after'): InfiniteData<TicketPage> {
  const all = data.pages.flatMap((p) => p.items);
  const moving = ids.map((id) => all.find((t) => t.id === id)).filter((t): t is TicketSummary => !!t);
  const rest = all.filter((t) => !ids.includes(t.id));
  const at = rest.findIndex((t) => t.id === target);
  if (at < 0 || moving.length === 0) return data;
  rest.splice(place === 'before' ? at : at + 1, 0, ...moving);
  let i = 0;
  return { ...data, pages: data.pages.map((p) => ({ ...p, items: rest.slice(i, (i += p.items.length)) })) };
}

async function rankMove(qc: QueryClient, key: QueryKey, ids: string[], target: string, place: 'before' | 'after') {
  // A poll in flight would land the old order over the optimistic one.
  await qc.cancelQueries({ queryKey: key });
  const previous = qc.getQueryData<InfiniteData<TicketPage>>(key);
  if (previous) qc.setQueryData(key, movedPages(previous, ids, target, place));
  let moved = 0;
  try {
    // The first goes next to the target; the rest follow it in order.
    await api.rankTicket(ids[0], place === 'before' ? { before: target } : { after: target });
    for (moved = 1; moved < ids.length; moved++) await api.rankTicket(ids[moved], { after: ids[moved - 1] });
  } catch (e) {
    if (previous) qc.setQueryData(key, previous);
    // One request per ticket: say when some moved (the refetch below shows where they are).
    if (moved > 0) throw new Error(`Moved ${moved} of ${ids.length} tickets: ${actionError(e)}`);
    throw e;
  } finally {
    invalidateTickets(qc);
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Rows

function TicketRowContent({ t, phone }: { t: TicketSummary; phone: boolean }) {
  const badges = (
    <>
      {t.parent && <ParentChip parent={t.parent} projectId={t.projectId} static />}
      {t.parts && <PartsStepper parts={t.parts} showCount className="w-20 shrink-0" />}
      <TicketBadges ticket={t} />
    </>
  );
  if (phone) {
    return (
      <div className="flex min-w-0 flex-1 flex-col gap-0.5 py-0.5">
        <div className="flex min-w-0 items-center gap-2">
          <TicketKey ticketKey={t.key} />
          <span className={cn('min-w-0 flex-1 truncate', isClosed(t.stage) && 'text-muted-foreground line-through')}>{t.title}</span>
        </div>
        <div className="flex min-w-0 items-center gap-1.5 text-meta text-muted-foreground">
          {t.agent && (() => {
            const line = agentLine(t.agent);
            return <><AgentDot state={line.dot} /><span className="max-w-32 shrink-0 truncate">{line.text}</span><span aria-hidden>·</span></>;
          })()}
          <span className="shrink-0">{t.pipelineStage && t.stage === 'in_progress' ? PIPELINE_LABEL[t.pipelineStage] : STAGE_LABEL[t.stage]}</span>
          {badges}
          <LabelChips labels={t.labels} className="min-w-0 overflow-hidden" />
          <span className="flex-1" />
          <PriorityIcon priority={t.priority} />
        </div>
      </div>
    );
  }
  return (
    <>
      <TicketKey ticketKey={t.key} className="w-20 justify-start" />
      <span className={cn('min-w-0 flex-1 truncate', isClosed(t.stage) && 'text-muted-foreground line-through')}>{t.title}</span>
      {badges}
      <LabelChips labels={t.labels} className="hidden max-w-60 overflow-hidden lg:flex" />
      <Badge className="w-20 justify-center">{STAGE_LABEL[t.stage]}</Badge>
      <PriorityIcon priority={t.priority} className="w-4" />
    </>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Quick add

/** `before`: the first row of a flat rank-ordered list, so the new ticket shows up where it was typed (without it the
 *  server puts it at the top of Draft, which may be far down). */
function QuickAdd({ projectId, before, onDetails }: { projectId: string; before?: string; onDetails: (title: string) => void }) {
  const qc = useQueryClient();
  const [title, setTitle] = useState('');
  const [pending, setPending] = useState(false);
  const onKeyDown = async (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== 'Enter' || e.nativeEvent.isComposing) return;
    e.preventDefault();
    if (e.shiftKey) {
      onDetails(title.trim());
      setTitle('');
      return;
    }
    const t = title.trim();
    if (!t || pending) return;
    setPending(true);
    const created = await runAction(() => api.createTicket(projectId, before ? { title: t, before } : { title: t }));
    setPending(false);
    if (created) {
      setTitle('');
      invalidateTickets(qc);
    }
  };
  return (
    <div className="flex items-center gap-2 border-b border-border px-2 py-1">
      <Plus aria-hidden className="size-4 shrink-0 text-muted-foreground" />
      <Input
        aria-label="New ticket title"
        placeholder="New ticket title… Enter to add, Shift+Enter for details"
        value={title}
        disabled={pending}
        maxLength={200}
        onChange={(e) => setTitle(e.target.value)}
        onKeyDown={(e) => void onKeyDown(e)}
        className="border-transparent bg-transparent shadow-none"
      />
    </div>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Peek

/** The ticket in a side sheet: its Overview in short, Open for the tab, ⋯ for the rest. A phone shows `PhoneTicket` instead. */
function TicketPeek({ ticketId, open: shown, missing, onClose, onLoaded, phone }: {
  ticketId: string | null; open: boolean; missing: boolean; onClose: () => void; onLoaded: (t: Ticket) => void; phone: boolean;
}) {
  const q = useTicket(ticketId);
  const qc = useQueryClient();
  const confirm = useConfirm();
  const open = useOpenTicket();
  const start = useStartTicket();
  const t = q.data;
  const [making, setMaking] = useState(false);
  // An unsaved description edit: closing the peek (a click outside, ×, Escape) asks before dropping it.
  const editing = useRef(false);
  const onDirtyChange = useCallback((d: boolean) => { editing.current = d; }, []);
  const close = async () => {
    if (editing.current) {
      const { ok } = await confirm({ title: 'Discard your edit?', body: "The description you're editing hasn't been saved.", action: 'Discard', cancel: 'Keep editing' });
      if (!ok) return;
    }
    onClose();
  };
  useCheckMerge(t);
  useEffect(() => {
    if (t) onLoaded(t);
  }, [t, onLoaded]);
  const title = t ? `${t.key} · ${t.title}` : 'Ticket';
  const actions = t && (
    <>
      <MenuButton
        label="Ticket actions"
        placement="top end"
        entries={() => [
          ...(t.waiting ? [stopWaitingEntry(qc, t)] : []),
          { submenu: 'Move to', entries: moveToEntries(qc, t, confirm) },
          ...(canMakeParts(t) ? [{ label: 'Make into parts…', onSelect: () => setMaking(true) }] : []),
          { separator: true },
          { label: 'Delete…', danger: true, onSelect: () => void runAction(async () => { if (await deleteTicket(confirm, qc, t)) onClose(); }) },
        ]}
      />
      {canStart(t) && <Button onClick={() => start(t)}>Start</Button>}
      {t.parts && !isClosed(t.stage) && <Button onClick={() => void runAction(() => startNextPart(start, t))}>Start next part</Button>}
      <Button variant="primary" onClick={() => { onClose(); open(t); }}>Open</Button>
    </>
  );
  const body = (
    !t ? (
      q.isPending && !missing ? <Skeleton lines={4} label="Loading ticket" /> : <p className="text-sm text-muted-foreground">{q.error?.message ?? 'Ticket not found.'}</p>
    ) : (
      // Sections divided by rules, so the sheet reads as blocks rather than one floating column.
      <div className="flex flex-col divide-y divide-border *:py-4 *:first:pt-1 *:last:pb-1">
        <div className="flex flex-col gap-2 pr-8">
          <span className="flex items-center gap-1 self-start">
            {t.parent && <ParentChip parent={t.parent} projectId={t.projectId} />}
            <TicketKey ticketKey={t.key} />
          </span>
          <TicketTitle ticket={t} className="text-base font-semibold" />
          <StageStepper stage={t.stage} compact />
          <TicketBadges ticket={t} className="flex-wrap" />
          {t.parts && <PartsStepper parts={t.parts} showCount className="w-40" />}
          <MergedBanner ticket={t} className="rounded-md border" />
        </div>
        <TicketDetails ticket={t} />
        <AttemptSection ticket={t} compact />
        {t.overlaps.length > 0 && <TicketOverlaps ticket={t} />}
        <TicketDescription key={t.id} ticket={t} onDirtyChange={onDirtyChange} />
        <PartsSection ticket={t} />
        <TicketLinks ticket={t} />
        <TicketTimeline ticketId={t.id} recent={5} />
      </div>
    )
  );
  const makeParts = making && t && <MakePartsDialog ticket={t} onClose={() => setMaking(false)} />;
  // On a phone the ticket is its own screen under the top bar (← is Back), not a sheet over it (PDT-112, PDT-119).
  if (phone) {
    if (!shown) return null;
    // Keyed by ticket: its edit, folds and section are its own.
    return t ? <PhoneTicket key={t.id} ticket={t} onDeleted={onClose} /> : <div className="px-4 py-3">{body}</div>;
  }
  return (
    <>
    {makeParts}
    <Sheet
      side="right"
      title={title}
      hideTitle
      open={shown}
      // The backlog stays usable beside the peek (UX spec §8.1). A click on the page closes it, except on a backlog
      // row or a board card or lane, which shows that ticket instead.
      modal={false}
      keepOpenOn={(el) => el.closest('[role="row"][data-key], [data-peek]') !== null}
      onOpenChange={(o) => { if (!o) void close(); }}
      className="w-[32rem] max-w-[90vw]"
      footer={actions}
    >
      {body}
    </Sheet>
    </>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// The view

export interface TicketsViewProps {
  backlog: BacklogState;
  projects: Project[];
  /** Opens the full New ticket form (in this project, with this title). */
  onNewTicket: (projectId: string | null, title?: string) => void;
  /** The ticket peeked at, by key or id, when something else holds it (the app's route, PDT-111). */
  peek?: TicketPeekState;
}

export interface TicketPeekState {
  ticket: string | null;
  /** `replace`: the same ticket, named by its key instead of its id. */
  onChange: (ref: string | null, opts?: { replace?: boolean }) => void;
}

/** The Tickets view's work area: header (Backlog | Board, New ticket), filters or bulk actions, the paged backlog, the peek. */
export function TicketsView({ backlog, projects, onNewTicket, peek }: TicketsViewProps) {
  const phone = useMediaQuery(PHONE_QUERY);
  const qc = useQueryClient();
  const confirm = useConfirm();
  const open = useOpenTicket();
  const start = useStartTicket();
  const { projectId, filters, mode } = backlog;
  const project = projects.find((p) => p.id === projectId) ?? null;
  // A project linked to Jira can import an issue by its key.
  const settings = useQuery({
    queryKey: ['project-settings', project?.id],
    queryFn: () => api.projectSettings(project!.id),
    enabled: !!project && project.kind !== 'personal',
  });
  const [importing, setImporting] = useState(false);
  const query = useMemo(() => toQuery(project?.id ?? null, filters), [project?.id, filters]);
  const pages = useTicketPages(query, mode === 'backlog');
  const items = useMemo(() => pages.data?.pages.flatMap((p) => p.items) ?? [], [pages.data]);
  const total = pages.data?.pages[0]?.total;
  useClearToStartToasts(mode === 'backlog' ? items : []);
  const labels = useLabels(project?.id ?? null);
  const [selection, setSelection] = useState<Set<string>>(new Set());
  // Phones: rows get checkboxes only in bulk mode (a row's menu → Select), which ends once nothing is selected.
  const [selecting, setSelecting] = useState(false);
  const [ownPeek, setOwnPeek] = useState<string | null>(null);
  const peekRef = peek ? peek.ticket : ownPeek;
  const setPeekId = useCallback((ref: string | null) => (peek ? peek.onChange(ref) : setOwnPeek(ref)), [peek]);
  const { id: peekId, missing: peekMissing } = useTicketId(peekRef);
  // Peeked at by id (a row, a card): the path names it by its key once it has loaded.
  const peekLoaded = useCallback((t: Ticket) => {
    if (peek && peek.ticket && peek.ticket !== t.key && (peek.ticket === t.id || peek.ticket.toUpperCase() === t.key)) peek.onChange(t.key, { replace: true });
  }, [peek]);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const { openMenu, menu } = useMenuAt();
  const rankable = canRank(project?.id ?? null, filters);

  // Only rows still in the list count as selected (a refetch may have dropped some).
  const selected = useMemo(() => items.filter((t) => selection.has(t.id)), [items, selection]);
  const selectedIds = selected.map((t) => t.id);
  const bulkMode = !phone || selecting;
  useEffect(() => {
    if (selecting && selected.length === 0) setSelecting(false);
  }, [selecting, selected.length]);
  const clearSelection = () => { setSelection(new Set()); setSelecting(false); };
  const allLabels = useMemo(
    () => [...new Set([...(labels.data ?? []), ...items.flatMap((t) => t.labels)])].sort(),
    [labels.data, items],
  );

  const sections = useMemo(() => {
    if (filters.group === 'none') return undefined;
    return groupTickets(items, filters.group).map((g) => ({ ...g, collapsed: collapsed.has(g.id) }));
  }, [items, filters.group, collapsed]);

  const listKey = ticketKeys.list(query);
  const moveRows = (ids: string[], target: string, place: 'before' | 'after') => void runAction(() => rankMove(qc, listKey, ids, target, place));
  const neighbour = (id: string, by: number) => items[items.findIndex((t) => t.id === id) + by];

  const bulk = (action: Parameters<typeof api.bulkTickets>[0]['action'], value?: string, keep = true) =>
    void runAction(async () => {
      await api.bulkTickets({ ids: selectedIds, action, ...(value !== undefined ? { value } : {}) });
      invalidateTickets(qc, selectedIds);
      if (!keep) clearSelection();
    });
  const bulkDelete = () =>
    void runAction(async () => {
      const n = selectedIds.length;
      const { ok } = await confirm({
        title: `Delete ${n} ticket${n === 1 ? '' : 's'}?`,
        body: `${selected.slice(0, 5).map((t) => t.key).join(', ')}${n > 5 ? '…' : ''} and their comments and links are deleted. This can't be undone.`,
        action: n === 1 ? 'Delete ticket' : 'Delete tickets',
        danger: true,
      });
      if (!ok) return;
      await api.bulkTickets({ ids: selectedIds, action: 'delete' });
      clearSelection();
      invalidateTickets(qc);
      void qc.invalidateQueries({ queryKey: ['workspaces'] });
    });
  const labelEntries = (): MenuEntry[] => {
    const onSelected = [...new Set(selected.flatMap((t) => t.labels))].sort();
    return [
      { heading: 'Add' },
      ...allLabels.map((l) => ({ label: l, onSelect: () => bulk('addLabel', l) })),
      {
        input: 'New label…',
        onSubmit: (v: string) => {
          const l = normaliseLabel(v);
          if (l) bulk('addLabel', l);
        },
      },
      ...(onSelected.length > 0
        ? [{ heading: 'Remove' }, ...onSelected.map((l) => ({ label: l, onSelect: () => bulk('removeLabel', l) }))]
        : []),
    ];
  };

  const rowMenu = (t: TicketSummary) => (at: { x: number; y: number }) =>
    openMenu(at, `Ticket ${t.key}`, () => [
      ...(phone && !selecting
        ? [{ label: 'Select', onSelect: () => { setSelection(new Set([t.id])); setSelecting(true); } }, { separator: true } as MenuEntry]
        : []),
      ...ticketMenu({ qc, confirm, ticket: t, onOpen: () => open(t), ...(canStart(t) ? { onStart: () => start(t) } : {}), rank: rankable, onDeleted: () => { if (peekId === t.id) setPeekId(null); } }),
    ]);

  const stageOptions = STAGES.map((s) => ({ value: s, label: STAGE_LABEL[s] }));
  const priorityOptions = PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }));
  const labelOptions = allLabels.map((l) => ({ value: l, label: l }));
  const scopeName = project ? project.name : 'All projects';

  const empty = (): ReactNode =>
    isDefaultFilter(filters) ? (
      <EmptyState action={<Button size="sm" variant="primary" onClick={() => onNewTicket(project?.id ?? null)}>New ticket</Button>}>
        No Draft or Ready tickets{project ? ` in ${project.name}` : ''}.
      </EmptyState>
    ) : (
      <EmptyState action={<Button size="sm" onClick={backlog.resetFilters}>Clear filters</Button>}>No tickets match these filters.</EmptyState>
    );

  const viewOptions = (
    <>
      <Switch size="sm" checked={filters.showClosed} onChange={(showClosed) => backlog.setFilters({ showClosed })} className="text-xs">
        Show closed
      </Switch>
      <Select aria-label="Group by" size="sm" options={GROUPS} value={filters.group} onChange={(group) => backlog.setFilters({ group })} className="w-36" />
      <Select aria-label="Sort by" size="sm" options={SORTS} value={filters.sort} onChange={(sort) => backlog.setFilters({ sort })} className="w-36" />
    </>
  );

  const priorityEntries = () => PRIORITIES.map((p): MenuEntry => ({ label: PRIORITY_LABEL[p], onSelect: () => bulk('priority', p) }));
  // Phones: the moves under one menu, labels and Delete under ⋯, so the bar fits a narrow screen.
  const bulkBar = phone ? (
    <BulkActionBar count={selected.length} onClear={clearSelection}
      className="h-auto min-h-12 border-t border-border pb-[env(safe-area-inset-bottom)]">
      <MenuButton
        label="Move"
        placement="top start"
        entries={() => [
          { label: 'Move to Ready', onSelect: () => bulk('stage', 'ready') },
          { label: 'Move to To do', onSelect: () => bulk('stage', 'todo') },
          { label: 'Move to top', onSelect: () => bulk('top') },
        ]}
      >
        <Button size="sm" compact variant="ghost">Move <ChevronDown aria-hidden /></Button>
      </MenuButton>
      <MenuButton label="Set priority" placement="top start" entries={priorityEntries}>
        <Button size="sm" compact variant="ghost">Priority <ChevronDown aria-hidden /></Button>
      </MenuButton>
      <MenuButton
        label="More bulk actions"
        placement="top end"
        entries={() => [{ submenu: 'Label', entries: labelEntries() }, { separator: true }, { label: 'Delete…', danger: true, onSelect: bulkDelete }]}
      />
    </BulkActionBar>
  ) : (
    <BulkActionBar count={selected.length} noun={selected.length === 1 ? 'ticket' : 'tickets'} onClear={clearSelection}>
      <Button size="sm" compact variant="ghost" onClick={() => bulk('stage', 'ready')}>Move to Ready</Button>
      <Button size="sm" compact variant="ghost" onClick={() => bulk('stage', 'todo')}>Move to To do</Button>
      <MenuButton label="Set priority" placement="bottom start" entries={priorityEntries}>
        <Button size="sm" compact variant="ghost">Priority <ChevronDown aria-hidden /></Button>
      </MenuButton>
      <MenuButton label="Labels" placement="bottom start" entries={labelEntries}>
        <Button size="sm" compact variant="ghost">Label <ChevronDown aria-hidden /></Button>
      </MenuButton>
      <Button size="sm" compact variant="ghost" onClick={() => bulk('top')}>Move to top</Button>
      <Button size="sm" compact variant="ghost" className="text-destructive" onClick={bulkDelete}>Delete…</Button>
    </BulkActionBar>
  );

  // Phones: Import from Jira under the top bar's ⋯ (the list screen only).
  const jira = phone && peekRef === null && !!project && !!settings.data?.jira;
  useTopBarActions(jira ? () => [{ label: 'Import from Jira…', onSelect: () => setImporting(true) }] : null);

  let body: ReactNode;
  if (projects.length === 0) {
    body = <EmptyState>Add a project in Settings to start writing tickets.</EmptyState>;
  } else if (mode === 'board') {
    body = <Board projectId={project?.id ?? null} onPeek={setPeekId} phone={phone} />;
  } else if (pages.isPending) {
    body = <div className="p-3"><Skeleton lines={3} label="Loading tickets" /></div>;
  } else if (pages.isError && !pages.data) {
    body = (
      <EmptyState action={<Button size="sm" onClick={() => void pages.refetch()}>Retry</Button>}>{pages.error.message}</EmptyState>
    );
  } else {
    body = (
      <DataList<TicketSummary>
        // React Aria's GridList can't gain or lose drag and drop in place: a new list when ranking turns on or off.
        key={rankable && !phone ? 'ranked' : 'plain'}
        aria-label="Tickets"
        striped
        {...(sections ? { sections } : { items })}
        dependencies={[phone, rankable, bulkMode]}
        rowHeight={phone ? 56 : 36}
        selectable={bulkMode}
        selectedKeys={selection}
        onSelectionChange={setSelection}
        onAction={setPeekId}
        // Phones rank from the row's menu (Move to top / bottom): no drag on touch.
        {...(rankable && !phone
          ? {
              dragKind: 'ticket',
              onMove: (m) => moveRows(m.ids, m.before ?? m.after!, m.before ? 'before' : 'after'),
              onMoveUp: (id) => { const n = neighbour(id, -1); if (n) moveRows([id], n.id, 'before'); },
              onMoveDown: (id) => { const n = neighbour(id, 1); if (n) moveRows([id], n.id, 'after'); },
            }
          : {})}
        onToggleSection={(id) => setCollapsed((c) => { const n = new Set(c); if (n.has(id)) n.delete(id); else n.add(id); return n; })}
        {...(pages.hasNextPage ? { onEndReached: () => void pages.fetchNextPage() } : {})}
        loadingMore={pages.isFetchingNextPage}
        renderEmpty={empty}
        // Phones: room under the last row for the floating ＋.
        className={cn('min-h-0 flex-1', phone && 'pb-20')}
      >
        {(t) => (
          <DataListRow
            id={t.id}
            textValue={`${t.key} ${t.title}`}
            onMenu={rowMenu(t)}
            trailing={!phone && <RelativeTime time={t.updatedAt} className="w-16 text-right text-meta text-muted-foreground" />}
          >
            <TicketRowContent t={t} phone={phone} />
          </DataListRow>
        )}
      </DataList>
    );
  }

  const ticketPeek = <TicketPeek ticketId={peekId} open={peekRef !== null} missing={peekMissing} onClose={() => setPeekId(null)} onLoaded={peekLoaded} phone={phone} />;
  // On a phone a peeked ticket is its own screen, in place of the list.
  if (phone && peekRef !== null) return <div className="flex min-h-0 flex-1 flex-col">{ticketPeek}</div>;
  if (phone) {
    return (
      <div className="relative flex min-h-0 flex-1 flex-col">
        {projects.length > 0 && <PhoneToolbar backlog={backlog} labels={allLabels} />}
        <div className="flex min-h-0 flex-1 flex-col">{body}</div>
        {mode === 'backlog' && selected.length > 0 ? (
          bulkBar
        ) : (
          projects.length > 0 && <NewTicketFab onClick={() => onNewTicket(project?.id ?? null)} />
        )}
        {importing && project && <ImportJiraDialog projectId={project.id} onClose={() => setImporting(false)} />}
        {menu}
        {ticketPeek}
      </div>
    );
  }
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <ViewHeader
        title="Tickets"
        meta={
          <>
            <SegmentedControl
              aria-label="Tickets view"
              size="sm"
              options={[{ value: 'backlog', label: 'Backlog' }, { value: 'board', label: 'Board' }]}
              value={mode}
              onChange={backlog.setMode}
            />
            <span className="truncate max-sm:hidden">
              {scopeName}
              {mode === 'backlog' && total !== undefined && ` · ${total} ticket${total === 1 ? '' : 's'}`}
            </span>
          </>
        }
        actions={
          <>
            {project && settings.data?.jira && (
              <Button size="sm" onClick={() => setImporting(true)}>Import from Jira…</Button>
            )}
            <Button variant="primary" size="sm" disabled={projects.length === 0} onClick={() => onNewTicket(project?.id ?? null)}>
              <Plus aria-hidden /> <span className="max-sm:sr-only">New ticket</span>
            </Button>
            {importing && project && <ImportJiraDialog projectId={project.id} onClose={() => setImporting(false)} />}
          </>
        }
      />
      {mode === 'backlog' && projects.length > 0 && (
        <>
          <div className="shrink-0 border-b border-border">
            {selected.length > 0 ? bulkBar : (
              <FilterBar
                search={filters.q}
                onSearchChange={(q) => backlog.setFilters({ q })}
                searchLabel="Search tickets"
                trailing={viewOptions}
              >
                <MultiChip<TicketStage>
                  label="Stage"
                  none={filters.showClosed ? undefined : BACKLOG_STAGES.map((s) => STAGE_LABEL[s]).join(', ')}
                  options={stageOptions}
                  selected={filters.stages} onChange={(stages) => backlog.setFilters({ stages })} />
                <MultiChip<Priority> label="Priority" options={priorityOptions} selected={filters.priorities} onChange={(priorities) => backlog.setFilters({ priorities })} />
                <MultiChip<string> label="Label" options={labelOptions} selected={filters.labels} onChange={(l) => backlog.setFilters({ labels: l })} />
                <ChoiceChip label="Blocked" yes="Blocked" no="Unblocked" value={filters.blocked} onChange={(blocked) => backlog.setFilters({ blocked })} />
                <ChoiceChip label="Overlap" yes="With overlap" no="No overlap" value={filters.conflicts} onChange={(conflicts) => backlog.setFilters({ conflicts })} />
              </FilterBar>
            )}
          </div>
          {project && <QuickAdd projectId={project.id} before={rankable ? items[0]?.id : undefined} onDetails={(title) => onNewTicket(project.id, title)} />}
        </>
      )}
      <div className="flex min-h-0 flex-1 flex-col">{body}</div>
      {menu}
      {ticketPeek}
    </div>
  );
}
