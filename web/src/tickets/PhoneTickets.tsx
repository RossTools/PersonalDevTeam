// The Tickets screen's phone parts (PDT-118): the toolbar under the top bar (Backlog | Board, ⌕ search, ⚲ filters),
// the filters' bottom sheet with Save filter, and the floating ＋ for New ticket.
import { useEffect, useRef, useState, type ReactNode } from 'react';
import { Filter, Plus, Search } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import { Button, Checkbox, EmptyState, FilterBar, IconButton, SectionLabel, SegmentedControl, Select, Sheet, Switch } from '@ui';
import type { Priority } from '../api/gen/Priority';
import type { TicketSort } from '../api/gen/TicketSort';
import type { TicketStage } from '../api/gen/TicketStage';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { STAGES, type BacklogFilters, type GroupBy } from './model';
import { BoardViewMenu } from './BoardViewMenu';
import { SaveFilterDialog } from './TicketsList';
import type { BacklogState, TicketsMode } from './useBacklog';

/** How many filters are set (the ⚲ count): search, grouping and sorting don't count. */
function filterCount(f: BacklogFilters): number {
  return [f.stages.length > 0, f.priorities.length > 0, f.labels.length > 0, f.showClosed, f.blocked !== null, f.conflicts !== null]
    .filter(Boolean).length;
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
          <Checks<TicketStage>
            title="Stage"
            options={STAGES.map((s) => ({ value: s, label: STAGE_LABEL[s] }))}
            selected={f.stages}
            onChange={(stages) => setFilters({ stages })}
          />
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
            <Switch checked={f.showClosed} onChange={(showClosed) => setFilters({ showClosed })}>Show closed</Switch>
          </div>
        </div>
      </Sheet>
      {saving && <SaveFilterDialog onSave={backlog.saveFilter} onClose={() => setSaving(false)} />}
    </>
  );
}

/** Under the top bar: Backlog | Board, then (backlog) ⌕ to show the search box and ⚲ for the filters, with a count. */
export function PhoneToolbar({ backlog, labels }: { backlog: BacklogState; labels: string[] }) {
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
