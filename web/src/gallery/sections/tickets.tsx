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
import { PartsStepper, type PartState, type PartsProgress } from '../../common/PartsStepper';

const progressOf = (segments: PartState[]): PartsProgress => {
  const n = (s: PartState) => segments.filter((x) => x === s).length;
  return { total: segments.length, done: n('done'), merged: n('merged'), review: n('review'), progress: n('progress'), todo: n('todo'), backlog: n('backlog'), segments };
};
import { TicketKey } from '../../common/TicketKey';
import { PriorityIcon, PRIORITIES, type Priority } from '../../common/PriorityIcon';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { agentState } from '../../common/AgentState';
import type { Project } from '../../api/gen/Project';
import type { PullRequestStatus } from '../../api/gen/PullRequestStatus';
import type { Ticket as FullTicket } from '../../api/gen/Ticket';
import type { TicketSummary } from '../../api/gen/TicketSummary';
import { BoardCard } from '../../tickets/Board';
import { StartDialog } from '../../tickets/StartDialog';
import { PrDetails } from '../../tickets/PullRequest';

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
      <Demo label="PartsStepper · every part state, with and without the count">
        <div className="flex max-w-80 flex-col gap-3">
          <PartsStepper parts={progressOf(['done', 'merged', 'review', 'progress', 'todo', 'backlog'])} showCount />
          <PartsStepper parts={progressOf(['done', 'done', 'review', 'backlog'])} showCount />
          <PartsStepper parts={progressOf(['done', 'done', 'done'])} />
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
const STAGES: TicketStage[] = ['draft', 'ready', 'todo', 'in_progress', 'done'];
const LABELS = ['ui', 'api', 'bug', 'perf', 'docs'];

function makeTickets(n: number, from = 0): Ticket[] {
  return Array.from({ length: n }, (_, j) => {
    const i = from + j;
    return {
      id: `t${i}`,
      key: `PDT-${i + 1}`,
      title: `${WORDS[i % WORDS.length]} ${WORDS[(i * 7) % WORDS.length]} ${WORDS[(i * 13) % WORDS.length]}`,
      stage: STAGES[(i * 3) % STAGES.length],
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

// --- Board cards and the Start dialog ----------------------------------------------------------------------------

const card = (n: number, over: Partial<TicketSummary> = {}): TicketSummary => ({
  id: `g${n}`, projectId: 'gp', key: `PDT-${n}`, number: n, title: 'Board cards show the title on up to two lines, then cut it off with an ellipsis',
  stage: 'ready', priority: 'none', labels: [], areas: [], rank: 'a', waiting: false, clearToStart: false, blockedBy: 0, attempt: null, overlap: 'none', overlapWith: [],
  createdAt: NOW, updatedAt: NOW, closedAt: null, parent: null, parts: null, pipelineStage: null, agent: null, autoStartAt: null, external: [], ...over,
});
const attempt = { id: 'ga', branch: 'pdt/PDT-44-board-and-start', baseBranch: 'main', worktreeId: 'gw', state: 'active' as const, createdAt: NOW, endedAt: null, lastHead: null, workspaceId: null, ci: null, pr: null };
const galleryPr = (over: Partial<PullRequestStatus> = {}): PullRequestStatus => ({
  number: 12, url: 'https://github.com/o/r/pull/12', title: 'Board and Start', state: 'open', draft: false, baseBranch: 'main',
  headSha: 'abc1234', mergeable: 'mergeable', behind: false, autoMerge: true, inMergeQueue: false, queuePosition: null,
  ci: { state: 'running', url: 'https://github.com/o/r/actions/runs/1', runs: [] }, queueCi: null, checkedAt: NOW, error: null, local: false, ...over,
});
const failedCi = { state: 'failed' as const, url: 'https://github.com/o/r/actions/runs/2',
  runs: [{ id: 2, attempt: 1, workflow: 'ci', state: 'failed' as const, url: 'https://github.com/o/r/actions/runs/2', failedJobs: ['test (linux)', 'web (linux)'] }] };
const noMenu = () => [{ label: 'Open', onSelect: () => {} }, { label: 'Start…', onSelect: () => {} }];

function Cards() {
  return (
    <div className="grid gap-3 sm:grid-cols-3">
      <div className="flex flex-col gap-2 rounded-md bg-surface-2 p-2">
        <Demo label="Ready · plain, labelled, blocked">
          <ul className="flex flex-col gap-2">
            <BoardCard ticket={card(41, { title: 'Short title' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(42, { priority: 'high', labels: ['web', 'tickets'] })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(43, { priority: 'urgent', blockedBy: 2, title: 'Waits for the schema' })} onOpen={() => {}} entries={noMenu} draggable />
          </ul>
        </Demo>
        <Demo label="Overlap, conflict, waiting, clear to start">
          <ul className="flex flex-col gap-2">
            <BoardCard ticket={card(48, { overlap: 'overlap', overlapWith: ['PDT-44'], title: 'Shares web/ui with PDT-44' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(49, { overlap: 'conflict', overlapWith: ['PDT-45', 'PDT-44'], waiting: true, title: 'Another migration' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(50, { waiting: true, clearToStart: true, title: 'Nothing in the way any more' })} onOpen={() => {}} entries={noMenu} draggable />
          </ul>
        </Demo>
      </div>
      <div className="flex flex-col gap-2 rounded-md bg-surface-2 p-2">
        <Demo label="In progress · working, needs you (unseen), no session">
          <ul className="flex flex-col gap-2">
            <BoardCard ticket={card(44, { stage: 'in_progress', attempt, title: 'Board and Start' })}
              activity={agentState({ kind: 'agent_tui', status: 'running', activity: 'working' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(45, { stage: 'in_progress', attempt: { ...attempt, branch: 'pdt/PDT-45-merge-loop' }, priority: 'medium' })}
              activity={agentState({ kind: 'agent_tui', status: 'running', activity: 'needs_you', unseen: true })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(46, { stage: 'in_progress', attempt: { ...attempt, branch: 'feat/no-agent' }, title: 'Its session ended' })} onOpen={() => {}} entries={noMenu} draggable />
          </ul>
        </Demo>
      </div>
      <div className="flex flex-col gap-2 rounded-md bg-surface-2 p-2">
        <Demo label="In progress · pull request: CI running, CI failed, conflict, in the merge queue, passed">
          <ul className="flex flex-col gap-2">
            <BoardCard ticket={card(51, { stage: 'in_progress', attempt: { ...attempt, branch: 'pdt/PDT-51-ci-running', pr: galleryPr() }, title: 'CI running' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(52, { stage: 'in_progress', attempt: { ...attempt, branch: 'pdt/PDT-52-ci-failed', pr: galleryPr({ number: 13, ci: failedCi }) }, title: 'CI failed' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(53, { stage: 'in_progress', attempt: { ...attempt, branch: 'pdt/PDT-53-conflict', pr: galleryPr({ number: 14, mergeable: 'conflicting' }) }, title: 'Conflicts with main' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(54, { stage: 'in_progress', attempt: { ...attempt, branch: 'pdt/PDT-54-queued', pr: galleryPr({ number: 15, inMergeQueue: true, queuePosition: 2, ci: { ...failedCi, state: 'passed', runs: [] } }) }, title: 'In the merge queue' })} onOpen={() => {}} entries={noMenu} draggable />
            <BoardCard ticket={card(55, { stage: 'in_progress', attempt: { ...attempt, branch: 'pdt/PDT-55-passed', pr: galleryPr({ number: 16, ci: { ...failedCi, state: 'passed', runs: [] } }) }, title: 'CI passed, auto-merge on' })} onOpen={() => {}} entries={noMenu} draggable />
          </ul>
        </Demo>
        <Demo label="Pull request details (ticket Attempt section)">
          <div className="flex flex-col gap-3 rounded-md bg-card p-2">
            <PrDetails pr={galleryPr({ ci: failedCi, behind: true })} />
            <PrDetails pr={galleryPr({ mergeable: 'conflicting', autoMerge: false, ci: null })} />
            <PrDetails pr={galleryPr({ inMergeQueue: true, queuePosition: 1, ci: { ...failedCi, state: 'passed', runs: [] }, queueCi: { state: 'running', url: 'https://github.com/o/r/actions/runs/3', runs: [] } })} />
            <PrDetails pr={galleryPr({ autoMerge: false, ci: { ...failedCi, state: 'passed', runs: [] }, queueCi: failedCi })} />
            <PrDetails pr={galleryPr({ state: 'merged' })} />
          </div>
        </Demo>
      </div>
      <div className="flex flex-col gap-2 rounded-md bg-surface-2 p-2">
        <Demo label="Done · merged attempt">
          <ul className="flex flex-col gap-2">
            <BoardCard ticket={card(40, { stage: 'done', attempt: { ...attempt, state: 'merged', branch: 'pdt/PDT-40-tickets-core' }, title: 'Tickets core' })} onOpen={() => {}} entries={noMenu} draggable />
          </ul>
        </Demo>
      </div>
    </div>
  );
}

const startTicket = (over: Partial<FullTicket> = {}): FullTicket => ({
  ...card(47, { title: 'Start a ticket from the board' }), description: '', links: [], attempts: [], defaultBranch: 'pdt/PDT-47-start-a-ticket', sessions: [], overlaps: [], partList: [], ...over,
});
const START_STATES: { label: string; ticket: FullTicket }[] = [
  { label: 'Ready', ticket: startTicket() },
  {
    label: 'Blocked (Start anyway)',
    ticket: startTicket({ links: [{ kind: 'blocks', direction: 'incoming', id: 'gx', key: 'PDT-12', title: 'Tickets schema', stage: 'in_progress' }] }),
  },
  {
    label: 'Conflict and overlap (Wait / Start anyway)',
    ticket: startTicket({
      overlaps: [
        { ticketId: 'g45', key: 'PDT-45', title: 'Merge loop', stage: 'in_progress', level: 'conflict', areas: ['migrations'],
          files: ['crates/pdt-store/migrations/0010_attempts.sql'], filesTotal: 1 },
        { ticketId: 'g44', key: 'PDT-44', title: 'Board and Start', stage: 'in_progress', level: 'overlap', areas: ['web/ui', 'old-area'],
          files: [], filesTotal: 0 },
      ],
    }),
  },
  { label: 'Waiting', ticket: startTicket({ waiting: true }) },
  { label: 'Active attempt', ticket: startTicket({ stage: 'in_progress', attempt }) },
];
const galleryProject: Project = { id: 'gp', name: 'PersonalDevTeam', key: 'PDT', repoPath: '/r', baseBranch: 'main', createdAt: NOW, githubRepo: null, kind: 'repo' };

/** The dialog over seeded data (there is no API here: Start shows the error state). */
function StartDemo() {
  const [open, setOpen] = useState<FullTicket | null>(null);
  const qc = useMemo(() => new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } }), []);
  const show = (t: FullTicket) => {
    qc.setQueryData(['ticket', t.id], t);
    qc.setQueryData(['projects'], [galleryProject]);
    qc.setQueryData(['areas', galleryProject.id], {
      areas: [{ name: 'migrations', paths: ['crates/pdt-store/migrations/**'], exclusive: true }, { name: 'web/ui', paths: ['web/ui/**'], exclusive: false }],
      error: null,
    });
    setOpen(t);
  };
  return (
    <QueryClientProvider client={qc}>
      <div className="flex flex-wrap gap-2">
        {START_STATES.map((s) => <Button key={s.label} size="sm" onClick={() => show({ ...s.ticket, id: `start-${s.label}` })}>{s.label}</Button>)}
      </div>
      {open && <StartDialog key={open.id} ticketId={open.id} onClose={() => setOpen(null)} onStarted={() => setOpen(null)} />}
    </QueryClientProvider>
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
      <Demo label="Board cards (drag, right-click, ⋯)"><Cards /></Demo>
      <Demo label="Start dialog · open it in each state"><StartDemo /></Demo>
    </div>
  ),
};
