import { useMemo, useRef, useState, type ReactNode } from 'react';
import { Button } from '@ui/Button';
import { Badge } from '@ui/Badge';
import { Chip } from '@ui/Chip';
import { MenuButton } from '@ui/Menu';
import { Switch } from '@ui/Switch';
import { DataList, DataListRow, type DataListMove, type DataListSection } from '@ui/DataList';
import { FilterBar } from '@ui/FilterBar';
import { BulkActionBar } from '@ui/BulkActionBar';
import { Timeline, RelativeTime, type TimelineItem } from '@ui/Timeline';
import { Markdown } from '../../common/Markdown';
import { StageStepper, STAGE_LABEL, type TicketStage } from '../../common/StageStepper';
import { TicketKey } from '../../common/TicketKey';
import { PriorityIcon, PRIORITIES, type Priority } from '../../common/PriorityIcon';

function Demo({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <div className="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{label}</div>
      {children}
    </div>
  );
}

const NOW = Date.now();
const MIN = 60_000;

// --- Small pieces ----------------------------------------------------------------------------------------------

function Pieces() {
  return (
    <div className="flex flex-col gap-4">
      <Demo label="TicketKey · plain, button, link">
        <div className="flex flex-wrap items-center gap-2">
          <TicketKey ticketKey="PDT-42" />
          <TicketKey ticketKey="PDT-43" title="Open the ticket" onClick={() => {}} />
          <TicketKey ticketKey="PDT-44" href="#PDT-44" />
        </div>
      </Demo>
      <Demo label="PriorityIcon · icon only, with label">
        <div className="flex flex-wrap items-center gap-4">
          {PRIORITIES.map((p) => <PriorityIcon key={p} priority={p} />)}
          {PRIORITIES.map((p) => <PriorityIcon key={p} priority={p} showLabel />)}
        </div>
      </Demo>
      <Demo label="StageStepper · every stage, compact">
        <div className="flex flex-col gap-2">
          {(['draft', 'ready', 'in_progress', 'done', 'wont_do'] as TicketStage[]).map((s) => <StageStepper key={s} stage={s} />)}
          <div className="flex flex-wrap gap-4">
            <StageStepper stage="ready" compact />
            <StageStepper stage="wont_do" compact />
          </div>
        </div>
      </Demo>
    </div>
  );
}

// --- Markdown and timeline ---------------------------------------------------------------------------------------

const BODY = `## Goal
Tickets exist: write them, rank them, **search** and filter a backlog that stays fast at 1,000+ tickets.

- [x] Migration and store
- [ ] API (\`GET /api/tickets\`)
- [ ] Backlog view

| Stage | Count |
|---|---|
| Draft | 12 |
| Ready | 5 |

~~Board in 4a~~ — the board arrives with Start. See [the plan](https://example.com/plan).

\`\`\`ts
const page = await api.tickets({ project: 'PDT', cursor });
\`\`\`

> Raw HTML shows as text: <script>alert(1)</script>
`;

const EVENTS: TimelineItem[] = [
  { id: 'e1', actor: 'user', time: NOW - 3 * 24 * 60 * MIN, title: 'Created ticket' },
  { id: 'e2', actor: 'system', time: NOW - 5 * 60 * MIN, title: 'Moved to Ready' },
  { id: 'e3', actor: 'user', time: NOW - 42 * MIN, title: 'Comment', body: <Markdown>{'Split the API into **two** PRs?'}</Markdown> },
  { id: 'e4', actor: 'reviewer', time: NOW - 20_000, title: 'Linked PDT-12 (blocks)' },
];

function Writing() {
  return (
    <div className="flex flex-wrap gap-6">
      <Demo label="Markdown · ticket body">
        <div className="w-[480px] max-w-full rounded-md border border-border bg-card p-3"><Markdown>{BODY}</Markdown></div>
      </Demo>
      <div className="flex flex-col gap-4">
        <Demo label="Timeline">
          <div className="w-80 max-w-full rounded-md border border-border bg-card p-3"><Timeline items={EVENTS} /></div>
        </Demo>
        <Demo label="Timeline · empty">
          <div className="w-80 max-w-full rounded-md border border-border bg-card p-3"><Timeline items={[]} /></div>
        </Demo>
      </div>
    </div>
  );
}

// --- Backlog: FilterBar / BulkActionBar over a 2,000-row DataList -----------------------------------------------------

interface Ticket {
  id: string;
  key: string;
  title: string;
  stage: TicketStage;
  priority: Priority;
  labels: string[];
  updated: number;
}

const WORDS = ['fix', 'login', 'redirect', 'tickets', 'schema', 'board', 'drag', 'rank', 'palette', 'terminal', 'resize', 'update', 'remote', 'tailscale', 'search', 'filter', 'sheet', 'peek'];
const STAGES: TicketStage[] = ['draft', 'ready', 'in_progress', 'done'];
const LABELS = ['ui', 'api', 'bug', 'perf', 'docs'];

function makeTickets(n: number, from = 0): Ticket[] {
  return Array.from({ length: n }, (_, j) => {
    const i = from + j;
    return {
      id: `t${i}`,
      key: `PDT-${i + 1}`,
      title: `${WORDS[i % WORDS.length]} ${WORDS[(i * 7) % WORDS.length]} ${WORDS[(i * 13) % WORDS.length]}`,
      stage: STAGES[(i * 3) % 4],
      priority: PRIORITIES[(i * 2 + Math.floor(i / 5)) % 5],
      labels: i % 3 ? [LABELS[i % 5]] : [],
      updated: NOW - i * 17 * MIN,
    };
  });
}

function move<T extends { id: string }>(list: T[], ids: string[], target: string, after: boolean): T[] {
  const moving = list.filter((t) => ids.includes(t.id));
  const rest = list.filter((t) => !ids.includes(t.id));
  const at = rest.findIndex((t) => t.id === target) + (after ? 1 : 0);
  return [...rest.slice(0, at), ...moving, ...rest.slice(at)];
}

function Backlog() {
  const [tickets, setTickets] = useState(() => makeTickets(2000));
  const [q, setQ] = useState('');
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [grouped, setGrouped] = useState(false);
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [opened, setOpened] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const loads = useRef(0);

  const shown = useMemo(() => {
    const needle = q.trim().toLowerCase();
    return needle ? tickets.filter((t) => t.title.includes(needle) || t.key.toLowerCase().includes(needle)) : tickets;
  }, [tickets, q]);
  const sections = useMemo<DataListSection<Ticket>[] | undefined>(
    () =>
      grouped
        ? STAGES.map((s) => ({ id: s, title: STAGE_LABEL[s], collapsed: collapsed.has(s), items: shown.filter((t) => t.stage === s) }))
        : undefined,
    [grouped, collapsed, shown],
  );

  const onMove = ({ ids, before, after }: DataListMove) => setTickets((l) => move(l, ids, (after ?? before)!, !!after));
  const nudge = (id: string, by: -1 | 1) =>
    setTickets((l) => {
      const i = l.findIndex((t) => t.id === id);
      const j = i + by;
      if (i < 0 || j < 0 || j >= l.length) return l;
      const next = [...l];
      [next[i], next[j]] = [next[j], next[i]];
      return next;
    });
  const loadMore = () => {
    if (loads.current >= 3) return;
    loads.current += 1;
    setLoading(true);
    setTimeout(() => {
      setTickets((l) => [...l, ...makeTickets(200, l.length)]);
      setLoading(false);
    }, 600);
  };

  const row = (t: Ticket) => (
    <DataListRow
      id={t.id}
      textValue={`${t.key} ${t.title}`}
      onMenu={() => setOpened(`menu for ${t.key}`)}
      trailing={<RelativeTime time={t.updated} className="text-meta text-muted-foreground" />}
    >
      <TicketKey ticketKey={t.key} className="w-16 justify-center" />
      <span className="min-w-0 flex-1 truncate">{t.title}</span>
      <Badge className="hidden sm:inline-flex">{STAGE_LABEL[t.stage]}</Badge>
      <PriorityIcon priority={t.priority} />
      <span className="hidden w-12 md:flex">{t.labels.map((l) => <Chip key={l}>{l}</Chip>)}</span>
    </DataListRow>
  );

  return (
    <div className="flex flex-col gap-1">
      <div className="flex h-[480px] flex-col overflow-hidden rounded-md border border-border bg-background">
        {selected.size > 0 ? (
          <BulkActionBar count={selected.size} onClear={() => setSelected(new Set())}>
            <Button size="sm" compact onClick={() => setTickets((l) => l.map((t) => (selected.has(t.id) ? { ...t, stage: 'ready' } : t)))}>
              Move to Ready
            </Button>
            <MenuButton label="Priority" entries={() => PRIORITIES.map((p) => ({ label: p, onSelect: () => {} }))}>
              <Button size="sm" compact>Priority ▾</Button>
            </MenuButton>
            <Button size="sm" compact variant="danger" onClick={() => {
              setTickets((l) => l.filter((t) => !selected.has(t.id)));
              setSelected(new Set());
            }}>
              Delete…
            </Button>
          </BulkActionBar>
        ) : (
          <FilterBar search={q} onSearchChange={setQ} searchLabel="Search tickets"
            trailing={<label className="flex items-center gap-1.5 text-xs"><Switch aria-label="Group by stage" checked={grouped} onChange={setGrouped} />Group</label>}>
            <Button size="sm" compact variant="secondary">Stage ▾</Button>
            <Button size="sm" compact variant="secondary">Priority ▾</Button>
          </FilterBar>
        )}
        <div className="border-t border-border" />
        <DataList
          aria-label="Tickets"
          striped
          items={sections ? undefined : shown}
          sections={sections}
          rowHeight={36}
          selectable
          selectedKeys={selected}
          onSelectionChange={setSelected}
          onAction={(id) => setOpened(tickets.find((t) => t.id === id)?.key ?? id)}
          dragKind="ticket"
          onMove={onMove}
          onMoveUp={(id) => nudge(id, -1)}
          onMoveDown={(id) => nudge(id, 1)}
          onToggleSection={(id) => setCollapsed((c) => { const n = new Set(c); if (n.has(id)) n.delete(id); else n.add(id); return n; })}
          onEndReached={grouped || q ? undefined : loadMore}
          loadingMore={loading}
          renderEmpty={() => <p className="p-6 text-center text-xs text-muted-foreground">No tickets match “{q}”.</p>}
          className="min-h-0 flex-1"
        >
          {row}
        </DataList>
      </div>
      <p className="text-meta text-muted-foreground">
        {tickets.length} rows · drag the handle or Alt+↑/↓ to rank · Shift-click, Ctrl/⌘+A to select · / to search · scroll
        to the end loads 200 more (3×) · last: {opened ?? 'nothing'}
      </p>
    </div>
  );
}

export const section = {
  title: 'Tickets',
  order: 45,
  render: () => (
    <div className="flex flex-col gap-6">
      <Pieces />
      <Writing />
      <Demo label="DataList · FilterBar ⇄ BulkActionBar (2,000 rows, virtualised)"><Backlog /></Demo>
    </div>
  ),
};
