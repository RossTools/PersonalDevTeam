// The Tickets screen's phone parts (PDT-118): the toolbar under the top bar (Backlog | Board, ⌕ search, ⚲ filters,
// then a line saying what the backlog shows), the filters' bottom sheet with Save filter, and the floating ＋ for New
// ticket.
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ChevronRight, Filter, Plus, Search } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, Checkbox, EmptyState, FilterBar, IconButton, SectionLabel, SegmentedControl, Select, Sheet } from '@ui';
import type { Priority } from '../api/gen/Priority';
import type { TicketSort } from '../api/gen/TicketSort';
import type { TicketStage } from '../api/gen/TicketStage';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { BACKLOG_STAGES, filterCount, sameStages, STAGES, stagesText, type BacklogFilters, type GroupBy } from './model';
import { plural } from '../common/text';
import { StagePresets } from './StagePresets';
import { BoardViewMenu } from './BoardViewMenu';
import { SaveFilterDialog } from './TicketsList';
import type { BacklogState, TicketsMode } from './useBacklog';

/** What the backlog shows, in a line: "Draft, Ready · 6 tickets", "All open · High · 2 tickets". */
function summary(f: BacklogFilters, total: number | undefined): string {
  const others = filterCount(f) - (sameStages(f.stages, BACKLOG_STAGES) ? 0 : 1);
  return [
    stagesText(f.stages),
    others > 0 && `${plural(others, 'more filter')}`,
    total !== undefined && plural(total, 'ticket'),
  ].filter(Boolean).join(' · ');
}

type Tri = 'any' | 'yes' | 'no';
const toTri = (v: boolean | null): Tri => (v === null ? 'any' : v ? 'yes' : 'no');
const fromTri = (v: Tri): boolean | null => (v === 'any' ? null : v === 'yes');

const GROUPS: { value: GroupBy; label: string }[] = [
  { value: 'none', label: 'No grouping' },
  { value: 'stage', label: 'Stage' },
  { value: 'priority', label: 'Priority' },
  { value: 'label', label: 'Label' },
];
const SORTS: { value: TicketSort; label: string }[] = [
  { value: 'rank', label: 'Rank' },
  { value: 'priority', label: 'Priority' },
  { value: 'updated', label: 'Updated' },
  { value: 'created', label: 'Created' },
];

function Checks<V extends string>({ title, options, selected, onChange }: {
  title: string;
  options: { value: V; label: string }[];
  selected: V[];
  onChange: (v: V[]) => void;
}) {
  return (
    <fieldset className="flex flex-col gap-1">
      <SectionLabel as="legend" className="pb-1">{title}</SectionLabel>
      {options.length === 0 && <EmptyState size="inline">None yet.</EmptyState>}
      <div className="grid grid-cols-2 gap-x-3">
        {options.map((o) => (
          <Checkbox
            key={o.value}
            checked={selected.includes(o.value)}
            onChange={(on) => onChange(on ? [...selected, o.value] : selected.filter((v) => v !== o.value))}
          >
            {o.label}
          </Checkbox>
        ))}
      </div>
    </fieldset>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center justify-between gap-3">
      <span className="text-sm">{label}</span>
      {children}
    </div>
  );
}

/** The backlog's filters in a bottom sheet: stage, priority, label, blocked, overlap, closed, grouping, sorting. */
function FilterSheet({ backlog, labels, open, onOpenChange }: {
  backlog: BacklogState;
  /** Every label in the scope. */
  labels: string[];
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const { filters: f, setFilters } = backlog;
  const [saving, setSaving] = useState(false);
  return (
    <>
      <Sheet
        side="bottom"
        title="Filters"
        open={open}
        onOpenChange={onOpenChange}
        className="max-h-[85dvh]"
        footer={
          <>
            <Button onClick={backlog.resetFilters} disabled={filterCount(f) === 0}>Clear</Button>
            <Button onClick={() => { onOpenChange(false); setSaving(true); }}>Save filter…</Button>
            <Button variant="primary" onClick={() => onOpenChange(false)}>Done</Button>
          </>
        }
      >
        <div className="flex flex-col gap-4 pb-2">
          <div className="flex flex-col gap-1">
            <Checks<TicketStage>
              title="Stage"
              options={STAGES.map((s) => ({ value: s, label: STAGE_LABEL[s] }))}
              selected={f.stages}
              onChange={(stages) => setFilters({ stages })}
            />
            <StagePresets stages={f.stages} onChange={(stages) => setFilters({ stages })} />
          </div>
          <Checks<Priority>
            title="Priority"
            options={PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }))}
            selected={f.priorities}
            onChange={(priorities) => setFilters({ priorities })}
          />
          <Checks<string> title="Label" options={labels.map((l) => ({ value: l, label: l }))} selected={f.labels} onChange={(l) => setFilters({ labels: l })} />
          <div className="flex flex-col gap-2">
            <Row label="Blocked">
              <Select<Tri>
                aria-label="Blocked"
                className="w-40"
                options={[{ value: 'any', label: 'Any' }, { value: 'yes', label: 'Blocked' }, { value: 'no', label: 'Unblocked' }]}
                value={toTri(f.blocked)}
                onChange={(v) => setFilters({ blocked: fromTri(v) })}
              />
            </Row>
            <Row label="Overlap">
              <Select<Tri>
                aria-label="Overlap"
                className="w-40"
                options={[{ value: 'any', label: 'Any' }, { value: 'yes', label: 'With overlap' }, { value: 'no', label: 'No overlap' }]}
                value={toTri(f.conflicts)}
                onChange={(v) => setFilters({ conflicts: fromTri(v) })}
              />
            </Row>
            <Row label="Group by">
              <Select<GroupBy> aria-label="Group by" className="w-40" options={GROUPS} value={f.group} onChange={(group) => setFilters({ group })} />
            </Row>
            <Row label="Sort by">
              <Select<TicketSort> aria-label="Sort by" className="w-40" options={SORTS} value={f.sort} onChange={(sort) => setFilters({ sort })} />
            </Row>
          </div>
        </div>
      </Sheet>
      {saving && <SaveFilterDialog onSave={backlog.saveFilter} onClose={() => setSaving(false)} />}
    </>
  );
}

/**
 * Under the top bar: Backlog | Board, then (backlog) ⌕ to show the search box and ⚲ for the filters, with a count;
 * below them a line saying what the backlog shows, which opens the filters too.
 */
export function PhoneToolbar({ backlog, labels, total }: {
  backlog: BacklogState;
  labels: string[];
  /** Tickets matching the filters, once loaded. */
  total?: number | undefined;
}) {
  const { mode, filters } = backlog;
  const [searching, setSearching] = useState(false);
  const [filtering, setFiltering] = useState(false);
  const search = useRef<HTMLInputElement>(null);
  const showSearch = mode === 'backlog' && (searching || filters.q !== '');
  useEffect(() => {
    if (searching) search.current?.focus();
  }, [searching]);
  const n = filterCount(filters);
  return (
    <div className="shrink-0 border-b border-border">
      <div className="flex items-center gap-1 px-3 py-1.5">
        <SegmentedControl<TicketsMode>
          aria-label="Tickets view"
          size="sm"
          options={[{ value: 'backlog', label: 'Backlog' }, { value: 'board', label: 'Board' }]}
          value={mode}
          onChange={backlog.setMode}
        />
        <span className="flex-1" />
        {mode === 'board' && backlog.projectId && <BoardViewMenu projectId={backlog.projectId} compact />}
        {mode === 'backlog' && (
          <>
            <IconButton
              label="Search tickets"
              noTooltip
              pressed={showSearch}
              onClick={() => {
                if (showSearch) { backlog.setFilters({ q: '' }); setSearching(false); } else setSearching(true);
              }}
            >
              <Search aria-hidden />
            </IconButton>
            <Button
              variant={n > 0 ? 'secondary' : 'ghost'}
              aria-label={n > 0 ? `Filters (${n})` : 'Filters'}
              aria-haspopup="dialog"
              onClick={() => setFiltering(true)}
            >
              <Filter aria-hidden />
              {n > 0 && <span className="tabular-nums">{n}</span>}
            </Button>
          </>
        )}
      </div>
      {mode === 'backlog' && (
        <div className="px-1.5 pb-1">
          <Button
            variant="ghost"
            size="sm"
            aria-haspopup="dialog"
            aria-label={`Showing ${summary(filters, total)}. Edit filters`}
            onClick={() => setFiltering(true)}
            className="w-full justify-between font-normal text-muted-foreground"
          >
            <span className="min-w-0 truncate">{summary(filters, total)}</span>
            <span className="flex shrink-0 items-center text-foreground">Edit <ChevronRight aria-hidden /></span>
          </Button>
        </div>
      )}
      {showSearch && (
        <FilterBar
          search={filters.q}
          onSearchChange={(q) => backlog.setFilters({ q })}
          searchLabel="Search tickets"
          searchRef={search}
          slashToFocus={false}
          className="pt-0"
        />
      )}
      <FilterSheet backlog={backlog} labels={labels} open={filtering} onOpenChange={setFiltering} />
    </div>
  );
}

/** The app's only floating button: New ticket, above the list's bottom right corner. */
export function NewTicketFab({ onClick, disabled }: { onClick: () => void; disabled?: boolean }) {
  return (
    <IconButton
      label="New ticket"
      variant="primary"
      noTooltip
      disabled={disabled}
      onClick={onClick}
      className={cn(
        'absolute right-4 bottom-[calc(1rem+env(safe-area-inset-bottom))] size-14 rounded-full shadow-popover',
        'pointer-coarse:size-14 [&_svg]:size-6',
      )}
    >
      <Plus aria-hidden />
    </IconButton>
  );
}
