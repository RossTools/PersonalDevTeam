// The pieces of a ticket's Overview, shared by the ticket tab (TicketView) and the backlog's peek sheet.
import { useState, type ReactNode } from 'react';
import { useInfiniteQuery, useQuery, useQueryClient } from '@tanstack/react-query';
import { ArrowRight, Clock, Link2, MessageSquare, Pencil, TriangleAlert } from 'lucide-react';
import { cn } from '@ui/lib/utils';
import {
  Alert, Badge, Button, Chip, Dialog, IconButton, MenuButton, SegmentedControl, Select, Textarea, Timeline,
  type TimelineItem,
} from '@ui';
import { api } from '../api/client';
import type { LinkedTicket } from '../api/gen/LinkedTicket';
import type { Priority } from '../api/gen/Priority';
import type { Ticket } from '../api/gen/Ticket';
import type { TicketEvent } from '../api/gen/TicketEvent';
import type { TicketOverlap } from '../api/gen/TicketOverlap';
import type { TicketStage } from '../api/gen/TicketStage';
import { Composer } from '../common/Composer';
import { InlineRename } from '../common/InlineRename';
import { Markdown } from '../common/Markdown';
import { PRIORITIES, PRIORITY_LABEL } from '../common/PriorityIcon';
import { STAGE_LABEL } from '../common/StageStepper';
import { TicketKey } from '../common/TicketKey';
import { actionError, useAction } from '../common/useAction';
import { patchTicket, useOpenTicket } from './actions';
import { eventTitle, LINK_ROLE_LABEL, linkCall, linkRole, STAGES, type LinkRole } from './model';
import { invalidateTickets, POLL, ticketKeys, useLabels, useProjectAreas } from './queries';
import { AreaChips, AreasEditor } from './Areas';
import { LabelsEditor } from './LabelsEditor';
import { TicketPicker } from './TicketPicker';

const sectionHeading = 'text-2xs font-medium tracking-wider text-muted-foreground uppercase';

/** A titled block of the Overview ("Details", "Links"). */
export function OverviewSection({ title, actions, children, className }: { title: string; actions?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section aria-label={title} className={cn('flex flex-col gap-2', className)}>
      <div className="flex min-h-6 items-center gap-2">
        <h3 className={cn(sectionHeading, 'flex-1')}>{title}</h3>
        {actions}
      </div>
      {children}
    </section>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Title

/** The title with a pencil: edits in place (Enter or leaving the field saves, Escape cancels). */
export function TicketTitle({ ticket, className }: { ticket: Pick<Ticket, 'id' | 'title'>; className?: string }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [save] = useAction((title: string) => patchTicket(qc, ticket.id, { title }));
  if (editing) {
    return (
      <InlineRename
        label="Title"
        initial={ticket.title}
        maxLength={200}
        size="sm"
        className={cn('w-full min-w-48', className)}
        onCommit={(t) => { setEditing(false); void save(t); }}
        onCancel={() => setEditing(false)}
      />
    );
  }
  return (
    <span className={cn('group/title flex min-w-0 items-center gap-1', className)}>
      <span className="min-w-0 truncate" title={ticket.title} onDoubleClick={() => setEditing(true)}>{ticket.title}</span>
      <IconButton label="Edit title" size="sm" noTooltip className="size-5 shrink-0" onClick={() => setEditing(true)}>
        <Pencil aria-hidden />
      </IconButton>
    </span>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Description

/** The description as Markdown; Edit switches to a text box with a Preview toggle, saved with Save. */
export function TicketDescription({ ticket }: { ticket: Pick<Ticket, 'id' | 'description'> }) {
  const qc = useQueryClient();
  const [draft, setDraft] = useState<string | null>(null);
  const [mode, setMode] = useState<'write' | 'preview'>('write');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const editing = draft !== null;
  const dirty = editing && draft !== ticket.description;
  const save = async () => {
    if (!dirty) return;
    setPending(true);
    setError(null);
    try {
      await patchTicket(qc, ticket.id, { description: draft });
      setDraft(null);
    } catch (e) {
      setError(actionError(e));
    } finally {
      setPending(false);
    }
  };
  return (
    <OverviewSection
      title="Description"
      actions={!editing && (
        <Button variant="ghost" size="sm" compact onClick={() => { setDraft(ticket.description); setMode('write'); }}>
          <Pencil aria-hidden /> Edit
        </Button>
      )}
    >
      {editing ? (
        <form className="flex flex-col gap-2" onSubmit={(e) => { e.preventDefault(); void save(); }}>
          <SegmentedControl
            aria-label="Description editor"
            size="sm"
            className="self-start"
            options={[{ value: 'write', label: 'Write' }, { value: 'preview', label: 'Preview' }]}
            value={mode}
            onChange={setMode}
          />
          {mode === 'write' ? (
            <Textarea
              aria-label="Description"
              autoFocus
              rows={10}
              value={draft}
              placeholder="Markdown: what and why, acceptance criteria…"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) { e.preventDefault(); void save(); }
                if (e.key === 'Escape' && !dirty) { e.preventDefault(); setDraft(null); }
              }}
              className="min-h-40 resize-y"
            />
          ) : (
            <div className="min-h-40 rounded-md border border-border p-2">
              {draft.trim() ? <Markdown>{draft}</Markdown> : <p className="text-xs text-muted-foreground">Nothing to preview.</p>}
            </div>
          )}
          {error && <Alert tone="error">{error}</Alert>}
          <div className="flex items-center justify-end gap-2">
            <span className="mr-auto text-meta text-muted-foreground pointer-coarse:hidden">Markdown. ⌘/Ctrl+Enter saves.</span>
            <Button variant="ghost" onClick={() => { setDraft(null); setError(null); }}>Cancel</Button>
            <Button type="submit" variant="primary" pending={pending} disabled={!dirty}>Save</Button>
          </div>
        </form>
      ) : ticket.description.trim() ? (
        <Markdown>{ticket.description}</Markdown>
      ) : (
        <p className="text-xs text-muted-foreground">No description.</p>
      )}
    </OverviewSection>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Details

const stageOptions = STAGES.map((s) => ({ value: s, label: STAGE_LABEL[s] }));
const priorityOptions = PRIORITIES.map((p) => ({ value: p, label: PRIORITY_LABEL[p] }));

function DetailRow({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="grid grid-cols-[5rem_minmax(0,1fr)] items-center gap-2 text-xs">
      <span className="text-muted-foreground">{label}</span>
      <div className="min-w-0">{children}</div>
    </div>
  );
}

/** Stage, priority, labels and areas; each change saves at once. Shown in the ticket tab and the peek. */
export function TicketDetails({ ticket }: { ticket: Ticket }) {
  const qc = useQueryClient();
  const labels = useLabels(ticket.projectId);
  const [save, saving] = useAction((patch: Parameters<typeof patchTicket>[2]) => patchTicket(qc, ticket.id, patch));
  return (
    <OverviewSection title="Details">
      <DetailRow label="Stage">
        <Select aria-label="Stage" size="sm" options={stageOptions} value={ticket.stage} disabled={saving}
          onChange={(stage: TicketStage) => void save({ stage })} />
      </DetailRow>
      <DetailRow label="Priority">
        <Select aria-label="Priority" size="sm" options={priorityOptions} value={ticket.priority} disabled={saving}
          onChange={(priority: Priority) => void save({ priority })} />
      </DetailRow>
      <DetailRow label="Labels">
        <LabelsEditor labels={ticket.labels} suggestions={labels.data ?? []} onChange={(l) => void save({ labels: l })} />
      </DetailRow>
      {/* The parts of the project the ticket expects to touch: overlap is checked on the ones .pdt/project.toml declares. */}
      <DetailRow label="Areas">
        <div role="group" aria-label="Areas">
          <AreasEditor projectId={ticket.projectId} areas={ticket.areas} onChange={(areas) => void save({ areas })} />
        </div>
      </DetailRow>
    </OverviewSection>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Overlaps

/** Shared files listed before "and N more". */
export const OVERLAP_FILES_SHOWN = 5;

/** The files two tickets' attempts both changed: the first few, then "and N more" (of `total`). */
export function SharedFiles({ files, total, className }: { files: string[]; total: number; className?: string }) {
  if (files.length === 0) return null;
  const shown = files.slice(0, OVERLAP_FILES_SHOWN);
  const more = Math.max(total, files.length) - shown.length;
  return (
    <ul aria-label="Shared files" className={cn('flex min-w-0 flex-col gap-0.5 font-mono text-meta', className)}>
      {shown.map((f) => <li key={f} className="min-w-0 truncate" title={f}>{f}</li>)}
      {more > 0 && <li className="font-sans text-muted-foreground">and {more} more</li>}
    </ul>
  );
}

function OverlapRow({ ticket, overlap: o }: { ticket: Ticket; overlap: TicketOverlap }) {
  const open = useOpenTicket();
  const areas = useProjectAreas(ticket.projectId);
  const config = areas.data && !areas.data.error ? areas.data.areas : undefined;
  return (
    <li aria-label={`${o.key} ${o.title}`} className="flex min-w-0 flex-col gap-1 text-xs">
      <div className="flex min-w-0 items-center gap-2">
        <TicketKey ticketKey={o.key} title={o.title} onClick={() => open({ id: o.ticketId, projectId: ticket.projectId })} />
        <span className="min-w-0 flex-1 truncate">{o.title}</span>
        <Badge tone={o.level === 'conflict' ? 'failed' : 'waiting'}>{o.level === 'conflict' ? 'Conflict' : 'Overlap'}</Badge>
      </div>
      <div className="flex min-w-0 flex-wrap items-center gap-1 pl-1 text-meta text-muted-foreground">
        <span>{STAGE_LABEL[o.stage]}</span>
        {o.areas.length > 0 && <><span aria-hidden>·</span><AreaChips areas={o.areas} config={config} /></>}
      </div>
      <SharedFiles files={o.files} total={o.filesTotal} className="pl-1" />
    </li>
  );
}

/** The project's active tickets this one shares areas or changed files with, conflicts first (only when there are any). */
export function TicketOverlaps({ ticket }: { ticket: Ticket }) {
  if (ticket.overlaps.length === 0) return null;
  return (
    <OverviewSection title="Overlaps">
      <ul aria-label="Overlaps" className="flex flex-col gap-3">
        {ticket.overlaps.map((o) => <OverlapRow key={o.ticketId} ticket={ticket} overlap={o} />)}
      </ul>
    </OverviewSection>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Links

const ROLES: LinkRole[] = ['blockedBy', 'blocks', 'related'];

function LinkRow({ ticket, link }: { ticket: Ticket; link: LinkedTicket }) {
  const qc = useQueryClient();
  const open = useOpenTicket();
  const [remove] = useAction(async () => {
    const c = linkCall(ticket.id, link.id, linkRole(link));
    await api.unlinkTicket(c.from, { to: c.to, kind: c.kind });
    invalidateTickets(qc, [ticket.id, link.id]);
  });
  const openIt = () => open({ id: link.id, projectId: ticket.projectId });
  return (
    <li className="flex min-w-0 items-center gap-2 text-xs">
      <TicketKey ticketKey={link.key} title={link.title} onClick={openIt} />
      <span className={cn('min-w-0 flex-1 truncate', (link.stage === 'done' || link.stage === 'wont_do') && 'text-muted-foreground line-through')}>
        {link.title}
      </span>
      <Badge>{STAGE_LABEL[link.stage]}</Badge>
      <MenuButton
        label={`Link to ${link.key}`}
        size="sm"
        entries={() => [
          { label: 'Open', onSelect: openIt },
          { label: 'Remove link', danger: true, onSelect: () => void remove() },
        ]}
      />
    </li>
  );
}

/** Blocked by / Blocks / Related, each ticket with its key, title and stage; "Link ticket…" adds one. */
export function TicketLinks({ ticket }: { ticket: Ticket }) {
  const [linking, setLinking] = useState(false);
  const byRole = (r: LinkRole) => ticket.links.filter((l) => linkRole(l) === r);
  return (
    <OverviewSection
      title="Links"
      actions={
        <Button variant="ghost" size="sm" compact onClick={() => setLinking(true)}>
          <Link2 aria-hidden /> Link ticket…
        </Button>
      }
    >
      {ticket.links.length === 0 && <p className="text-xs text-muted-foreground">No links.</p>}
      {ROLES.map((r) => {
        const list = byRole(r);
        if (list.length === 0) return null;
        return (
          <div key={r} className="flex flex-col gap-1">
            <h4 className="text-meta text-muted-foreground">{LINK_ROLE_LABEL[r]}</h4>
            <ul aria-label={LINK_ROLE_LABEL[r]} className="flex flex-col gap-1">
              {list.map((l) => <LinkRow key={`${l.kind}:${l.direction}:${l.id}`} ticket={ticket} link={l} />)}
            </ul>
          </div>
        );
      })}
      {linking && <LinkTicketDialog ticket={ticket} onClose={() => setLinking(false)} />}
    </OverviewSection>
  );
}

/** Pick the kind (blocked by, blocks, related), then the ticket. A link that would close a cycle shows why here. */
export function LinkTicketDialog({ ticket, onClose }: { ticket: Ticket; onClose: () => void }) {
  const qc = useQueryClient();
  const [role, setRole] = useState<LinkRole>('blockedBy');
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const link = async (otherId: string) => {
    if (pending) return;
    setPending(true);
    setError(null);
    try {
      const c = linkCall(ticket.id, otherId, role);
      await api.linkTicket(c.from, { to: c.to, kind: c.kind });
      invalidateTickets(qc, [ticket.id, otherId]);
      onClose();
    } catch (e) {
      setError(actionError(e));
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()} title={`Link ${ticket.key} to…`} size="md">
      <div className="flex flex-col gap-2">
        <SegmentedControl
          aria-label="Link kind"
          size="sm"
          className="self-start"
          options={ROLES.map((r) => ({ value: r, label: LINK_ROLE_LABEL[r] }))}
          value={role}
          onChange={setRole}
        />
        <TicketPicker
          projectId={ticket.projectId}
          exclude={[ticket.id]}
          label="Ticket to link"
          onPick={(t) => void link(t.id)}
          className="max-h-[50dvh]"
        />
        {error && <Alert tone="error">{error}</Alert>}
      </div>
    </Dialog>
  );
}

// ---------------------------------------------------------------------------------------------------------------
// Timeline

const EVENT_PAGE = 50;

function iconFor(e: TicketEvent) {
  if (e.kind === 'comment') return <MessageSquare aria-hidden />;
  if (e.kind === 'moved') return <ArrowRight aria-hidden />;
  if (e.kind === 'linked' || e.kind === 'unlinked') return <Link2 aria-hidden />;
  if (e.kind === 'waiting') return <Clock aria-hidden />;
  if (e.kind === 'overlap') return <TriangleAlert aria-hidden />;
  return undefined;
}

function toItem(e: TicketEvent): TimelineItem {
  const icon = iconFor(e);
  return {
    id: e.id,
    actor: e.actor,
    time: e.createdAt,
    title: eventTitle(e),
    ...(icon ? { icon } : {}),
    ...(e.kind === 'comment' && e.body ? { body: <Markdown>{e.body}</Markdown> } : {}),
  };
}

/**
 * The ticket's activity, oldest first (newest last). Loads the newest page and "Show earlier" pages back; with
 * `recent` only the last few entries (the peek). `composer` adds a comment box under it.
 */
export function TicketTimeline({ ticketId, recent, composer = false }: { ticketId: string; recent?: number; composer?: boolean }) {
  const qc = useQueryClient();
  const full = useInfiniteQuery({
    queryKey: [...ticketKeys.events(ticketId), 'all'],
    queryFn: ({ pageParam }) => api.ticketEvents(ticketId, { newestFirst: true, limit: EVENT_PAGE, ...(pageParam ? { cursor: pageParam } : {}) }),
    initialPageParam: '' as string,
    getNextPageParam: (last) => last.nextCursor ?? undefined,
    refetchInterval: POLL,
    enabled: recent === undefined,
  });
  const short = useQuery({
    queryKey: [...ticketKeys.events(ticketId), 'recent', recent],
    queryFn: () => api.ticketEvents(ticketId, { newestFirst: true, limit: recent }),
    refetchInterval: POLL,
    enabled: recent !== undefined,
  });
  const newestFirst = recent === undefined ? full.data?.pages.flatMap((p) => p.items) ?? [] : short.data?.items ?? [];
  const items = [...newestFirst].reverse().map(toItem);
  const loading = recent === undefined ? full.isPending : short.isPending;
  return (
    <OverviewSection title="Activity">
      {recent === undefined && full.hasNextPage && (
        <Button variant="ghost" size="sm" compact className="self-start" pending={full.isFetchingNextPage} onClick={() => void full.fetchNextPage()}>
          Show earlier
        </Button>
      )}
      {loading ? <p className="text-xs text-muted-foreground">Loading…</p> : <Timeline items={items} aria-label="Activity" />}
      {composer && (
        <Composer
          label="Comment"
          sendLabel="Comment"
          submitOn="mod-enter"
          rows={2}
          autoFocus={false}
          placeholder="Add a comment (Markdown). ⌘/Ctrl+Enter sends."
          className="rounded-md border"
          onSend={async (text) => {
            await api.commentOnTicket(ticketId, text);
            void qc.invalidateQueries({ queryKey: ticketKeys.events(ticketId) });
          }}
        />
      )}
    </OverviewSection>
  );
}

/** Label chips, for headers and rows. */
export function LabelChips({ labels, className }: { labels: string[]; className?: string }) {
  if (labels.length === 0) return null;
  return (
    <span className={cn('flex min-w-0 items-center gap-1', className)}>
      {labels.map((l) => <Chip key={l}>{l}</Chip>)}
    </span>
  );
}
